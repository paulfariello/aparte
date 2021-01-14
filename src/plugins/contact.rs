/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use uuid::Uuid;
use xmpp_parsers::iq::{Iq, IqType};
use xmpp_parsers::{ns, presence, roster, BareJid, Element, Jid};

use crate::contact;
use crate::core::{Aparte, Event, Plugin};

impl From<roster::Group> for contact::Group {
    fn from(item: roster::Group) -> Self {
        Self(item.0)
    }
}

impl From<roster::Item> for contact::Contact {
    fn from(item: roster::Item) -> Self {
        let mut groups = Vec::new();
        for group in item.groups {
            groups.push(group.into());
        }

        Self {
            jid: item.jid.clone(),
            name: item.name.clone(),
            subscription: item.subscription.clone(),
            presence: contact::Presence::Unavailable,
            groups: groups,
        }
    }
}

pub struct ContactPlugin {
    pub contacts: HashMap<BareJid, contact::Contact>,
}

impl ContactPlugin {
    fn request(&self) -> Element {
        let id = Uuid::new_v4().to_hyphenated().to_string();
        let iq = Iq::from_get(
            id,
            roster::Roster {
                ver: None,
                items: Vec::new(),
            },
        );
        iq.into()
    }
}

impl Plugin for ContactPlugin {
    fn new() -> ContactPlugin {
        Self {
            contacts: HashMap::new(),
        }
    }

    fn init(&mut self, _aparte: &mut Aparte) -> Result<(), ()> {
        Ok(())
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::Connected(_jid) => aparte.send(self.request()),
            Event::Iq(iq) => {
                if let IqType::Result(Some(payload)) = iq.payload.clone() {
                    if payload.is("query", ns::ROSTER) {
                        if let Ok(roster) = roster::Roster::try_from(payload.clone()) {
                            for item in roster.items {
                                let contact: contact::Contact = item.clone().into();
                                self.contacts.insert(contact.jid.clone(), contact.clone());
                                aparte.schedule(Event::Contact(contact.clone()));
                            }
                        }
                    }
                }
            }
            Event::Presence(presence) => {
                if let Some(from) = &presence.from {
                    let jid = match from {
                        Jid::Bare(jid) => jid.clone(),
                        Jid::Full(jid) => jid.clone().into(),
                    };
                    if let Some(contact) = self.contacts.get_mut(&jid) {
                        contact.presence = match presence.show {
                            Some(presence::Show::Away) => contact::Presence::Away,
                            Some(presence::Show::Chat) => contact::Presence::Chat,
                            Some(presence::Show::Dnd) => contact::Presence::Dnd,
                            Some(presence::Show::Xa) => contact::Presence::Xa,
                            None => contact::Presence::Available,
                        };
                        aparte.schedule(Event::ContactUpdate(contact.clone()));
                    }
                }
            }
            _ => {}
        }
    }
}

impl fmt::Display for ContactPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Contact management")
    }
}
