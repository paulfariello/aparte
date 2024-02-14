/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use diesel::prelude::*;

#[derive(Queryable)]
pub struct OmemoOwnDevice {
    pub own_device_pk: i32,
    pub account: String,
    pub id: i64,
    pub identity: Option<Vec<u8>>,
}

#[derive(Queryable)]
pub struct OmemoContactDevice {
    pub contact_device_pk: i32,
    pub account: String,
    pub contact: String,
    pub id: i64,
}

impl From<&OmemoOwnDevice> for OmemoContactDevice {
    fn from(value: &OmemoOwnDevice) -> Self {
        OmemoContactDevice {
            contact_device_pk: -1,
            account: value.account.clone(),
            contact: value.account.clone(),
            id: value.id,
        }
    }
}
