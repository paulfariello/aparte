/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::collections::HashSet;
use std::convert::TryFrom;
use std::fmt;

use xmpp_parsers::delay::Delay;
use xmpp_parsers::legacy_omemo;
use xmpp_parsers::message::{Message as XmppParsersMessage, MessageType as XmppParsersMessageType};

use crate::account::Account;
use crate::core::{Aparte, Event, ModTrait};
use crate::message::{Direction, Message, VersionedXmppMessage, XmppMessageType};

#[derive(Default)]
pub struct MessageStoreMod {
    sent_muc_ids: HashSet<String>,
}

fn has_omemo_payload(message: &XmppParsersMessage) -> bool {
    message.payloads.iter().any(|p| {
        (p.name() == "encrypted"
            && p.ns() == xmpp_parsers::ns::LEGACY_OMEMO
            && legacy_omemo::Encrypted::try_from((*p).clone()).is_ok())
            || (p.name() == "encryption" && p.ns() == "urn:xmpp:eme:0")
    })
}

impl ModTrait for MessageStoreMod {
    fn init(&mut self, _aparte: &mut Aparte) -> Result<(), ()> {
        Ok(())
    }

    /// Claim OMEMO-encrypted Chat/Groupchat messages when we have stored cleartext
    /// for the message ID. OMEMO messages always carry a plaintext fallback body
    /// ("I sent you an OMEMO encrypted message…") so checking for an empty body is
    /// not sufficient. Instead we look up the cleartext store and return confidence
    /// 0.02 — beating MessagesMod (0.01) — only when a stored body is found.
    /// Messages with no stored cleartext fall through to MessagesMod which shows
    /// the fallback text.
    fn can_handle_xmpp_message(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        _delay: &Option<Delay>,
    ) -> f64 {
        let relevant_type = matches!(
            message.type_,
            XmppParsersMessageType::Chat | XmppParsersMessageType::Groupchat
        );
        if !relevant_type {
            return 0.0;
        }
        if !has_omemo_payload(message) {
            return 0.0;
        }
        let Some(id) = message.id.as_ref().map(|id| id.0.as_str()) else {
            log::debug!("message_store: OMEMO message has no id, cannot look up archive");
            return 0.0;
        };
        log::debug!(
            "message_store: archive lookup for OMEMO message id={id} from={:?}",
            message.from
        );
        match aparte.storage.get_message_cleartext(account, id) {
            Ok(Some(_)) => {
                log::debug!("message_store: found cleartext for {id}, claiming message");
                0.02
            }
            Ok(None) => {
                log::debug!("message_store: no cleartext stored for {id}, falling through");
                0.0
            }
            Err(e) => {
                log::debug!("message_store: archive lookup failed for {id}: {e}");
                0.0
            }
        }
    }

    /// Look up stored cleartext by message ID and emit Event::Message if found.
    /// The <encrypted> OMEMO payload is kept in the cloned stanza so that
    /// Message::from_xmpp sets encrypted=true, preserving the 🔒 lock icon.
    fn handle_xmpp_message(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        delay: &Option<Delay>,
        archive: bool,
    ) {
        let Some(id) = message.id.as_ref().map(|id| id.0.as_str()) else {
            return;
        };
        // MUC echoes of our own sent messages: already displayed via Event::SendMessage.
        if !archive
            && message.type_ == XmppParsersMessageType::Groupchat
            && self.sent_muc_ids.contains(id)
        {
            return;
        }
        match aparte.storage.get_message_cleartext(account, id) {
            Ok(Some(body)) => {
                let mut msg = message.clone();
                msg.bodies
                    .insert(xmpp_parsers::message::Lang(String::new()), body);
                // Some XMPP servers archive sent messages without stamping the
                // `from` attribute (they store the stanza as the client submitted
                // it). Message::from_xmpp returns Err(()) when `from` is absent,
                // so we default it to the account JID — making the message appear
                // as outgoing, which is correct for a message we sent.
                if msg.from.is_none() {
                    msg.from = Some(account.clone().into());
                }
                if let Ok(message) = Message::from_xmpp(account, &msg, delay, archive) {
                    aparte.schedule(Event::Message(Some(account.clone()), message));
                }
            }
            Ok(None) => {
                if archive {
                    log::warn!("No cleartext stored for archived OMEMO message {id}");
                }
            }
            Err(e) => log::error!("Cleartext store lookup failed: {e}"),
        }
    }

    /// Save cleartext for every successfully-decrypted OMEMO message.
    /// Guarded by !archive to avoid re-saving messages reconstructed from the
    /// store (the read path emits Event::Message with archive=true).
    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::SendMessage(_, message) => {
                if let Message::Xmpp(xmpp) = message {
                    if xmpp.type_ == XmppMessageType::Channel {
                        self.sent_muc_ids.insert(message.id().to_string());
                    }
                }
            }
            Event::Message(Some(account), Message::Xmpp(xmpp)) => {
                save_if_encrypted(aparte, account, xmpp);
            }
            _ => {}
        }
    }
}

fn save_if_encrypted(aparte: &mut Aparte, account: &Account, xmpp: &VersionedXmppMessage) {
    if !xmpp.encrypted {
        log::debug!("message_store: skip save id={} — not encrypted", xmpp.id);
        return;
    }
    if xmpp.archive {
        log::debug!(
            "message_store: skip save id={} — archive flag set (reconstructed from store)",
            xmpp.id
        );
        return;
    }
    let body = xmpp.get_last_body(vec![]);
    if body.is_empty() {
        log::debug!("message_store: skip save id={} — empty body", xmpp.id);
        return;
    }
    let conversation_jid = match xmpp.direction {
        Direction::Outgoing => xmpp.to.to_string(),
        Direction::Incoming => xmpp.from.to_string(),
    };
    let ts = xmpp.get_original_timestamp().to_rfc3339();
    log::debug!(
        "message_store: saving cleartext for id={} conversation={conversation_jid}",
        xmpp.id
    );
    if let Err(e) = aparte.storage.save_message_cleartext(
        account,
        &xmpp.id,
        &conversation_jid,
        body,
        &xmpp.from.to_string(),
        &ts,
        true,
    ) {
        log::error!("Failed to persist cleartext for {}: {e}", xmpp.id);
    }
}

impl fmt::Display for MessageStoreMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Message Store")
    }
}
