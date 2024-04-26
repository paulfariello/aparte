/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
mod models;
mod schema;

use std::convert::TryFrom;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{anyhow, Error, Result};
use async_trait::async_trait;
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use xmpp_parsers::BareJid;

use crate::account::Account;

pub use models::{
    OmemoContactDevice, OmemoIdentity, OmemoOwnDevice, OmemoPreKey, OmemoSenderKey, OmemoSession,
    OmemoSignedPreKey,
};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

#[derive(Clone)]
pub struct Storage {
    pub(crate) pool: Pool<ConnectionManager<SqliteConnection>>,
}

impl Storage {
    pub fn new(path: PathBuf) -> Result<Self> {
        let path = path
            .into_os_string()
            .into_string()
            .map_err(|e| Error::msg(format!("Invalid path {e:?}")))?;
        let manager = ConnectionManager::<SqliteConnection>::new(path);
        let pool = Pool::builder().build(manager)?;

        let mut conn = pool.get()?;
        conn.run_pending_migrations(MIGRATIONS).unwrap();

        Ok(Self { pool })
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
        log::debug!("Save {address}'s identity");
        // The return value represents whether an existing identity was replaced (`Ok(true)`). If it is
        // new or hasn't changed, the return value should be `Ok(false)`.
        let ret = if let Some(stored) = self.get_omemo_identity(account, address)? {
            &stored != identity
        } else {
            false
        };

        use schema::omemo_identity;
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;
        diesel::insert_into(omemo_identity::table)
            .values((
                omemo_identity::account.eq(account.to_string()),
                omemo_identity::user_id.eq(address.name()),
                omemo_identity::device_id.eq(u32::from(address.device_id()) as i64),
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
            _ => false,
        })
    }

