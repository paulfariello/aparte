/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::collections::{HashMap, HashSet};
use std::convert::TryFrom;
use std::fmt;
use std::str::FromStr;

use anyhow::{anyhow, Result};

use tokio_xmpp::IqResponse;
use xmpp_parsers::disco;
use xmpp_parsers::disco::Feature;
use xmpp_parsers::iq::Iq;
use xmpp_parsers::jid::Jid;
use xmpp_parsers::ns;

use crate::account::Account;
use crate::core::{Aparte, AparteAsync, Event, ModTrait};
use crate::i18n;

pub struct DiscoMod {
    identity: disco::Identity,
    client_features: HashSet<Feature>,
    server_features: HashMap<Account, Vec<String>>,
    jid_features: HashMap<(Account, Jid), Vec<String>>,
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
            jid_features: HashMap::new(),
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

    pub fn has_jid_feature(&self, account: &Account, jid: &Jid, feature: &str) -> bool {
        self.jid_features
            .get(&(account.clone(), jid.clone()))
            .map(|features| features.iter().any(|f| f == feature))
            .unwrap_or(false)
    }

    async fn get_jid_disco(aparte: &mut AparteAsync, account: &Account, jid: &Jid) -> Result<()> {
        match aparte
            .iq(account, Self::disco_info_query_iq(jid, None))
            .await
        {
            Ok(IqResponse::Result(Some(el))) => {
                if let Ok(disco) = disco::DiscoInfoResult::try_from(el) {
                    aparte.schedule(Event::JidDisco(
                        account.clone(),
                        jid.clone(),
                        disco.features.iter().map(|i| i.var.clone()).collect(),
                    ));
                    Ok(())
                } else {
                    Err(anyhow!("Cannot get jid disco info: invalid response"))
                }
            }
            Ok(IqResponse::Error(error)) => Err(anyhow!(
                "Cannot get jid disco info: {}",
                i18n::xmpp_err_to_string(&error, vec![]).1
            )),
            Ok(IqResponse::Result(None)) | Err(_) => {
                Err(anyhow!("Cannot get jid disco info: invalid response"))
            }
        }
    }

    async fn get_server_disco(
        aparte: &mut AparteAsync,
        account: &Account,
        jid: &Jid,
    ) -> Result<()> {
        match aparte
            .iq(
                account,
                Self::disco_info_query_iq(&Jid::from_str(jid.domain().as_ref()).unwrap(), None),
            )
            .await
        {
            Ok(IqResponse::Result(Some(el))) => {
                if let Ok(disco) = disco::DiscoInfoResult::try_from(el) {
                    aparte.schedule(Event::Disco(
                        account.clone(),
                        disco.features.iter().map(|i| i.var.clone()).collect(),
                    ));

                    Ok(())
                } else {
                    Err(anyhow!("Cannot get server disco info: invalid response"))
                }
            }
            Ok(IqResponse::Error(error)) => Err(anyhow!(
                "Cannot get server disco info: {}",
                i18n::xmpp_err_to_string(&error, vec![]).1
            )),
            Ok(IqResponse::Result(None)) | Err(_) => {
                Err(anyhow!("Cannot get server disco info: invalid response"))
            }
        }
    }

    fn disco_info_query_iq(jid: &Jid, node: Option<String>) -> Iq {
        let query = disco::DiscoInfoQuery { node };
        Iq::from_get("", query).with_to(jid.clone())
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

                Aparte::spawn({
                    let mut aparte = aparte.proxy();
                    let account = account.clone();
                    let jid = jid.clone();
                    async move {
                        if let Err(err) = Self::get_server_disco(&mut aparte, &account, &jid).await
                        {
                            crate::error!(aparte, err, "Cannot get server disco");
                        }
                    }
                });
            }
            Event::Disco(account, features) => {
                if let Some(server_features) = self.server_features.get_mut(account) {
                    server_features.extend(features.clone());
                }
            }
            Event::JidDisco(account, jid, features) => {
                self.jid_features
                    .entry((account.clone(), jid.clone()))
                    .or_default()
                    .extend(features.clone());
            }
            Event::Joined {
                account, channel, ..
            } => {
                let jid = Jid::from(channel.to_bare());
                Aparte::spawn({
                    let mut aparte = aparte.proxy();
                    let account = account.clone();
                    async move {
                        if let Err(err) = Self::get_jid_disco(&mut aparte, &account, &jid).await {
                            crate::error!(aparte, err, "Cannot get MUC disco");
                        }
                    }
                });
            }
            Event::Iq(account, iq) => {
                if let Iq::Get { payload: el, .. } = iq.clone() {
                    if let Ok(_disco) = disco::DiscoInfoQuery::try_from(el) {
                        let id = iq.id().to_string();
                        let disco = self.get_disco();
                        let iq = Iq::from_result(id, Some(disco));
                        aparte.send(account, iq);
                    }
                }
            }
            _ => {}
        }
    }
}

impl fmt::Display for DiscoMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0030: Service Discovery")
    }
}
