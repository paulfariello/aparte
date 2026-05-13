/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::cell::Cell;
use std::cell::RefCell;
use std::cmp::{self, Ordering};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::hash::{self, Hash};
#[cfg(feature = "image")]
use std::io::Cursor;
#[cfg(feature = "image")]
use std::sync::Arc;
#[cfg(feature = "image")]
use std::sync::RwLock;

use anyhow::Result;
use chrono::format::Locale;
use chrono::offset::{Local, TimeZone};
use chrono::{DateTime, FixedOffset, Local as LocalTz, NaiveDate};
#[cfg(feature = "image")]
use image::io::Reader as ImageReader;
#[cfg(feature = "image")]
use sixel_image::SixelImage;
use std::collections::HashSet;

use terminus::charxel::{Charxel, Charxels, IntoCharxels};
use terminus::rendering::ScreenFrame;
use terminus::{
    self, BgColor, Dimensions, FgColor, MeasureSpec, MeasureSpecs, RequestedDimension,
    RequestedDimensions, Searchable, Style, View,
};
use unicode_segmentation::UnicodeSegmentation as _;
use uuid::Uuid;
use xmpp_parsers::delay::Delay;
use xmpp_parsers::jid::{BareJid, Jid};
use xmpp_parsers::message::{
    Id as XmppParsersMessageId, Lang as XmppParsersLang, Message as XmppParsersMessage,
    MessageType as XmppParsersMessageType,
};
use xmpp_parsers::oob::Oob;
use xmpp_parsers::stanza_id::StanzaId;

use crate::account::Account;
use crate::color::id_to_rgb;
#[cfg(feature = "image")]
use crate::core::Aparte;
use crate::core::AparteAsync;
#[cfg(feature = "image")]
use crate::core::Event;
use crate::i18n;
#[cfg(feature = "image")]
use crate::image::convert_to_sixel;

#[derive(Debug, Clone)]
pub struct XmppMessageVersion {
    pub id: String,
    pub timestamp: DateTime<FixedOffset>,
    pub bodies: HashMap<String, String>,
    pub oobs: Vec<Oob>,
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
    pub fn get_best_body<'a>(&'a self, preferred_langs: Vec<&str>) -> &'a String {
        i18n::get_best(&self.bodies, preferred_langs).unwrap().1
    }
}

#[derive(Debug, Clone, Default, Eq, PartialEq, PartialOrd, Ord)]
pub enum DeliveryStatus {
    #[default]
    None,
    Delivered,
    Displayed,
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
    pub delayed: bool,
    pub encrypted: bool,
    pub stanza_id: Option<String>,
    pub reactions: HashMap<BareJid, Vec<String>>,
    pub delivery_status: DeliveryStatus,
}

impl VersionedXmppMessage {
    pub fn get_last_bodies(&self) -> impl Iterator<Item = (&String, &String)> {
        let last = self.history.iter().max().unwrap();
        last.bodies.iter()
    }
    pub fn get_last_body(&self, preferred_langs: Vec<&str>) -> &str {
        let last = self.history.iter().max().unwrap();
        last.get_best_body(preferred_langs)
    }

    pub fn get_original_timestamp(&self) -> &DateTime<FixedOffset> {
        let first = self.history.iter().min().unwrap();
        &first.timestamp
    }

