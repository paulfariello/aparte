/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::hash::{Hash, Hasher};
use xmpp_parsers::roster::Subscription;
use xmpp_parsers::{BareJid, Element};

#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub enum Presence {
    Unavailable,
    Available,
    Away,
    Chat,
    Dnd,
    Xa,
}

#[derive(Clone, Debug)]
pub struct Group(pub String);

impl Hash for Group {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl PartialEq for Group {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Group {}

#[derive(Clone, Debug)]
pub struct Contact {
    pub jid: BareJid,
    pub name: Option<String>,
    pub subscription: Subscription,
    pub presence: Presence,
    pub groups: Vec<Group>,
}

impl Hash for Contact {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.jid.hash(state);
    }
}

impl PartialEq for Contact {
    fn eq(&self, other: &Self) -> bool {
        self.jid == other.jid
    }
}

impl Eq for Contact {}

#[derive(Clone, Debug)]
pub struct Bookmark {
    pub jid: BareJid,
    pub name: Option<String>,
    pub nick: Option<String>,
    pub autojoin: bool,
    pub password: Option<String>,
    pub extensions: Option<Element>,
}

impl Hash for Bookmark {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.jid.hash(state);
    }
}

impl PartialEq for Bookmark {
    fn eq(&self, other: &Self) -> bool {
        self.jid == other.jid
    }
}

impl Eq for Bookmark {}
