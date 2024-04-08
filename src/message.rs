/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::cmp::{self, Ordering};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::hash::{self, Hash};
use std::io::Write;
use std::os::fd::AsFd;

use chrono::offset::{Local, TimeZone};
use chrono::{DateTime, FixedOffset, Local as LocalTz};
use sixel_image::SixelImage;
use termion::color;
use unicode_segmentation::UnicodeSegmentation;
use uuid::Uuid;
use xmpp_parsers::delay::Delay;
use xmpp_parsers::message::{Message as XmppParsersMessage, MessageType as XmppParsersMessageType};
use xmpp_parsers::{BareJid, Jid};

use crate::account::Account;
use crate::color::id_to_rgb;
use crate::i18n;
use crate::terminus::{
    self, term_string_visible_len, Dimensions, MeasureSpec, MeasureSpecs, RequestedDimension,
    RequestedDimensions, Screen, View,
};

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
        i18n::get_best(&self.bodies, prefered_langs).unwrap().1
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
    pub archive: bool,
}

impl VersionedXmppMessage {
    pub fn get_last_bodies<'a>(&'a self) -> impl Iterator<Item = (&'a String, &'a String)> {
        let last = self.history.iter().max().unwrap();
        last.bodies.iter()
    }
    pub fn get_last_body<'a>(&'a self) -> &'a str {
        let last = self.history.iter().max().unwrap();
        last.get_best_body(vec![])
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
            .find_map(|payload| Delay::try_from(payload.clone()).ok());
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
pub enum Body {
    Text(String),
    Image(SixelImage),
}

