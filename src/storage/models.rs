/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use diesel::prelude::*;

#[derive(Queryable)]
pub struct OmemoOwnDevice {
    pub id: i32,
    pub account: String,
    pub device_id: i32,
    pub current: bool,
}

#[derive(Queryable)]
pub struct OmemoContactDevice {
    pub id: i32,
    pub account: String,
    pub contact: String,
    pub device_id: i32,
}
