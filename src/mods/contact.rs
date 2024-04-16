/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;

use anyhow::Result;
use uuid::Uuid;
use xmpp_parsers::iq::{Iq, IqType};
use xmpp_parsers::{ns, presence, roster, BareJid};

use crate::account::Account;
use crate::contact;
use crate::core::{Aparte, AparteAsync, Event, ModTrait};

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
            subscription: item.subscription,
            presence: contact::Presence::Unavailable,
            groups,
        }
    }
}

#[derive(Eq, PartialEq, Hash)]
pub struct ContactIndex {
    account: Account,
    jid: BareJid,
}

pub struct ContactMod {
    pub contacts: HashMap<ContactIndex, contact::Contact>,
}

impl ContactMod {
    pub fn new() -> Self {
        Self {
            contacts: HashMap::new(),
        }
    }

    async fn get_roster(aparte: &mut AparteAsync, account: &Account) -> Result<()> {
        let response = aparte.iq(&account, Self::get_roster_iq()).await?;

        if let IqType::Result(Some(payload)) = response.payload.clone() {
            if payload.is("query", ns::ROSTER) {
                if let Ok(roster) = roster::Roster::try_from(payload) {
                    log::info!("Got roster");
                    for item in roster.items {
                        aparte.schedule(Event::Contact(account.clone(), item.into()));
                    }
                }
            }
        }

        Ok(())
    }

    fn get_roster_iq() -> Iq {
        let id = Uuid::new_v4().hyphenated().to_string();
        Iq::from_get(
            id,
            roster::Roster {
                ver: None,
                items: Vec::new(),
            },
        )
    }
}

impl ModTrait for ContactMod {
    fn init(&mut self, _aparte: &mut Aparte) -> Result<(), ()> {
        Ok(())
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::Connected(account, _jid) => {
                log::info!("Requesting roster");
                Aparte::spawn({
                    let mut aparte = aparte.proxy();
                    let account = account.clone();
                    async move {
                        if let Err(err) = Self::get_roster(&mut aparte, &account).await {
                            crate::error!(aparte, err, "Cannot sync OMEMO bundle");
                        }
                    }
                });
            }
            Event::Contact(account, contact) => {
                let index = ContactIndex {
                    account: account.clone(),
                    jid: contact.jid.clone(),
                };
                self.contacts.insert(index, contact.clone());
            }
            Event::Presence(account, presence) => {
                if let Some(from) = &presence.from {
                    let jid = from.clone().into_bare();
                    let index = ContactIndex {
                        account: account.clone(),
                        jid,
                    };
                    if let Some(contact) = self.contacts.get_mut(&index) {
                        contact.presence = match presence.show {
                            Some(presence::Show::Away) => contact::Presence::Away,
                            Some(presence::Show::Chat) => contact::Presence::Chat,
                            Some(presence::Show::Dnd) => contact::Presence::Dnd,
                            Some(presence::Show::Xa) => contact::Presence::Xa,
                            None => contact::Presence::Available,
                        };
                        aparte.schedule(Event::ContactUpdate(account.clone(), contact.clone()));
                    }
                }
            }
            _ => {}
        }
    }
}

impl fmt::Display for ContactMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Contact management")
    }
}
