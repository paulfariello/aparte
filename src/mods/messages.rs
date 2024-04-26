/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use xmpp_parsers::delay::Delay;
use xmpp_parsers::message::{Message as XmppParsersMessage, MessageType as XmppParsersMessageType};
use xmpp_parsers::ns;

use crate::account::Account;
use crate::core::{Aparte, Event, ModTrait};
use crate::message::Message;
use crate::mods::disco;

#[derive(Default)]
pub struct MessagesMod {
    messages: HashMap<Option<Account>, HashMap<String, Message>>,
}

impl MessagesMod {
    pub fn get<'a>(&'a self, account: &Option<Account>, id: &String) -> Option<&'a Message> {
        self.messages.get(account)?.get(id)
    }

    pub fn get_mut<'a>(
        &'a mut self,
        account: &Option<Account>,
        id: &String,
    ) -> Option<&'a mut Message> {
        self.messages.get_mut(account)?.get_mut(id)
    }

    pub fn handle_message(&mut self, account: &Option<Account>, message: &Message) {
        let messages = self.messages.entry(account.clone()).or_default();
        messages.insert(message.id().to_string(), message.clone());
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
                aparte.schedule(Event::PubSub {
                    account: account.clone(),
                    from: message.from.clone(),
                    event: pubsub_event,
                });
            }
        }
    }
}

impl ModTrait for MessagesMod {
    fn init(&mut self, aparte: &mut Aparte) -> Result<(), ()> {
        let mut disco = aparte.get_mod_mut::<disco::DiscoMod>();
        disco.add_feature(ns::MESSAGE_CORRECT);

        Ok(())
    }

    fn can_handle_xmpp_message(
        &mut self,
        _aparte: &mut Aparte,
        _account: &Account,
        message: &XmppParsersMessage,
        _delay: &Option<Delay>,
    ) -> f64 {
        match message.type_ {
            XmppParsersMessageType::Chat => {
                if message.bodies.is_empty() {
                    0f64
                } else {
                    0.01f64
                }
            }
            XmppParsersMessageType::Groupchat => {
                if message.bodies.is_empty() && message.subjects.is_empty() {
                    0f64
                } else {
                    0.01f64
                }
            }
            XmppParsersMessageType::Headline => {
                if message
                    .payloads
                    .iter()
                    .any(|p| p.is("event", ns::PUBSUB_EVENT))
                {
                    0.01f64
                } else {
                    0f64
                }
            }
            _ => 0f64,
        }
    }

    fn handle_xmpp_message(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        delay: &Option<Delay>,
        archive: bool,
    ) {
        match message.type_ {
            XmppParsersMessageType::Chat => {
                if let Ok(message) = Message::from_xmpp(account, message, delay, archive) {
                    aparte.schedule(Event::Message(Some(account.clone()), message));
                }
            }
            XmppParsersMessageType::Groupchat => {
                if !message.bodies.is_empty() {
                    if let Ok(message) = Message::from_xmpp(account, message, delay, archive) {
                        aparte.schedule(Event::Message(Some(account.clone()), message));
                    }
                }

                if !archive && !message.subjects.is_empty() {
                    if let Ok(destination) =
                        Message::get_local_destination_from_xmpp(account, message)
                    {
                        aparte.schedule(Event::Subject(
                            account.clone(),
                            destination.clone(),
                            message
                                .subjects
                                .iter()
                                .map(|(lang, subject)| (lang.clone(), subject.0.clone()))
                                .collect(),
                        ));
                    }
                }
            }
            XmppParsersMessageType::Headline => {
                self.handle_headline_message(aparte, account, message, delay)
            }
            XmppParsersMessageType::Error => {}
            XmppParsersMessageType::Normal => {}
        };
    }

    fn on_event(&mut self, _aparte: &mut Aparte, event: &Event) {
        if let Event::Message(account, message) = event {
            self.handle_message(account, message)
        }
    }
}

impl fmt::Display for MessagesMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Message store")
    }
}
