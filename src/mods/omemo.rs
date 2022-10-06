/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//use std::collections::HashMap;
//use std::convert::TryFrom;
use std::convert::TryFrom;
use std::fmt;
use std::str::FromStr;

use anyhow::Result;
use rand::random;
use uuid::Uuid;

//use xmpp_parsers::ns;
use xmpp_parsers::iq::{Iq, IqType};
use xmpp_parsers::legacy_omemo as Omemo;
use xmpp_parsers::ns;
use xmpp_parsers::pubsub;
use xmpp_parsers::pubsub::{ItemId, PubSub};
use xmpp_parsers::{BareJid, Jid};
//use xmpp_parsers::omemo;

use crate::account::Account;
use crate::core::{Aparte, Event, ModTrait};
//use crate::mods::disco::DiscoMod;

pub struct OmemoMod {
    //devices_cache: HashMap<BareJid, Vec<omemo::Device>>,
}

impl OmemoMod {
    pub fn new() -> Self {
        Self {
            //devices_cache: HashMap::new(),
        }
    }

    fn configure(&self, aparte: &mut Aparte, account: &Account) -> Result<()> {
        let device = match aparte.storage.get_omemo_device(account)? {
            Some(device) => device,
            None => {
                // TODO create device_id
                let device_id: i32 = random();
                aparte.storage.set_omemo_device(account, device_id)?
            }
        };

        debug!("{}", device.device_id);

        let mut aparte = aparte.proxy();
        let account = account.clone();

        Aparte::spawn(async move {
            let response = aparte
                .iq(&account, Self::get_devices(&account.clone().into()))
                .await;
            // TODO check namespace
            match response.payload {
                IqType::Result(Some(pubsub)) => {
                    if let Ok(PubSub::Items(items)) = PubSub::try_from(pubsub) {
                        let current = Some(ItemId("current".to_string()));
                        if let Some(current) = items.items.iter().find(|item| item.id == current) {
                            if let Some(payload) = &current.payload {
                                if let Ok(_list) = Omemo::DeviceList::try_from(payload.clone()) {
                                    //match list.devices.iter().find(|_device| todo!()) {
                                    //    Some(_device) => todo!(),
                                    //    None => todo!(),
                                    //};
                                }
                            }
                        }
                    }
                }
                IqType::Result(None) => todo!(),
                IqType::Error(_) => todo!(),
                _ => todo!(),
            };
        });

        Ok(())
    }

    //fn subscribe(&self, contact: &BareJid, subscriber: &BareJid) -> Iq {
    //    let id = Uuid::new_v4().to_hyphenated().to_string();
    //    let pubsub = pubsub::PubSub::Subscribe {
    //        subscribe: Some(pubsub::pubsub::Subscribe {
    //            node: Some(pubsub::NodeName::from_str(ns::OMEMO_DEVICES).unwrap()),
    //            jid: Jid::Bare(subscriber.clone()),
    //        }),
    //        options: None,
    //    };
    //    Iq::from_set(id, pubsub).with_to(Jid::Bare(contact.clone()))
    //}

    fn get_devices(contact: &BareJid) -> Iq {
        let id = Uuid::new_v4();

        let id = id.to_hyphenated().to_string();
        let items = pubsub::pubsub::Items {
            max_items: None,
            node: pubsub::NodeName::from_str(ns::LEGACY_OMEMO_DEVICELIST).unwrap(),
            subid: None,
            items: vec![],
        };
        let pubsub = pubsub::PubSub::Items(items);
        Iq::from_get(id, pubsub).with_to(Jid::Bare(contact.clone()))
    }

    //fn handle_devices(&mut self, contact: &BareJid, devices: &omemo::Devices) {
    //    info!("Updating OMEMO devices cache for {}", contact);
    //    let cache = self.devices_cache.entry(contact.clone()).or_insert(Vec::new());
    //    cache.extend(devices.devices.iter().cloned());
    //}
}

impl ModTrait for OmemoMod {
    fn init(&mut self, _aparte: &mut Aparte) -> Result<(), ()> {
        //let mut disco = aparte.get_mod_mut::<DiscoMod>();
        //disco.add_feature(ns::OMEMO_DEVICES);
        //disco.add_feature(format!("{}+notify", ns::OMEMO_DEVICES));

        Ok(())
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::Connected(account, _jid) => {
                if let Err(err) = self.configure(aparte, account) {
                    aparte.log(format!("Cannot configure OMEMO: {}", err));
                }
            }
            //Event::PubSub { account: _, from: Some(from), event } => match event {
            //    pubsub::PubSubEvent::PublishedItems { node, items } => {
            //        if node == &pubsub::NodeName::from_str(ns::OMEMO_DEVICES).unwrap() {
            //            for item in items {
            //                if item.id == pubsub::ItemId::from_str("current").ok() {
            //                    if let Some(payload) = item.payload.clone() {
            //                        if let Ok(devices) = omemo::Devices::try_from(payload) {
            //                            self.handle_devices(&from.clone().into(), &devices);
            //                        }
            //                    }
            //                }
            //            }
            //        }
            //    }
            //    _ => {}
            //}
            //Event::IqResult { account: _, uuid, from, payload } => {
            //    if let Some(jid) = self.pending_device_query.remove(&uuid) {
            //        if &Some(Jid::Bare(jid.clone())) != from {
            //            warn!("Mismatching from for pending iq request: {:?} != {:?}", jid, from);
            //        } else {
            //            if let Some(payload) = payload {
            //                if let Ok(devices) = omemo::Devices::try_from(payload.clone()) {
            //                    self.handle_devices(&jid, &devices);
            //                } else {
            //                    warn!("Malformed devices element in OMEMO iq result {}", uuid);
            //                }
            //            } else {
            //                warn!("Missing devices element in OMEMO iq result {}", uuid);
            //            }
            //        }
            //    }
            //}
            //Event::Chat { account, contact } => {
            //    //aparte.iq::<OmemoMod>(account, self.subscribe(contact, &account.clone().into()));
            //    //aparte.iq::<OmemoMod>(account, self.get_devices(contact));
            //}
            _ => {}
        }
    }
}

impl fmt::Display for OmemoMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0384: OMEMO")
    }
}
