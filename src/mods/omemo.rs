/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::fmt::{self, Debug};
use std::str::FromStr;

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes128Gcm,
};
use anyhow::{anyhow, Context, Result};
use futures::future::FutureExt;
use itertools::Itertools;
use libsignal_protocol::{
    process_prekey_bundle, CiphertextMessage, IdentityKey, PreKeyBundle, ProtocolAddress, PublicKey,
};
use rand::{random, seq::SliceRandom, thread_rng};
use uuid::Uuid;

//use xmpp_parsers::ns;
use xmpp_parsers::iq::{Iq, IqType};
use xmpp_parsers::legacy_omemo;
use xmpp_parsers::message::Message as XmppParsersMessage;
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
use crate::storage::{OmemoOwnDevice, SignalStorage};

use libsignal_protocol::{
    message_decrypt, message_encrypt, IdentityKeyPair, IdentityKeyStore, KeyPair, PreKeyStore,
    SignedPreKeyStore,
};

const KEY_SIZE: usize = 16;
const MAC_SIZE: usize = 16;

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

command_def!(
    omemo_fingerprint,
    r#"/omemo fingerprint [<jid>]

Description:
    Show OMEMO own or given jid fingerprint

Examples:
    /omemo fingerprint
"#,
{
    jid: Option<String>,
},
    |aparte, _command| {
        let mut current = {
            let ui = aparte.get_mod::<UIMod>();
            ui.current_window().cloned()
        };

        if current == Some(String::from("console")) {
            current = None;
        }
        let contact = jid.or(current).map(|jid| BareJid::from_str(&jid))
            .transpose()?;

        if let Some(account) = aparte.current_account() {
            aparte.schedule(Event::Omemo(OmemoEvent::ShowFingerprints {
                account,
                jid: contact,
            }));
        }
        Ok(())
    }
);

command_def!(omemo,
r#"/omemo enable"#,
{
    action: Command = {
        children: {
            "enable": omemo_enable,
            "fingerprint": omemo_fingerprint,
        }
    },
});

#[derive(Debug, Clone)]
pub enum OmemoEvent {
    Enable {
        account: Account,
        jid: BareJid,
    },
    ShowFingerprints {
        account: Account,
        jid: Option<BareJid>,
    },
}

struct OmemoEngine {
    account: Account,
    contact: BareJid,
    signal_storage: SignalStorage,
}

impl OmemoEngine {
    fn new(account: &Account, signal_storage: SignalStorage, contact: &BareJid) -> Self {
        Self {
            account: account.clone(),
            signal_storage,
            contact: contact.clone(),
        }
    }

    fn update_bundle(&mut self, device_id: u32, bundle: &legacy_omemo::Bundle) -> Result<()> {
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
            .choose(&mut thread_rng())
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

        log::info!(
            "Blind trust of {address}: {}",
            fingerprint(identity_key.public_key())
        );

        self.signal_storage
            .save_identity(&address, &identity_key, None)
            .now_or_never()
            .ok_or(anyhow!("Cannot trust {address}"))??;

        log::debug!("Process {address}'s bundle");

        process_prekey_bundle(
            &address,
            &mut self.signal_storage.clone(),
            &mut self.signal_storage.clone(),
            &prekey_bundle,
            &mut thread_rng(),
            None,
        )
        .now_or_never()
        .ok_or(anyhow!("Cannot start session with {device_id}"))??;

        Ok(())
    }

