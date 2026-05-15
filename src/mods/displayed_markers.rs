/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::collections::HashMap;
use std::fmt;

use xmpp_parsers::delay::Delay;
use xmpp_parsers::jid::{BareJid, Jid};
use xmpp_parsers::message::{Message as XmppParsersMessage, MessageType};
use xmpp_parsers::minidom::Element;
use xmpp_parsers::ns;

use crate::account::Account;
use crate::conversation::Conversation;
use crate::core::{Aparte, Event, ModTrait};
use crate::mods::conversation::ConversationMod;
use crate::mods::disco;
use crate::mods::ui::UIMod;

const CHAT_MARKERS_NS: &str = "urn:xmpp:chat-markers:0";

#[derive(Default)]
pub struct DisplayedMarkersMod {
    /// Pending markers for MUC windows: (account, `muc_bare_jid`) → `stanza_id`.
    ///
    /// An entry is added when MAM catchup finishes but either the MUC window
    /// is not currently focused or XEP-0359 disco isn't known yet.
    /// The entry is consumed and the marker is sent when both conditions are met.
    pending: HashMap<(Account, BareJid), String>,
}

impl DisplayedMarkersMod {
    fn send_marker(aparte: &mut Aparte, account: &Account, jid: &BareJid, sid: &str) {
        let marker: Element = format!("<displayed xmlns='{CHAT_MARKERS_NS}' id='{sid}'/>")
            .parse()
            .expect("valid displayed element");
        let mut msg = XmppParsersMessage::new(Some(Jid::from(jid.clone())));
        msg.type_ = MessageType::Groupchat;
        msg.payloads.push(marker);
        aparte.send(account, msg);
    }

    fn muc_has_sid(aparte: &mut Aparte, account: &Account, jid: &BareJid) -> bool {
        aparte.get_mod::<disco::DiscoMod>().has_jid_feature(
            account,
            &Jid::from(jid.clone()),
            ns::SID,
        )
    }

    fn current_window_is(aparte: &mut Aparte, jid: &BareJid) -> bool {
        aparte
            .get_mod::<UIMod>()
            .current_window()
            .is_some_and(|w| w == &jid.to_string())
    }
}

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
        let Some(from) = message
            .from
            .as_ref()
            .and_then(|j| j.clone().try_into_full().ok())
        else {
            return;
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
                if !Self::muc_has_sid(aparte, account, &conv_jid) {
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

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::MucMamComplete {
                account,
                jid,
                last_stanza_id: Some(sid),
            } => {
                // Send immediately if both conditions are met; defer otherwise.
                // "Disco not yet known" is treated as a defer — JidDisco will retry.
                if Self::muc_has_sid(aparte, account, jid) && Self::current_window_is(aparte, jid) {
                    Self::send_marker(aparte, account, jid, sid);
                } else {
                    self.pending
                        .insert((account.clone(), jid.clone()), sid.clone());
                }
            }
            Event::JidDisco(account, jid, features) => {
                let bare_jid = jid.to_bare();
                if let Some(sid) = self
                    .pending
                    .get(&(account.clone(), bare_jid.clone()))
                    .cloned()
                {
                    let has_sid = features.iter().any(|f| f == ns::SID);
                    if has_sid {
                        if Self::current_window_is(aparte, &bare_jid) {
                            self.pending.remove(&(account.clone(), bare_jid.clone()));
                            Self::send_marker(aparte, account, &bare_jid, &sid);
                        }
                        // Window not current: keep pending, ChangeWindow will handle it.
                    } else {
                        // MUC doesn't support XEP-0359 — drop the pending marker.
                        self.pending.remove(&(account.clone(), bare_jid.clone()));
                    }
                }
            }
            Event::ChangeWindow(name) => {
                if let Ok(jid) = BareJid::new(name) {
                    let keys: Vec<_> = self
                        .pending
                        .keys()
                        .filter(|(_, j)| j == &jid)
                        .cloned()
                        .collect();
                    for (account, jid) in keys {
                        if let Some(sid) =
                            self.pending.get(&(account.clone(), jid.clone())).cloned()
                        {
                            if Self::muc_has_sid(aparte, &account, &jid) {
                                self.pending.remove(&(account.clone(), jid.clone()));
                                Self::send_marker(aparte, &account, &jid, &sid);
                            }
                            // XEP-0359 not known yet: keep pending, JidDisco will handle it.
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

impl fmt::Display for DisplayedMarkersMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0333: Chat markers")
    }
}
