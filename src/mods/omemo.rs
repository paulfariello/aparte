/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::fmt;
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
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
use crate::command::{Command, CommandParser};
use crate::core::{Aparte, AparteAsync, Event, ModTrait};
use crate::i18n;
//use crate::mods::disco::DiscoMod;
use crate::mods::ui::UIMod;

command_def!(omemo_enable,
r#"/omemo enable [<jid>]

    jid    jid of the OMEMO enabled contact/channel

Description:
    Enable OMEMO on a given contact/channel

Examples:
    /omemo enable
    /omemo enable aparte@conference.fariello.eu
"#,
{
    jid: Option<String>,
},
|aparte, _command| {
    let current =  {
        let ui = aparte.get_mod::<UIMod>();
        ui.current_window().cloned()
    };
    let jid = jid.or(current).clone();
    if let Some(jid) = jid {
        if let Some(account) = aparte.current_account() {
            if let Ok(jid) = BareJid::from_str(&jid) {
                aparte.schedule(Event::Omemo(OmemoEvent::Enable { account, jid }));
            }
        }
    }
    Ok(())
});

command_def!(omemo,
r#"/omemo enable"#,
{
    action: Command = {
        children: {
            "enable": omemo_enable,
        }
    },
});

#[derive(Debug, Clone)]
pub enum OmemoEvent {
    Enable { account: Account, jid: BareJid },
}

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
        let device = match aparte.storage.get_omemo_own_device(account)? {
            Some(device) => device,
            None => {
                log::info!("Generating device id");
                let device_id: u32 = random::<u32>();
                aparte
                    .storage
                    .set_omemo_current_device(account, device_id)?
            }
        };

        let device_id: u32 = device.id.try_into().unwrap();
        log::info!("device id: {}", device_id);

        let mut aparte = aparte.proxy();
        let account = account.clone();

        Aparte::spawn(async move {
            if let Err(err) =
                Self::ensure_device_is_registered(&mut aparte, account, device_id).await
            {
                aparte.error("Cannot configure OMEMO", err);
            }
        });

        Ok(())
    }

    async fn enable(aparte: &mut AparteAsync, account: Account, jid: BareJid) -> Result<()> {
        Self::subscribe_to_device_list(aparte, &account, &jid)
            .await
            .context("Cannot subscribe to device list")?;
        log::info!("Subscribed to {}'s OMEMO device list", jid);

        let device_list = Self::get_device_list(aparte, &account, &jid)
            .await
            .context("Cannot get device list")?;
        log::info!("Got {}'s OMEMO device list", jid);

        for device in device_list.devices {
            let device = aparte.storage.upsert_omemo_contact_device(
                &account,
                &jid,
                device.id.try_into()?,
            )?;
            match Self::get_bundle(aparte, &account, &jid, device.id.try_into().unwrap()).await {
                Ok(bundle) => {
                    if let Some(prekeys) = bundle.prekeys {
                        aparte
                            .storage
                            .upsert_omemo_contact_prekeys(&device, &prekeys.keys)?;
                    }
                }
                Err(err) => aparte.error(
                    format!("Cannot load {}'s device {} bundle", jid, device.id),
                    err,
                ),
            }
        }
        log::info!("Update {}'s OMEMO device list cache", jid);

        Ok(())
    }

    async fn subscribe_to_device_list(
        aparte: &mut AparteAsync,
        account: &Account,
        jid: &BareJid,
    ) -> Result<()> {
        let response = aparte
            .iq(
                &account,
                Self::subscribe_to_device_list_iq(&jid, &BareJid::from(account.clone())),
            )
            .await?;
        match response.payload {
            IqType::Result(None) => Err(anyhow!("Empty iq response")),
            IqType::Error(err) => {
                let text = match i18n::get_best(&err.texts, vec![]) {
                    Some((_, text)) => text.to_string(),
                    None => format!("{:?}", err.defined_condition),
                };
                Err(anyhow!("Iq error {}: {}", err.type_, text))
            }
            IqType::Result(Some(pubsub)) => match PubSub::try_from(pubsub) {
                Ok(PubSub::Subscription(subscription)) => match subscription.subscription {
                    Some(pubsub::Subscription::Subscribed) => Ok(()),
                    Some(status) => Err(anyhow!("Invalid subscription result: {:?}", status)),
                    None => Err(anyhow!("Empty subscription result")),
                },
                Err(err) => Err(err.into()),
                Ok(el) => Err(anyhow!("Invalid pubsub response: {:?}", el)),
            },
            iq => Err(anyhow!("Invalid IQ response: {:?}", iq)),
        }
    }

    async fn get_device_list(
        aparte: &mut AparteAsync,
        account: &Account,
        jid: &BareJid,
    ) -> Result<Omemo::DeviceList> {
        let response = aparte.iq(&account, Self::get_devices_iq(&jid)).await?;
        match response.payload {
            IqType::Result(None) => Err(anyhow!("Empty iq response")),
            IqType::Error(err) => {
                let text = match i18n::get_best(&err.texts, vec![]) {
                    Some((_, text)) => text.to_string(),
                    None => format!("{:?}", err.defined_condition),
                };
                Err(anyhow!("Iq error {}: {}", err.type_, text))
            }
            IqType::Result(Some(pubsub)) => match PubSub::try_from(pubsub)? {
                PubSub::Items(items) => {
                    let current = Some(ItemId("current".to_string()));
                    match items.items.iter().find(|item| item.id == current) {
                        Some(current) => {
                            let payload = current
                                .payload
                                .clone()
                                .ok_or(anyhow!("Missing pubsub payload"))?;
                            let list = Omemo::DeviceList::try_from(payload)?;
                            Ok(list)
                        }
                        None => Err(anyhow!("No device list")),
                    }
                }
                _ => Err(anyhow!("Invalid pubsub response")),
            },
            iq => Err(anyhow!("Invalid IQ response: {:?}", iq)),
        }
    }

    //fn handle_devices(&mut self, contact: &BareJid, devices: &omemo::Devices) {
    //    log::info!("Updating OMEMO devices cache for {}", contact);
    //    let cache = self.devices_cache.entry(contact.clone()).or_insert(Vec::new());
    //    cache.extend(devices.devices.iter().cloned());
    //}

    async fn ensure_device_is_registered(
        aparte: &mut AparteAsync,
        account: Account,
        device_id: u32,
    ) -> Result<()> {
        let response = aparte
            .iq(&account, Self::get_devices_iq(&account.clone().into()))
            .await?;
        match response.payload {
            IqType::Result(None) => Err(anyhow!("Empty iq response")),
            IqType::Error(err) => {
                let text = match i18n::get_best(&err.texts, vec![]) {
                    Some((_, text)) => text.to_string(),
                    None => format!("{:?}", err.defined_condition),
                };
                Err(anyhow!("Iq error {}: {}", err.type_, text))
            }
            IqType::Result(Some(pubsub)) => match PubSub::try_from(pubsub)? {
                PubSub::Items(items) => {
                    let current = Some(ItemId("current".to_string()));
                    match items.items.iter().find(|item| item.id == current) {
                        Some(current) => {
                            let payload = current
                                .payload
                                .clone()
                                .ok_or(anyhow!("Missing pubsub payload"))?;
                            let list = Omemo::DeviceList::try_from(payload)?;
                            match list.devices.iter().find(|device| device.id == device_id) {
                                None => {
                                    Self::register_device(aparte, account, device_id, Some(list))
                                        .await
                                }
                                Some(_) => {
                                    log::info!("Device registered");
                                    Ok(())
                                }
                            }
                        }
                        None => Self::register_device(aparte, account, device_id, None).await,
                    }
                }
                _ => Err(anyhow!("Invalid pubsub response")),
            },
            iq => Err(anyhow!("Invalid IQ response: {:?}", iq)),
        }
    }

    async fn register_device(
        aparte: &mut AparteAsync,
        account: Account,
        device_id: u32,
        list: Option<Omemo::DeviceList>,
    ) -> Result<()> {
        // TODO handle race

        let mut list = match list {
            Some(list) => list,
            None => Omemo::DeviceList { devices: vec![] },
        };

        list.devices.push(Omemo::Device { id: device_id });
        log::debug!("{:?}", list);

        let response = aparte
            .iq(
                &account,
                Self::set_devices_iq(&account.clone().into(), list),
            )
            .await
            .context("Cannot register OMEMO device")?;
        log::debug!("{:?}", response);
        // match response.payload {
        //     IqType::Result(None) => todo!(),
        //     IqType::Error(_) => todo!(),
        //     _ => todo!(),
        // }

        Ok(())
    }

    async fn get_bundle(
        aparte: &mut AparteAsync,
        account: &Account,
        contact: &BareJid,
        device_id: u32,
    ) -> Result<Omemo::Bundle> {
        let response = aparte
            .iq(&account, Self::get_bundle_iq(contact, device_id))
            .await?;
        match response.payload {
            IqType::Result(None) => Err(anyhow!("Empty iq response")),
            IqType::Error(err) => {
                let text = match i18n::get_best(&err.texts, vec![]) {
                    Some((_, text)) => text.to_string(),
                    None => format!("{:?}", err.defined_condition),
                };
                Err(anyhow!("Iq error {}: {}", err.type_, text))
            }
            IqType::Result(Some(pubsub)) => match PubSub::try_from(pubsub)? {
                PubSub::Items(items) => {
                    let current = Some(ItemId("current".to_string()));
                    match items.items.iter().find(|item| item.id == current) {
                        Some(current) => {
                            let payload = current
                                .payload
                                .clone()
                                .ok_or(anyhow!("Missing pubsub payload"))?;
                            let bundle = Omemo::Bundle::try_from(payload)?;
                            Ok(bundle)
                        }
                        None => Err(anyhow!("No device list")),
                    }
                }
                _ => Err(anyhow!("Invalid pubsub response")),
            },
            iq => Err(anyhow!("Invalid IQ response: {:?}", iq)),
        }
    }

    //
    // Iq building
    //
    fn get_devices_iq(contact: &BareJid) -> Iq {
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

    fn set_devices_iq(jid: &BareJid, devices: Omemo::DeviceList) -> Iq {
        let id = Uuid::new_v4();

        let id = id.to_hyphenated().to_string();
        let item = pubsub::pubsub::Item(pubsub::Item {
            id: Some(pubsub::ItemId("current".to_string())),
            publisher: Some(jid.clone().into()),
            payload: Some(devices.into()),
        });
        let pubsub = pubsub::PubSub::Publish {
            publish: pubsub::pubsub::Publish {
                node: pubsub::NodeName::from_str(ns::LEGACY_OMEMO_DEVICELIST).unwrap(),
                items: vec![item],
            },
            publish_options: None,
        };
        Iq::from_set(id, pubsub).with_to(Jid::Bare(jid.clone()))
    }

    fn subscribe_to_device_list_iq(contact: &BareJid, subscriber: &BareJid) -> Iq {
        let id = Uuid::new_v4().to_hyphenated().to_string();
        let pubsub = pubsub::PubSub::Subscribe {
            subscribe: Some(pubsub::pubsub::Subscribe {
                node: Some(pubsub::NodeName::from_str(ns::LEGACY_OMEMO_DEVICELIST).unwrap()),
                jid: Jid::Bare(subscriber.clone()),
            }),
            options: None,
        };
        Iq::from_set(id, pubsub).with_to(Jid::Bare(contact.clone()))
    }

    fn get_bundle_iq(contact: &BareJid, device_id: u32) -> Iq {
        let id = Uuid::new_v4();

        let id = id.to_hyphenated().to_string();
        let items = pubsub::pubsub::Items {
            max_items: None,
            node: pubsub::NodeName(format!("{}:{}", ns::LEGACY_OMEMO_BUNDLES, device_id)),
            subid: None,
            items: vec![],
        };
        let pubsub = pubsub::PubSub::Items(items);
        Iq::from_get(id, pubsub).with_to(Jid::Bare(contact.clone()))
    }
}

impl ModTrait for OmemoMod {
    fn init(&mut self, aparte: &mut Aparte) -> Result<(), ()> {
        aparte.add_command(omemo::new());
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
            Event::Omemo(event) => match event {
                // TODO context()?
                OmemoEvent::Enable { account, jid } => {
                    let mut aparte = aparte.proxy();
                    let account = account.clone();
                    let jid = jid.clone();

                    Aparte::spawn(async move {
                        if let Err(err) = Self::enable(&mut aparte, account, jid.clone()).await {
                            aparte.error(format!("Can't enable OMEMO with {}", jid), err);
                        }
                    })
                }
            },
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
            //            log::warn!("Mismatching from for pending iq request: {:?} != {:?}", jid, from);
            //        } else {
            //            if let Some(payload) = payload {
            //                if let Ok(devices) = omemo::Devices::try_from(payload.clone()) {
            //                    self.handle_devices(&jid, &devices);
            //                } else {
            //                    log::warn!("Malformed devices element in OMEMO iq result {}", uuid);
            //                }
            //            } else {
            //                log::warn!("Missing devices element in OMEMO iq result {}", uuid);
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
