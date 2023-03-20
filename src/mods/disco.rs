/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::collections::{HashMap, HashSet};
use std::convert::TryFrom;
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;
use xmpp_parsers::disco;
use xmpp_parsers::disco::Feature;
use xmpp_parsers::iq::{Iq, IqType};
use xmpp_parsers::{ns, Element, Jid};

use crate::account::Account;
use crate::core::{Aparte, Event, ModTrait};

pub struct DiscoMod {
    identity: disco::Identity,
    client_features: HashSet<Feature>,
    server_features: HashMap<Account, Vec<String>>,
}

impl DiscoMod {
    pub fn new<C: Into<String>, T: Into<String>, L: Into<String>, N: Into<String>>(
        category: C,
        type_: T,
        lang: L,
        name: N,
    ) -> Self {
        Self {
            identity: disco::Identity::new(category, type_, lang, name),
            client_features: HashSet::new(),
            server_features: HashMap::new(),
        }
    }

    pub fn add_feature<S: Into<String>>(&mut self, feature: S) {
        let feature = Feature::new(feature.into());
        log::debug!("Adding `{}` feature", feature.var);
        self.client_features.insert(feature);
    }

    pub fn has_feature(&self, account: &Account, feature: &str) -> bool {
        self.server_features
            .get(account)
            .unwrap()
            .iter()
            .any(|i| i == feature)
    }

    pub fn disco(&mut self, jid: Jid, node: Option<String>) -> Element {
        let id = Uuid::new_v4().to_hyphenated().to_string();
        let query = disco::DiscoInfoQuery { node };
        let iq = Iq::from_get(id, query).with_to(Jid::from_str(&jid.domain()).unwrap());
        iq.into()
    }

    pub fn get_disco(&self) -> disco::DiscoInfoResult {
        let identities = vec![self.identity.clone()];
        disco::DiscoInfoResult {
            node: None,
            identities,
            features: self.client_features.iter().cloned().collect(),
            extensions: vec![],
        }
    }
}

impl ModTrait for DiscoMod {
    fn init(&mut self, _aparte: &mut Aparte) -> Result<(), ()> {
        self.add_feature(ns::DISCO_INFO);
        // TODO? self.add_feature(ns::DISCO_ITEMS);
        Ok(())
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::Connected(account, jid) => {
                self.server_features.insert(account.clone(), Vec::new());
                aparte.send(account, self.disco(jid.clone(), None));
            }
            Event::Iq(account, iq) => match iq.payload.clone() {
                IqType::Result(Some(el)) => {
                    if let Ok(disco) = disco::DiscoInfoResult::try_from(el) {
                        if let Some(features) = self.server_features.get_mut(account) {
                            features.extend(disco.features.iter().map(|i| i.var.clone()));
                            aparte.schedule(Event::Disco(account.clone()));
                        }
                    }
                }
                IqType::Get(el) => {
                    if let Ok(_disco) = disco::DiscoInfoQuery::try_from(el) {
                        let id = iq.id.clone();
                        let disco = self.get_disco();
                        let iq = Iq::from_result(id, Some(disco));
                        aparte.send(account, iq.into());
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

impl fmt::Display for DiscoMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0030: Service Discovery")
    }
}
