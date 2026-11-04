/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::convert::TryFrom;
use std::fmt;

use xmpp_parsers::delay::Delay;
use xmpp_parsers::message::{Message as XmppParsersMessage, MessageType};
use xmpp_parsers::ns;
use xmpp_parsers::receipts;

use crate::account::Account;
use crate::core::{Aparte, Event, ModTrait};
use crate::message::{DeliveryStatus, Direction, Message};
use crate::mods::{disco, messages};

const CHAT_MARKERS_NS: &str = "urn:xmpp:chat-markers:0";

pub struct DeliveryMod;

impl DeliveryMod {
    fn update_delivery(aparte: &mut Aparte, account: &Account, id: &str, status: DeliveryStatus) {
        let event = {
            let mut messages_mod = aparte.get_mod_mut::<messages::MessagesMod>();
            let account_key = Some(account.clone());
            if let Some(Message::Xmpp(xmpp)) = messages_mod.get_mut(&account_key, &id.to_string()) {
                if xmpp.direction == Direction::Outgoing && xmpp.delivery_status < status {
                    xmpp.delivery_status = status;
                    Some(Event::Message(account_key, Message::Xmpp(xmpp.clone())))
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(event) = event {
            aparte.schedule(event);
        }
    }
}

impl ModTrait for DeliveryMod {
    fn init(&mut self, aparte: &mut Aparte) -> Result<(), ()> {
        let mut disco_mod = aparte.get_mod_mut::<disco::DiscoMod>();
        disco_mod.add_feature(ns::RECEIPTS);
        Ok(())
    }

    fn can_handle_xmpp_message(
        &mut self,
        _aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        _delay: &Option<Delay>,
    ) -> f64 {
        if message.type_ != MessageType::Chat {
            return 0.0;
        }
        let from_self = message
            .from
            .as_ref()
            .is_some_and(|j| j.to_bare() == account.to_bare());
        if from_self {
            return 0.0;
        }
        for p in &message.payloads {
            let is_receipt = receipts::Received::try_from(p.clone()).is_ok();
            let is_displayed = p.ns() == CHAT_MARKERS_NS
                && (p.name() == "displayed" || p.name() == "acknowledged");
            if is_receipt || is_displayed {
                return 0.6;
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
        let Some(from) = message
            .from
            .as_ref()
            .and_then(|j| j.clone().try_into_full().ok())
        else {
            return;
        };
        for p in &message.payloads {
            if let Ok(received) = receipts::Received::try_from(p.clone()) {
                aparte.schedule(Event::MessageDelivered {
                    account: account.clone(),
                    jid: from.to_bare(),
                    id: received.id,
                });
            }
            if p.ns() == CHAT_MARKERS_NS && (p.name() == "displayed" || p.name() == "acknowledged")
            {
                if let Some(id) = p.attr("id").map(str::to_string) {
                    aparte.schedule(Event::DisplayedMarker {
                        account: account.clone(),
                        jid: from.to_bare(),
                        id,
                    });
                }
            }
        }
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::MessageDelivered { account, id, .. } => {
                Self::update_delivery(aparte, account, id, DeliveryStatus::Delivered);
            }
            Event::DisplayedMarker { account, id, .. } => {
                Self::update_delivery(aparte, account, id, DeliveryStatus::Displayed);
            }
            _ => {}
        }
    }
}

impl fmt::Display for DeliveryMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0184/XEP-0333: Message delivery status")
    }
}
