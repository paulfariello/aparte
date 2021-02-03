/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use chrono::Local as LocalTz;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use uuid::Uuid;
use xmpp_parsers::delay::Delay;
use xmpp_parsers::message::{Message as XmppParsersMessage, MessageType as XmppParsersMessageType};
use xmpp_parsers::ns;

use crate::account::Account;
use crate::core::{Aparte, Event, ModTrait};
use crate::message::Message;
use crate::mods::disco;

pub struct MessagesMod {
    messages: HashMap<(Option<Account>, String), Message>,
}

impl MessagesMod {
    pub fn new() -> Self {
        Self {
            messages: HashMap::new(),
        }
    }

    pub fn handle_message(&mut self, account: &Option<Account>, message: &Message) {
        self.messages
            .insert((account.clone(), message.id().to_string()), message.clone());
    }

    fn handle_chat_message(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        delay: &Option<Delay>,
    ) {
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

            let message =
                if from.clone().node() == account.node && from.clone().domain() == account.domain {
                    Message::outgoing_chat(
                        id,
                        delay
                            .map(|delay| delay.stamp.0)
                            .unwrap_or(LocalTz::now().into()),
                        &from,
                        &to,
                        &bodies,
                    )
                } else {
                    Message::incoming_chat(
                        id,
                        delay
                            .map(|delay| delay.stamp.0)
                            .unwrap_or(LocalTz::now().into()),
                        &from,
                        &to,
                        &bodies,
                    )
                };
            aparte.schedule(Event::Message(Some(account.clone()), message));
        }
    }

    fn handle_channel_message(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        delay: &Option<Delay>,
    ) {
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
            let message = Message::incoming_channel(
                id,
                delay
                    .map(|delay| delay.stamp.0)
                    .unwrap_or(LocalTz::now().into()),
                &from,
                &to,
                &bodies,
            );
            aparte.schedule(Event::Message(Some(account.clone()), message));
        }
    }

    fn handle_headline_message(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        _delay: &Option<Delay>,
    ) {
        for payload in message.payloads.iter().cloned() {
            if let Ok(pubsub_event) = xmpp_parsers::pubsub::event::PubSubEvent::try_from(payload) {
                // TODO move to pubsub mod
                aparte.schedule(Event::PubSub(account.clone(), pubsub_event));
            }
        }
    }
}

impl ModTrait for MessagesMod {
    fn init(&mut self, aparte: &mut Aparte) -> Result<(), ()> {
        let mut disco = aparte.get_mod_mut::<disco::DiscoMod>().unwrap();
        disco.add_feature(ns::MESSAGE_CORRECT)
    }

    fn can_handle_xmpp_message(
        &mut self,
        _aparte: &mut Aparte,
        _account: &Account,
        message: &XmppParsersMessage,
        _delay: &Option<Delay>,
    ) -> f64 {
        if message.bodies.is_empty() {
            return 0f64;
        }

        return 0.01f64;
    }

    fn handle_xmpp_message(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        delay: &Option<Delay>,
    ) {
        match message.type_ {
            XmppParsersMessageType::Chat => {
                self.handle_chat_message(aparte, account, message, delay)
            }
            XmppParsersMessageType::Groupchat => {
                self.handle_channel_message(aparte, account, message, delay)
            }
            XmppParsersMessageType::Headline => {
                self.handle_headline_message(aparte, account, message, delay)
            }
            XmppParsersMessageType::Error => {}
            XmppParsersMessageType::Normal => {}
        };
    }

    fn on_event(&mut self, _aparte: &mut Aparte, event: &Event) {
        match event {
            Event::Message(account, message) => self.handle_message(account, message),
            _ => {}
        }
    }
}

impl fmt::Display for MessagesMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Message store")
    }
}
