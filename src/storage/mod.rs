/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
mod models;
mod schema;

use std::path::PathBuf;

use anyhow::{Error, Result};
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::sqlite::SqliteConnection;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use xmpp_parsers::BareJid;

use crate::account::Account;

pub use models::{OmemoContactDevice, OmemoOwnDevice};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

#[derive(Clone)]
pub struct Storage {
    pool: Pool<ConnectionManager<SqliteConnection>>,
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

    pub fn get_omemo_own_device(&mut self, account: &Account) -> Result<Option<OmemoOwnDevice>> {
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