    pub fn add_version_from_xmpp(&mut self, message: &XmppParsersMessage) {
        let id = message
            .id
            .as_ref()
            .map(|id| id.0.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let bodies: HashMap<String, String> = message
            .bodies
            .iter()
            .map(|(lang, body)| (lang.0.clone(), body.clone()))
            .collect();

        let oobs: Vec<Oob> = message
            .payloads
            .iter()
            .filter_map(|payload| Oob::try_from(payload.clone()).ok())
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
            oobs,
        });
    }

    pub fn has_multiple_version(&self) -> bool {
        self.history.len() > 1
    }

    pub fn update_reactions(&mut self, from: BareJid, emojis: Vec<String>) {
        if emojis.is_empty() {
            self.reactions.remove(&from);
        } else {
            self.reactions.insert(from, emojis);
        }
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
    pub body: Charxels,
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
            .as_ref()
            .map(|id| id.0.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        if let Some(from) = message.from.clone() {
            let bodies: HashMap<String, String> = message
                .bodies
                .iter()
                .map(|(lang, body)| (lang.0.clone(), body.clone()))
                .collect();
            let oobs: Vec<_> = message
                .payloads
                .iter()
                .filter_map(|payload| Oob::try_from(payload.clone()).ok())
                .collect();
            let encrypted = message.payloads.iter().any(|p| {
                (p.name() == "encrypted" && p.ns() == "eu.siacs.conversations.axolotl")
                    || (p.name() == "encryption" && p.ns() == "urn:xmpp:eme:0")
            });
            let muc_bare = from.to_bare();
            let stanza_id = message
                .payloads
                .iter()
                .find_map(|p| {
                    StanzaId::try_from(p.clone())
                        .ok()
                        .filter(|sid| sid.by.to_bare() == muc_bare)
                })
                .map(|sid| sid.id);
            let delay = match delay {
                Some(delay) => Some(delay.clone()),
                None => message
                    .payloads
                    .iter()
                    .find_map(|payload| Delay::try_from(payload.clone()).ok()),
            };
            let delayed = delay.is_some();
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
                        let mut msg = Message::outgoing_chat(
                            id,
                            timestamp,
                            &from,
                            &to,
                            bodies,
                            Some(oobs),
                            archive,
                        );
                        msg.set_encrypted(encrypted);
                        msg.set_stanza_id(stanza_id);
                        msg.set_delayed(delayed);
                        Ok(msg)
                    } else {
                        let mut msg = Message::incoming_chat(
                            id,
                            timestamp,
                            &from,
                            &to,
                            bodies,
                            Some(oobs),
                            archive,
                        );
                        msg.set_encrypted(encrypted);
                        msg.set_stanza_id(stanza_id);
                        msg.set_delayed(delayed);
                        Ok(msg)
                    }
                }
                XmppParsersMessageType::Groupchat => {
                    let mut msg = Message::incoming_channel(
                        id,
                        timestamp,
                        &from,
                        &to,
                        bodies,
                        Some(oobs),
                        archive,
                    );
                    msg.set_encrypted(encrypted);
                    msg.set_stanza_id(stanza_id);
                    msg.set_delayed(delayed);
                    Ok(msg)
                }
                _ => Err(()),
            }
        } else {
            Err(())
        }
    }

    pub fn set_encrypted(&mut self, encrypted: bool) {
        if let Message::Xmpp(ref mut xmpp) = self {
            xmpp.encrypted = encrypted;
        }
    }

    pub fn set_stanza_id(&mut self, stanza_id: Option<String>) {
        if let Message::Xmpp(ref mut xmpp) = self {
            xmpp.stanza_id = stanza_id;
        }
    }

    pub fn set_delayed(&mut self, delayed: bool) {
        if let Message::Xmpp(ref mut xmpp) = self {
            xmpp.delayed = delayed;
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
        bodies: HashMap<String, String>,
        oobs: Option<Vec<Oob>>,
        archive: bool,
    ) -> Self {
        let id = id.into();

        let version = XmppMessageVersion {
            id: id.clone(),
            timestamp,
            bodies,
            oobs: oobs.unwrap_or_default(),
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
            delayed: false,
            encrypted: false,
            stanza_id: None,
            reactions: HashMap::new(),
            delivery_status: DeliveryStatus::None,
        })
    }

    pub fn outgoing_chat<I: Into<String>>(
        id: I,
        timestamp: DateTime<FixedOffset>,
        from: &Jid,
        to: &Jid,
        bodies: HashMap<String, String>,
        oobs: Option<Vec<Oob>>,
        archive: bool,
    ) -> Self {
        let id = id.into();

        let version = XmppMessageVersion {
            id: id.clone(),
            timestamp,
            bodies,
            oobs: oobs.unwrap_or_default(),
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
            delayed: false,
            encrypted: false,
            stanza_id: None,
            reactions: HashMap::new(),
            delivery_status: DeliveryStatus::None,
        })
    }

    pub fn incoming_channel<I: Into<String>>(
        id: I,
        timestamp: DateTime<FixedOffset>,
        from: &Jid,
        to: &Jid,
        bodies: HashMap<String, String>,
        oobs: Option<Vec<Oob>>,
        archive: bool,
    ) -> Self {
        let id = id.into();

        let version = XmppMessageVersion {
            id: id.clone(),
            timestamp,
            bodies,
            oobs: oobs.unwrap_or_default(),
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
            delayed: false,
            encrypted: false,
            stanza_id: None,
            reactions: HashMap::new(),
            delivery_status: DeliveryStatus::None,
        })
    }

    pub fn outgoing_channel<I: Into<String>>(
        id: I,
        timestamp: DateTime<FixedOffset>,
        from: &Jid,
        to: &Jid,
        bodies: HashMap<String, String>,
        oobs: Option<Vec<Oob>>,
        archive: bool,
    ) -> Self {
        let id = id.into();

        let version = XmppMessageVersion {
            id: id.clone(),
            timestamp,
            bodies,
            oobs: oobs.unwrap_or_default(),
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
            delayed: false,
            encrypted: false,
            stanza_id: None,
            reactions: HashMap::new(),
            delivery_status: DeliveryStatus::None,
        })
    }

    pub fn log(msg: impl IntoCharxels) -> Self {
        Message::Log(LogMessage {
            id: Uuid::new_v4().to_string(),
            timestamp: LocalTz::now().into(),
            body: msg.into_charxels(),
        })
    }

    pub fn encryption_recipient(&self) -> Option<BareJid> {
        match self {
            Message::Log(_) => None,
            Message::Xmpp(message) => match message.direction {
                Direction::Outgoing => match message.type_ {
                    XmppMessageType::Chat => Some(message.to.clone()),
                    XmppMessageType::Channel => Some(message.to.clone()),
                },
                Direction::Incoming => None,
            },
        }
    }

    pub fn body(&self) -> Charxels {
        match self {
            Message::Xmpp(message) => message.get_last_body(vec![]).into_charxels(),
            Message::Log(LogMessage { body, .. }) => body.clone(),
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Message::Xmpp(VersionedXmppMessage { id, .. })
            | Message::Log(LogMessage { id, .. }) => id,
        }
    }

    pub fn timestamp(&self) -> &DateTime<FixedOffset> {
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
        self.timestamp()
            .cmp(other.timestamp())
            .then_with(|| self.id().cmp(other.id()))
    }
}