    pub fn get_omemo_identity(
        &self,
        account: &Account,
        address: &libsignal_protocol::ProtocolAddress,
    ) -> Result<Option<libsignal_protocol::IdentityKey>> {
        log::debug!("Get {address}'s identity");
        use schema::omemo_identity;
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        Ok(omemo_identity::table
            .filter(omemo_identity::account.eq(account.to_string()))
            .filter(omemo_identity::user_id.eq(address.name()))
            .filter(omemo_identity::device_id.eq(u32::from(address.device_id()) as i64))
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
        log::debug!("Load session for {address}");
        use schema::omemo_session;
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        Ok(omemo_session::table
            .filter(omemo_session::account.eq(account.to_string()))
            .filter(omemo_session::user_id.eq(address.name()))
            .filter(omemo_session::device_id.eq(u32::from(address.device_id()) as i64))
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
        log::debug!("Store session for {address}");
        use schema::omemo_session;
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;
        diesel::insert_into(omemo_session::table)
            .values((
                omemo_session::account.eq(account.to_string()),
                omemo_session::user_id.eq(address.name()),
                omemo_session::device_id.eq(u32::from(address.device_id()) as i64),
                omemo_session::session.eq(session.serialize()?.to_vec()),
            ))
            .on_conflict((
                omemo_session::account,
                omemo_session::user_id,
                omemo_session::device_id,
            ))
            .do_update()
            .set(omemo_session::session.eq(session.serialize()?.to_vec()))
            .execute(&mut conn)
            .map_err(signal_storage_display_error())?;

        Ok(())
    }

    pub fn get_omemo_pre_key(
        &self,
        account: &Account,
        pre_key_id: libsignal_protocol::PreKeyId,
    ) -> Result<libsignal_protocol::PreKeyRecord> {
        log::debug!("Get pre key {pre_key_id}");
        use schema::omemo_pre_key;
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        Ok(omemo_pre_key::table
            .filter(omemo_pre_key::account.eq(account.to_string()))
            .filter(omemo_pre_key::pre_key_id.eq(u32::from(pre_key_id) as i64))
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
        log::debug!("Get all pre key");
        use schema::omemo_pre_key;
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
        log::debug!("Save pre key {pre_key_id}");
        use schema::omemo_pre_key;
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;
        diesel::insert_into(omemo_pre_key::table)
            .values((
                omemo_pre_key::account.eq(account.to_string()),
                omemo_pre_key::pre_key_id.eq(u32::from(pre_key_id) as i64),
                omemo_pre_key::pre_key.eq(pre_key.serialize()?.to_vec()),
            ))
            .on_conflict((omemo_pre_key::account, omemo_pre_key::pre_key_id))
            .do_update()
            .set(omemo_pre_key::pre_key.eq(pre_key.serialize()?.to_vec()))
            .execute(&mut conn)
            .map_err(signal_storage_display_error())?;

        Ok(())
    }

    pub fn remove_omemo_pre_key(
        &mut self,
        account: &Account,
        pre_key_id: libsignal_protocol::PreKeyId,
    ) -> Result<()> {
        log::debug!("Remove pre key {pre_key_id}");
        use schema::omemo_pre_key;
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        diesel::delete(
            omemo_pre_key::table
                .filter(omemo_pre_key::account.eq(account.to_string()))
                .filter(omemo_pre_key::pre_key_id.eq(u32::from(pre_key_id) as i64)),
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
        log::debug!("Get signed pre key {signed_pre_key_id}");
        use schema::omemo_signed_pre_key;
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        Ok(omemo_signed_pre_key::table
            .filter(omemo_signed_pre_key::account.eq(account.to_string()))
            .filter(omemo_signed_pre_key::signed_pre_key_id.eq(u32::from(signed_pre_key_id) as i64))
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
        log::debug!("Save signed pre key {signed_pre_key_id}");
        use schema::omemo_signed_pre_key;
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;
        diesel::insert_into(omemo_signed_pre_key::table)
            .values((
                omemo_signed_pre_key::account.eq(account.to_string()),
                omemo_signed_pre_key::signed_pre_key_id.eq(u32::from(signed_pre_key_id) as i64),
                omemo_signed_pre_key::signed_pre_key.eq(signed_pre_key.serialize()?.to_vec()),
            ))
            .on_conflict((
                omemo_signed_pre_key::account,
                omemo_signed_pre_key::signed_pre_key_id,
            ))
            .do_update()
            .set(omemo_signed_pre_key::signed_pre_key.eq(signed_pre_key.serialize()?.to_vec()))
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
        log::debug!("Store sender key {sender}");
        use schema::omemo_sender_key;
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;
        diesel::insert_into(omemo_sender_key::table)
            .values((
                omemo_sender_key::account.eq(account.to_string()),
                omemo_sender_key::sender_id.eq(sender.name()),
                omemo_sender_key::device_id.eq(u32::from(sender.device_id()) as i64),
                omemo_sender_key::distribution_id.eq(distribution_id.as_bytes().to_vec()),
                omemo_sender_key::sender_key.eq(sender_key.serialize()?.to_vec()),
            ))
            .on_conflict((
                omemo_sender_key::account,
                omemo_sender_key::sender_id,
                omemo_sender_key::device_id,
                omemo_sender_key::distribution_id,
            ))
            .do_update()
            .set(omemo_sender_key::sender_key.eq(sender_key.serialize()?.to_vec()))
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
        log::debug!("Load sender key {sender}");
        use schema::omemo_sender_key;
        let mut conn = self
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        Ok(omemo_sender_key::table
            .filter(omemo_sender_key::account.eq(account.to_string()))
            .filter(omemo_sender_key::sender_id.eq(sender.name()))
            .filter(omemo_sender_key::device_id.eq(u32::from(sender.device_id()) as i64))
            .filter(omemo_sender_key::distribution_id.eq(distribution_id.as_bytes().to_vec()))
            .first(&mut conn)
            .optional()
            .map_err(signal_storage_display_error())?
            .map(|sender_key: OmemoSenderKey| {
                libsignal_protocol::SenderKeyRecord::deserialize(&sender_key.sender_key)
            })
            .transpose()?)
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
            Box::new(UnwindSafeResultError(format!("{}", e))),
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
