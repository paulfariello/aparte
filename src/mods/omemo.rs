/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use futures::future::FutureExt;
use libsignal_protocol::{
    process_prekey_bundle, IdentityKey, PreKeyBundle, ProtocolAddress, PublicKey,
};
use rand::random;
use rand::thread_rng;
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
use crate::crypto::CryptoEngineTrait;
use crate::message::Message;
use crate::mods::ui::UIMod;

use libsignal_protocol::{
    message_encrypt,
    IdentityKeyPair,
    InMemSignalProtocolStore, //SessionStore,
};

type SignalStore = Arc<Mutex<InMemSignalProtocolStore>>;

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

struct OmemoEngine {
    contact: BareJid,
    signal_store: SignalStore,
}

impl OmemoEngine {
    fn new(signal_store: SignalStore, contact: &BareJid) -> Self {
        Self {
            signal_store,
            contact: contact.clone(),
        }
    }

    async fn update_bundle(&mut self, device_id: u32, bundle: &Omemo::Bundle) -> Result<()> {
        let address = ProtocolAddress::new(self.contact.to_string(), device_id.into());

        let signed_pre_key = PublicKey::deserialize(
            &bundle
                .signed_pre_key_public
                .as_ref()
                .ok_or(anyhow!("missing signed pre key"))?
                .data,
        )?;

        let signed_pre_key_id = bundle
            .signed_pre_key_public
            .as_ref()
            .ok_or(anyhow!("missing signed pre key"))?
            .signed_pre_key_id
            .ok_or(anyhow!("missing signed pre key id"))?
            .into();

        let signed_pre_key_signature = &bundle
            .signed_pre_key_signature
            .as_ref()
            .ok_or(anyhow!("missing signed pre key signature"))?
            .data;

        let identity_key = IdentityKey::decode(
            &bundle
                .identity_key
                .as_ref()
                .ok_or(anyhow!("missing identity key"))?
                .data,
        )?;

        let bundle = PreKeyBundle::new(
            0, // registration_id: u32,
            device_id.into(),
            todo!("choose a random prekey from the bundle"), // pre_key: Option<(PreKeyId, PublicKey)>,
            signed_pre_key_id,
            signed_pre_key,
            signed_pre_key_signature.to_vec(),
            identity_key,
        )?;

        let signal_store = &mut *self.signal_store.lock().unwrap();
        process_prekey_bundle(
            &address,
            &mut signal_store.session_store,
            &mut signal_store.identity_store,
            &bundle,
            &mut thread_rng(),
            None,
        )
        .now_or_never();

        Ok(())
    }
}

impl CryptoEngineTrait for OmemoEngine {
    #[allow(unreachable_code)]
    fn encrypt(&mut self, message: &mut Message) -> Result<()> {
        match message {
            Message::Xmpp(message) => {
                let _store = self.signal_store.lock().unwrap();
                let _version = message
                    .history
                    .get(0)
                    .ok_or(anyhow!("No message to encrypt"))?;
                let _address = ProtocolAddress::new(self.contact.to_string(), todo!());
                let _encrypted = message_encrypt(
                    &[],
                    &_address,
                    &mut _store.session_store,
                    &mut _store.identity_store,
                    None,
                )
                .now_or_never()
                .ok_or(anyhow!("Cannot encrypt"))?;
                todo!();
                Ok(())
            }
            _ => Err(anyhow!("Invalid message type")),
        }
    }

    fn decrypt(&mut self, _message: &mut Message) -> Result<()> {
        todo!()
    }
}

pub struct OmemoMod {
    signal_stores: HashMap<Account, SignalStore>,
}

impl OmemoMod {
    pub fn new() -> Self {
        Self {
            signal_stores: HashMap::new(),
        }
    }