    fn sync_bundle(&self, aparte: &Aparte) -> Result<()> {
        log::info!("Syncing {}'s own bundle", self.account);
        let device = aparte
            .storage
            .get_omemo_own_device(&self.account)?
            .context("OMEMO isn't configured")?;

        let device_id: u32 = device.id.try_into().context("Corrupted own device id")?;
        let identity = device.identity.context("Missing own identity")?;
        let identity_key_pair =
            IdentityKeyPair::try_from(identity.as_ref()).context("Corrupted own identity")?;

        let signed_pre_key_id = 0;
        let signed_pre_key = aparte.storage.get_omemo_signed_pre_key(
            &self.account,
            libsignal_protocol::SignedPreKeyId::from(signed_pre_key_id),
        )?;
        let signed_pre_key_public = signed_pre_key.public_key()?;
        let signed_pre_key_signature = signed_pre_key.signature()?;
        let pre_keys = aparte
            .storage
            .get_all_omemo_pre_key(&self.account)?
            .into_iter()
            .map(|pre_key| match (pre_key.id(), pre_key.public_key()) {
                (Ok(id), Ok(public_key)) => Ok((u32::from(id), public_key)),
                (Err(e), _) => Err(e),
                (_, Err(e)) => Err(e),
            })
            .collect::<std::result::Result<Vec<(_, _)>, _>>()?;

        Aparte::spawn({
            let mut aparte = aparte.proxy();
            let account = self.account.clone();
            async move {
                if let Err(err) = OmemoMod::publish_bundle(
                    &mut aparte,
                    &account,
                    device_id,
                    identity_key_pair,
                    signed_pre_key_id,
                    signed_pre_key_public,
                    signed_pre_key_signature,
                    pre_keys,
                )
                .await
                {
                    crate::error!(aparte, err, "Cannot sync OMEMO bundle");
                }
            }
        });

        Ok(())
    }
}

