use std::fmt;
use std::rc::Rc;
use std::hash::{Hash, Hasher};
use std::collections::HashSet;
use uuid::Uuid;
use xmpp_parsers::{Element, roster, ns, BareJid};
use xmpp_parsers::iq::{Iq, IqType};
use std::convert::TryFrom;

use crate::core::{Plugin, Aparte, Event};

#[derive(Clone, Debug)]
pub struct Contact {
    jid: BareJid,
    name: Option<String>,
}

impl Hash for Contact {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.jid.hash(state);
    }
}

impl PartialEq for Contact {
    fn eq(&self, other: &Self) -> bool {
        self.jid == other.jid
    }
}

impl Eq for Contact {}

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
                                self.contacts.insert(item.clone().into());
                                match item.name {
                                    Some(name) => info!("roster: {} ({})", name, item.jid),
                                    None => info!("roster: {}", item.jid),
                                };
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
