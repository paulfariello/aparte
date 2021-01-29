/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use chrono::{DateTime, FixedOffset, Local};
use std::cmp::Ordering;
use std::convert::TryFrom;
use std::hash;
use uuid::Uuid;
use xmpp_parsers::{BareJid, Jid};

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub id: String,
    pub timestamp: DateTime<FixedOffset>,
    pub from: BareJid,
    pub from_full: Jid,
    pub to: BareJid,
    pub to_full: Jid,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct ChannelMessage {
    pub id: String,
    pub timestamp: DateTime<FixedOffset>,
    pub from: BareJid,
    pub from_full: Jid,
    pub to: BareJid,
    pub to_full: Jid,
    pub body: String,
}

#[derive(Debug, Clone)]
pub enum XmppMessage {
    Chat(ChatMessage),
    Channel(ChannelMessage),
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
        body: &str,
    ) -> Self {
        let from = match from_full {
            Jid::Bare(from_full) => from_full.clone(),
            Jid::Full(from_full) => from_full.clone().into(),
        };

        let to = match to_full {
            Jid::Bare(to_full) => to_full.clone(),
            Jid::Full(to_full) => to_full.clone().into(),
        };

        Message::Incoming(XmppMessage::Chat(ChatMessage {
            id: id.into(),
            timestamp: timestamp,
            from: from,
            from_full: from_full.clone(),
            to: to.clone(),
            to_full: to_full.clone(),
            body: body.to_string(),
        }))
    }

    pub fn outgoing_chat<I: Into<String>>(
        id: I,
        timestamp: DateTime<FixedOffset>,
        from_full: &Jid,
        to_full: &Jid,
        body: &str,
    ) -> Self {
        let from = match from_full {
            Jid::Bare(from_full) => from_full.clone(),
            Jid::Full(from_full) => from_full.clone().into(),
        };

        let to = match to_full {
            Jid::Bare(to_full) => to_full.clone(),
            Jid::Full(to_full) => to_full.clone().into(),
        };

        Message::Outgoing(XmppMessage::Chat(ChatMessage {
            id: id.into(),
            timestamp: timestamp,
            from: from,
            from_full: from_full.clone(),
            to: to.clone(),
            to_full: to_full.clone(),
            body: body.to_string(),
        }))
    }

    pub fn incoming_channel<I: Into<String>>(
        id: I,
        timestamp: DateTime<FixedOffset>,
        from_full: &Jid,
        to_full: &Jid,
        body: &str,
    ) -> Self {
        let from = match from_full {
            Jid::Bare(from_full) => from_full.clone(),
            Jid::Full(from_full) => from_full.clone().into(),
        };

        let to = match to_full {
            Jid::Bare(to_full) => to_full.clone(),
            Jid::Full(to_full) => to_full.clone().into(),
        };

        Message::Incoming(XmppMessage::Channel(ChannelMessage {
            id: id.into(),
            timestamp: timestamp,
            from: from,
            from_full: from_full.clone(),
            to: to.clone(),
            to_full: to_full.clone(),
            body: body.to_string(),
        }))
    }

    pub fn outgoing_channel<I: Into<String>>(
        id: I,
        timestamp: DateTime<FixedOffset>,
        from_full: &Jid,
        to_full: &Jid,
        body: &str,
    ) -> Self {
        let from = match from_full {
            Jid::Bare(from_full) => from_full.clone(),
            Jid::Full(from_full) => from_full.clone().into(),
        };

        let to = match to_full {
            Jid::Bare(to_full) => to_full.clone(),
            Jid::Full(to_full) => to_full.clone().into(),
        };

        Message::Outgoing(XmppMessage::Channel(ChannelMessage {
            id: id.into(),
            timestamp: timestamp,
            from: from,
            from_full: from_full.clone(),
            to: to.clone(),
            to_full: to_full.clone(),
            body: body.to_string(),
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
            Message::Outgoing(XmppMessage::Chat(ChatMessage { body, .. }))
            | Message::Incoming(XmppMessage::Chat(ChatMessage { body, .. }))
            | Message::Outgoing(XmppMessage::Channel(ChannelMessage { body, .. }))
            | Message::Incoming(XmppMessage::Channel(ChannelMessage { body, .. }))
            | Message::Log(LogMessage { body, .. }) => &body,
        }
    }

    #[allow(dead_code)]
    pub fn id<'a>(&'a self) -> &'a str {
        match self {
            Message::Outgoing(XmppMessage::Chat(ChatMessage { id, .. }))
            | Message::Incoming(XmppMessage::Chat(ChatMessage { id, .. }))
            | Message::Outgoing(XmppMessage::Channel(ChannelMessage { id, .. }))
            | Message::Incoming(XmppMessage::Channel(ChannelMessage { id, .. }))
            | Message::Log(LogMessage { id, .. }) => &id,
        }
    }

    #[allow(dead_code)]
    pub fn timestamp<'a>(&'a self) -> &'a DateTime<FixedOffset> {
        match self {
            Message::Outgoing(XmppMessage::Chat(ChatMessage { timestamp, .. }))
            | Message::Incoming(XmppMessage::Chat(ChatMessage { timestamp, .. }))
            | Message::Outgoing(XmppMessage::Channel(ChannelMessage { timestamp, .. }))
            | Message::Incoming(XmppMessage::Channel(ChannelMessage { timestamp, .. }))
            | Message::Log(LogMessage { timestamp, .. }) => timestamp,
        }
    }
}

impl hash::Hash for Message {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        match self {
            Message::Log(message) => message.id.hash(state),
            Message::Incoming(XmppMessage::Chat(message))
            | Message::Outgoing(XmppMessage::Chat(message)) => message.id.hash(state),
            Message::Incoming(XmppMessage::Channel(message))
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
                    xmpp_parsers::message::Message::new(Some(Jid::Bare(message.to)));
                xmpp_message.id = Some(message.id);
                xmpp_message.type_ = xmpp_parsers::message::MessageType::Chat;
                xmpp_message
                    .bodies
                    .insert(String::new(), xmpp_parsers::message::Body(message.body));
                Ok(xmpp_message.into())
            }
            Message::Outgoing(XmppMessage::Channel(message)) => {
                let mut xmpp_message =
                    xmpp_parsers::message::Message::new(Some(Jid::Bare(message.to)));
                xmpp_message.id = Some(message.id);
                xmpp_message.type_ = xmpp_parsers::message::MessageType::Groupchat;
                xmpp_message
                    .bodies
                    .insert(String::new(), xmpp_parsers::message::Body(message.body));
                Ok(xmpp_message.into())
            }
        }
    }
}
