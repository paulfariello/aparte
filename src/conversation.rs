use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use xmpp_parsers::BareJid;

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
}

#[derive(Clone, Debug)]
pub struct Occupant {
    pub nick: String,
    pub jid: Option<BareJid>,
    pub affiliation: Affiliation,
    pub role: Role,
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub jid: BareJid,
    pub nick: String,
    pub name: Option<String>,
    pub occupants: HashMap<String, Occupant>,
}

pub struct Chat {
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