impl PartialOrd for Message {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl TryFrom<Message> for xmpp_parsers::minidom::Element {
    type Error = ();

    fn try_from(message: Message) -> Result<Self, Self::Error> {
        match message {
            Message::Log(_) => Err(()),
            Message::Xmpp(message) => match message.direction {
                Direction::Outgoing => match message.type_ {
                    XmppMessageType::Chat => {
                        let mut xmpp_message = xmpp_parsers::message::Message::new(Some(
                            Jid::from(message.to.clone()),
                        ));
                        xmpp_message.id = Some(XmppParsersMessageId(message.id.clone()));
                        xmpp_message.type_ = xmpp_parsers::message::MessageType::Chat;
                        xmpp_message.bodies = message
                            .get_last_bodies()
                            .map(|(lang, body)| (XmppParsersLang(lang.clone()), body.clone()))
                            .collect();
                        let request: xmpp_parsers::minidom::Element =
                            "<request xmlns='urn:xmpp:receipts'/>"
                                .parse()
                                .expect("valid XEP-0184 request");
                        xmpp_message.payloads.push(request);
                        Ok(xmpp_message.into())
                    }
                    XmppMessageType::Channel => {
                        let mut xmpp_message = xmpp_parsers::message::Message::new(Some(
                            Jid::from(message.to.clone()),
                        ));
                        xmpp_message.id = Some(XmppParsersMessageId(message.id.clone()));
                        xmpp_message.type_ = xmpp_parsers::message::MessageType::Groupchat;
                        xmpp_message.bodies = message
                            .get_last_bodies()
                            .map(|(lang, body)| (XmppParsersLang(lang.clone()), body.clone()))
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
    #[cfg(feature = "image")]
    image: Arc<RwLock<Option<SixelImage>>>,
    measure_cache: RefCell<Option<(u16, Vec<Charxels>)>>,
    selected: Cell<Option<BgColor>>,
    highlight: RefCell<Option<(String, FgColor, BgColor)>>,
    show_date_sep: Cell<bool>,
    date_sep_fg: FgColor,
    date_sep_bg: BgColor,
    preferred_langs: Vec<String>,
}

impl Eq for MessageView {}

impl PartialEq for MessageView {
    fn eq(&self, other: &Self) -> bool {
        self.message.eq(&other.message)
    }
}

impl PartialOrd for MessageView {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
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

fn charxel_with_styles(g: &str, styles: &HashSet<Style>) -> Charxel {
    let mut c = Charxel::new(g.into());
    for s in styles {
        c.add_style(*s);
    }
    c
}

fn span_marker_style(g: &str) -> Option<Style> {
    match g {
        "*" => Some(Style::Bold),
        "_" => Some(Style::Italic),
        "~" => Some(Style::CrossedOut),
        _ => None,
    }
}

// Returns the index of the matching closing marker, if any.
// Closing marker must not be preceded by whitespace and must be on the same line.
fn find_closing_span(graphemes: &[&str], open: usize) -> Option<usize> {
    let marker = graphemes[open];
    let mut j = open + 2;
    while j < graphemes.len() {
        if graphemes[j] == "\n" {
            return None;
        }
        if graphemes[j] == marker && !graphemes[j - 1].chars().all(char::is_whitespace) {
            return Some(j);
        }
        j += 1;
    }
    None
}

// XEP-0393 §2.1: parse inline spans (*bold*, _italic_, ~strike~) in `graphemes`,
// applying `styles` to all chars plus any additional style for each matched span.
fn push_spans(graphemes: &[&str], styles: &HashSet<Style>, out: &mut Charxels) {
    let mut i = 0;
    while i < graphemes.len() {
        let g = graphemes[i];
        if let Some(style) = span_marker_style(g) {
            // Opening rule: must be followed by non-whitespace (XEP-0393 §2.1)
            let next_nonws = graphemes
                .get(i + 1)
                .is_some_and(|n| !n.chars().all(char::is_whitespace));
            if next_nonws {
                if let Some(close) = find_closing_span(graphemes, i) {
                    // Marker chars inherit outer styles only
                    out.push(charxel_with_styles(g, styles));
                    let mut inner = styles.clone();
                    inner.insert(style);
                    push_spans(&graphemes[i + 1..close], &inner, out);
                    out.push(charxel_with_styles(graphemes[close], styles));
                    i = close + 1;
                    continue;
                }
            }
        }
        out.push(charxel_with_styles(g, styles));
        i += 1;
    }
}

// XEP-0393 §2.2: parse one message line, detecting block-quote prefix (`> `).
fn parse_message_line(line: &str) -> Charxels {
    let mut result = Charxels::default();
    if let Some(rest) = line.strip_prefix("> ") {
        let mut quote_style = HashSet::new();
        quote_style.insert(Style::Faint);
        for g in "> ".graphemes(true) {
            result.push(charxel_with_styles(g, &quote_style));
        }
        let rest_graphemes: Vec<&str> = rest.graphemes(true).collect();
        push_spans(&rest_graphemes, &HashSet::new(), &mut result);
    } else {
        let graphemes: Vec<&str> = line.graphemes(true).collect();
        push_spans(&graphemes, &HashSet::new(), &mut result);
    }
    result
}

fn langs_to_locale(langs: &[String]) -> Locale {
    for lang in langs {
        let posix = lang.replace('-', "_");
        if let Ok(locale) = posix.parse::<Locale>() {
            return locale;
        }
        if let Some(lang_only) = lang.split('-').next() {
            for country in &["US", "GB", "FR", "DE", "BR", "ES", "IT", "RU", "JP", "CN"] {
                if let Ok(locale) = format!("{}_{}", lang_only, country).parse::<Locale>() {
                    return locale;
                }
            }
        }
    }
    Locale::POSIX
}

impl MessageView {
    #[cfg(not(feature = "image"))]
    pub fn new(aparte: &mut AparteAsync, message: Message) -> Self {
        let theme = aparte.config.get_theme();
        MessageView {
            message,
            dimensions: None,
            measure_cache: RefCell::new(None),
            selected: Cell::new(None),
            highlight: RefCell::new(None),
            show_date_sep: Cell::new(false),
            date_sep_fg: theme.date_separator_fg,
            date_sep_bg: theme.date_separator_bg,
            preferred_langs: aparte.config.preferred_langs.clone(),
        }
    }

    #[cfg(feature = "image")]
    pub fn new(aparte: &mut AparteAsync, message: Message) -> Self {
        let image = match &message {
            Message::Xmpp(message) => message
                .history
                .iter()
                .max()
                .and_then(|version| {
                    version
                        .oobs
                        .iter()
                        .find(|oob| oob.url.ends_with(".jpg"))
                        .map(|oob| {
                            let image: Arc<RwLock<Option<SixelImage>>> =
                                Arc::new(RwLock::new(None));
                            Aparte::spawn({
                                let url = oob.url.clone();
                                let image = Arc::clone(&image);
                                let mut aparte = aparte.clone();
                                async move {
                                    log::debug!("Loading OOB: {}", url);
                                    match Self::load_oob(&url).await {
                                        Ok(sixel) => {
                                            log::debug!("Loaded OOB from {}", url);
                                            let mut image = image.write().unwrap();
                                            *image = Some(sixel);
                                            aparte.schedule(Event::UIRender(false));
                                        }
                                        Err(err) => log::error!("{}", err),
                                    }
                                }
                            });
                            image
                        })
                })
                .unwrap_or(Arc::new(RwLock::new(None))),
            Message::Log(_) => Arc::new(RwLock::new(None)),
        };
        let theme = aparte.config.get_theme();
        MessageView {
            message,
            dimensions: None,
            image,
            measure_cache: RefCell::new(None),
            selected: Cell::new(None),
            highlight: RefCell::new(None),
            show_date_sep: Cell::new(false),
            date_sep_fg: theme.date_separator_fg,
            date_sep_bg: theme.date_separator_bg,
            preferred_langs: aparte.config.preferred_langs.clone(),
        }
    }

    #[cfg(feature = "image")]
    async fn load_oob(url: &str) -> Result<SixelImage> {
        let client = reqwest::Client::new();
        let response = client
            .get(url)
            .timeout(std::time::Duration::from_secs(180))
            .send()
            .await?;
        log::debug!(
            "Got http response, expected image size: {:?}",
            response.content_length()
        );
        let raw_image = response.bytes().await?;
        log::debug!("Got raw image, size: {}", raw_image.len());
        let image_reader = ImageReader::new(Cursor::new(raw_image)).with_guessed_format()?;
        log::debug!("Guessed image format: {:?}", image_reader.format());
        let image = image_reader.decode()?;
        let image = image.resize_to_fill(300, 300, image::imageops::FilterType::Nearest);

        log::debug!("Convert {} to oob", url);
        convert_to_sixel(image)
    }

    fn format_log(message: &LogMessage, max_width: Option<u16>) -> Vec<Charxels> {
        let timestamp = Local.from_utc_datetime(&message.timestamp.naive_local());
        let mut lines = Vec::new();
        let prefixed_line = format!("{} - ", timestamp.format("%T")).into_charxels();
        for line in message.body.lines() {
            let mut formatted_line = prefixed_line.clone();
            formatted_line.append(line.clone());
            lines.append(&mut Self::format_text(formatted_line, max_width))
        }
        lines
    }

    fn format_header(message: &VersionedXmppMessage) -> Charxels {
        let author = terminus::clean_str(&match &message.type_ {
            XmppMessageType::Channel => match &message.from_full.try_as_full() {
                Ok(full_jid) => full_jid.resource().to_string(),
                Err(bare_jid) => bare_jid.to_string(),
            },
            XmppMessageType::Chat => message.from.to_string(),
        });

        let timestamp = Local.from_utc_datetime(&message.get_original_timestamp().naive_local());
        let body = message.get_last_body(vec![]);
        let me = body.starts_with("/me");

        let foreground = terminus::FgColor(id_to_rgb(&author));

        let mut attributes = "".to_string();
        if message.has_multiple_version() {
            attributes.push_str("✎ ");
        }
        if message.encrypted {
            attributes.push_str("🔒 ");
        } else {
            attributes.push_str("🔓 ");
        }
        if message.direction == Direction::Outgoing {
            match message.delivery_status {
                DeliveryStatus::Delivered => attributes.push_str("✓ "),
                DeliveryStatus::Displayed => attributes.push_str("✓✓ "),
                DeliveryStatus::None => {}
            }
        }

        let mut header = format!("{} - {}", timestamp.format("%T"), attributes).into_charxels();

        let author = match me {
            true => format!("* {} ", author).with_foreground(foreground),
            false => format!("{}: ", author).with_foreground(foreground),
        };

        header.append(author);

        header
    }

    fn format_xmpp_text(message: &VersionedXmppMessage, max_width: Option<u16>) -> Vec<Charxels> {
        let mut header = Self::format_header(message);

        let padding_len = header.display_width();
        let padding = " ".repeat(padding_len.into());

        let body = message.get_last_body(vec![]);
        let mut iter = body.strip_prefix("/me").unwrap_or(body).lines();

        if let Some(line) = iter.next() {
            header.append(parse_message_line(&terminus::clean_str(line)));
        }
        for line in iter {
            header.append(format!("\n{}", padding));
            header.append(parse_message_line(&terminus::clean_str(line)));
        }

        if !message.reactions.is_empty() {
            let mut counts: HashMap<&str, usize> = HashMap::new();
            for emojis in message.reactions.values() {
                for emoji in emojis {
                    *counts.entry(emoji.as_str()).or_insert(0) += 1;
                }
            }
            let mut sorted: Vec<(&str, usize)> = counts.into_iter().collect();
            sorted.sort_by_key(|(e, _)| *e);
            let reaction_str: String = sorted
                .iter()
                .map(|(e, n)| format!("{} {}", e, n))
                .collect::<Vec<_>>()
                .join("  ");
            header.append(format!("\n{}[{}]", padding, reaction_str));
        }

        Self::format_text(header, max_width)
    }

    fn format(&self, max_width: Option<u16>) -> Vec<Charxels> {
        let mut lines = match &self.message {
            Message::Log(message) => Self::format_log(message, max_width),
            Message::Xmpp(message) => Self::format_xmpp_text(message, max_width),
        };
        if self.show_date_sep.get() {
            let width = max_width.unwrap_or(80);
            let date = self.message.timestamp().with_timezone(&Local).date_naive();
            let sep = Self::format_date_separator(
                date,
                width,
                self.date_sep_fg,
                self.date_sep_bg,
                &self.preferred_langs,
            );
            lines.insert(0, sep);
        }
        lines
    }

    fn format_text(text: impl IntoCharxels, max_width: Option<u16>) -> Vec<Charxels> {
        let mut buffers: Vec<Charxels> = Vec::new();
        let text = text.into_charxels();
        for line in text.lines() {
            let mut line_len = 0;
            let mut chunk = Charxels::default();
            for word in line.split_word_bounds() {
                let display_width = word.iter().map(|c| c.display_width()).sum::<u16>();

                if max_width.is_some_and(|max_width| line_len + display_width > max_width) {
                    // Wrap line
                    buffers.push(chunk);
                    chunk = Charxels::default();
                    line_len = 0;
                }

                chunk.append(word.into_iter().cloned().collect::<Charxels>());
                line_len += display_width;
            }

            buffers.push(chunk);
        }

        buffers
    }

    pub fn select(&self, color: BgColor) {
        self.selected.set(Some(color));
    }

    pub fn deselect(&self) {
        self.selected.set(None);
    }

    pub fn set_highlight(&self, data: Option<(String, FgColor, BgColor)>) {
        *self.highlight.borrow_mut() = data;
    }

    pub fn set_show_date_sep(&self, show: bool) {
        self.show_date_sep.set(show);
        *self.measure_cache.borrow_mut() = None;
    }

    fn format_date_separator(
        date: NaiveDate,
        width: u16,
        fg: FgColor,
        bg: BgColor,
        preferred_langs: &[String],
    ) -> Charxels {
        let locale = langs_to_locale(preferred_langs);
        let label = date.format_localized(" %Y-%m-%d, %A ", locale).to_string();
        let label_len = label.chars().count() as u16;
        let line_width = width.saturating_sub(label_len);
        let left = "─".repeat((line_width / 2) as usize);
        let right = "─".repeat((line_width - line_width / 2) as usize);
        format!("{}{}{}", left, label, right)
            .with_foreground(fg)
            .with_background(bg)
    }

    fn render_text(&self, frame: &mut ScreenFrame) {
        let width = frame.width();
        let cache = self.measure_cache.borrow();
        let formatted_owned;
        let formatted = if cache.as_ref().map(|(w, _)| *w) == Some(width) {
            &cache.as_ref().unwrap().1
        } else {
            drop(cache);
            formatted_owned = self.format(Some(width));
            &formatted_owned
        };

        // Format as much as possible starting from bottom line
        for (top, line) in formatted[formatted.len() - frame.height() as usize..]
            .iter()
            .enumerate()
        {
            frame.write_at((0, top as u16), line);
        }
    }

    #[cfg(feature = "image")]
    fn render_image(&self, frame: &mut ScreenFrame) {
        let Message::Xmpp(message) = &self.message else {
            unreachable!()
        };

        frame.write(Self::format_header(message));
        if let Some(image) = self.image.read().unwrap().as_ref() {
            frame.write(image.into_charxels());
        } else {
            frame.write("…");
        }
    }

    fn measure_text(&self, measure_specs: &MeasureSpecs) -> RequestedDimensions {
        match measure_specs.width {
            MeasureSpec::Unspecified => RequestedDimensions {
                height: RequestedDimension::Absolute(1),
                width: RequestedDimension::Absolute(
                    self.format(None).first().map_or(0, |l| l.display_width()),
                ),
            },
            MeasureSpec::AtMost(at_most_width) => {
                let mut cache = self.measure_cache.borrow_mut();
                if cache.as_ref().map(|(w, _)| *w) != Some(at_most_width) {
                    *cache = Some((at_most_width, self.format(Some(at_most_width))));
                }
                let formatted = &cache.as_ref().unwrap().1;
                RequestedDimensions {
                    height: RequestedDimension::Absolute(formatted.len() as u16),
                    width: RequestedDimension::Absolute(cmp::min(
                        formatted
                            .iter()
                            .map(|line| line.display_width())
                            .max()
                            .unwrap_or(0),
                        at_most_width,
                    )),
                }
            }
        }
    }

    #[cfg(feature = "image")]
    fn measure_image(&self, _measure_specs: &MeasureSpecs) -> RequestedDimensions {
        let image = self.image.read().unwrap();
        let image = image.as_ref().unwrap();
        let ws = crossterm::terminal::window_size().expect("Can't get terminal size");

        let resolution = (ws.width / ws.columns, ws.height / ws.rows);

        let (x, y) = image.pixel_size();

        RequestedDimensions {
            height: RequestedDimension::Absolute((y as u16).div_ceil(resolution.1)),
            width: RequestedDimension::Absolute((x as u16).div_ceil(resolution.0)),
        }
    }
}

impl<E, C> View<E, C> for MessageView {
    fn measure(&self, measure_specs: &MeasureSpecs) -> RequestedDimensions {
        #[cfg(feature = "image")]
        if self.image.read().unwrap().is_some() {
            self.measure_image(measure_specs)
        } else {
            self.measure_text(measure_specs)
        }

        #[cfg(not(feature = "image"))]
        self.measure_text(measure_specs)
    }

    fn layout(&mut self, dimensions: &Dimensions) {
        log::debug!("layout {} {:?}", std::any::type_name::<Self>(), dimensions);

        self.dimensions.replace(dimensions.clone());
    }

    fn render(&self, mut frame: ScreenFrame, _config: &C) {
        log::debug!(
            "rendering {} at {:?}",
            std::any::type_name::<Self>(),
            self.dimensions
        );

        #[cfg(feature = "image")]
        if self.image.read().unwrap().is_some() {
            self.render_image(&mut frame)
        } else {
            self.render_text(&mut frame)
        }

        #[cfg(not(feature = "image"))]
        self.render_text(&mut frame);

        if let Some((ref query, fg, bg)) = *self.highlight.borrow() {
            frame.highlight_text(query, fg, bg);
        }

        if let Some(color) = self.selected.get() {
            frame.set_background(color);
        }
    }

    fn event(&mut self, _event: &mut E) {}
}

impl Searchable for MessageView {
    fn matches(&self, query: &str) -> bool {
        let lower = query.to_lowercase();
        match &self.message {
            Message::Xmpp(msg) => msg.get_last_body(vec![]).to_lowercase().contains(&lower),
            Message::Log(log_msg) => log_msg.body.to_string().to_lowercase().contains(&lower),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use chrono::Utc;
    use test_log::test;

    use terminus::rendering::{OffscreenRenderBuffer, ScreenFrame};

    use super::*;

    fn log_message_view(log: &str) -> (impl View<()>, DateTime<Local>) {
        let epoch: DateTime<Utc> = DateTime::from(UNIX_EPOCH);
        (
            MessageView {
                message: Message::Log(LogMessage {
                    id: String::from(""),
                    timestamp: epoch.into(),
                    body: String::from(log).into_charxels(),
                }),
                dimensions: None,
                #[cfg(feature = "image")]
                image: Arc::new(RwLock::new(None)),
                measure_cache: RefCell::new(None),
                selected: Cell::new(None),
                highlight: RefCell::new(None),
                show_date_sep: Cell::new(false),
                date_sep_fg: FgColor(terminus::Color::Default),
                date_sep_bg: BgColor(terminus::Color::Default),
                preferred_langs: vec![],
            },
            Local.from_utc_datetime(&epoch.naive_utc()),
        )
    }

    fn mock_buffer(dimensions: &Dimensions) -> OffscreenRenderBuffer {
        let mut buffer = OffscreenRenderBuffer::default();
        buffer.set_size(
            (
                dimensions.left + dimensions.width,
                dimensions.top + dimensions.height,
            )
                .into(),
        );
        buffer.clear();
        buffer
    }

    fn buffer_line(buffer: &OffscreenRenderBuffer, row: u16, from: u16, width: u16) -> String {
        (from..from + width)
            .map(|col| buffer[row][col].grapheme.to_string())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn test_render_single_line() {
        // Given
        let (mut message_view, timestamp) = log_message_view("a log");
        let dimensions = Dimensions {
            top: 1,
            left: 1,
            height: 1,
            width: 100,
        };
        let mut buffer = mock_buffer(&dimensions);

        // When
        message_view.layout(&dimensions);
        message_view.render(ScreenFrame::new(&mut buffer, &dimensions), &());

        // Then
        assert_eq!(
            buffer_line(&buffer, 1, 1, 100),
            format!("{} - a log", timestamp.format("%T"))
        );
    }

    #[test]
    fn test_render_multiple_lines() {
        // Given
        // a log that should render in more than width
        // 00:00:00 - a very very long long message log
        // is 44 char long but we allow only 2 lines of 40.
        let (mut message_view, timestamp) = log_message_view("a very very long long message log");
        let dimensions = Dimensions {
            top: 1,
            left: 1,
            height: 2,
            width: 40,
        };
        let mut buffer = mock_buffer(&dimensions);

        // When
        message_view.layout(&dimensions);
        message_view.render(ScreenFrame::new(&mut buffer, &dimensions), &());

        // Then
        // we should render:
        // 00:00:00 - a very very long long message
        //  log
        assert_eq!(
            buffer_line(&buffer, 1, 1, 40),
            format!("{} - a very very long long message", timestamp.format("%T"))
        );
        assert_eq!(buffer_line(&buffer, 2, 1, 40), " log");
    }

    #[test]
    fn test_render_partial_lines() {
        // Given
        // a log that should render in more than width
        // 00:00:00 - a very very long long message log
        // is 44 char long but we allow only 1 line of 40.
        let (mut message_view, _timestamp) = log_message_view("a very very long long message log");
        let dimensions = Dimensions {
            top: 1,
            left: 1,
            height: 1,
            width: 40,
        };
        let mut buffer = mock_buffer(&dimensions);

        // When
        message_view.layout(&dimensions);
        message_view.render(ScreenFrame::new(&mut buffer, &dimensions), &());

        // Then
        // we should only render:
        //  log
        assert_eq!(buffer_line(&buffer, 1, 1, 40), " log");
    }

    // ── XEP-0393 style parsing ───────────────────────────────────────────────

    fn styles(charxels: Charxels) -> Vec<(String, HashSet<Style>)> {
        charxels
            .into_iter()
            .map(|c| (c.grapheme.to_string(), c.styles))
            .collect()
    }

    #[test]
    fn test_plain_text_no_styles() {
        let result = styles(parse_message_line("hello world"));
        for (_, s) in &result {
            assert!(s.is_empty(), "plain text must have no styles");
        }
    }

    #[test]
    fn test_bold_span() {
        let result = styles(parse_message_line("*bold*"));
        assert_eq!(result.len(), 6);
        assert!(result[0].1.is_empty(), "opening * marker must be unstyled");
        for (g, s) in &result[1..5] {
            assert!(s.contains(&Style::Bold), "'{g}' must be Bold", g = g);
        }
        assert!(result[5].1.is_empty(), "closing * marker must be unstyled");
    }

    #[test]
    fn test_italic_span() {
        let result = styles(parse_message_line("_hi_"));
        assert!(result[0].1.is_empty(), "opening _ must be unstyled");
        assert!(result[1].1.contains(&Style::Italic));
        assert!(result[2].1.contains(&Style::Italic));
        assert!(result[3].1.is_empty(), "closing _ must be unstyled");
    }

    #[test]
    fn test_strikethrough_span() {
        let result = styles(parse_message_line("~del~"));
        assert_eq!(result.len(), 5);
        assert!(result[0].1.is_empty(), "opening ~ must be unstyled");
        assert!(result[1].1.contains(&Style::CrossedOut));
        assert!(result[2].1.contains(&Style::CrossedOut));
        assert!(result[3].1.contains(&Style::CrossedOut));
        assert!(result[4].1.is_empty(), "closing ~ must be unstyled");
    }

    #[test]
    fn test_opening_followed_by_space_is_not_a_span() {
        // XEP-0393 §2.1: opening directive must not be followed by whitespace
        let result = styles(parse_message_line("* text*"));
        for (_, s) in &result {
            assert!(
                s.is_empty(),
                "no span when opening marker followed by space"
            );
        }
    }

    #[test]
    fn test_closing_preceded_by_space_is_not_a_span() {
        // XEP-0393 §2.1: closing directive must not be preceded by whitespace
        let result = styles(parse_message_line("*text *"));
        for (_, s) in &result {
            assert!(
                s.is_empty(),
                "no span when closing marker preceded by space"
            );
        }
    }

    #[test]
    fn test_unmatched_opening_marker_no_style() {
        let result = styles(parse_message_line("*unclosed"));
        for (_, s) in &result {
            assert!(s.is_empty());
        }
    }

    #[test]
    fn test_empty_span_not_valid() {
        // ** has no content between markers so find_closing_span returns None
        let result = styles(parse_message_line("**"));
        assert_eq!(result.len(), 2);
        for (_, s) in &result {
            assert!(s.is_empty());
        }
    }

    #[test]
    fn test_multiple_spans_on_one_line() {
        let result = styles(parse_message_line("*a* _b_"));
        // *a* → unstyled *, Bold a, unstyled *
        assert!(result[0].1.is_empty());
        assert!(result[1].1.contains(&Style::Bold));
        assert!(result[2].1.is_empty());
        // space
        assert!(result[3].1.is_empty());
        // _b_ → unstyled _, Italic b, unstyled _
        assert!(result[4].1.is_empty());
        assert!(result[5].1.contains(&Style::Italic));
        assert!(result[6].1.is_empty());
    }

    #[test]
    fn test_nested_spans() {
        // *_bold italic_* — outer Bold, inner Bold+Italic
        let result = styles(parse_message_line("*_hi_*"));
        assert!(result[0].1.is_empty(), "outer * has no styles");
        // inner _ marker inherits outer Bold but not Italic
        assert!(result[1].1.contains(&Style::Bold));
        assert!(!result[1].1.contains(&Style::Italic), "_ marker not Italic");
        // content between _ markers has both
        assert!(result[2].1.contains(&Style::Bold));
        assert!(result[2].1.contains(&Style::Italic));
        assert!(result[3].1.contains(&Style::Bold));
        assert!(result[3].1.contains(&Style::Italic));
        // closing _ marker inherits Bold but not Italic
        assert!(result[4].1.contains(&Style::Bold));
        assert!(!result[4].1.contains(&Style::Italic));
        assert!(result[5].1.is_empty(), "outer * has no styles");
    }

    #[test]
    fn test_block_quote_prefix_is_faint() {
        // XEP-0393 §2.2: lines beginning with "> " are block quotes
        let result = styles(parse_message_line("> hello"));
        assert!(result[0].1.contains(&Style::Faint), "'>' must be Faint");
        assert!(result[1].1.contains(&Style::Faint), "' ' must be Faint");
        for (g, s) in &result[2..] {
            assert!(
                !s.contains(&Style::Faint),
                "'{g}' content must not be Faint",
                g = g
            );
        }
    }

    #[test]
    fn test_block_quote_content_supports_spans() {
        let result = styles(parse_message_line("> *hi*"));
        // "> " faint
        assert!(result[0].1.contains(&Style::Faint));
        assert!(result[1].1.contains(&Style::Faint));
        // * marker unstyled
        assert!(result[2].1.is_empty());
        // "hi" bold
        assert!(result[3].1.contains(&Style::Bold));
        assert!(result[4].1.contains(&Style::Bold));
        // closing * unstyled
        assert!(result[5].1.is_empty());
    }

    #[test]
    fn test_greater_than_without_space_is_not_a_block_quote() {
        // XEP-0393 §2.2 requires "> " (with space)
        let result = styles(parse_message_line(">no space"));
        for (_, s) in &result {
            assert!(
                !s.contains(&Style::Faint),
                "'>no space' must not be a quote"
            );
        }
    }
}
