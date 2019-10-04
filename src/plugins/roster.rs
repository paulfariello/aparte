use std::fmt;
use std::rc::Rc;
use std::collections::HashSet;
use uuid::Uuid;
use xmpp_parsers::{Element, roster, ns};
use xmpp_parsers::iq::{Iq, IqType};
use std::convert::TryFrom;

use crate::core::{Plugin, Aparte, Event, Contact};

impl From<roster::Item> for Contact {
    fn from(item: roster::Item) -> Self {
        Self {
            jid: item.jid.clone(),
            name: item.name.clone(),
        }
    }
}

pub struct RosterPlugin {
    contacts: HashSet<Contact>,
}

impl RosterPlugin {
    fn request(&self) -> Element {
        let id = Uuid::new_v4().to_hyphenated().to_string();
        let iq = Iq::from_get(id, roster::Roster { ver: None, items: Vec::new() });
        iq.into()
    }
}

impl Plugin for RosterPlugin {
    fn new() -> RosterPlugin {
        RosterPlugin {
            contacts: HashSet::new(),
        }
    }

    fn init(&mut self, _aparte: &Aparte) -> Result<(), ()> {
        Ok(())
    }

    fn on_event(&mut self, aparte: Rc<Aparte>, event: &Event) {
        match event {
            Event::Connected(_jid) => aparte.send(self.request()),
            Event::Iq(iq) => {
                if let IqType::Result(Some(payload)) = iq.payload.clone() {
                    if payload.is("query", ns::ROSTER) {
                        if let Ok(roster) = roster::Roster::try_from(payload) {
                            for item in roster.items {
                                let contact: Contact = item.clone().into();
                                self.contacts.insert(contact.clone());
                                Rc::clone(&aparte).event(Event::Contact(contact.clone()));
                            }
                        }
                    }
                }
            },
            _ => {},
        }
    }
}

impl fmt::Display for RosterPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Roster management")
    }
}
