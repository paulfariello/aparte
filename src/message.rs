/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use chrono::{DateTime, FixedOffset, Local};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::hash;
use uuid::Uuid;
use xmpp_parsers::{BareJid, Jid};

#[derive(Debug, Clone)]
pub struct XmppMessageVersion {
    pub id: String,
    pub timestamp: DateTime<FixedOffset>,
    pub bodies: HashMap<String, String>,
}

impl Eq for XmppMessageVersion {}

impl PartialEq for XmppMessageVersion {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Ord for XmppMessageVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        if self == other {
            Ordering::Equal
        } else {
            self.timestamp.cmp(&other.timestamp)
        }
    }
}

impl PartialOrd for XmppMessageVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl XmppMessageVersion {
    pub fn get_best_body<'a>(&'a self, prefered_langs: Vec<&str>) -> &'a String {
        for lang in prefered_langs {
            if let Some(body) = self.bodies.get(lang) {
                return body;
            }
        }

        if let Some(body) = self.bodies.get("") {
            return body;
        }

        self.bodies.iter().map(|(_, body)| body).next().unwrap()
    }
}

#[derive(Debug, Clone)]
pub struct VersionedXmppMessage {
    pub id: String,
    pub from: BareJid,
    pub from_full: Jid,
    pub to: BareJid,
    pub to_full: Jid,
    pub history: Vec<XmppMessageVersion>,
}

impl VersionedXmppMessage {
    pub fn get_last_bodies<'a>(&'a self) -> impl Iterator<Item = (&'a String, &'a String)> {
        let last = self.history.iter().max().unwrap();
        last.bodies.iter()
    }
    pub fn get_last_body<'a>(&'a self) -> &'a str {
        let last = self.history.iter().max().unwrap();
        &last.get_best_body(vec![])
    }

    pub fn get_original_timestamp<'a>(&'a self) -> &'a DateTime<FixedOffset> {
        let first = self.history.iter().min().unwrap();
        &first.timestamp
    }
}

#[derive(Debug, Clone)]
pub enum XmppMessage {
    Chat(VersionedXmppMessage),
    Channel(VersionedXmppMessage),
}

#[derive(Debug, Clone)]
pub struct LogMessage {
    pub id: String,
    pub timestamp: DateTime<FixedOffset>,
    pub body: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    Incoming(XmppMessage),
    Outgoing(XmppMessage),
    Log(LogMessage),
}

impl Message {
    pub fn incoming_chat<I: Into<String>>(
        id: I,
        timestamp: DateTime<FixedOffset>,
        from_full: &Jid,
        to_full: &Jid,
        bodies: &HashMap<String, String>,
    ) -> Self {
        let from = match from_full {
            Jid::Bare(from_full) => from_full.clone(),
            Jid::Full(from_full) => from_full.clone().into(),
        };

        let to = match to_full {
            Jid::Bare(to_full) => to_full.clone(),
            Jid::Full(to_full) => to_full.clone().into(),
        };

        let id = id.into();

        let version = XmppMessageVersion {
            id: id.clone(),
            timestamp,
            bodies: bodies.clone(),
        };

        Message::Incoming(XmppMessage::Chat(VersionedXmppMessage {
            id,
            from,
            from_full: from_full.clone(),
            to,
            to_full: to_full.clone(),
            history: vec![version],
        }))
    }

    pub fn outgoing_chat<I: Into<String>>(
        id: I,
        timestamp: DateTime<FixedOffset>,
        from_full: &Jid,
        to_full: &Jid,
        bodies: &HashMap<String, String>,
    ) -> Self {
        let from = match from_full {
            Jid::Bare(from_full) => from_full.clone(),
            Jid::Full(from_full) => from_full.clone().into(),
        };

        let to = match to_full {
            Jid::Bare(to_full) => to_full.clone(),
            Jid::Full(to_full) => to_full.clone().into(),
        };

        let id = id.into();

        let version = XmppMessageVersion {
            id: id.clone(),
            timestamp,
            bodies: bodies.clone(),
        };

        Message::Outgoing(XmppMessage::Chat(VersionedXmppMessage {
            id,
            from,
            from_full: from_full.clone(),
            to,
            to_full: to_full.clone(),
            history: vec![version],
        }))
    }

    pub fn incoming_channel<I: Into<String>>(
        id: I,
        timestamp: DateTime<FixedOffset>,
        from_full: &Jid,
        to_full: &Jid,
        bodies: &HashMap<String, String>,
    ) -> Self {
        let from = match from_full {
            Jid::Bare(from_full) => from_full.clone(),
            Jid::Full(from_full) => from_full.clone().into(),
        };

        let to = match to_full {
            Jid::Bare(to_full) => to_full.clone(),
            Jid::Full(to_full) => to_full.clone().into(),
        };

        let id = id.into();

        let version = XmppMessageVersion {
            id: id.clone(),
            timestamp,
            bodies: bodies.clone(),
        };

        Message::Incoming(XmppMessage::Channel(VersionedXmppMessage {
            id,
            from,
            from_full: from_full.clone(),
            to,
            to_full: to_full.clone(),
            history: vec![version],
        }))
    }