#[derive(Debug, Clone)]
pub struct LogMessage {
    pub id: String,
    pub timestamp: DateTime<FixedOffset>,
    pub body: Body,
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
        archive: bool,
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
                    .find_map(|payload| Delay::try_from(payload.clone()).ok()),
            };
            let timestamp = delay
                .map(|delay| delay.stamp.0)
                .unwrap_or(LocalTz::now().into());
            let to = match message.to.clone() {
                Some(to) => to,
                None => account.clone().into(),
            };

            match message.type_ {
                XmppParsersMessageType::Chat => {
                    if from.clone().node() == account.node()
                        && from.clone().domain() == account.domain()
                    {
                        Ok(Message::outgoing_chat(
                            id, timestamp, &from, &to, &bodies, archive,
                        ))
                    } else {
                        Ok(Message::incoming_chat(
                            id, timestamp, &from, &to, &bodies, archive,
                        ))
                    }
                }
                XmppParsersMessageType::Groupchat => Ok(Message::incoming_channel(
                    id, timestamp, &from, &to, &bodies, archive,
                )),
                _ => Err(()),
            }
        } else {
            Err(())
        }
    }

    pub fn get_local_destination_from_xmpp<'a>(
        account: &Account,
        message: &'a XmppParsersMessage,
    ) -> Result<&'a Jid, String> {
        match Message::get_direction_from_xmpp(account, message)? {
            Direction::Incoming => message.from.as_ref().ok_or(String::from(
                "Missing 'from' attribute for incoming message",
            )),
            Direction::Outgoing => message
                .to
                .as_ref()
                .ok_or(String::from("Missing 'to' attribute for outgoing message")),
        }
    }

    pub fn get_direction_from_xmpp(
        account: &Account,
        message: &XmppParsersMessage,
    ) -> Result<Direction, String> {
        let from: Option<BareJid> = message.from.as_ref().map(|f| f.to_bare());
        let to: Option<BareJid> = message.to.as_ref().map(|f| f.to_bare());
        let bare_account: BareJid = account.to_bare();

        match (from.as_ref(), to.as_ref()) {
            (Some(from), Some(_to)) => {
                if from == &bare_account {
                    Ok(Direction::Outgoing)
                } else {
                    Ok(Direction::Incoming)
                }
            }
            (None, Some(to)) => {
                if to == &bare_account {
                    Ok(Direction::Incoming)
                } else {
                    Ok(Direction::Outgoing)
                }
            }
            (Some(from), None) => {
                if from == &bare_account {
                    Ok(Direction::Outgoing)
                } else {
                    Ok(Direction::Incoming)
                }
            }
            (None, None) => Err("Message as no 'from' nor 'to' attributes".to_string()),
        }
    }

    pub fn incoming_chat<I: Into<String>>(
        id: I,
        timestamp: DateTime<FixedOffset>,
        from: &Jid,
        to: &Jid,
        bodies: &HashMap<String, String>,
        archive: bool,
    ) -> Self {
        let id = id.into();

        let version = XmppMessageVersion {
            id: id.clone(),
            timestamp,
            bodies: bodies.clone(),
        };

        Message::Xmpp(VersionedXmppMessage {
            id,
            from: from.to_bare(),
            from_full: from.clone(),
            to: to.to_bare(),
            to_full: to.clone(),
            history: vec![version],
            type_: XmppMessageType::Chat,
            direction: Direction::Incoming,
            archive,
        })
    }

    pub fn outgoing_chat<I: Into<String>>(
        id: I,
        timestamp: DateTime<FixedOffset>,
        from: &Jid,
        to: &Jid,
        bodies: &HashMap<String, String>,
        archive: bool,
    ) -> Self {
        let id = id.into();

        let version = XmppMessageVersion {
            id: id.clone(),
            timestamp,
            bodies: bodies.clone(),
        };

        Message::Xmpp(VersionedXmppMessage {
            id,
            from: from.to_bare(),
            from_full: from.clone(),
            to: to.to_bare(),
            to_full: to.clone(),
            history: vec![version],
            type_: XmppMessageType::Chat,
            direction: Direction::Outgoing,
            archive,
        })
    }

    pub fn incoming_channel<I: Into<String>>(
        id: I,
        timestamp: DateTime<FixedOffset>,
        from: &Jid,
        to: &Jid,
        bodies: &HashMap<String, String>,
        archive: bool,
    ) -> Self {
        let id = id.into();

        let version = XmppMessageVersion {
            id: id.clone(),
            timestamp,
            bodies: bodies.clone(),
        };

        Message::Xmpp(VersionedXmppMessage {
            id,
            from: from.to_bare(),
            from_full: from.clone(),
            to: to.to_bare(),
            to_full: to.clone(),
            history: vec![version],
            type_: XmppMessageType::Channel,
            direction: Direction::Incoming,
            archive,
        })
    }

    pub fn outgoing_channel<I: Into<String>>(
        id: I,
        timestamp: DateTime<FixedOffset>,
        from: &Jid,
        to: &Jid,
        bodies: &HashMap<String, String>,
        archive: bool,
    ) -> Self {
        let id = id.into();

        let version = XmppMessageVersion {
            id: id.clone(),
            timestamp,
            bodies: bodies.clone(),
        };

        Message::Xmpp(VersionedXmppMessage {
            id,
            from: from.to_bare(),
            from_full: from.clone(),
            to: to.to_bare(),
            to_full: to.clone(),
            history: vec![version],
            type_: XmppMessageType::Channel,
            direction: Direction::Outgoing,
            archive,
        })
    }

    pub fn log(msg: String) -> Self {
        Message::Log(LogMessage {
            id: Uuid::new_v4().to_string(),
            timestamp: LocalTz::now().into(),
            body: Body::Text(msg),
        })
    }

    pub fn encryption_recipient(&self) -> Option<BareJid> {
        match self {
            Message::Log(_) => None,
            Message::Xmpp(message) => match message.direction {
                Direction::Outgoing => match message.type_ {
                    XmppMessageType::Chat => Some(message.to.clone()),
                    XmppMessageType::Channel => None, // TODO fetch all participants?
                },
                Direction::Incoming => None,
            },
        }
    }

    pub fn body<'a>(&'a self) -> &'a str {
        match self {
            Message::Xmpp(message) => message.get_last_body(),
            Message::Log(LogMessage {
                body: Body::Text(body),
                ..
            }) => body,
            Message::Log(LogMessage {
                body: Body::Image(image),
                ..
            }) => todo!(),
        }
    }

    pub fn id<'a>(&'a self) -> &'a str {
        match self {
            Message::Xmpp(VersionedXmppMessage { id, .. })
            | Message::Log(LogMessage { id, .. }) => id,
        }
    }

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

