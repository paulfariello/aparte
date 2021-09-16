/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use xmpp_parsers::{muc, BareJid, Jid};

use crate::account::Account;
use crate::conversation;
use crate::message;
use crate::core::{Aparte, Event, ModTrait};

#[derive(Eq, PartialEq, Hash)]
struct ConversationIndex {
    account: Account,
    jid: BareJid,
}

pub struct ConversationMod {
    /// Collections of currently opened conversations.
    conversations: HashMap<ConversationIndex, conversation::Conversation>,
}

impl ConversationMod {
    pub fn new() -> Self {
        Self {
            conversations: HashMap::new(),
        }
    }

    pub fn get<'a>(
        &'a self,
        account: &Account,
        jid: &BareJid,
    ) -> Option<&'a conversation::Conversation> {
        let index = ConversationIndex {
            account: account.clone(),
            jid: jid.clone(),
        };
        self.conversations.get(&index)
    }
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

impl ModTrait for ConversationMod {
    fn init(&mut self, _aparte: &mut Aparte) -> Result<(), ()> {
        Ok(())
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::Chat { account, contact } => {
                let conversation = conversation::Conversation::Chat(conversation::Chat {
                    account: account.clone(),
                    contact: contact.clone(),
                });

                let index = ConversationIndex {
                    account: account.clone(),
                    jid: contact.clone(),
                };
                self.conversations.insert(index, conversation);
            }
            Event::Message(account, message) => {
                if let message::Message::Xmpp(message) = message {
                    let account = account.as_ref().unwrap();
                    if message.type_ == message::XmppMessageType::Chat && message.direction == message::Direction::Incoming {
                        let index = ConversationIndex {
                            account: account.clone(),
                            jid: message.from.clone(),
                        };

                        if self.conversations.get(&index).is_none() {
                            let conversation = conversation::Conversation::Chat(conversation::Chat {
                                account: account.clone(),
                                contact: message.from.clone(),
                            });
                            self.conversations.insert(index, conversation);
                        }
                    }
                }
            }
            Event::Joined {
                account, channel, ..
            } => {
                let channel_jid: BareJid = channel.clone().into();
                let conversation = conversation::Conversation::Channel(conversation::Channel {
                    account: account.clone(),
                    jid: channel_jid.clone(),
                    nick: channel.resource.clone(),
                    name: None,
                    occupants: HashMap::new(),
                });

                let index = ConversationIndex {
                    account: account.clone(),
                    jid: channel_jid,
                };
                self.conversations.insert(index, conversation);
            }
            Event::Presence(account, presence) => {
                if let Some(Jid::Full(from)) = &presence.from {
                    let index = ConversationIndex {
                        account: account.clone(),
                        jid: from.clone().into(),
                    };
                    if let Some(conversation::Conversation::Channel(channel)) =
                        self.conversations.get_mut(&index)
                    {
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
                                    aparte.schedule(Event::Occupant {
                                        account: index.account.clone(),
                                        conversation: index.jid.clone(),
                                        occupant: occupant.clone(),
                                    });
                                    channel.occupants.insert(occupant.nick.clone(), occupant);
                                }
                            }
                        }
                    }
                }
            }
            Event::Leave(channel) => {
                self.conversations.remove(&channel.clone().into());
            }
            _ => {}
        }
    }
}

impl Into<ConversationIndex> for conversation::Channel {
    fn into(self) -> ConversationIndex {
        ConversationIndex {
            account: self.account,
            jid: self.jid,
        }
    }
}

impl Into<ConversationIndex> for conversation::Chat {
    fn into(self) -> ConversationIndex {
        ConversationIndex {
            account: self.account,
            jid: self.contact,
        }
    }
}

impl Into<ConversationIndex> for conversation::Conversation {
    fn into(self) -> ConversationIndex {
        match self {
            conversation::Conversation::Channel(channel) => channel.into(),
            conversation::Conversation::Chat(chat) => chat.into(),
        }
    }
}

impl fmt::Display for ConversationMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Conversations management")
    }
}
