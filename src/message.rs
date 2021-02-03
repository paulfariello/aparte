/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use chrono::{DateTime, FixedOffset, Local as LocalTz};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::hash;
use uuid::Uuid;
use xmpp_parsers::delay::Delay;
use xmpp_parsers::message::{Message as XmppParsersMessage, MessageType as XmppParsersMessageType};
use xmpp_parsers::{BareJid, Jid};

use crate::account::Account;

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
    pub type_: XmppMessageType,
    pub direction: Direction,
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

    pub fn add_version_from_xmpp(&mut self, message: &XmppParsersMessage) {
        let id = message
            .id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let bodies: HashMap<String, String> = message
            .bodies
            .iter()
            .map(|(lang, body)| (lang.clone(), body.0.clone()))
            .collect();

        let delay = message
            .payloads
            .iter()
            .filter_map(|payload| Delay::try_from(payload.clone()).ok())
            .nth(0);
        let timestamp = delay
            .map(|delay| delay.stamp.0)
            .unwrap_or(LocalTz::now().into());

        self.history.push(XmppMessageVersion {
            id,
            timestamp,
            bodies,
        });
    }

    pub fn has_multiple_version(&self) -> bool {
        self.history.len() > 1
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum XmppMessageType {
    Chat,
    Channel,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Direction {
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone)]
pub struct LogMessage {
    pub id: String,
    pub timestamp: DateTime<FixedOffset>,
    pub body: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    Xmpp(VersionedXmppMessage),
    Log(LogMessage),
}

impl Message {
    pub fn from_xmpp(
        account: &Account,
        message: &XmppParsersMessage,
        delay: &Option<Delay>,
    ) -> Result<Self, ()> {
        let id = message
            .id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        if let Some(from) = message.from.clone() {
            let bodies: HashMap<String, String> = message
                .bodies
                .iter()
                .map(|(lang, body)| (lang.clone(), body.0.clone()))
                .collect();
            let delay = match delay {
                Some(delay) => Some(delay.clone()),
                None => message
                    .payloads
                    .iter()
                    .filter_map(|payload| Delay::try_from(payload.clone()).ok())
                    .nth(0),
            };
            let to = match message.to.clone() {
                Some(to) => to,
                None => account.clone().into(),
            };

            match message.type_ {
                XmppParsersMessageType::Chat => {
                    if from.clone().node() == account.node
                        && from.clone().domain() == account.domain
                    {
                        Ok(Message::outgoing_chat(
                            id,
                            delay
                                .map(|delay| delay.stamp.0)
                                .unwrap_or(LocalTz::now().into()),
                            &from,
                            &to,
                            &bodies,
                        ))
                    } else {
                        Ok(Message::incoming_chat(
                            id,
                            delay
                                .map(|delay| delay.stamp.0)
                                .unwrap_or(LocalTz::now().into()),
                            &from,
                            &to,
                            &bodies,
                        ))
                    }
                }
                XmppParsersMessageType::Groupchat => Ok(Message::incoming_channel(
                    id,
                    delay
                        .map(|delay| delay.stamp.0)
                        .unwrap_or(LocalTz::now().into()),
                    &from,
                    &to,
                    &bodies,
                )),
                _ => Err(()),
            }
        } else {
            Err(())
        }
    }

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

        Message::Xmpp(VersionedXmppMessage {
            id,
            from,
            from_full: from_full.clone(),
            to,
            to_full: to_full.clone(),
            history: vec![version],
            type_: XmppMessageType::Chat,
            direction: Direction::Incoming,
        })
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

        Message::Xmpp(VersionedXmppMessage {
            id,
            from,
            from_full: from_full.clone(),
            to,
            to_full: to_full.clone(),
            history: vec![version],
            type_: XmppMessageType::Chat,
            direction: Direction::Outgoing,
        })
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

        Message::Xmpp(VersionedXmppMessage {
            id,
            from,
            from_full: from_full.clone(),
            to,
            to_full: to_full.clone(),
            history: vec![version],
            type_: XmppMessageType::Channel,
            direction: Direction::Incoming,
        })
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

        Message::Xmpp(VersionedXmppMessage {
            id,
            from,
            from_full: from_full.clone(),
            to,
            to_full: to_full.clone(),
            history: vec![version],
            type_: XmppMessageType::Channel,
            direction: Direction::Outgoing,
        })
    }

    pub fn log(msg: String) -> Self {
        Message::Log(LogMessage {
            id: Uuid::new_v4().to_string(),
            timestamp: LocalTz::now().into(),
            body: msg,
        })
    }

    #[allow(dead_code)]
    pub fn body<'a>(&'a self) -> &'a str {
        match self {
            Message::Xmpp(message) => message.get_last_body(),
            Message::Log(LogMessage { body, .. }) => &body,
        }
    }

    #[allow(dead_code)]
    pub fn id<'a>(&'a self) -> &'a str {
        match self {
            Message::Xmpp(VersionedXmppMessage { id, .. })
            | Message::Log(LogMessage { id, .. }) => &id,
        }
    }

    #[allow(dead_code)]
    pub fn timestamp<'a>(&'a self) -> &'a DateTime<FixedOffset> {
        match self {
            Message::Xmpp(message) => message.get_original_timestamp(),
            Message::Log(LogMessage { timestamp, .. }) => timestamp,
        }
    }
}

impl hash::Hash for Message {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.id().hash(state)
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
            Message::Xmpp(message) => match message.direction {
                Direction::Outgoing => match message.type_ {
                    XmppMessageType::Chat => {
                        let mut xmpp_message = xmpp_parsers::message::Message::new(Some(
                            Jid::Bare(message.to.clone()),
                        ));
                        xmpp_message.id = Some(message.id.clone());
                        xmpp_message.type_ = xmpp_parsers::message::MessageType::Chat;
                        xmpp_message.bodies = message
                            .get_last_bodies()
                            .map(|(lang, body)| {
                                (lang.clone(), xmpp_parsers::message::Body(body.clone()))
                            })
                            .collect();
                        Ok(xmpp_message.into())
                    }
                    XmppMessageType::Channel => {
                        let mut xmpp_message = xmpp_parsers::message::Message::new(Some(
                            Jid::Bare(message.to.clone()),
                        ));
                        xmpp_message.id = Some(message.id.clone());
                        xmpp_message.type_ = xmpp_parsers::message::MessageType::Groupchat;
                        xmpp_message.bodies = message
                            .get_last_bodies()
                            .map(|(lang, body)| {
                                (lang.clone(), xmpp_parsers::message::Body(body.clone()))
                            })
                            .collect();
                        Ok(xmpp_message.into())
                    }
                },
                Direction::Incoming => Err(()),
            },
        }
    }
}