#[derive(Debug, Clone)]
pub struct MessageView {
    pub message: Message,
    dimensions: Option<Dimensions>,
}

impl Eq for MessageView {}

impl PartialEq for MessageView {
    fn eq(&self, other: &Self) -> bool {
        self.message.eq(&other.message)
    }
}

impl PartialOrd for MessageView {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.message.partial_cmp(&other.message)
    }
}

impl Ord for MessageView {
    fn cmp(&self, other: &Self) -> Ordering {
        self.message.cmp(&other.message)
    }
}

impl Hash for MessageView {
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.message.hash(state)
    }
}

impl From<Message> for MessageView {
    fn from(message: Message) -> Self {
        MessageView {
            message,
            dimensions: None,
        }
    }
}

impl MessageView {
    fn format(&self, max_width: Option<u16>) -> Vec<String> {
        match &self.message {
            Message::Log(message) => {
                let timestamp = Local.from_utc_datetime(&message.timestamp.naive_local());
                match &message.body {
                    Body::Text(body) => {
                        let mut lines = Vec::new();
                        for line in body.lines() {
                            lines.append(&mut self.format_text(
                                format!(
                                    "{}{}{} - {}",
                                    color::Bg(color::Reset),
                                    color::Fg(color::Reset),
                                    timestamp.format("%T"),
                                    line
                                ),
                                max_width,
                            ))
                        }
                        lines
                    }
                    Body::Image(image) => {
                        vec![String::from("image with Sixel")]
                    }
                }
            }
            Message::Xmpp(message) => {
                let author = terminus::clean_str(&match &message.type_ {
                    XmppMessageType::Channel => match &message.from_full {
                        Jid::Full(from) => from.resource().to_string(),
                        Jid::Bare(from) => from.to_string(),
                    },
                    XmppMessageType::Chat => message.from.to_string(),
                });

                let timestamp =
                    Local.from_utc_datetime(&message.get_original_timestamp().naive_local());
                let body = message.get_last_body();
                let me = body.starts_with("/me");
                let padding_len = match me {
                    true => format!("{} - {}: ", timestamp.format("%T"), author).len(),
                    false => format!("{} - * {}", timestamp.format("%T"), author).len(),
                };
                let padding = " ".repeat(padding_len);

                let (r, g, b) = id_to_rgb(&author);

                let mut attributes = "".to_string();
                if message.has_multiple_version() {
                    attributes.push_str("✎ ");
                }

                let mut buffer = match me {
                    true => format!(
                        "{}{}{} - {}* {}{}{}",
                        color::Bg(color::Reset),
                        color::Fg(color::Reset),
                        timestamp.format("%T"),
                        attributes,
                        color::Fg(color::Rgb(r, g, b)),
                        author,
                        color::Fg(color::Reset)
                    ),
                    false => format!(
                        "{}{}{} - {}{}{}:{} ",
                        color::Bg(color::Reset),
                        color::Fg(color::Reset),
                        timestamp.format("%T"),
                        attributes,
                        color::Fg(color::Rgb(r, g, b)),
                        author,
                        color::Fg(color::Reset)
                    ),
                };

                let mut iter = match me {
                    true => body.strip_prefix("/me").unwrap().lines(),
                    false => body.lines(),
                };

                if let Some(line) = iter.next() {
                    buffer.push_str(&terminus::clean_str(line));
                }
                for line in iter {
                    buffer.push_str(format!("\n{}{}", padding, terminus::clean_str(line)).as_str());
                }

                self.format_text(buffer, max_width)
            }
        }
    }

