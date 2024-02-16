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

#[derive(Queryable)]
pub struct OmemoIdentity {
    pub identity_pk: i32,
    pub account: String,
    pub user_id: String,
    pub device_id: i64,
    pub identity: Vec<u8>,
}

#[derive(Queryable)]
pub struct OmemoSession {
    pub identity_pk: i32,
    pub account: String,
    pub user_id: String,
    pub device_id: i64,
    pub session: Vec<u8>,
}

#[derive(Queryable)]
pub struct OmemoPreKey {
    pub pre_key_pk: i32,
    pub account: String,
    pub pre_key_id: i64,
    pub pre_key: Vec<u8>,
}

#[derive(Queryable)]
pub struct OmemoSignedPreKey {
    pub signed_pre_key_pk: i32,
    pub account: String,
    pub signed_pre_key_id: i64,
    pub signed_pre_key: Vec<u8>,
}

#[derive(Queryable)]
pub struct OmemoSenderKey {
    pub sender_key_pk: i32,
    pub account: String,
    pub sender_id: String,
    pub device_id: i64,
    pub distribution_id: Vec<u8>,
    pub sender_key: Vec<u8>,
}
