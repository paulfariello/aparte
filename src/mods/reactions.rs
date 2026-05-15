/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;

use xmpp_parsers::delay::Delay;
use xmpp_parsers::jid::BareJid;
use xmpp_parsers::message::Message as XmppParsersMessage;
use xmpp_parsers::ns;
use xmpp_parsers::reactions::Reactions;

use crate::account::Account;
use crate::core::{Aparte, Event, ModTrait};
use crate::message::Message;
use crate::mods::{disco, messages};

#[derive(Default)]
pub struct ReactionsMod {
    waiting_reactions: HashMap<String, Vec<(BareJid, Vec<String>)>>,
}

impl ReactionsMod {
    fn apply_reactions(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        from: BareJid,
        reactions: Reactions,
    ) {
        let emojis: Vec<String> = reactions.reactions.into_iter().map(|r| r.emoji).collect();
        let id = reactions.id;

        let event = {
            let mut messages = aparte.get_mod_mut::<messages::MessagesMod>();
            // In MUC context the reactions id refers to the stanza_id, not the message id.
            // Resolve to the internal message id first so we can do a single get_mut.
            let resolved_id = if messages.get(&Some(account.clone()), &id).is_some() {
                Some(id.clone())
            } else {
                messages
                    .get_by_stanza_id(&Some(account.clone()), &id)
                    .map(|m| m.id().to_string())
            };

            if let Some(msg_id) = resolved_id {
                if let Some(Message::Xmpp(original)) =
                    messages.get_mut(&Some(account.clone()), &msg_id)
                {
                    original.update_reactions(from, emojis);
                    Some(Event::Message(
                        Some(account.clone()),
                        Message::Xmpp(original.clone()),
                    ))
                } else {
                    None
                }
            } else {
                self.waiting_reactions
                    .entry(id)
                    .or_default()
                    .push((from, emojis));
                None
            }
        };

        if let Some(event) = event {
            aparte.schedule(event);
        }
    }
}

impl ModTrait for ReactionsMod {
    fn init(&mut self, aparte: &mut Aparte) -> Result<(), ()> {
        let mut disco = aparte.get_mod_mut::<disco::DiscoMod>();
        disco.add_feature(ns::REACTIONS);
        Ok(())
    }

    fn can_handle_xmpp_message(
        &mut self,
        _aparte: &mut Aparte,
        _account: &Account,
        message: &XmppParsersMessage,
        _delay: &Option<Delay>,
    ) -> f64 {
        if message
            .payloads
            .iter()
            .any(|p| Reactions::try_from(p.clone()).is_ok())
        {
            return 1f64;
        }
        0f64
    }

    fn handle_xmpp_message(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        _delay: &Option<Delay>,
        _archive: bool,
    ) {
        let from = match &message.from {
            Some(jid) => jid.to_bare(),
            None => return,
        };
        for payload in &message.payloads {
            if let Ok(reactions) = Reactions::try_from(payload.clone()) {
                self.apply_reactions(aparte, account, from.clone(), reactions);
            }
        }
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        if let Event::Message(account, Message::Xmpp(msg)) = event {
            if let Some(pending) = self.waiting_reactions.remove(&msg.id) {
                // Apply all pending reactions directly to the incoming message rather
                // than looking it up in the messages store. MessagesMod.on_event may
                // not have run yet (HashMap iteration order is non-deterministic), so
                // the store might not contain this message yet.
                let mut updated = msg.clone();
                for (from, emojis) in pending {
                    updated.update_reactions(from, emojis);
                }
                aparte.schedule(Event::Message(account.clone(), Message::Xmpp(updated)));
            }
        }
    }
}

impl fmt::Display for ReactionsMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0444: Message Reactions")
    }
}
