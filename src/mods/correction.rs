use std::collections::HashMap;
/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::convert::TryFrom;
use std::fmt;

use xmpp_parsers::delay::Delay;
use xmpp_parsers::message::Message as XmppParsersMessage;
use xmpp_parsers::message_correct::Replace;
use xmpp_parsers::ns;

use crate::account::Account;
use crate::core::{Aparte, Event, ModTrait};
use crate::message::Message;
use crate::mods::disco;
use crate::mods::messages;

#[derive(Default)]
pub struct CorrectionMod {
    waiting_corrections: HashMap<String, Vec<XmppParsersMessage>>,
}

impl CorrectionMod {
    fn handle_replace(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        replace: Replace,
        archive: bool,
    ) {
        let event = {
            let mut messages = aparte.get_mod_mut::<messages::MessagesMod>();
            if let Some(original) = messages.get_mut(&Some(account.clone()), &replace.id.0) {
                match original {
                    Message::Xmpp(original) => {
                        original.add_version_from_xmpp(message);
                        if !archive {
                            original.archive = archive;
                        }
                    }
                    Message::Log(_) => log::error!(
                        "Can't replace a log message (conflicting id? {})",
                        replace.id.0
                    ),
                }
                Some(Event::Message(Some(account.clone()), original.clone()))
            } else {
                log::info!(
                    "Missing original message: {} (correction from {:?})",
                    replace.id.0,
                    message.from
                );
                let waiting_corrections = self.waiting_corrections.entry(replace.id.0).or_default();
                waiting_corrections.push(message.clone());
                None
            }
        };

        if let Some(event) = event {
            aparte.schedule(event);
        }
    }

    #[allow(clippy::ref_option)]
    fn handle_original_message(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        delay: &Option<Delay>,
        archive: bool,
    ) {
        log::info!(
            "Got missing original message: {}, scheduling original and corrections",
            message.id.as_ref().unwrap().0
        );
        let original = Event::RawMessage {
            account: account.clone(),
            message: message.clone(),
            delay: delay.clone(),
            archive,
        };

        aparte.schedule(original);
        for correction in self
            .waiting_corrections
            .remove(&message.id.as_ref().unwrap().0) // id is not None (guaranteed by caller)
            .unwrap_or_default()
        {
            let correction = Event::RawMessage {
                account: account.clone(),
                message: correction,
                delay: None,
                archive,
            };
            aparte.schedule(correction);
        }
    }
}

impl ModTrait for CorrectionMod {
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
        for payload in &message.payloads {
            if Replace::try_from(payload.clone()).is_ok() {
                return 1f64;
            }
        }

        if let Some(id) = message.id.as_ref() {
            if self.waiting_corrections.contains_key(&id.0) {
                return 1f64;
            }
        }

        0f64
    }

    fn handle_xmpp_message(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        delay: &Option<Delay>,
        archive: bool,
    ) {
        if let Some(id) = message.id.as_ref() {
            if self.waiting_corrections.contains_key(&id.0) {
                self.handle_original_message(aparte, account, message, delay, archive);
            }
        }

        for payload in &message.payloads {
            if let Ok(replace) = Replace::try_from(payload.clone()) {
                self.handle_replace(aparte, account, message, replace, archive);
            }
        }
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        if let Event::RawMessage {
            account,
            message,
            delay: _,
            archive,
        } = event
        {
            for payload in &message.payloads {
                if let Ok(replace) = Replace::try_from(payload.clone()) {
                    self.handle_replace(aparte, account, message, replace, *archive);
                }
            }
        }
    }
}

impl fmt::Display for CorrectionMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0280: Message Correction")
    }
}
