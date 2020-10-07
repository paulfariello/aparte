/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use xmpp_parsers::{Jid, BareJid, muc};

use crate::core::{Plugin, Aparte, Event};
use crate::conversation;

pub struct ConversationPlugin {
    conversations: HashMap<String, conversation::Conversation>,
}

impl ConversationPlugin {
}

impl From<muc::user::Role> for conversation::Role {
    fn from(role: muc::user::Role) -> Self {
        match role {
            muc::user::Role::Moderator => conversation::Role::Moderator,
            muc::user::Role::Participant => conversation::Role::Participant,
            muc::user::Role::Visitor => conversation::Role::Visitor,
            muc::user::Role::None => conversation::Role::None,
        }
    }
}

impl From<muc::user::Affiliation> for conversation::Affiliation {
    fn from(role: muc::user::Affiliation) -> Self {
        match role {
            muc::user::Affiliation::Owner => conversation::Affiliation::Owner,
            muc::user::Affiliation::Admin => conversation::Affiliation::Admin,
            muc::user::Affiliation::Member => conversation::Affiliation::Member,
            muc::user::Affiliation::Outcast => conversation::Affiliation::Outcast,
            muc::user::Affiliation::None => conversation::Affiliation::None,
        }
    }
}

impl Plugin for ConversationPlugin {
    fn new() -> ConversationPlugin {
        Self {
            conversations: HashMap::new(),
        }
    }

    fn init(&mut self, _aparte: &mut Aparte) -> Result<(), ()> {
        Ok(())
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::Chat(jid) => {
                let conversation = conversation::Conversation::Chat(conversation::Chat {
                    contact: jid.clone(),
                });
                self.conversations.insert(jid.to_string(), conversation);
            },
            Event::Joined(jid, _) => {
                let channel_jid: BareJid = jid.clone().into();
                let conversation = conversation::Conversation::Channel(conversation::Channel {
                    jid: channel_jid.clone(),
                    nick: jid.resource.clone(),
                    name: None,
                    occupants: HashMap::new(),
                });
                self.conversations.insert(channel_jid.to_string(), conversation);
            },
            Event::Presence(presence) => {
                if let Some(Jid::Full(from)) = &presence.from {
                    let channel_jid: BareJid = from.clone().into();
                    if let Some(conversation::Conversation::Channel(channel)) = self.conversations.get_mut(&channel_jid.to_string()) {
                        for payload in presence.clone().payloads {
                            if let Some(muc_user) = muc::user::MucUser::try_from(payload).ok() {
                                for item in muc_user.items {
                                    let occupant_jid = match item.jid {
                                        Some(full) => Some(full.into()),
                                        None => None,
                                    };
                                    let occupant = conversation::Occupant {
                                        nick: from.resource.clone(),
                                        jid: occupant_jid,
                                        affiliation: item.affiliation.into(),
                                        role: item.role.into(),
                                    };
                                    aparte.schedule(Event::Occupant{conversation: channel_jid.clone(), occupant: occupant.clone()});
                                    channel.occupants.insert(occupant.nick.clone(), occupant);
                                }
                            }
                        }
                    }
                }
            },
            _ => {},
        }
    }
}

impl fmt::Display for ConversationPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Conversations management")
    }
}