impl CryptoEngineTrait for OmemoEngine {
    fn ns(&self) -> &'static str {
        ns::LEGACY_OMEMO
    }

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
            .ok_or(anyhow!("Missing own device"))?;

        let own_devices = aparte
            .storage
            .get_omemo_contact_devices(account, &account.to_bare())?;

        let nonce = Aes128Gcm::generate_nonce(&mut OsRng);
        let dek = Aes128Gcm::generate_key(OsRng);

        let cipher = Aes128Gcm::new(&dek);
        let body = message.get_last_body();
        let encrypted = cipher
            .encrypt(&nonce, body.as_bytes())
            .map_err(|e| anyhow!("{e}"))?;

        assert!(encrypted.len() - body.len() == MAC_SIZE);

        let mut dek_and_mac = [0u8; KEY_SIZE + MAC_SIZE];
        dek_and_mac[..KEY_SIZE].copy_from_slice(&dek);
        dek_and_mac[KEY_SIZE..KEY_SIZE + MAC_SIZE].copy_from_slice(&encrypted[body.len()..]);

        // Encrypt DEK with each recipient key
        let keys = aparte
            .storage
            .get_omemo_contact_devices(account, &message.to)?
            .iter()
            .chain(
                own_devices
                    .iter()
                    .filter(|device| device.id != own_device.id),
            )
            .filter_map(|device| {
                let remote_address =
                    ProtocolAddress::new(device.contact.clone(), (device.id as u32).into());
                message_encrypt(
                    &dek_and_mac,
                    &remote_address,
                    &mut self.signal_storage.clone(),
                    &mut self.signal_storage.clone(),
                    None,
                )
                .now_or_never()
                .and_then(|encrypted| match encrypted {
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
                        log::error!("Cannot encrypt for {remote_address}: {e}");
                        None
                    }
                })
            })
            .collect();

        let mut xmpp_message =
            xmpp_parsers::message::Message::new(Some(Jid::from(message.to.clone())));
        xmpp_message.id = Some(message.id.clone());
        xmpp_message.type_ = xmpp_parsers::message::MessageType::Chat;
        xmpp_message.bodies.insert(
            String::new(),
            xmpp_parsers::message::Body(String::from("I sent you an OMEMO encrypted message but your client doesn’t seem to support that.")),
        );
        xmpp_message.payloads.push(
            legacy_omemo::Encrypted {
                header: legacy_omemo::Header {
                    sid: own_device
                        .id
                        .try_into()
                        .context("Corrupted own device id")?,
                    iv: legacy_omemo::IV {
                        data: nonce.to_vec(),
                    },
                    keys,
                },
                payload: Some(legacy_omemo::Payload {
                    data: encrypted[..body.len()].to_vec(),
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
        aparte: &Aparte,
        account: &Account,
        message: &XmppParsersMessage,
    ) -> Result<XmppParsersMessage> {
        log::info!(
            "Decrypting message from {}",
            message.from.clone().context("Missing from attribute")?
        );
        let own_device = aparte
            .storage
            .get_omemo_own_device(account)?
            .ok_or(anyhow!("Omemo isn't configured"))?;

        let encrypted = message
            .payloads
            .iter()
            .find_map(|p| legacy_omemo::Encrypted::try_from((*p).clone()).ok())
            .ok_or(anyhow!("Missing encrypted element in EME OMEMO message"))?;

        let key = encrypted
            .header
            .keys
            .iter()
            .find(|k| i64::from(k.rid) == own_device.id)
            .ok_or(anyhow!("Missing OMEMO key for current device"))?;

        log::debug!("Found encrypted DEK for current device ({})", own_device.id);

        let ciphertext_message = match key.prekey {
            legacy_omemo::IsPreKey::True => {
                log::debug!("Prekey message");
                libsignal_protocol::CiphertextMessage::PreKeySignalMessage(
                    libsignal_protocol::PreKeySignalMessage::try_from(key.data.as_slice())
                        .context("Invalid prekey signal message")?,
                )
            }
            legacy_omemo::IsPreKey::False => libsignal_protocol::CiphertextMessage::SignalMessage(
                libsignal_protocol::SignalMessage::try_from(key.data.as_slice())
                    .context("Invalid signal message")?,
            ),
        };

        let remote_address = ProtocolAddress::new(
            self.contact.to_string(),
            libsignal_protocol::DeviceId::from(encrypted.header.sid),
        );

        let dek_and_mac = message_decrypt(
            &ciphertext_message,
            &remote_address,
            &mut self.signal_storage.clone(),
            &mut self.signal_storage.clone(),
            &mut self.signal_storage.clone(),
            &mut self.signal_storage.clone(),
            &mut thread_rng(),
            None,
        )
        .now_or_never()
        .ok_or(anyhow!("Cannot decrypt DEK"))??;

        if self
            .signal_storage
            .deleted_pre_keys
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            self.sync_bundle(aparte)?;
        }

        if dek_and_mac.len() != MAC_SIZE + KEY_SIZE {
            anyhow::bail!("Invalid DEK and MAC size");
        }

        let mut decrypted_message = message.clone();

        if let Some(payload) = encrypted.payload {
            let dek = aes_gcm::Key::<Aes128Gcm>::from_slice(&dek_and_mac[..KEY_SIZE]);
            let mac = &dek_and_mac[KEY_SIZE..KEY_SIZE + MAC_SIZE];
            let mut payload_and_mac = Vec::with_capacity(payload.data.len() + mac.len());
            payload_and_mac.extend(payload.data);
            payload_and_mac.extend(mac);

            let cipher = Aes128Gcm::new(dek);
            let nonce = aes_gcm::Nonce::<<Aes128Gcm as AeadCore>::NonceSize>::from_slice(
                encrypted.header.iv.data.as_slice(),
            );
            let cleartext = cipher
                .decrypt(nonce, payload_and_mac.as_slice())
                .map_err(|_| anyhow!("Message decryption failed"))?;
            let message = String::from_utf8(cleartext)
                .context("Message decryption resulted in invalid utf-8")?;
            decrypted_message
                .bodies
                .insert(String::new(), xmpp_parsers::message::Body(message));
        }

        Ok(decrypted_message)
    }
}

#[derive(Default)]
pub struct OmemoMod {
    signal_stores: HashMap<Account, SignalStorage>,
}

fn fingerprint(pub_key: &PublicKey) -> String {
    // TODO fallback to standard library when intersperse will be added
    itertools::Itertools::intersperse(
        pub_key
            .serialize()
            .iter()
            .skip(1)
            .map(|byte| format!("{byte:02x}"))
            .chunks(4)
            .into_iter()
            .map(|word| word.collect::<String>()),
        String::from(" "),
    )
    .collect()
}

impl OmemoMod {
    fn configure(&mut self, aparte: &mut Aparte, account: &Account) -> Result<()> {
        log::info!("Configure omemo for {account}");
        let signal_store = SignalStorage::new(account.clone(), aparte.storage.clone());
        self.signal_stores.insert(account.clone(), signal_store);

        let device = match aparte
            .storage
            .get_omemo_own_device(account)
            .context("Can't get or create own device")?
        {
            Some(device) => {
                log::info!("Reusing existing device");
                device
            }
            None => self
                .initialize_crypto(aparte, account)
                .context("Cannot initialize crypto")?,
        };

        log::debug!("Retrieve crypto assets");
        let device_id: u32 = device.id.try_into().context("Corrupted own device id")?;
        log::info!("Device id: {device_id}");
        let identity = device.identity.context("Missing own identity")?;
        let identity_key_pair =
            IdentityKeyPair::try_from(identity.as_ref()).context("Corrupted own identity")?;
        let fingerprint = fingerprint(identity_key_pair.public_key());
        log::info!("Device fingerprint: {fingerprint}");

        let mut aparte = aparte.proxy();
        let account = account.clone();

        let signed_pre_key_id = 0;
        let signed_pre_key = aparte
            .storage
            .get_omemo_signed_pre_key(
                &account,
                libsignal_protocol::SignedPreKeyId::from(signed_pre_key_id),
            )
            .context("Can't get stored signed pre key")?;
        let signed_pre_key_public = signed_pre_key.public_key()?;
        let signed_pre_key_signature = signed_pre_key.signature()?;
        let pre_keys = aparte
            .storage
            .get_all_omemo_pre_key(&account)?
            .into_iter()
            .map(|pre_key| match (pre_key.id(), pre_key.public_key()) {
                (Ok(id), Ok(public_key)) => Ok((u32::from(id), public_key)),
                (Err(e), _) => Err(e),
                (_, Err(e)) => Err(e),
            })
            .collect::<std::result::Result<Vec<(_, _)>, _>>()?;

        let signal_store = self.signal_stores.get(&account).unwrap();
        Aparte::spawn({
            let signal_store = SignalStorage::clone(signal_store);
            async move {
                if let Err(err) =
                    Self::ensure_device_is_registered(&mut aparte, &account, device_id).await
                {
                    crate::error!(aparte, err, "Cannot configure OMEMO");
                    return;
                }

                if let Err(err) = Self::ensure_device_bundle_is_published(
                    &mut aparte,
                    &account,
                    device_id,
                    identity_key_pair,
                    signed_pre_key_id,
                    signed_pre_key_public,
                    signed_pre_key_signature,
                    pre_keys,
                )
                .await
                {
                    crate::error!(aparte, err, "Cannot configure OMEMO");
                }

                if let Err(err) =
                    Self::start_session(&mut aparte, &signal_store, &account, &account.to_bare())
                        .await
                {
                    crate::error!(aparte, err, "Can't start OMEMO session own devices",);
                }
            }
        });

        Ok(())
    }

    fn initialize_crypto(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
    ) -> Result<OmemoOwnDevice> {
        crate::info!(aparte, "Initializing OMEMO device");

        let signal_storage = self
            .signal_stores
            .get_mut(account)
            .context("Missing signal store")?;

        // XEP 0384 4.1 The Device ID is a randomly generated integer between 1 and 2^31 - 1.
        let device_id: u32 = random::<u32>() % (2u32.pow(31) - 1) + 1;
        let identity_key_pair = IdentityKeyPair::generate(&mut thread_rng());
        let own_device = aparte.storage.set_omemo_current_device(
            account,
            device_id,
            identity_key_pair.serialize().to_vec(),
        )?;

        let pre_keys = (1..101)
            .map(|i| (i, KeyPair::generate(&mut thread_rng())))
            .collect::<Vec<(u32, KeyPair)>>();

        let signed_pre_key_id = 0;
        let signed_pre_key = KeyPair::generate(&mut thread_rng());
        let signed_pre_key_signature = identity_key_pair
            .private_key()
            .calculate_signature(&signed_pre_key.public_key.serialize(), &mut thread_rng())?;

        for (i, pre_key) in pre_keys.iter() {
            signal_storage
                .save_pre_key(
                    libsignal_protocol::PreKeyId::from(*i),
                    &libsignal_protocol::PreKeyRecord::new(
                        libsignal_protocol::PreKeyId::from(*i),
                        pre_key,
                    ),
                    None,
                )
                .now_or_never()
                .ok_or(anyhow!("Cannot save pre_keys"))??;
        }

        signal_storage
            .save_signed_pre_key(
                libsignal_protocol::SignedPreKeyId::from(signed_pre_key_id),
                &libsignal_protocol::SignedPreKeyRecord::new(
                    libsignal_protocol::SignedPreKeyId::from(signed_pre_key_id),
                    chrono::Local::now().timestamp().try_into().unwrap(),
                    &signed_pre_key,
                    &signed_pre_key_signature,
                ),
                None,
            )
            .now_or_never()
            .ok_or(anyhow!("Cannot save pre_keys"))??;

        crate::info!(
            aparte,
            "OMEMO device initialized: id: {device_id} fingerprint: {}",
            fingerprint(identity_key_pair.public_key())
        );

        Ok(own_device)
    }

    fn show_fingerprints(
        &self,
        aparte: &mut Aparte,
        account: &Account,
        jid: &Option<BareJid>,
    ) -> Result<()> {
        let signal_store = self
            .signal_stores
            .get(account)
            .ok_or(anyhow!("OMEMO not configured for {account}"))?;

        let identities = match jid {
            None => vec![*IdentityKeyPair::try_from(
                signal_store
                    .storage
                    .get_omemo_own_device(account)?
                    .context("No current OMEMO device")?
                    .identity
                    .context("Missing identity for device")?
                    .as_slice(),
            )?
            .public_key()],
            Some(jid) => signal_store
                .storage
                .get_omemo_contact_identities(account, jid)?
                .into_iter()
                .map(|identity| *identity.public_key())
                .collect(),
        };

        match jid {
            Some(jid) => crate::info!(aparte, "OMEMO fingerprint for {jid}:"),
            None => crate::info!(aparte, "OMEMO own fingerprint:"),
        }
        for identity in identities {
            crate::info!(aparte, "🛡 {}", fingerprint(&identity));
        }

        Ok(())
    }

    fn restore_sessions(&mut self, aparte: &mut Aparte, account: &Account) -> Result<()> {
        let signal_store = self
            .signal_stores
            .get(account)
            .context("Missing signal store")?;

        for contact in aparte.storage.get_all_omemo_contacts(account)?.iter() {
            aparte.add_crypto_engine(
                account,
                contact,
                Box::new(OmemoEngine::new(account, signal_store.clone(), contact)),
            );
        }

        Ok(())
    }

    async fn start_session(
        aparte: &mut AparteAsync,
        signal_store: &SignalStorage,
        account: &Account,
        jid: &BareJid,
    ) -> Result<()> {
        log::info!("Start OMEMO session on {account} with {jid}");
        let mut omemo_engine = OmemoEngine::new(account, signal_store.clone(), jid);

        let own_device = signal_store
            .storage
            .get_omemo_local_registration_id(&signal_store.account)?;

        Self::subscribe_to_device_list(aparte, account, jid)
            .await
            .context("Cannot subscribe to device list")?;
        log::info!("Subscribed to {jid}'s OMEMO device list");

        let device_list = Self::get_device_list(aparte, account, jid)
            .await
            .context("Cannot get device list")?;
        log::info!("Got {jid}'s OMEMO device list");

        for device in device_list
            .devices
            .iter()
            .filter(|device| device.id != own_device)
        {
            let device = aparte
                .storage
                .upsert_omemo_contact_device(account, jid, device.id)?;

            log::info!("Update {jid}'s OMEMO device {0} bundle", device.id);
            let device_id = device.id.try_into().context("Corrupted device id")?;
            match Self::get_bundle(aparte, account, jid, device_id).await {
                Ok(Some(bundle)) => {
                    if let Err(err) = omemo_engine.update_bundle(device_id, &bundle) {
                        crate::error!(
                            aparte,
                            err,
                            "Cannot load {jid}'s device {} bundle",
                            device.id,
                        );
                    }
                }
                Ok(None) => crate::info!(aparte, "No bundle found for {jid}.{}", device.id),
                Err(err) => crate::error!(
                    aparte,
                    err,
                    "Cannot load {jid}'s device {} bundle",
                    device.id,
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
                Self::subscribe_to_device_list_iq(jid, &account.to_bare()),
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
        account: &Account,
        device_id: u32,
    ) -> Result<()> {
        log::info!("Ensure device {device_id} is registered");
        let response = aparte
            .iq(account, Self::get_devices_iq(&account.to_bare()))
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
                                    Self::register_device(
                                        aparte,
                                        account,
                                        device_id,
                                        Some(list.clone()),
                                    )
                                    .await
                                }
                                Some(_) => {
                                    log::info!("Device already registered");
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

    #[allow(clippy::too_many_arguments)]
    async fn ensure_device_bundle_is_published(
        aparte: &mut AparteAsync,
        account: &Account,
        device_id: u32,
        identity_key_pair: IdentityKeyPair,
        signed_pre_key_id: u32,
        signed_pre_key_pub: PublicKey,
        signed_pre_key_signature: Vec<u8>,
        pre_keys: Vec<(u32, PublicKey)>,
    ) -> Result<()> {
        log::info!("Ensure device {device_id}'s bundle is published");
        match Self::get_bundle(aparte, account, &account.to_bare(), device_id).await? {
            None => {
                Self::publish_bundle(
                    aparte,
                    account,
                    device_id,
                    identity_key_pair,
                    signed_pre_key_id,
                    signed_pre_key_pub,
                    signed_pre_key_signature,
                    pre_keys,
                )
                .await
            }
            Some(legacy_omemo::Bundle {
                prekeys: Some(legacy_omemo::Prekeys { keys }),
                ..
            }) if keys.len() < 20 => {
                log::info!("Published bundle doesn't have enough prekeys");
                todo!()
            }
            _ => {
                log::info!("Bundle already published with enough prekeys");
                Ok(())
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn publish_bundle(
        aparte: &mut AparteAsync,
        account: &Account,
        device_id: u32,
        identity_key_pair: IdentityKeyPair,
        signed_pre_key_id: u32,
        signed_pre_key_pub: PublicKey,
        signed_pre_key_signature: Vec<u8>,
        pre_keys: Vec<(u32, PublicKey)>,
    ) -> Result<()> {
        log::info!("Publish device {device_id}'s bundle");
        let _response = aparte
            .iq(
                account,
                Self::publish_bundle_iq(
                    &account.to_bare(),
                    device_id,
                    identity_key_pair,
                    signed_pre_key_id,
                    signed_pre_key_pub,
                    signed_pre_key_signature,
                    pre_keys,
                ),
            )
            .await?;

        Ok(())
    }

    async fn register_device(
        aparte: &mut AparteAsync,
        account: &Account,
        device_id: u32,
        list: Option<legacy_omemo::DeviceList>,
    ) -> Result<()> {
        log::info!("Registering device {device_id}");

        let mut list = match list {
            Some(list) => list,
            None => legacy_omemo::DeviceList { devices: vec![] },
        };

        list.devices.push(legacy_omemo::Device { id: device_id });
        log::debug!("{:?}", list);

        let response = aparte
            .iq(account, Self::set_devices_iq(&account.to_bare(), list))
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
    ) -> Result<Option<legacy_omemo::Bundle>> {
        let response = aparte
            .iq(account, Self::get_bundle_iq(contact, device_id))
            .await?;
        match response.payload {
            IqType::Result(None) => Ok(None),
            IqType::Error(err)
                if err.defined_condition
                    == xmpp_parsers::stanza_error::DefinedCondition::ItemNotFound =>
            {
                Ok(None)
            }
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
                            Ok(Some(bundle))
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

        let id = id.hyphenated().to_string();
        let items = pubsub::pubsub::Items {
            max_items: None,
            node: pubsub::NodeName::from_str(ns::LEGACY_OMEMO_DEVICELIST).unwrap(),
            subid: None,
            items: vec![],
        };
        let pubsub = pubsub::PubSub::Items(items);
        Iq::from_get(id, pubsub).with_to(Jid::from(contact.clone()))
    }

    fn set_devices_iq(jid: &BareJid, devices: legacy_omemo::DeviceList) -> Iq {
        let id = Uuid::new_v4();

        let id = id.hyphenated().to_string();
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
        Iq::from_set(id, pubsub).with_to(Jid::from(jid.clone()))
    }

    fn subscribe_to_device_list_iq(contact: &BareJid, subscriber: &BareJid) -> Iq {
        let id = Uuid::new_v4().hyphenated().to_string();
        let pubsub = pubsub::PubSub::Subscribe {
            subscribe: Some(pubsub::pubsub::Subscribe {
                node: Some(pubsub::NodeName::from_str(ns::LEGACY_OMEMO_DEVICELIST).unwrap()),
                jid: Jid::from(subscriber.clone()),
            }),
            options: None,
        };
        Iq::from_set(id, pubsub).with_to(Jid::from(contact.clone()))
    }

    fn get_bundle_iq(contact: &BareJid, device_id: u32) -> Iq {
        let id = Uuid::new_v4();

        let id = id.hyphenated().to_string();
        let items = pubsub::pubsub::Items {
            max_items: None,
            node: pubsub::NodeName(format!("{}:{device_id}", ns::LEGACY_OMEMO_BUNDLES)),
            subid: None,
            items: vec![],
        };
        let pubsub = pubsub::PubSub::Items(items);
        Iq::from_get(id, pubsub).with_to(Jid::from(contact.clone()))
    }

    fn publish_bundle_iq(
        jid: &BareJid,
        device_id: u32,
        identity_key_pair: IdentityKeyPair,
        signed_pre_key_id: u32,
        signed_pre_key_pub: PublicKey,
        signed_pre_key_signature: Vec<u8>,
        pre_keys: Vec<(u32, PublicKey)>,
    ) -> Iq {
        let id = Uuid::new_v4();

        let id = id.hyphenated().to_string();
        let bundle = legacy_omemo::Bundle {
            signed_pre_key_public: Some(legacy_omemo::SignedPreKeyPublic {
                signed_pre_key_id: Some(signed_pre_key_id),
                data: signed_pre_key_pub.serialize().to_vec(),
            }),
            signed_pre_key_signature: Some(legacy_omemo::SignedPreKeySignature {
                data: signed_pre_key_signature.to_vec(),
            }),
            identity_key: Some(legacy_omemo::IdentityKey {
                data: identity_key_pair.public_key().serialize().to_vec(),
            }),
            prekeys: Some(legacy_omemo::Prekeys {
                keys: pre_keys
                    .into_iter()
                    .map(|(id, pre_key)| legacy_omemo::PreKeyPublic {
                        pre_key_id: id,
                        data: pre_key.serialize().to_vec(),
                    })
                    .collect(),
            }),
        };
        let item = pubsub::pubsub::Item(pubsub::Item {
            id: Some(pubsub::ItemId("current".to_string())),
            publisher: Some(jid.clone().into()),
            payload: Some(bundle.into()),
        });
        let pubsub = pubsub::PubSub::Publish {
            publish: pubsub::pubsub::Publish {
                node: pubsub::NodeName::from_str(&format!(
                    "{}:{}",
                    ns::LEGACY_OMEMO_BUNDLES,
                    device_id
                ))
                .unwrap(),
                items: vec![item],
            },
            publish_options: None,
        };
        Iq::from_set(id, pubsub).with_to(Jid::from(jid.clone()))
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
                    crate::error!(aparte, err, "Cannot configure OMEMO");
                }
                if let Err(err) = self.restore_sessions(aparte, account) {
                    crate::error!(aparte, err, "Cannot restore OMEMO sessions");
                }
            }
            Event::Omemo(event) => match event {
                // TODO context()?
                OmemoEvent::Enable { account, jid } => {
                    let mut aparte = aparte.proxy();
                    let account = account.clone();
                    let jid = jid.clone();
                    match self.signal_stores.get(&account) {
                        None => crate::info!(aparte, "OMEMO not configured for {account}"),
                        Some(signal_store) => Aparte::spawn({
                            let signal_store = SignalStorage::clone(signal_store);
                            async move {
                                if let Err(err) =
                                    Self::start_session(&mut aparte, &signal_store, &account, &jid)
                                        .await
                                {
                                    crate::error!(
                                        aparte,
                                        err,
                                        "Can't start OMEMO session with {jid}",
                                    );
                                }
                            }
                        }),
                    }
                }
                OmemoEvent::ShowFingerprints { account, jid } => {
                    let account = account.clone();

                    if let Err(e) = self.show_fingerprints(aparte, &account, jid) {
                        crate::error!(aparte, e, "Cannot get own OMEMO fingerprint");
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
            //    //aparte.iq::<OmemoMod>(account, self.subscribe(contact, &account.to_bare()));
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
