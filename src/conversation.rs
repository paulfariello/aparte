/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::cmp;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use xmpp_parsers::{BareJid, FullJid};

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
        self.nick.cmp(&other.nick.to_string())
    }
}

impl PartialOrd for Occupant {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub account: FullJid,
    pub jid: BareJid,
    pub nick: String,
    pub name: Option<String>,
    /// Collections of occupants of this channel, key is occupant.nick
    pub occupants: HashMap<String, Occupant>,
}

pub struct Chat {
    pub account: FullJid,
    pub contact: BareJid,
}

pub enum Conversation {
    Chat(Chat),
    Channel(Channel),
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
