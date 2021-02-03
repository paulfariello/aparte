/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::convert::TryFrom;
use std::fmt;
use xmpp_parsers::message::Message as XmppParsersMessage;
use xmpp_parsers::message_correct::Replace;
use xmpp_parsers::ns;
use xmpp_parsers::delay::Delay;

use crate::account::Account;
use crate::core::{Aparte, Event, ModTrait};
use crate::message::Message;
use crate::mods::disco;
use crate::mods::messages;

pub struct CorrectionMod {}

impl CorrectionMod {
    pub fn new() -> Self {
        Self {}
    }

    fn handle_replace(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        replace: Replace,
    ) {
        let event = {
            let mut messages = aparte.get_mod_mut::<messages::MessagesMod>();
            if let Some(original) = messages.get_mut(&Some(account.clone()), &replace.id) {
                match original {
                    Message::Xmpp(original) => original.add_version_from_xmpp(message),
                    Message::Log(_) => error!(
                        "Can't replace a log message (conflicting id? {})",
                        replace.id
                    ),
                }
                Event::Message(Some(account.clone()), original.clone())
            } else {
                let mut message = message.clone();
                message.payloads = message
                    .payloads
                    .iter()
                    .filter(|payload| !payload.is("replace", ns::MESSAGE_CORRECT))
                    .cloned()
                    .collect();
                Event::RawMessage(account.clone(), message, None)
            }
        };
        aparte.schedule(event);
    }
}

impl ModTrait for CorrectionMod {
    fn init(&mut self, aparte: &mut Aparte) -> Result<(), ()> {
        let mut disco = aparte.get_mod_mut::<disco::DiscoMod>();
        disco.add_feature(ns::MESSAGE_CORRECT)
    }

    fn can_handle_xmpp_message(
        &mut self,
        _aparte: &mut Aparte,
        _account: &Account,
        message: &XmppParsersMessage,
        _delay: &Option<Delay>,
    ) -> f64 {
        for payload in message.payloads.iter().cloned() {
            if Replace::try_from(payload.clone()).is_ok() {
                return 1f64;
            }
        }

        return 0f64;
    }

    fn handle_xmpp_message(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        delay: &Option<Delay>,
    ) {
        for payload in message.payloads.iter().cloned() {
            if let Ok(replace) = Replace::try_from(payload.clone()) {
                self.handle_replace(aparte, account, message, replace);
            }
        }
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::RawMessage(account, message, _delay) => {
                for payload in message.payloads.iter().cloned() {
                    if let Ok(replace) = Replace::try_from(payload.clone()) {
                        self.handle_replace(aparte, account, message, replace);
                    }
                }
            }
            _ => {}
        }
    }
}

impl fmt::Display for CorrectionMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0280: Message Correction")
    }
}
