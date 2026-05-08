/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::fmt;

use xmpp_parsers::delay::Delay;
use xmpp_parsers::jid::Jid;
use xmpp_parsers::message::{Message as XmppParsersMessage, MessageType};
use xmpp_parsers::ns;

use crate::account::Account;
use crate::conversation::Conversation;
use crate::core::{Aparte, Event, ModTrait};
use crate::mods::conversation::ConversationMod;
use crate::mods::disco;

const CHAT_MARKERS_NS: &str = "urn:xmpp:chat-markers:0";

#[derive(Default)]
pub struct DisplayedMarkersMod;

impl ModTrait for DisplayedMarkersMod {
    fn init(&mut self, aparte: &mut Aparte) -> Result<(), ()> {
        let mut disco = aparte.get_mod_mut::<disco::DiscoMod>();
        disco.add_feature(CHAT_MARKERS_NS);
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
            if p.ns() == CHAT_MARKERS_NS && (p.name() == "displayed" || p.name() == "acknowledged")
            {
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
                let marker_id = message
                    .payloads
                    .iter()
                    .find(|p| {
                        p.ns() == CHAT_MARKERS_NS
                            && (p.name() == "displayed" || p.name() == "acknowledged")
                    })
                    .and_then(|p| p.attr("id").map(str::to_string));
                if let (Some(to), Some(id)) =
                    (message.to.as_ref().map(|j: &Jid| j.to_bare()), marker_id)
                {
                    aparte.schedule(Event::DisplayedMarker {
                        account: account.clone(),
                        jid: to,
                        id,
                    });
                }
            }
            MessageType::Groupchat => {
                let conv_jid = from.to_bare();

                // Per XEP-0333, in a MUC supporting XEP-0359, the marker id
                // references the MUC-assigned stanza-id. Only trust the marker
                // when the MUC has announced XEP-0359 support via disco.
                let muc_has_sid = aparte.get_mod::<disco::DiscoMod>().has_jid_feature(
                    account,
                    &Jid::from(conv_jid.clone()),
                    ns::SID,
                );
                if !muc_has_sid {
                    return;
                }

                let our_nick = {
                    let conv_mod = aparte.get_mod::<ConversationMod>();
                    match conv_mod.get(account, &conv_jid) {
                        Some(Conversation::Channel(ch)) => Some(ch.nick.clone()),
                        _ => None,
                    }
                };
                if let Some(nick) = our_nick {
                    if nick.as_str() == from.resource().as_ref() {
                        let marker_id = message
                            .payloads
                            .iter()
                            .find(|p| {
                                p.ns() == CHAT_MARKERS_NS
                                    && (p.name() == "displayed" || p.name() == "acknowledged")
                            })
                            .and_then(|p| p.attr("id").map(str::to_string));
                        if let Some(id) = marker_id {
                            aparte.schedule(Event::DisplayedMarker {
                                account: account.clone(),
                                jid: conv_jid,
                                id,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn on_event(&mut self, _aparte: &mut Aparte, _event: &Event) {}
}

impl fmt::Display for DisplayedMarkersMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0333: Chat markers")
    }
}