    fn format_text(&self, text: String, max_width: Option<u16>) -> Vec<String> {
        let mut buffers: Vec<String> = Vec::new();
        for line in text.lines() {
            let mut words = line.split_word_bounds();

            let mut line_len = 0;
            let mut chunk = String::new();
            while let Some(word) = words.next() {
                let visible_word;
                let mut remaining = String::new();

                // We can safely unwrap here because split_word_bounds produce non empty words
                let first_char = word.chars().next().unwrap();

                if first_char == '\x1b' {
                    // Handle Escape sequence: see https://www.ecma-international.org/publications/files/ECMA-ST/Ecma-048.pdf
                    // First char is a word boundary
                    //
                    // We must ignore them for the visible length count but include them in the
                    // final chunk that will be written to the terminal

                    if let Some(word) = words.next() {
                        match word {
                            "[" => {
                                // Control Sequence Introducer are accepted and can safely be
                                // written to terminal
                                let mut escape = String::from("\x1b[");
                                let mut end = false;

                                for word in words.by_ref() {
                                    for c in word.chars() {
                                        // Push all char belonging to escape sequence
                                        // but keep remaining for wrap computation
                                        if !end {
                                            escape.push(c);
                                            match c {
                                                '\x30'..='\x3f' => {} // parameter bytes
                                                '\x20'..='\x2f' => {} // intermediate bytes
                                                '\x40'..='\x7e' => {
                                                    // final byte
                                                    chunk.push_str(&escape);
                                                    end = true;
                                                }
                                                _ => {
                                                    // Invalid escape sequence, just ignore it
                                                    end = true;
                                                }
                                            }
                                        } else {
                                            remaining.push(c);
                                        }
                                    }

                                    if end {
                                        break;
                                    }
                                }
                            }
                            _ => {
                                // Other sequence are not handled and just ignored
                            }
                        }
                    } else {
                        // Nothing is following the escape char
                        // We can simply ignore it
                    }
                    visible_word = remaining.as_str();
                } else {
                    visible_word = word;
                }

                if visible_word.is_empty() {
                    continue;
                }

                let grapheme_count = visible_word.graphemes(true).count();

                if max_width.map_or(false, |max_width| {
                    line_len + grapheme_count > max_width as usize
                }) {
                    // Wrap line
                    buffers.push(chunk);
                    chunk = String::new();
                    line_len = 0;
                }

                chunk.push_str(visible_word);
                line_len += grapheme_count;
            }

            buffers.push(chunk);
        }

        buffers
    }
}

impl<E, W> View<E, W> for MessageView
where
    W: Write + AsFd,
{
    fn measure(&self, measure_specs: &MeasureSpecs) -> RequestedDimensions {
        // TODO: we could avoid creating the real buffers
        match measure_specs.width {
            MeasureSpec::Unspecified => RequestedDimensions {
                height: RequestedDimension::Absolute(1),
                width: RequestedDimension::Absolute(
                    self.format(None)
                        .iter()
                        .next()
                        .map_or(0, |line| term_string_visible_len(&line) as u16),
                ),
            },
            MeasureSpec::AtMost(at_most_width) => {
                let formatted = self.format(Some(at_most_width));
                RequestedDimensions {
                    height: RequestedDimension::Absolute(formatted.len() as u16),
                    width: RequestedDimension::Absolute(cmp::min(
                        formatted.iter().map(|line| line.len()).max().unwrap_or(0) as u16,
                        at_most_width,
                    )),
                }
            }
        }
    }

    fn layout(&mut self, dimensions: &Dimensions) {
        log::debug!("layout {} {:?}", std::any::type_name::<Self>(), dimensions);
        self.dimensions.replace(dimensions.clone());
    }

    fn render(&self, screen: &mut Screen<W>) {
        log::debug!(
            "rendering {} at {:?}",
            std::any::type_name::<Self>(),
            self.dimensions
        );
        let dimensions = self.dimensions.as_ref().unwrap();
        terminus::save_cursor!(screen);

        let mut top = dimensions.top;
        for line in self.format(Some(dimensions.width)) {
            if dimensions.left == 1 {
                // Use fast erase if possible
                terminus::goto!(screen, dimensions.left + dimensions.width, top);
                terminus::vprint!(screen, "{}", "\x1B[1K");
                terminus::goto!(screen, dimensions.left, top);
                terminus::vprint!(screen, "{}", line);
            } else {
                terminus::goto!(screen, dimensions.left, top);
                let padding = dimensions.width - term_string_visible_len(&line) as u16;
                terminus::vprint!(screen, "{: <1$}", line, padding as usize);
            }
            top += 1;
        }

        terminus::restore_cursor!(screen);
    }

    fn event(&mut self, _event: &mut E) {}

    fn is_layout_dirty(&self) -> bool {
        <MessageView as View<E, W>>::is_dirty(self)
    }

    fn is_dirty(&self) -> bool {
        false
    }
}