    pub fn outgoing_channel<I: Into<String>>(
        id: I,
        timestamp: DateTime<FixedOffset>,
        from_full: &Jid,
        to_full: &Jid,
        bodies: &HashMap<String, String>,
    ) -> Self {
        let from = match from_full {
            Jid::Bare(from_full) => from_full.clone(),
            Jid::Full(from_full) => from_full.clone().into(),
        };

        let to = match to_full {
            Jid::Bare(to_full) => to_full.clone(),
            Jid::Full(to_full) => to_full.clone().into(),
        };

        let id = id.into();

        let version = XmppMessageVersion {
            id: id.clone(),
            timestamp,
            bodies: bodies.clone(),
        };

        Message::Outgoing(XmppMessage::Channel(VersionedXmppMessage {
            id,
            from,
            from_full: from_full.clone(),
            to,
            to_full: to_full.clone(),
            history: vec![version],
        }))
    }

    pub fn log(msg: String) -> Self {
        Message::Log(LogMessage {
            id: Uuid::new_v4().to_string(),
            timestamp: Local::now().into(),
            body: msg,
        })
    }

    #[allow(dead_code)]
    pub fn body<'a>(&'a self) -> &'a str {
        match self {
            Message::Outgoing(XmppMessage::Chat(message))
            | Message::Incoming(XmppMessage::Chat(message))
            | Message::Outgoing(XmppMessage::Channel(message))
            | Message::Incoming(XmppMessage::Channel(message)) => message.get_last_body(),
            Message::Log(LogMessage { body, .. }) => &body,
        }
    }

    #[allow(dead_code)]
    pub fn id<'a>(&'a self) -> &'a str {
        match self {
            Message::Outgoing(XmppMessage::Chat(VersionedXmppMessage { id, .. }))
            | Message::Incoming(XmppMessage::Chat(VersionedXmppMessage { id, .. }))
            | Message::Outgoing(XmppMessage::Channel(VersionedXmppMessage { id, .. }))
            | Message::Incoming(XmppMessage::Channel(VersionedXmppMessage { id, .. }))
            | Message::Log(LogMessage { id, .. }) => &id,
        }
    }

    #[allow(dead_code)]
    pub fn timestamp<'a>(&'a self) -> &'a DateTime<FixedOffset> {
        match self {
            Message::Outgoing(XmppMessage::Chat(message))
            | Message::Incoming(XmppMessage::Chat(message))
            | Message::Outgoing(XmppMessage::Channel(message))
            | Message::Incoming(XmppMessage::Channel(message)) => message.get_original_timestamp(),
            Message::Log(LogMessage { timestamp, .. }) => timestamp,
        }
    }
}

impl hash::Hash for Message {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        match self {
            Message::Log(message) => message.id.hash(state),
            Message::Incoming(XmppMessage::Chat(message))
            | Message::Outgoing(XmppMessage::Chat(message))
            | Message::Incoming(XmppMessage::Channel(message))
            | Message::Outgoing(XmppMessage::Channel(message)) => message.id.hash(state),
        }
    }
}

impl PartialEq for Message {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl std::cmp::Eq for Message {}

impl Ord for Message {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.eq(other) {
            Ordering::Equal
        } else {
            self.timestamp().cmp(other.timestamp())
        }
    }
}

impl PartialOrd for Message {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl TryFrom<Message> for xmpp_parsers::Element {
    type Error = ();

    fn try_from(message: Message) -> Result<Self, Self::Error> {
        match message {
            Message::Log(_) => Err(()),
            Message::Incoming(_) => Err(()),
            Message::Outgoing(XmppMessage::Chat(message)) => {
                let mut xmpp_message =
                    xmpp_parsers::message::Message::new(Some(Jid::Bare(message.to.clone())));
                xmpp_message.id = Some(message.id.clone());
                xmpp_message.type_ = xmpp_parsers::message::MessageType::Chat;
                xmpp_message.bodies = message
                    .get_last_bodies()
                    .map(|(lang, body)| (lang.clone(), xmpp_parsers::message::Body(body.clone())))
                    .collect();
                Ok(xmpp_message.into())
            }
            Message::Outgoing(XmppMessage::Channel(message)) => {
                let mut xmpp_message =
                    xmpp_parsers::message::Message::new(Some(Jid::Bare(message.to.clone())));
                xmpp_message.id = Some(message.id.clone());
                xmpp_message.type_ = xmpp_parsers::message::MessageType::Groupchat;
                xmpp_message.bodies = message
                    .get_last_bodies()
                    .map(|(lang, body)| (lang.clone(), xmpp_parsers::message::Body(body.clone())))
                    .collect();
                Ok(xmpp_message.into())
            }
        }
    }
}