    fn configure(&mut self, aparte: &mut Aparte, account: &Account) -> Result<()> {
        let device = match aparte.storage.get_omemo_own_device(account)? {
            Some(device) => device,
            None => {
                log::info!("Generating device id");
                let device_id: u32 = random::<u32>();
                let identity_key_pair = IdentityKeyPair::generate(&mut thread_rng());
                aparte.storage.set_omemo_current_device(
                    account,
                    device_id,
                    identity_key_pair.serialize().to_vec(),
                )?
            }
        };

        let device_id: u32 = device.id.try_into().unwrap();
        log::info!("Device id: {device_id}");
        let identity = device.identity.unwrap();
        let identity_key_pair = IdentityKeyPair::try_from(identity.as_ref()).unwrap();

        // TODO generate crypto

        let signal_store = InMemSignalProtocolStore::new(identity_key_pair, device_id)?;
        self.signal_stores
            .insert(account.clone(), Arc::new(Mutex::new(signal_store)));

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

    async fn start_session(
        aparte: &mut AparteAsync,
        signal_store: SignalStore,
        account: &Account,
        jid: &BareJid,
    ) -> Result<()> {
        let mut omemo_engine = OmemoEngine::new(signal_store.clone(), jid);

        Self::subscribe_to_device_list(aparte, account, jid)
            .await
            .context("Cannot subscribe to device list")?;
        log::info!("Subscribed to {jid}'s OMEMO device list");

        let device_list = Self::get_device_list(aparte, account, jid)
            .await
            .context("Cannot get device list")?;
        log::info!("Got {jid}'s OMEMO device list");

        for device in device_list.devices {
            let device =
                aparte
                    .storage
                    .upsert_omemo_contact_device(account, jid, device.id.try_into()?)?;

            // TODO create OMEMO session
            match Self::get_bundle(aparte, account, jid, device.id.try_into().unwrap()).await {
                Ok(bundle) => {
                    if let Err(err) = omemo_engine
                        .update_bundle(device.id.try_into().unwrap(), &bundle)
                        .await
                    {
                        aparte.error(
                            format!("Cannot load {jid}'s device {} bundle", device.id),
                            err,
                        );
                    }
                }
                Err(err) => aparte.error(
                    format!("Cannot load {jid}'s device {} bundle", device.id),
                    err,
                ),
            };
        }
        log::info!("Update {jid}'s OMEMO device list cache");

        aparte.add_crypto_engine(account, jid, Box::new(omemo_engine));

        Ok(())
    }

    async fn subscribe_to_device_list(
        aparte: &mut AparteAsync,
        account: &Account,
        jid: &BareJid,
    ) -> Result<()> {
        let response = aparte
            .iq(
                account,
                Self::subscribe_to_device_list_iq(jid, &BareJid::from(account.clone())),
            )
            .await?;
        match response.payload {
            IqType::Result(None) => Err(anyhow!("Empty iq response")),
            IqType::Error(err) => {
                let text = match i18n::get_best(&err.texts, vec![]) {
                    Some((_, text)) => text.to_string(),
                    None => format!("{:?}", err.defined_condition),
                };
                Err(anyhow!("Iq error {}: {text}", err.type_))
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
        let response = aparte.iq(account, Self::get_devices_iq(jid)).await?;
        match response.payload {
            IqType::Result(None) => Err(anyhow!("Empty iq response")),
            IqType::Error(err) => {
                let text = match i18n::get_best(&err.texts, vec![]) {
                    Some((_, text)) => text.to_string(),
                    None => format!("{:?}", err.defined_condition),
                };
                Err(anyhow!("Iq error {}: {text}", err.type_))
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
    //    log::info!("Updating OMEMO devices cache for {contact}");
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
                Err(anyhow!("Iq error {}: {text}", err.type_))
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
        // TODO publish device identity

        Ok(())
    }

    async fn get_bundle(
        aparte: &mut AparteAsync,
        account: &Account,
        contact: &BareJid,
        device_id: u32,
    ) -> Result<Omemo::Bundle> {
        let response = aparte
            .iq(account, Self::get_bundle_iq(contact, device_id))
            .await?;
        match response.payload {
            IqType::Result(None) => Err(anyhow!("Empty iq response")),
            IqType::Error(err) => {
                let text = match i18n::get_best(&err.texts, vec![]) {
                    Some((_, text)) => text.to_string(),
                    None => format!("{:?}", err.defined_condition),
                };
                Err(anyhow!("Iq error {}: {text}", err.type_))
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
            node: pubsub::NodeName(format!("{}:{device_id}", ns::LEGACY_OMEMO_BUNDLES)),
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
        //disco.add_feature(format!("{ns::OMEMO_DEVICES}+notify"));

        Ok(())
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::Connected(account, _jid) => {
                if let Err(err) = self.configure(aparte, account) {
                    aparte.log(format!("Cannot configure OMEMO: {err}"));
                }
            }
            Event::Omemo(event) => match event {
                // TODO context()?
                OmemoEvent::Enable { account, jid } => {
                    let mut aparte = aparte.proxy();
                    let account = account.clone();
                    let jid = jid.clone();
                    match self.signal_stores.get(&account) {
                        None => aparte.log(format!("OMEMO not configured for {account}")),
                        Some(signal_store) => {
                            let signal_store = signal_store.clone();
                            Aparte::spawn(async move {
                                if let Err(err) =
                                    Self::start_session(&mut aparte, signal_store, &account, &jid)
                                        .await
                                {
                                    aparte.error(
                                        format!("Can't start OMEMO session with {jid}"),
                                        err,
                                    );
                                }
                            })
                        }
                    }
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
            //                    log::warn!("Malformed devices element in OMEMO iq result {uuid}");
            //                }
            //            } else {
            //                log::warn!("Missing devices element in OMEMO iq result {uuid}");
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
