/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::cmp;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use xmpp_parsers::BareJid;

use crate::account::Account;

#[derive(Hash, Eq, PartialEq, Clone, Debug, Copy)]
pub enum Affiliation {
    Owner,
    Admin,
    Member,
    Outcast,
    None,
}

#[derive(Hash, Eq, PartialEq, Clone, Debug, Copy)]
pub enum Role {
    Visitor,
    Participant,
    Moderator,
    None,
}

#[derive(Clone, Debug)]
pub struct Occupant {
    pub nick: String,
    pub jid: Option<BareJid>,
    pub affiliation: Affiliation,
    pub role: Role,
}

impl Ord for Occupant {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.nick
            .to_lowercase()
            .cmp(&other.nick.to_string().to_lowercase())
    }
}

impl PartialOrd for Occupant {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub account: Account,
    pub jid: BareJid,
    pub nick: String,
    pub name: Option<String>,
    /// Collections of occupants of this channel, key is occupant.nick
    pub occupants: HashMap<String, Occupant>,
}

impl Channel {
    pub fn get_name(&self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => self.jid.to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Chat {
    pub account: Account,
    pub contact: BareJid,
}

#[derive(Clone, Debug)]
pub enum Conversation {
    Chat(Chat),
    Channel(Channel),
}

impl Conversation {
    pub fn get_account(&self) -> &Account {
        match self {
            Conversation::Chat(chat) => &chat.account,
            Conversation::Channel(channel) => &channel.account,
        }
    }

    pub fn get_jid(&self) -> &BareJid {
        match self {
            Conversation::Chat(chat) => &chat.contact,
            Conversation::Channel(channel) => &channel.jid,
        }
    }
}

impl Hash for Occupant {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.nick.hash(state);
    }
}

impl PartialEq for Occupant {
    fn eq(&self, other: &Self) -> bool {
        self.nick == other.nick
    }
}

impl Eq for Occupant {}
