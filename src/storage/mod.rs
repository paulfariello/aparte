/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
mod models;
mod schema;

use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm,
};
use anyhow::{anyhow, Error, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use async_trait::async_trait;
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use secrecy::{ExposeSecret, Secret};
use xmpp_parsers::jid::BareJid;

use crate::account::{Account, Password};

const NONCE_LEN: usize = 12;
const DEK_LEN: usize = 32;
const KDF_SALT_LEN: usize = 16;
const KDF_MEMORY: u32 = 65536;
const KDF_TIME: u32 = 3;
const KDF_PARALLELISM: u32 = 1;

#[derive(Debug)]
pub struct PasswordMismatch;

impl std::fmt::Display for PasswordMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "password mismatch: stored DEK cannot be decrypted with the provided password"
        )
    }
}

impl std::error::Error for PasswordMismatch {}

pub use models::{
    OmemoContactDevice, OmemoIdentity, OmemoOwnDevice, OmemoPreKey, OmemoSenderKey, OmemoSession,
    OmemoSignedPreKey,
};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

#[derive(Clone)]
pub struct Storage {
    pub(crate) pool: Pool<ConnectionManager<SqliteConnection>>,
    deks: Arc<Mutex<HashMap<String, Secret<Vec<u8>>>>>,
}

impl Storage {
    #[allow(clippy::unnecessary_debug_formatting)]
    pub fn new(path: PathBuf) -> Result<Self> {
        let path = path
            .into_os_string()
            .into_string()
            .map_err(|e| Error::msg(format!("Invalid path {e:?}")))?;
        let manager = ConnectionManager::<SqliteConnection>::new(path);
        let pool = Pool::builder().build(manager)?;

        let mut conn = pool.get()?;
        conn.run_pending_migrations(MIGRATIONS).unwrap();

        Ok(Self {
            pool,
            deks: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn derive_kek(password: &Password, salt: &[u8]) -> Result<[u8; DEK_LEN]> {
        let params = Params::new(KDF_MEMORY, KDF_TIME, KDF_PARALLELISM, Some(DEK_LEN))
            .map_err(|e| anyhow!("Invalid Argon2 params: {e}"))?;
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let mut kek = [0u8; DEK_LEN];
        argon2
            .hash_password_into(password.expose_secret().as_bytes(), salt, &mut kek)
            .map_err(|e| anyhow!("Argon2 KDF failed: {e}"))?;
        Ok(kek)
    }

    fn aes256gcm_encrypt(key: &[u8; DEK_LEN], plaintext: &[u8]) -> Result<Vec<u8>> {
        let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow!("AES key error: {e}"))?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| anyhow!("Encryption failed: {e}"))?;
        let mut out = nonce.to_vec();
        out.extend(ciphertext);
        Ok(out)
    }

    fn aes256gcm_decrypt(key: &[u8; DEK_LEN], data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < NONCE_LEN {
            return Err(anyhow!("Ciphertext too short"));
        }
        let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow!("AES key error: {e}"))?;
        let nonce = aes_gcm::Nonce::from_slice(&data[..NONCE_LEN]);
        cipher
            .decrypt(nonce, &data[NONCE_LEN..])
            .map_err(|_| anyhow!("Decryption failed"))
    }

    pub fn init_account_crypto(&mut self, account: &Account, password: &Password) -> Result<()> {
        use schema::account_crypto_config;
        let bare = account.to_bare().to_string();
        let mut conn = self.pool.get()?;

        let config: Option<models::AccountCryptoConfig> = account_crypto_config::table
            .filter(account_crypto_config::account.eq(&bare))
            .first(&mut conn)
            .optional()?;

        let dek: [u8; DEK_LEN] = if let Some(cfg) = config {
            let salt: [u8; KDF_SALT_LEN] = cfg
                .kdf_salt
                .as_slice()
                .try_into()
                .map_err(|_| anyhow!("Invalid salt length"))?;
            let kek = Self::derive_kek(password, &salt)?;
            let dek_bytes =
                Self::aes256gcm_decrypt(&kek, &cfg.wrapped_dek).map_err(|_| PasswordMismatch)?;
            dek_bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow!("Invalid DEK length in storage"))?
        } else {
            let dek: [u8; DEK_LEN] = rand::random();
            let salt: [u8; KDF_SALT_LEN] = rand::random();
            let kek = Self::derive_kek(password, &salt)?;
            let wrapped_dek = Self::aes256gcm_encrypt(&kek, &dek)?;
            diesel::insert_into(account_crypto_config::table)
                .values((
                    account_crypto_config::account.eq(&bare),
                    account_crypto_config::kdf_salt.eq(salt.as_slice()),
                    account_crypto_config::wrapped_dek.eq(&wrapped_dek),
                ))
                .execute(&mut conn)?;
            dek
        };

