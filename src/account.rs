/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Account {
    pub jid: String,
    pub server: Option<String>,
    pub port: Option<u16>,
    pub autoconnect: bool,
}
