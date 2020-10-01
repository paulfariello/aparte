/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::fmt;
use uuid::Uuid;
use std::convert::TryFrom;
use xmpp_parsers::disco;
use xmpp_parsers::iq::{Iq, IqType};
use xmpp_parsers::Element;

use crate::core::{Aparte, Event, Plugin};

pub struct Disco {
    client_features: Vec<String>,
    server_features: Vec<String>,
}

impl Disco {
    pub fn add_feature(&mut self, feature: &str) -> Result<(), ()> {
        debug!("Adding `{}` feature", feature);
        self.client_features.push(feature.to_string());

        Ok(())
    }

    pub fn has_feature(&self, feature: &str) -> bool {
        debug!("Adding `{}` feature", feature);
        self.server_features.iter().any(|i| i == feature)
    }

    pub fn disco(&mut self) -> Element {
        let id = Uuid::new_v4().to_hyphenated().to_string();
        let query = disco::DiscoInfoQuery { node: None };
        let iq = Iq::from_get(id, query);
        iq.into()
    }
}

impl Plugin for Disco {
    fn new() -> Disco {
        Disco {
            client_features: Vec::new(),
            server_features: Vec::new(),
        }
    }

    fn init(&mut self, _aparte: &mut Aparte) -> Result<(), ()> {
        Ok(())
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::Connected(_jid) => {
                aparte.send(self.disco());
            }
            Event::Iq(iq) => match iq.payload.clone() {
                IqType::Result(Some(el)) => {
                    if let Ok(disco) = disco::DiscoInfoResult::try_from(el) {
                        self.server_features.extend(disco.features.iter().map(|i| i.var.clone()));
                        aparte.schedule(Event::Disco);
                    }
                },
                _ => {}
            },
            _ => {}
        }
    }
}

impl fmt::Display for Disco {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0030: Service Discovery")
    }
}
