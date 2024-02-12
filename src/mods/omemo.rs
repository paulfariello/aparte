/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::fmt::{self, Debug};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use aes_gcm::{
    self,
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes128Gcm, KeySizeUser,
};
use anyhow::{anyhow, Context, Result};
use futures::future::FutureExt;
use libsignal_protocol::{
    process_prekey_bundle, CiphertextMessage, IdentityKey, PreKeyBundle, ProtocolAddress, PublicKey,
};
use rand::{random, seq::SliceRandom, thread_rng};
use uuid::Uuid;

//use xmpp_parsers::ns;
use xmpp_parsers::iq::{Iq, IqType};
use xmpp_parsers::legacy_omemo;
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

    async fn update_bundle(&mut self, device_id: u32, bundle: &legacy_omemo::Bundle) -> Result<()> {
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

        let prekey = bundle
            .prekeys
            .as_ref()
            .ok_or(anyhow!("No prekey in bundle"))?
            .keys
            .choose(&mut rand::thread_rng())
            .ok_or(anyhow!("No prekey in bundle"))?;

        let prekey_bundle = PreKeyBundle::new(
            0, // registration_id: u32,
            device_id.into(),
            Some((
                prekey.pre_key_id.into(),
                PublicKey::deserialize(&prekey.data)?,
            )),
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
            &prekey_bundle,
            &mut thread_rng(),
            None,
        )
        .now_or_never();

        Ok(())
    }
}

impl CryptoEngineTrait for OmemoEngine {
    fn encrypt(
        &mut self,
        aparte: &Aparte,
        account: &Account,
        message: &Message,
    ) -> Result<xmpp_parsers::Element> {
        let Message::Xmpp(message) = message else {
            unreachable!()
        };

        let own_device = aparte
            .storage
            .get_omemo_own_device(account)?
            .ok_or(anyhow!("Omemo isn't configured"))?;

        let nonce = Aes128Gcm::generate_nonce(&mut OsRng);
        let dek = Aes128Gcm::generate_key(OsRng);

        let cipher = Aes128Gcm::new(&dek);
        let body = message.get_last_body();
        let encrypted = cipher
            .encrypt(&nonce, body.as_bytes())
            .map_err(|e| anyhow!("{e}"))?;

        const KEY_SIZE: usize = 16;
        const KEY_TAG: usize = 16;

        assert!(encrypted.len() - body.len() == KEY_TAG);

        let mut dek_and_tag = [0u8; KEY_SIZE + KEY_TAG];
        dek_and_tag[..KEY_SIZE].copy_from_slice(&dek);
        dek_and_tag[KEY_SIZE..KEY_SIZE + KEY_TAG].copy_from_slice(&encrypted[body.len()..]);

        // Encrypt DEK with each recipient key
        let signal_store = &mut *self.signal_store.lock().unwrap();
        let keys = aparte
            .storage
            .get_omemo_contact_devices(account, &message.to)?
            .iter()
            .chain(Some((&own_device).into()).iter())
            .filter_map(|device| {
                let address =
                    ProtocolAddress::new(self.contact.to_string(), (device.id as u32).into());
                message_encrypt(
                    &dek_and_tag,
                    &address,
                    &mut signal_store.session_store,
                    &mut signal_store.identity_store,
                    None,
                )
                .now_or_never()
                .inspect(|msg| match msg {
                    Ok(CiphertextMessage::PreKeySignalMessage(msg)) => {
                        log::info!("{}", msg.message_version())
                    }
                    Ok(CiphertextMessage::SignalMessage(msg)) => {
                        log::info!("{}", msg.message_version())
                    }
                    _ => {}
                })
                .map_or(None, |encrypted| match encrypted {
                    Ok(CiphertextMessage::SignalMessage(msg)) => Some(legacy_omemo::Key {
                        rid: device.id.try_into().unwrap(),
                        prekey: legacy_omemo::IsPreKey::False,
                        data: msg.serialized().to_vec(),
                    }),
                    Ok(CiphertextMessage::PreKeySignalMessage(msg)) => Some(legacy_omemo::Key {
                        rid: device.id.try_into().unwrap(),
                        prekey: legacy_omemo::IsPreKey::True,
                        data: msg.serialized().to_vec(),
                    }),
                    Ok(_) => {
                        unreachable!();
                    }
                    Err(e) => {
                        log::error!("Cannot encrypt for {address}: {e}");
                        None
                    }
                })
            })
            .collect();

        let mut xmpp_message =
            xmpp_parsers::message::Message::new(Some(Jid::Bare(message.to.clone())));
        xmpp_message.id = Some(message.id.clone());
        xmpp_message.type_ = xmpp_parsers::message::MessageType::Chat;
        xmpp_message.bodies.insert(
            String::new(),
            xmpp_parsers::message::Body(String::from("coucou")),
        );
        xmpp_message.payloads.push(
            legacy_omemo::Encrypted {
                header: legacy_omemo::Header {
                    sid: own_device.id.try_into().unwrap(),
                    iv: legacy_omemo::IV {
                        data: nonce.to_vec(),
                    },
                    keys,
                },
                payload: Some(legacy_omemo::Payload {
                    data: encrypted.to_vec(),
                }),
            }
            .into(),
        );
        xmpp_message.payloads.push(
            xmpp_parsers::eme::ExplicitMessageEncryption {
                namespace: String::from("eu.siacs.conversations.axolotl"),
                name: Some(String::from("OMEMO")),
            }
            .into(),
        );
        Ok(xmpp_message.into())
    }

    fn decrypt(
        &mut self,
        _aparte: &Aparte,
        _account: &Account,
        _message: &mut Message,
    ) -> Result<Message> {
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

        // TODO Should use proper ProtocolStore respecting  libsignal_protocol::storage::traits
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
        log::info!("Start OMEMO session on {account} with {jid}");
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
            log::info!("Update {jid}'s OMEMO device {0} bundle", device.id);
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
    ) -> Result<legacy_omemo::DeviceList> {
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
                            let list = legacy_omemo::DeviceList::try_from(payload)?;
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

    //fn handle_devices(&mut self, contact: &BareJid, devices: &legacy_omemo::Devices) {
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
                            let list = legacy_omemo::DeviceList::try_from(payload)?;
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
        list: Option<legacy_omemo::DeviceList>,
    ) -> Result<()> {
        // TODO handle race

        let mut list = match list {
            Some(list) => list,
            None => legacy_omemo::DeviceList { devices: vec![] },
        };

        list.devices.push(legacy_omemo::Device { id: device_id });
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
    ) -> Result<legacy_omemo::Bundle> {
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
                            let bundle = legacy_omemo::Bundle::try_from(payload)?;
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

    fn set_devices_iq(jid: &BareJid, devices: legacy_omemo::DeviceList) -> Iq {
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
            //                        if let Ok(devices) = legacy_omemo::Devices::try_from(payload) {
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
            //                if let Ok(devices) = legacy_omemo::Devices::try_from(payload.clone()) {
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