        self.deks
            .lock()
            .unwrap()
            .insert(bare, Secret::new(dek.to_vec()));
        Ok(())
    }

    pub fn rewrap_dek(&mut self, account: &Account, new_password: &Password) -> Result<()> {
        use schema::account_crypto_config;
        let bare = account.to_bare().to_string();

        let dek_bytes = self
            .deks
            .lock()
            .unwrap()
            .get(&bare)
            .map(|s| s.expose_secret().clone())
            .ok_or_else(|| {
                anyhow!("No DEK in memory for {bare}; call init_account_crypto first")
            })?;

        let salt: [u8; KDF_SALT_LEN] = rand::random();
        let kek = Self::derive_kek(new_password, &salt)?;
        let wrapped_dek = Self::aes256gcm_encrypt(&kek, &dek_bytes)?;

        let mut conn = self.pool.get()?;
        diesel::update(
            account_crypto_config::table.filter(account_crypto_config::account.eq(&bare)),
        )
        .set((
            account_crypto_config::kdf_salt.eq(salt.as_slice()),
            account_crypto_config::wrapped_dek.eq(&wrapped_dek),
        ))
        .execute(&mut conn)?;
        Ok(())
    }

    /// Re-wrap the DEK when a password change is detected: decrypt with the old password,
    /// re-encrypt with the new password, then cache the DEK for this session.
    pub fn rewrap_dek_with_old(
        &mut self,
        account: &BareJid,
        old_password: &Password,
        new_password: &Password,
    ) -> Result<()> {
        use schema::account_crypto_config;
        let bare = account.to_string();
        let mut conn = self.pool.get()?;

        let cfg: models::AccountCryptoConfig = account_crypto_config::table
            .filter(account_crypto_config::account.eq(&bare))
            .first(&mut conn)?;

        let old_salt: [u8; KDF_SALT_LEN] = cfg
            .kdf_salt
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("Invalid salt length"))?;
        let old_kek = Self::derive_kek(old_password, &old_salt)?;
        let dek_bytes =
            Self::aes256gcm_decrypt(&old_kek, &cfg.wrapped_dek).map_err(|_| PasswordMismatch)?;

        let new_salt: [u8; KDF_SALT_LEN] = rand::random();
        let new_kek = Self::derive_kek(new_password, &new_salt)?;
        let wrapped_dek = Self::aes256gcm_encrypt(&new_kek, &dek_bytes)?;

        diesel::update(
            account_crypto_config::table.filter(account_crypto_config::account.eq(&bare)),
        )
        .set((
            account_crypto_config::kdf_salt.eq(new_salt.as_slice()),
            account_crypto_config::wrapped_dek.eq(&wrapped_dek),
        ))
        .execute(&mut conn)?;

        let dek: [u8; DEK_LEN] = dek_bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("Invalid DEK length"))?;
        self.deks
            .lock()
            .unwrap()
            .insert(bare, Secret::new(dek.to_vec()));
        Ok(())
    }

    fn get_dek(&self, bare: &str) -> Result<[u8; DEK_LEN]> {
        let deks = self.deks.lock().unwrap();
        let bytes = deks
            .get(bare)
            .ok_or_else(|| anyhow!("No DEK for account {bare}; was init_account_crypto called?"))?
            .expose_secret();
        bytes
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("Corrupt DEK length"))
    }

    pub fn get_omemo_own_device(&self, account: &Account) -> Result<Option<OmemoOwnDevice>> {
        use schema::omemo_own_device;
        let mut conn = self.pool.get()?;
        let res = omemo_own_device::table
            .filter(omemo_own_device::account.eq(account.to_string()))
            .first(&mut conn)
            .optional()?;
        Ok(res)
    }

    pub fn set_omemo_current_device(
        &mut self,
        account: &Account,
        device_id: u32,
        identity_key_pair: Vec<u8>,
    ) -> Result<OmemoOwnDevice> {
        use schema::omemo_own_device;
        let mut conn = self.pool.get()?;
        let device = diesel::insert_into(omemo_own_device::table)
            .values((
                omemo_own_device::account.eq(account.to_string()),
                omemo_own_device::id.eq::<i64>(device_id.into()),
                omemo_own_device::identity.eq(Some(identity_key_pair)),
            ))
            .get_result(&mut conn)?;
        Ok(device)
    }

    pub fn upsert_omemo_contact_device(
        &mut self,
        account: &Account,
        contact: &BareJid,
        device_id: u32,
    ) -> Result<OmemoContactDevice> {
        use schema::omemo_contact_device;
        let mut conn = self.pool.get()?;
        let result = diesel::insert_into(omemo_contact_device::table)
            .values((
                omemo_contact_device::account.eq(account.to_string()),
                omemo_contact_device::contact.eq(contact.to_string()),
                omemo_contact_device::id.eq::<i64>(device_id.into()),
            ))
            .on_conflict((
                omemo_contact_device::account,
                omemo_contact_device::contact,
                omemo_contact_device::id,
            ))
            .do_nothing()
            .get_result(&mut conn)
            .optional()?;

        let device = match result {
            Some(device) => device,
            None => omemo_contact_device::table
                .filter(omemo_contact_device::account.eq(account.to_string()))
                .filter(omemo_contact_device::contact.eq(contact.to_string()))
                .filter(omemo_contact_device::id.eq::<i64>(device_id.into()))
                .first(&mut conn)?,
        };

        Ok(device)
    }

    pub fn get_all_omemo_contacts(&self, account: &Account) -> Result<Vec<BareJid>> {
        use schema::omemo_contact_device;
        let mut conn = self.pool.get()?;

        Ok(omemo_contact_device::table
            .group_by(omemo_contact_device::contact)
            .select(omemo_contact_device::contact)
            .filter(omemo_contact_device::account.eq(account.to_string()))
            .get_results::<String>(&mut conn)?
            .iter()
            .filter_map(|contact| BareJid::from_str(contact).ok())
            .collect())
    }

    pub fn add_omemo_muc_room(&self, account: &Account, room: &BareJid) -> Result<()> {
        use schema::omemo_muc_room;
        let mut conn = self.pool.get()?;
        diesel::insert_into(omemo_muc_room::table)
            .values((
                omemo_muc_room::account.eq(account.to_string()),
                omemo_muc_room::room.eq(room.to_string()),
            ))
            .on_conflict((omemo_muc_room::account, omemo_muc_room::room))
            .do_nothing()
            .execute(&mut conn)?;
        Ok(())
    }

    pub fn remove_omemo_muc_room(&self, account: &Account, room: &BareJid) -> Result<()> {
        use schema::omemo_muc_room;
        let mut conn = self.pool.get()?;
        diesel::delete(
            omemo_muc_room::table
                .filter(omemo_muc_room::account.eq(account.to_string()))
                .filter(omemo_muc_room::room.eq(room.to_string())),
        )
        .execute(&mut conn)?;
        Ok(())
    }

    pub fn get_omemo_muc_rooms(&self, account: &Account) -> Result<Vec<BareJid>> {
        use schema::omemo_muc_room;
        let mut conn = self.pool.get()?;
        Ok(omemo_muc_room::table
            .filter(omemo_muc_room::account.eq(account.to_string()))
            .select(omemo_muc_room::room)
            .get_results::<String>(&mut conn)?
            .iter()
            .filter_map(|room| BareJid::from_str(room).ok())
            .collect())
    }

    pub fn get_omemo_contact_devices(
        &self,
        account: &Account,
        contact: &BareJid,
    ) -> Result<Vec<OmemoContactDevice>> {
        use schema::omemo_contact_device;
        let mut conn = self.pool.get()?;

        Ok(omemo_contact_device::table
            .filter(omemo_contact_device::account.eq(account.to_string()))
            .filter(omemo_contact_device::contact.eq(contact.to_string()))
            .get_results(&mut conn)?)
    }

    pub fn get_omemo_contact_identities(
        &self,
        account: &Account,
        contact: &BareJid,
    ) -> Result<Vec<libsignal_protocol::IdentityKey>> {
        use schema::omemo_identity;
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        Ok(omemo_identity::table
            .filter(omemo_identity::account.eq(account.to_string()))
            .filter(omemo_identity::user_id.eq(contact.to_string()))
            .get_results(&mut conn)?
            .into_iter()
            .filter_map(|identity: OmemoIdentity| {
                libsignal_protocol::IdentityKey::decode(&identity.identity).ok()
            })
            .collect())
    }

    pub fn get_omemo_identity_key_pair(
        &self,
        account: &Account,
    ) -> Result<libsignal_protocol::IdentityKeyPair> {
        log::debug!("Get own identity key pair");
        let identity = self
            .get_omemo_own_device(account)?
            .and_then(|device| device.identity)
            .ok_or(anyhow!("Missing own device identity"))?;
        Ok(libsignal_protocol::IdentityKeyPair::try_from(
            identity.as_ref(),
        )?)
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn get_omemo_local_registration_id(&self, account: &Account) -> Result<u32> {
        log::debug!("Get local registration id");
        self.get_omemo_own_device(account)?
            .map(|device| device.id as u32)
            .ok_or(anyhow!("Missing own device"))
    }

    pub fn save_omemo_identity(
        &mut self,
        account: &Account,
        address: &libsignal_protocol::ProtocolAddress,
        identity: &libsignal_protocol::IdentityKey,
    ) -> Result<bool> {
        use schema::omemo_identity;
        log::debug!("Save {address}'s identity");
        // The return value represents whether an existing identity was replaced (`Ok(true)`). If it is
        // new or hasn't changed, the return value should be `Ok(false)`.
        let ret = if let Some(stored) = self.get_omemo_identity(account, address)? {
            &stored != identity
        } else {
            false
        };
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;
        diesel::insert_into(omemo_identity::table)
            .values((
                omemo_identity::account.eq(account.to_string()),
                omemo_identity::user_id.eq(address.name()),
                omemo_identity::device_id.eq(i64::from(u32::from(address.device_id()))),
                omemo_identity::identity.eq(identity.serialize().to_vec()),
            ))
            .on_conflict((
                omemo_identity::account,
                omemo_identity::user_id,
                omemo_identity::device_id,
            ))
            .do_update()
            .set(omemo_identity::identity.eq(identity.serialize().to_vec()))
            .execute(&mut conn)?;

        Ok(ret)
    }

    pub fn is_omemo_trusted_identity(
        &self,
        account: &Account,
        address: &libsignal_protocol::ProtocolAddress,
        identity: &libsignal_protocol::IdentityKey,
        _direction: libsignal_protocol::Direction,
    ) -> Result<bool> {
        log::debug!("Is {address}'s identity trusted?");
        Ok(match self.get_omemo_identity(account, address)? {
            Some(stored) => &stored == identity,
            // TOFU: no stored identity means first contact with this device — trust it.
            None => true,
        })
    }

    pub fn get_omemo_identity(
        &self,
        account: &Account,
        address: &libsignal_protocol::ProtocolAddress,
    ) -> Result<Option<libsignal_protocol::IdentityKey>> {
        use schema::omemo_identity;
        log::debug!("Get {address}'s identity");
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        Ok(omemo_identity::table
            .filter(omemo_identity::account.eq(account.to_string()))
            .filter(omemo_identity::user_id.eq(address.name()))
            .filter(omemo_identity::device_id.eq(i64::from(u32::from(address.device_id()))))
            .first(&mut conn)
            .optional()?
            .map(|identity: OmemoIdentity| {
                libsignal_protocol::IdentityKey::decode(&identity.identity)
            })
            .transpose()?)
    }

    pub fn load_omemo_session(
        &self,
        account: &Account,
        address: &libsignal_protocol::ProtocolAddress,
    ) -> Result<Option<libsignal_protocol::SessionRecord>> {
        use schema::omemo_session;
        log::debug!("Load session for {address}");
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        Ok(omemo_session::table
            .filter(omemo_session::account.eq(account.to_string()))
            .filter(omemo_session::user_id.eq(address.name()))
            .filter(omemo_session::device_id.eq(i64::from(u32::from(address.device_id()))))
            .first(&mut conn)
            .optional()?
            .map(|session: OmemoSession| {
                libsignal_protocol::SessionRecord::deserialize(&session.session)
            })
            .transpose()?)
    }

    pub fn store_omemo_session(
        &mut self,
        account: &Account,
        address: &libsignal_protocol::ProtocolAddress,
        session: &libsignal_protocol::SessionRecord,
    ) -> Result<()> {
        use schema::omemo_session;
        log::debug!("Store session for {address}");
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;
        diesel::insert_into(omemo_session::table)
            .values((
                omemo_session::account.eq(account.to_string()),
                omemo_session::user_id.eq(address.name()),
                omemo_session::device_id.eq(i64::from(u32::from(address.device_id()))),
                omemo_session::session.eq(session.serialize()?.clone()),
            ))
            .on_conflict((
                omemo_session::account,
                omemo_session::user_id,
                omemo_session::device_id,
            ))
            .do_update()
            .set(omemo_session::session.eq(session.serialize()?.clone()))
            .execute(&mut conn)
            .map_err(signal_storage_display_error())?;

        Ok(())
    }

    pub fn get_omemo_pre_key(
        &self,
        account: &Account,
        pre_key_id: libsignal_protocol::PreKeyId,
    ) -> Result<libsignal_protocol::PreKeyRecord> {
        use schema::omemo_pre_key;
        log::debug!("Get pre key {pre_key_id}");
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        Ok(omemo_pre_key::table
            .filter(omemo_pre_key::account.eq(account.to_string()))
            .filter(omemo_pre_key::pre_key_id.eq(i64::from(u32::from(pre_key_id))))
            .first(&mut conn)
            .optional()?
            .ok_or_else(signal_storage_empty_error("PreKey not found"))
            .map(|pre_key: OmemoPreKey| {
                libsignal_protocol::PreKeyRecord::deserialize(&pre_key.pre_key)
            })??)
    }

    pub fn get_all_omemo_pre_key(
        &self,
        account: &Account,
    ) -> Result<Vec<libsignal_protocol::PreKeyRecord>> {
        use schema::omemo_pre_key;
        log::debug!("Get all pre key");
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        Ok(omemo_pre_key::table
            .filter(omemo_pre_key::account.eq(account.to_string()))
            .get_results(&mut conn)?
            .into_iter()
            .filter_map(|pre_key: OmemoPreKey| {
                libsignal_protocol::PreKeyRecord::deserialize(&pre_key.pre_key).ok()
            })
            .collect())
    }

    pub fn save_omemo_pre_key(
        &mut self,
        account: &Account,
        pre_key_id: libsignal_protocol::PreKeyId,
        pre_key: &libsignal_protocol::PreKeyRecord,
    ) -> Result<()> {
        use schema::omemo_pre_key;
        log::debug!("Save pre key {pre_key_id}");
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;
        diesel::insert_into(omemo_pre_key::table)
            .values((
                omemo_pre_key::account.eq(account.to_string()),
                omemo_pre_key::pre_key_id.eq(i64::from(u32::from(pre_key_id))),
                omemo_pre_key::pre_key.eq(pre_key.serialize()?.clone()),
            ))
            .on_conflict((omemo_pre_key::account, omemo_pre_key::pre_key_id))
            .do_update()
            .set(omemo_pre_key::pre_key.eq(pre_key.serialize()?.clone()))
            .execute(&mut conn)
            .map_err(signal_storage_display_error())?;

        Ok(())
    }

    pub fn remove_omemo_pre_key(
        &mut self,
        account: &Account,
        pre_key_id: libsignal_protocol::PreKeyId,
    ) -> Result<()> {
        use schema::omemo_pre_key;
        log::debug!("Remove pre key {pre_key_id}");
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        diesel::delete(
            omemo_pre_key::table
                .filter(omemo_pre_key::account.eq(account.to_string()))
                .filter(omemo_pre_key::pre_key_id.eq(i64::from(u32::from(pre_key_id)))),
        )
        .execute(&mut conn)
        .map_err(signal_storage_display_error())?;

        Ok(())
    }

    pub fn get_omemo_signed_pre_key(
        &self,
        account: &Account,
        signed_pre_key_id: libsignal_protocol::SignedPreKeyId,
    ) -> Result<libsignal_protocol::SignedPreKeyRecord> {
        use schema::omemo_signed_pre_key;
        log::debug!("Get signed pre key {signed_pre_key_id}");
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        Ok(omemo_signed_pre_key::table
            .filter(omemo_signed_pre_key::account.eq(account.to_string()))
            .filter(
                omemo_signed_pre_key::signed_pre_key_id.eq(i64::from(u32::from(signed_pre_key_id))),
            )
            .first(&mut conn)
            .optional()
            .map_err(signal_storage_display_error())?
            .ok_or_else(signal_storage_empty_error("PreKey not found"))
            .map(|signed_pre_key: OmemoSignedPreKey| {
                libsignal_protocol::SignedPreKeyRecord::deserialize(&signed_pre_key.signed_pre_key)
            })??)
    }

    pub fn save_omemo_signed_pre_key(
        &mut self,
        account: &Account,
        signed_pre_key_id: libsignal_protocol::SignedPreKeyId,
        signed_pre_key: &libsignal_protocol::SignedPreKeyRecord,
    ) -> Result<()> {
        use schema::omemo_signed_pre_key;
        log::debug!("Save signed pre key {signed_pre_key_id}");
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;
        diesel::insert_into(omemo_signed_pre_key::table)
            .values((
                omemo_signed_pre_key::account.eq(account.to_string()),
                omemo_signed_pre_key::signed_pre_key_id.eq(i64::from(u32::from(signed_pre_key_id))),
                omemo_signed_pre_key::signed_pre_key.eq(signed_pre_key.serialize()?.clone()),
            ))
            .on_conflict((
                omemo_signed_pre_key::account,
                omemo_signed_pre_key::signed_pre_key_id,
            ))
            .do_update()
            .set(omemo_signed_pre_key::signed_pre_key.eq(signed_pre_key.serialize()?.clone()))
            .execute(&mut conn)
            .map_err(signal_storage_display_error())?;

        Ok(())
    }

    pub fn store_omemo_sender_key(
        &mut self,
        account: &Account,
        sender: &libsignal_protocol::ProtocolAddress,
        distribution_id: uuid::Uuid,
        sender_key: &libsignal_protocol::SenderKeyRecord,
    ) -> Result<()> {
        use schema::omemo_sender_key;
        log::debug!("Store sender key {sender}");
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;
        diesel::insert_into(omemo_sender_key::table)
            .values((
                omemo_sender_key::account.eq(account.to_string()),
                omemo_sender_key::sender_id.eq(sender.name()),
                omemo_sender_key::device_id.eq(i64::from(u32::from(sender.device_id()))),
                omemo_sender_key::distribution_id.eq(distribution_id.as_bytes().to_vec()),
                omemo_sender_key::sender_key.eq(sender_key.serialize()?.clone()),
            ))
            .on_conflict((
                omemo_sender_key::account,
                omemo_sender_key::sender_id,
                omemo_sender_key::device_id,
                omemo_sender_key::distribution_id,
            ))
            .do_update()
            .set(omemo_sender_key::sender_key.eq(sender_key.serialize()?.clone()))
            .execute(&mut conn)
            .map_err(signal_storage_display_error())?;

        Ok(())
    }

    pub fn load_omemo_sender_key(
        &mut self,
        account: &Account,
        sender: &libsignal_protocol::ProtocolAddress,
        distribution_id: uuid::Uuid,
    ) -> Result<Option<libsignal_protocol::SenderKeyRecord>> {
        use schema::omemo_sender_key;
        log::debug!("Load sender key {sender}");
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        Ok(omemo_sender_key::table
            .filter(omemo_sender_key::account.eq(account.to_string()))
            .filter(omemo_sender_key::sender_id.eq(sender.name()))
            .filter(omemo_sender_key::device_id.eq(i64::from(u32::from(sender.device_id()))))
            .filter(omemo_sender_key::distribution_id.eq(distribution_id.as_bytes().to_vec()))
            .first(&mut conn)
            .optional()
            .map_err(signal_storage_display_error())?
            .map(|sender_key: OmemoSenderKey| {
                libsignal_protocol::SenderKeyRecord::deserialize(&sender_key.sender_key)
            })
            .transpose()?)
    }

    pub fn save_message_cleartext(
        &mut self,
        account: &Account,
        message_id: &str,
        conversation_jid: &str,
        body: &str,
        from_jid: &str,
        timestamp: &str,
        encrypted: bool,
    ) -> Result<()> {
        use schema::archives;
        let bare = account.to_bare().to_string();
        let dek = self.get_dek(&bare)?;
        let body_enc = Self::aes256gcm_encrypt(&dek, body.as_bytes())?;
        let mut conn = self.pool.get()?;
        diesel::insert_into(archives::table)
            .values((
                archives::account.eq(&bare),
                archives::message_id.eq(message_id),
                archives::conversation_jid.eq(conversation_jid),
                archives::body_enc.eq(&body_enc),
                archives::from_jid.eq(from_jid),
                archives::timestamp.eq(timestamp),
                archives::encrypted.eq(encrypted),
            ))
            .on_conflict((archives::account, archives::message_id))
            .do_update()
            .set(archives::body_enc.eq(&body_enc))
            .execute(&mut conn)?;
        Ok(())
    }

    pub fn get_message_cleartext(
        &self,
        account: &Account,
        message_id: &str,
    ) -> Result<Option<String>> {
        use schema::archives;
        let bare = account.to_bare().to_string();
        let dek = self.get_dek(&bare)?;
        let mut conn = self.pool.get()?;
        let maybe_blob: Option<Vec<u8>> = archives::table
            .filter(archives::account.eq(&bare))
            .filter(archives::message_id.eq(message_id))
            .select(archives::body_enc)
            .first(&mut conn)
            .optional()?;
        match maybe_blob {
            None => Ok(None),
            Some(blob) => {
                let plaintext = Self::aes256gcm_decrypt(&dek, &blob)?;
                Ok(Some(String::from_utf8(plaintext)?))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn make_storage() -> (Storage, tempfile::NamedTempFile) {
        let file = tempfile::NamedTempFile::new().unwrap();
        let storage = Storage::new(file.path().to_path_buf()).unwrap();
        (storage, file)
    }

    fn password(s: &str) -> Password {
        Secret::new(s.to_string())
    }

    /// Encrypted message saved under one resource must be decryptable when looked up with a
    /// different resource for the same bare JID.
    #[test]
    fn cleartext_lookup_succeeds_across_resources() {
        let (mut storage, _file) = make_storage();

        let account1 = Account::from_str("alice@example.org/aparte_AAAAA").unwrap();
        let account2 = Account::from_str("alice@example.org/aparte_BBBBB").unwrap();
        let pw = password("hunter2");

        storage.init_account_crypto(&account1, &pw).unwrap();

        storage
            .save_message_cleartext(
                &account1,
                "msg-uuid-x1",
                "bob@example.org",
                "Hello across resources",
                "alice@example.org",
                "2024-01-01T00:00:00Z",
                true,
            )
            .unwrap();

        let result = storage
            .get_message_cleartext(&account2, "msg-uuid-x1")
            .unwrap();
        assert_eq!(
            result.as_deref(),
            Some("Hello across resources"),
            "cleartext stored in one session must be found in a new session with a different resource"
        );
    }

    #[test]
    fn crypto_roundtrip() {
        let (mut storage, _file) = make_storage();
        let account = Account::from_str("alice@example.org/aparte_AAAAA").unwrap();
        let pw = password("correcthorsebatterystaple");

        storage.init_account_crypto(&account, &pw).unwrap();
        storage
            .save_message_cleartext(
                &account,
                "msg-1",
                "bob@example.org",
                "Secret message",
                "alice@example.org",
                "2025-01-01T00:00:00Z",
                true,
            )
            .unwrap();

        let body = storage.get_message_cleartext(&account, "msg-1").unwrap();
        assert_eq!(body.as_deref(), Some("Secret message"));
    }

    #[test]
    fn rewrap_dek_allows_new_password() {
        let (mut storage, _file) = make_storage();
        let account = Account::from_str("alice@example.org/aparte_AAAAA").unwrap();
        let pw_a = password("old_password");
        let pw_b = password("new_password");

        // First session: init + save with password A
        storage.init_account_crypto(&account, &pw_a).unwrap();
        storage
            .save_message_cleartext(
                &account,
                "msg-rekey",
                "bob@example.org",
                "Rekeyed body",
                "alice@example.org",
                "2025-01-01T00:00:00Z",
                true,
            )
            .unwrap();

        // Simulate password change
        storage.rewrap_dek(&account, &pw_b).unwrap();

        // Second session: fresh storage, init with password B
        let path = {
            // We need to reopen with a new storage instance using the same DB file.
            // Clone the pool to simulate "same file but fresh in-memory state".
            let mut storage2 = Storage {
                pool: storage.pool.clone(),
                deks: Arc::new(Mutex::new(HashMap::new())),
            };
            storage2.init_account_crypto(&account, &pw_b).unwrap();
            let body = storage2
                .get_message_cleartext(&account, "msg-rekey")
                .unwrap();
            assert_eq!(body.as_deref(), Some("Rekeyed body"));
        };
        let _ = path;
    }

    #[test]
    fn wrong_password_returns_mismatch() {
        let (mut storage, _file) = make_storage();
        let account = Account::from_str("alice@example.org/aparte_AAAAA").unwrap();
        let pw_a = password("correct");
        let pw_b = password("wrong");

        storage.init_account_crypto(&account, &pw_a).unwrap();

        // Simulate new session with wrong password
        let mut storage2 = Storage {
            pool: storage.pool.clone(),
            deks: Arc::new(Mutex::new(HashMap::new())),
        };
        let err = storage2.init_account_crypto(&account, &pw_b).unwrap_err();
        assert!(
            err.downcast_ref::<PasswordMismatch>().is_some(),
            "expected PasswordMismatch, got: {err}"
        );
    }
}

fn signal_storage_error<T>(
    str: &'static str,
) -> impl Fn(T) -> libsignal_protocol::error::SignalProtocolError
where
    T: std::error::Error + Send + Sync + std::panic::UnwindSafe + 'static,
{
    move |e: T| {
        libsignal_protocol::error::SignalProtocolError::ApplicationCallbackError(str, Box::new(e))
    }
}

fn signal_storage_display_error<T>() -> impl Fn(T) -> libsignal_protocol::error::SignalProtocolError
where
    T: std::fmt::Display,
{
    move |e: T| {
        libsignal_protocol::error::SignalProtocolError::ApplicationCallbackError(
            "Storage Error",
            Box::new(UnwindSafeResultError(format!("{e}"))),
        )
    }
}

fn signal_storage_empty_error(
    str: &'static str,
) -> impl Fn() -> libsignal_protocol::error::SignalProtocolError {
    move || {
        libsignal_protocol::error::SignalProtocolError::ApplicationCallbackError(
            str,
            Box::new(UnwindSafeResultError(String::new())),
        )
    }
}

#[derive(Debug)]
struct UnwindSafeResultError(String);

impl std::fmt::Display for UnwindSafeResultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for UnwindSafeResultError {}

#[derive(Clone)]
pub struct SignalStorage {
    pub account: Account,
    pub storage: Storage,
    pub deleted_pre_keys: Arc<AtomicBool>,
}

impl SignalStorage {
    pub fn new(account: Account, storage: Storage) -> Self {
        Self {
            account,
            storage,
            deleted_pre_keys: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[async_trait(?Send)]
impl libsignal_protocol::IdentityKeyStore for SignalStorage {
    async fn get_identity_key_pair(
        &self,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<libsignal_protocol::IdentityKeyPair> {
        self.storage
            .get_omemo_identity_key_pair(&self.account)
            .map_err(signal_storage_display_error())
    }

    async fn get_local_registration_id(
        &self,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<u32> {
        log::debug!("Get local registration id");
        self.storage
            .get_omemo_local_registration_id(&self.account)
            .map_err(signal_storage_display_error())
    }

    async fn save_identity(
        &mut self,
        address: &libsignal_protocol::ProtocolAddress,
        identity: &libsignal_protocol::IdentityKey,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<bool> {
        self.storage
            .save_omemo_identity(&self.account, address, identity)
            .map_err(signal_storage_display_error())
    }

    async fn is_trusted_identity(
        &self,
        address: &libsignal_protocol::ProtocolAddress,
        identity: &libsignal_protocol::IdentityKey,
        _direction: libsignal_protocol::Direction,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<bool> {
        self.storage
            .is_omemo_trusted_identity(&self.account, address, identity, _direction)
            .map_err(signal_storage_display_error())
    }

    async fn get_identity(
        &self,
        address: &libsignal_protocol::ProtocolAddress,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<Option<libsignal_protocol::IdentityKey>> {
        self.storage
            .get_omemo_identity(&self.account, address)
            .map_err(signal_storage_display_error())
    }
}

#[async_trait(?Send)]
impl libsignal_protocol::SessionStore for SignalStorage {
    async fn load_session(
        &self,
        address: &libsignal_protocol::ProtocolAddress,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<Option<libsignal_protocol::SessionRecord>> {
        self.storage
            .load_omemo_session(&self.account, address)
            .map_err(signal_storage_display_error())
    }

    async fn store_session(
        &mut self,
        address: &libsignal_protocol::ProtocolAddress,
        session: &libsignal_protocol::SessionRecord,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<()> {
        self.storage
            .store_omemo_session(&self.account, address, session)
            .map_err(signal_storage_display_error())
    }
}

#[async_trait(?Send)]
impl libsignal_protocol::PreKeyStore for SignalStorage {
    async fn get_pre_key(
        &self,
        pre_key_id: libsignal_protocol::PreKeyId,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<libsignal_protocol::PreKeyRecord> {
        self.storage
            .get_omemo_pre_key(&self.account, pre_key_id)
            .map_err(signal_storage_display_error())
    }

    async fn save_pre_key(
        &mut self,
        pre_key_id: libsignal_protocol::PreKeyId,
        pre_key: &libsignal_protocol::PreKeyRecord,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<()> {
        self.storage
            .save_omemo_pre_key(&self.account, pre_key_id, pre_key)
            .map_err(signal_storage_display_error())
    }

    async fn remove_pre_key(
        &mut self,
        pre_key_id: libsignal_protocol::PreKeyId,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<()> {
        self.storage
            .remove_omemo_pre_key(&self.account, pre_key_id)
            .map_err(signal_storage_display_error())?;

        self.deleted_pre_keys
            .store(true, std::sync::atomic::Ordering::Relaxed);

        Ok(())
    }
}

#[async_trait(?Send)]
impl libsignal_protocol::SignedPreKeyStore for SignalStorage {
    async fn get_signed_pre_key(
        &self,
        signed_pre_key_id: libsignal_protocol::SignedPreKeyId,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<libsignal_protocol::SignedPreKeyRecord> {
        self.storage
            .get_omemo_signed_pre_key(&self.account, signed_pre_key_id)
            .map_err(signal_storage_display_error())
    }

    async fn save_signed_pre_key(
        &mut self,
        signed_pre_key_id: libsignal_protocol::SignedPreKeyId,
        signed_pre_key: &libsignal_protocol::SignedPreKeyRecord,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<()> {
        self.storage
            .save_omemo_signed_pre_key(&self.account, signed_pre_key_id, signed_pre_key)
            .map_err(signal_storage_display_error())
    }
}

#[async_trait(?Send)]
impl libsignal_protocol::SenderKeyStore for SignalStorage {
    async fn store_sender_key(
        &mut self,
        sender: &libsignal_protocol::ProtocolAddress,
        distribution_id: uuid::Uuid,
        sender_key: &libsignal_protocol::SenderKeyRecord,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<()> {
        self.storage
            .store_omemo_sender_key(&self.account, sender, distribution_id, sender_key)
            .map_err(signal_storage_display_error())
    }

    async fn load_sender_key(
        &mut self,
        sender: &libsignal_protocol::ProtocolAddress,
        distribution_id: uuid::Uuid,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<Option<libsignal_protocol::SenderKeyRecord>> {
        self.storage
            .load_omemo_sender_key(&self.account, sender, distribution_id)
            .map_err(signal_storage_display_error())
    }
}
