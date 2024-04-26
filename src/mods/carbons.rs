/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::convert::TryFrom;
use std::fmt;

use uuid::Uuid;
use xmpp_parsers::carbons;
use xmpp_parsers::delay::Delay;
use xmpp_parsers::forwarding::Forwarded;
use xmpp_parsers::iq::Iq;
use xmpp_parsers::message::Message as XmppParsersMessage;
use xmpp_parsers::ns;
use xmpp_parsers::Element;

use crate::account::Account;
use crate::core::{Aparte, Event, ModTrait};
use crate::mods::disco;

#[derive(Default)]
pub struct CarbonsMod {}

impl CarbonsMod {
    fn enable(&self) -> Element {
        let id = Uuid::new_v4().hyphenated().to_string();
        let iq = Iq::from_set(id, carbons::Enable);
        iq.into()
    }

    fn handle_carbon(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        forwarded: Forwarded,
        archive: bool,
    ) {
        if let Some(message) = forwarded.stanza {
            aparte.schedule(Event::RawMessage {
                account: account.clone(),
                message,
                delay: forwarded.delay,
                archive,
            });
        }
    }
}

impl ModTrait for CarbonsMod {
    fn init(&mut self, aparte: &mut Aparte) -> Result<(), ()> {
        let mut disco = aparte.get_mod_mut::<disco::DiscoMod>();
        disco.add_feature(ns::CARBONS);

        Ok(())
    }

    fn can_handle_xmpp_message(
        &mut self,
        _aparte: &mut Aparte,
        _account: &Account,
        message: &XmppParsersMessage,
        _delay: &Option<Delay>,
    ) -> f64 {
        for payload in message.payloads.iter() {
            if carbons::Received::try_from(payload.clone()).is_ok()
                || carbons::Sent::try_from(payload.clone()).is_ok()
            {
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
        _delay: &Option<Delay>,
        archive: bool,
    ) {
        for payload in message.payloads.iter() {
            if let Ok(received) = carbons::Received::try_from(payload.clone()) {
                self.handle_carbon(aparte, account, received.forwarded, archive);
            } else if let Ok(sent) = carbons::Sent::try_from(payload.clone()) {
                self.handle_carbon(aparte, account, sent.forwarded, archive);
            }
        }
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        if let Event::Connected(account, _jid) = event {
            aparte.send(account, self.enable());
        }
    }
}

impl fmt::Display for CarbonsMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0280: Message Carbons")
    }
}
