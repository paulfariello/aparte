/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::convert::TryFrom;
use std::fmt;
use uuid::Uuid;
use xmpp_parsers::carbons;
use xmpp_parsers::forwarding::Forwarded;
use xmpp_parsers::iq::Iq;
use xmpp_parsers::ns;
use xmpp_parsers::Element;

use crate::account::Account;
use crate::core::{Aparte, Event, Plugin};
use crate::plugins::disco;

pub struct CarbonsPlugin {}

impl CarbonsPlugin {
    fn enable(&self) -> Element {
        let id = Uuid::new_v4().to_hyphenated().to_string();
        let iq = Iq::from_set(id, carbons::Enable);
        iq.into()
    }

    fn handle_carbon(&mut self, aparte: &mut Aparte, account: &Account, forwarded: Forwarded) {
        if let Some(message) = forwarded.stanza {
            aparte.handle_message(account.clone(), message, forwarded.delay);
        }
    }
}

impl Plugin for CarbonsPlugin {
    fn new() -> CarbonsPlugin {
        CarbonsPlugin {}
    }

    fn init(&mut self, aparte: &mut Aparte) -> Result<(), ()> {
        let mut disco = aparte.get_plugin_mut::<disco::Disco>().unwrap();
        disco.add_feature(ns::CARBONS)
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::Connected(account, _jid) => aparte.send(account, self.enable()),
            Event::MessagePayload(account, payload, _delay) => {
                if let Ok(received) = xmpp_parsers::carbons::Received::try_from(payload.clone()) {
                    self.handle_carbon(aparte, account, received.forwarded);
                } else if let Ok(sent) = xmpp_parsers::carbons::Sent::try_from(payload.clone()) {
                    self.handle_carbon(aparte, account, sent.forwarded);
                }
            }
            _ => {}
        }
    }
}

impl fmt::Display for CarbonsPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0280: Message Carbons")
    }
}
