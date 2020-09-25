/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use xmpp_parsers::roster::Subscription;
use std::hash::{Hash, Hasher};
use xmpp_parsers::BareJid;

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

#[derive(Clone, Debug)]
pub enum ContactOrBookmark {
    Contact(Contact),
    Bookmark(Bookmark),
}

impl Hash for ContactOrBookmark {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Contact(contact) => contact.jid.hash(state),
            Self::Bookmark(bookmark) => bookmark.jid.hash(state),
        };
    }
}

impl PartialEq for ContactOrBookmark {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Contact(a), Self::Contact(b)) => a.eq(b),
            (Self::Bookmark(a), Self::Bookmark(b)) => a.eq(b),
            _ => false,
        }
    }
}

impl Eq for ContactOrBookmark {}
