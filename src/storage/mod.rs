/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
mod models;
mod schema;

use std::convert::TryFrom;
use std::path::PathBuf;

use anyhow::{Error, Result};
use async_trait::async_trait;
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::sqlite::SqliteConnection;
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

fn signal_storage_display_error<T>(
    str: &'static str,
) -> impl Fn(T) -> libsignal_protocol::error::SignalProtocolError
where
    T: std::fmt::Display,
{
    move |e: T| {
        libsignal_protocol::error::SignalProtocolError::ApplicationCallbackError(
            str,
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
}

impl SignalStorage {
    pub fn new(account: Account, storage: Storage) -> Self {
        Self { account, storage }
    }
}

#[async_trait(?Send)]
impl libsignal_protocol::IdentityKeyStore for SignalStorage {
    async fn get_identity_key_pair(
        &self,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<libsignal_protocol::IdentityKeyPair> {
        log::debug!("Get own identity key pair");
        self.storage
            .get_omemo_own_device(&self.account)
            .map_err(signal_storage_display_error("Cannot get own device"))?
            .map(|device| device.identity)
            .flatten()
            .ok_or_else(signal_storage_empty_error("Missing own device identity"))
            .map(|identity| libsignal_protocol::IdentityKeyPair::try_from(identity.as_ref()))?
            .map_err(signal_storage_error("Corrupted stored own identity"))
    }

    async fn get_local_registration_id(
        &self,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<u32> {
        log::debug!("Get local registration id");
        self.storage
            .get_omemo_own_device(&self.account)
            .map_err(signal_storage_display_error("Cannot get own device"))?
            .map(|device| device.id as u32)
            .ok_or_else(signal_storage_empty_error("Missing own device"))
    }

    async fn save_identity(
        &mut self,
        address: &libsignal_protocol::ProtocolAddress,
        identity: &libsignal_protocol::IdentityKey,
        ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<bool> {
        log::debug!("Save {address}'s identity");
        // The return value represents whether an existing identity was replaced (`Ok(true)`). If it is
        // new or hasn't changed, the return value should be `Ok(false)`.
        let ret = if let Some(stored) = self.get_identity(address, ctx).await? {
            if &stored != identity {
                true
            } else {
                false
            }
        } else {
            false
        };

        use schema::omemo_identity;
        let mut conn = self
            .storage
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;
        diesel::insert_into(omemo_identity::table)
            .values((
                omemo_identity::account.eq(self.account.to_string()),
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
            .execute(&mut conn)
            .map_err(signal_storage_display_error("Cannot upsert identity"))?;

        Ok(ret)
    }

    async fn is_trusted_identity(
        &self,
        address: &libsignal_protocol::ProtocolAddress,
        identity: &libsignal_protocol::IdentityKey,
        _direction: libsignal_protocol::Direction,
        ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<bool> {
        log::debug!("Is {address}'s identity trusted");
        Ok(match self.get_identity(address, ctx).await? {
            Some(stored) => &stored == identity,
            _ => false,
        })
    }

    async fn get_identity(
        &self,
        address: &libsignal_protocol::ProtocolAddress,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<Option<libsignal_protocol::IdentityKey>> {
        log::debug!("Get {address}'s identity");
        use schema::omemo_identity;
        let mut conn = self
            .storage
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        omemo_identity::table
            .filter(omemo_identity::account.eq(self.account.to_string()))
            .filter(omemo_identity::user_id.eq(address.name()))
            .filter(omemo_identity::device_id.eq(u32::from(address.device_id()) as i64))
            .first(&mut conn)
            .optional()
            .map_err(signal_storage_display_error("Cannot fetch identity"))?
            .map(|identity: OmemoIdentity| {
                libsignal_protocol::IdentityKey::decode(&identity.identity)
            })
            .transpose()
    }
}

#[async_trait(?Send)]
impl libsignal_protocol::SessionStore for SignalStorage {
    async fn load_session(
        &self,
        address: &libsignal_protocol::ProtocolAddress,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<Option<libsignal_protocol::SessionRecord>> {
        log::debug!("Load session for {address}");
        use schema::omemo_session;
        let mut conn = self
            .storage
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        omemo_session::table
            .filter(omemo_session::account.eq(self.account.to_string()))
            .filter(omemo_session::user_id.eq(address.name()))
            .filter(omemo_session::device_id.eq(u32::from(address.device_id()) as i64))
            .first(&mut conn)
            .optional()
            .map_err(signal_storage_display_error("Cannot fetch session"))?
            .map(|session: OmemoSession| {
                libsignal_protocol::SessionRecord::deserialize(&session.session)
            })
            .transpose()
    }

    async fn store_session(
        &mut self,
        address: &libsignal_protocol::ProtocolAddress,
        session: &libsignal_protocol::SessionRecord,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<()> {
        log::debug!("Store session for {address}");
        use schema::omemo_session;
        let mut conn = self
            .storage
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;
        diesel::insert_into(omemo_session::table)
            .values((
                omemo_session::account.eq(self.account.to_string()),
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
            .map_err(signal_storage_display_error("Cannot upsert session"))?;

        Ok(())
    }
}

#[async_trait(?Send)]
impl libsignal_protocol::PreKeyStore for SignalStorage {
    async fn get_pre_key(
        &self,
        pre_key_id: libsignal_protocol::PreKeyId,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<libsignal_protocol::PreKeyRecord> {
        log::debug!("Get pre key {pre_key_id}");
        use schema::omemo_pre_key;
        let mut conn = self
            .storage
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        omemo_pre_key::table
            .filter(omemo_pre_key::account.eq(self.account.to_string()))
            .filter(omemo_pre_key::pre_key_id.eq(u32::from(pre_key_id) as i64))
            .first(&mut conn)
            .optional()
            .map_err(signal_storage_display_error("Cannot fetch pre_key"))?
            .ok_or_else(signal_storage_empty_error("PreKey not found"))
            .map(|pre_key: OmemoPreKey| {
                libsignal_protocol::PreKeyRecord::deserialize(&pre_key.pre_key)
            })?
    }

    async fn save_pre_key(
        &mut self,
        pre_key_id: libsignal_protocol::PreKeyId,
        pre_key: &libsignal_protocol::PreKeyRecord,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<()> {
        log::debug!("Save pre key {pre_key_id}");
        use schema::omemo_pre_key;
        let mut conn = self
            .storage
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;
        diesel::insert_into(omemo_pre_key::table)
            .values((
                omemo_pre_key::account.eq(self.account.to_string()),
                omemo_pre_key::pre_key_id.eq(u32::from(pre_key_id) as i64),
                omemo_pre_key::pre_key.eq(pre_key.serialize()?.to_vec()),
            ))
            .on_conflict((
                omemo_pre_key::account,
                omemo_pre_key::pre_key_id,
                omemo_pre_key::pre_key,
            ))
            .do_update()
            .set(omemo_pre_key::pre_key.eq(pre_key.serialize()?.to_vec()))
            .execute(&mut conn)
            .map_err(signal_storage_display_error("Cannot upsert pre_key"))?;

        Ok(())
    }

    async fn remove_pre_key(
        &mut self,
        pre_key_id: libsignal_protocol::PreKeyId,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<()> {
        log::debug!("Remove pre key {pre_key_id}");
        use schema::omemo_pre_key;
        let mut conn = self
            .storage
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        diesel::delete(
            omemo_pre_key::table
                .filter(omemo_pre_key::account.eq(self.account.to_string()))
                .filter(omemo_pre_key::pre_key_id.eq(u32::from(pre_key_id) as i64)),
        )
        .execute(&mut conn)
        .map_err(signal_storage_display_error("Cannot delete pre_key"))?;

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
        log::debug!("Get signed pre key {signed_pre_key_id}");
        use schema::omemo_signed_pre_key;
        let mut conn = self
            .storage
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        omemo_signed_pre_key::table
            .filter(omemo_signed_pre_key::account.eq(self.account.to_string()))
            .filter(omemo_signed_pre_key::signed_pre_key_id.eq(u32::from(signed_pre_key_id) as i64))
            .first(&mut conn)
            .optional()
            .map_err(signal_storage_display_error("Cannot fetch signed_pre_key"))?
            .ok_or_else(signal_storage_empty_error("PreKey not found"))
            .map(|signed_pre_key: OmemoSignedPreKey| {
                libsignal_protocol::SignedPreKeyRecord::deserialize(&signed_pre_key.signed_pre_key)
            })?
    }

    async fn save_signed_pre_key(
        &mut self,
        signed_pre_key_id: libsignal_protocol::SignedPreKeyId,
        signed_pre_key: &libsignal_protocol::SignedPreKeyRecord,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<()> {
        log::debug!("Save signed pre key {signed_pre_key_id}");
        use schema::omemo_signed_pre_key;
        let mut conn = self
            .storage
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;
        diesel::insert_into(omemo_signed_pre_key::table)
            .values((
                omemo_signed_pre_key::account.eq(self.account.to_string()),
                omemo_signed_pre_key::signed_pre_key_id.eq(u32::from(signed_pre_key_id) as i64),
                omemo_signed_pre_key::signed_pre_key.eq(signed_pre_key.serialize()?.to_vec()),
            ))
            .on_conflict((
                omemo_signed_pre_key::account,
                omemo_signed_pre_key::signed_pre_key_id,
                omemo_signed_pre_key::signed_pre_key,
            ))
            .do_update()
            .set(omemo_signed_pre_key::signed_pre_key.eq(signed_pre_key.serialize()?.to_vec()))
            .execute(&mut conn)
            .map_err(signal_storage_display_error("Cannot upsert signed_pre_key"))?;

        Ok(())
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
        log::debug!("Store sender key {sender}");
        use schema::omemo_sender_key;
        let mut conn = self
            .storage
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;
        diesel::insert_into(omemo_sender_key::table)
            .values((
                omemo_sender_key::account.eq(self.account.to_string()),
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
                omemo_sender_key::sender_key,
            ))
            .do_update()
            .set(omemo_sender_key::sender_key.eq(sender_key.serialize()?.to_vec()))
            .execute(&mut conn)
            .map_err(signal_storage_display_error("Cannot upsert sender_key"))?;

        Ok(())
    }

    async fn load_sender_key(
        &mut self,
        sender: &libsignal_protocol::ProtocolAddress,
        distribution_id: uuid::Uuid,
        _ctx: libsignal_protocol::Context,
    ) -> libsignal_protocol::error::Result<Option<libsignal_protocol::SenderKeyRecord>> {
        log::debug!("Load sender key {sender}");
        use schema::omemo_sender_key;
        let mut conn = self
            .storage
            .pool
            .get()
            .map_err(signal_storage_error("Cannot connect to storage"))?;

        omemo_sender_key::table
            .filter(omemo_sender_key::account.eq(self.account.to_string()))
            .filter(omemo_sender_key::sender_id.eq(sender.name()))
            .filter(omemo_sender_key::device_id.eq(u32::from(sender.device_id()) as i64))
            .filter(omemo_sender_key::distribution_id.eq(distribution_id.as_bytes().to_vec()))
            .first(&mut conn)
            .optional()
            .map_err(signal_storage_display_error("Cannot fetch sender_key"))?
            .map(|sender_key: OmemoSenderKey| {
                libsignal_protocol::SenderKeyRecord::deserialize(&sender_key.sender_key)
            })
            .transpose()
    }
}
