/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::convert::TryFrom;
use std::fmt;

use xmpp_parsers::delay::Delay;
use xmpp_parsers::jid::Jid;
use xmpp_parsers::message::{Message as XmppParsersMessage, MessageType};
use xmpp_parsers::receipts;

use crate::account::Account;
use crate::conversation::Conversation;
use crate::core::{Aparte, Event, ModTrait};
use crate::mods::conversation::ConversationMod;

const CHAT_MARKERS_NS: &str = "urn:xmpp:chat-markers:0";

#[derive(Default)]
pub struct ReadMarkersMod;

impl ModTrait for ReadMarkersMod {
    fn init(&mut self, _aparte: &mut Aparte) -> Result<(), ()> {
        Ok(())
    }

    fn can_handle_xmpp_message(
        &mut self,
        _aparte: &mut Aparte,
        _account: &Account,
        message: &XmppParsersMessage,
        _delay: &Option<Delay>,
    ) -> f64 {
        for p in &message.payloads {
            let is_xep184 = receipts::Received::try_from(p.clone()).is_ok();
            let is_xep333 = p.ns() == CHAT_MARKERS_NS
                && (p.name() == "displayed" || p.name() == "acknowledged");
            if is_xep184 || is_xep333 {
                return 0.5;
            }
        }
        0.0
    }

    fn handle_xmpp_message(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        _delay: &Option<Delay>,
        _archive: bool,
    ) {
        let from = match message
            .from
            .as_ref()
            .and_then(|j| j.clone().try_into_full().ok())
        {
            Some(f) => f,
            None => return,
        };

        match message.type_ {
            MessageType::Chat if from.to_bare() == account.to_bare() => {
                if let Some(to) = message.to.as_ref().map(|j: &Jid| j.to_bare()) {
                    aparte.schedule(Event::ReadMarker {
                        account: account.clone(),
                        jid: to,
                    });
                }
            }
            MessageType::Groupchat => {
                let conv_jid = from.to_bare();
                let our_nick = {
                    let conv_mod = aparte.get_mod::<ConversationMod>();
                    match conv_mod.get(account, &conv_jid) {
                        Some(Conversation::Channel(ch)) => Some(ch.nick.clone()),
                        _ => None,
                    }
                };
                if let Some(nick) = our_nick {
                    if nick.as_str() == from.resource().as_ref() {
                        aparte.schedule(Event::ReadMarker {
                            account: account.clone(),
                            jid: conv_jid,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    fn on_event(&mut self, _aparte: &mut Aparte, _event: &Event) {}
}

impl fmt::Display for ReadMarkersMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0184/XEP-0333: Read markers")
    }
}
