/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
mod models;
mod schema;

use std::path::PathBuf;

use anyhow::{Error, Result};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use diesel::Connection;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

use crate::account::Account;

pub use models::OmemoDevice;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

pub struct Storage {
    conn: SqliteConnection,
}

impl Storage {
    pub fn new(path: PathBuf) -> Result<Self> {
        let path = path
            .into_os_string()
            .into_string()
            .map_err(|e| Error::msg(format!("Invalid path {:?}", e)))?;
        let mut conn = SqliteConnection::establish(&path)?;

        conn.run_pending_migrations(MIGRATIONS).unwrap();

        Ok(Self { conn })
    }

    pub fn get_omemo_device(&mut self, account: &Account) -> Result<Option<OmemoDevice>> {
        use schema::omemo_device;
        let res = omemo_device::table
            .filter(omemo_device::account.eq(account.to_string()))
            .first(&mut self.conn)
            .optional()?;
        Ok(res)
    }

    pub fn set_omemo_device(&mut self, account: &Account, device_id: i32) -> Result<OmemoDevice> {
        use schema::omemo_device;
        let device = diesel::insert_into(omemo_device::table)
            .values((
                omemo_device::device_id.eq(device_id),
                omemo_device::account.eq(account.to_string()),
            ))
            .get_result(&mut self.conn)?;
        Ok(device)
    }
}
