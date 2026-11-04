use std::convert::TryFrom;

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes128Gcm,
};
use futures::FutureExt;
use libsignal_protocol::{
    message_decrypt, message_encrypt, process_prekey_bundle, CiphertextMessage, IdentityKey,
    IdentityKeyPair, InMemSignalProtocolStore, KeyPair, PreKeyBundle, PreKeyRecord,
    PreKeySignalMessage, PreKeyStore, ProtocolAddress, PublicKey, SignalMessage,
    SignedPreKeyRecord, SignedPreKeyStore,
};
use rand::{random, thread_rng};
use xmpp_parsers::{jid::BareJid, legacy_omemo};

/// Keys and Signal store for a simulated remote contact in tests.
pub struct ContactKeys {
    pub device_id: u32,
    pub bundle: legacy_omemo::Bundle,
    store: InMemSignalProtocolStore,
    identity_key_pair: IdentityKeyPair,
}

impl ContactKeys {
    pub fn generate() -> Self {
        let mut rng = thread_rng();
        let device_id: u32 = random::<u32>() % (2u32.pow(31) - 1) + 1;
        let identity = IdentityKeyPair::generate(&mut rng);
        let signed_pre_key = KeyPair::generate(&mut rng);
        let signed_pre_key_id = 1u32;
        let sig: Vec<u8> = identity
            .private_key()
            .calculate_signature(&signed_pre_key.public_key.serialize(), &mut rng)
            .expect("signature")
            .into_vec();
        let pre_key = KeyPair::generate(&mut rng);
        let pre_key_id = 1u32;

        let bundle = legacy_omemo::Bundle {
            signed_pre_key_public: Some(legacy_omemo::SignedPreKeyPublic {
                signed_pre_key_id: Some(signed_pre_key_id),
                data: signed_pre_key.public_key.serialize().to_vec(),
            }),
            signed_pre_key_signature: Some(legacy_omemo::SignedPreKeySignature {
                data: sig.to_vec(),
            }),
            identity_key: Some(legacy_omemo::IdentityKey {
                data: identity.public_key().serialize().to_vec(),
            }),
            prekeys: Some(legacy_omemo::Prekeys {
                keys: vec![legacy_omemo::PreKeyPublic {
                    pre_key_id,
                    data: pre_key.public_key.serialize().to_vec(),
                }],
            }),
        };

        let mut store = InMemSignalProtocolStore::new(identity, device_id).expect("store");
        store
            .save_pre_key(
                pre_key_id.into(),
                &PreKeyRecord::new(pre_key_id.into(), &pre_key),
                None,
            )
            .now_or_never()
            .expect("future resolved")
            .expect("save pre key");
        store
            .save_signed_pre_key(
                signed_pre_key_id.into(),
                &SignedPreKeyRecord::new(signed_pre_key_id.into(), 0, &signed_pre_key, &sig),
                None,
            )
            .now_or_never()
            .expect("future resolved")
            .expect("save signed pre key");

        Self {
            device_id,
            bundle,
            store,
            identity_key_pair: identity,
        }
    }

    /// Encrypt `body` for `aparte`'s device and return the OMEMO Encrypted element.
    pub fn encrypt_for(
        &mut self,
        aparte_jid: &BareJid,
        aparte_device_id: u32,
        aparte_bundle: &legacy_omemo::Bundle,
        body: &str,
    ) -> legacy_omemo::Encrypted {
        let aparte_addr = ProtocolAddress::new(aparte_jid.to_string(), aparte_device_id.into());
        let prekey_bundle = bundle_to_prekey_bundle(aparte_device_id, aparte_bundle);

        let mut store2 = self.store.clone();
        process_prekey_bundle(
            &aparte_addr,
            &mut store2,
            &mut self.store,
            &prekey_bundle,
            &mut thread_rng(),
            None,
        )
        .now_or_never()
        .expect("future resolved")
        .expect("process_prekey_bundle");
        self.store = store2;

        const KEY_SIZE: usize = 16;
        const MAC_SIZE: usize = 16;

        let dek = Aes128Gcm::generate_key(OsRng);
        let nonce = Aes128Gcm::generate_nonce(&mut OsRng);
        let cipher = Aes128Gcm::new(&dek);
        let encrypted_body = cipher
            .encrypt(&nonce, body.as_bytes())
            .expect("encrypt body");

        let mut dek_and_mac = vec![0u8; KEY_SIZE + MAC_SIZE];
        dek_and_mac[..KEY_SIZE].copy_from_slice(&dek);
        dek_and_mac[KEY_SIZE..].copy_from_slice(&encrypted_body[body.len()..]);

        let mut store2 = self.store.clone();
        let ciphertext = message_encrypt(
            &dek_and_mac,
            &aparte_addr,
            &mut self.store,
            &mut store2,
            None,
        )
        .now_or_never()
        .expect("future resolved")
        .expect("message_encrypt");
        self.store = store2;

        let (prekey, data) = match ciphertext {
            CiphertextMessage::PreKeySignalMessage(m) => (true, m.serialized().to_vec()),
            CiphertextMessage::SignalMessage(m) => (false, m.serialized().to_vec()),
            _ => unreachable!("unexpected ciphertext type"),
        };

        legacy_omemo::Encrypted {
            header: legacy_omemo::Header {
                sid: self.device_id,
                iv: legacy_omemo::IV {
                    data: nonce.to_vec(),
                },
                keys: vec![legacy_omemo::Key {
                    rid: aparte_device_id,
                    prekey,
                    data,
                }],
            },
            payload: Some(legacy_omemo::Payload {
                data: encrypted_body[..body.len()].to_vec(),
            }),
        }
    }

    /// Decrypt an OMEMO-encrypted stanza sent to this contact's device.
    pub fn decrypt_from(
        &mut self,
        sender_jid: &BareJid,
        sender_device_id: u32,
        encrypted: &legacy_omemo::Encrypted,
    ) -> anyhow::Result<String> {
        const KEY_SIZE: usize = 16;
        const MAC_SIZE: usize = 16;

        let key = encrypted
            .header
            .keys
            .iter()
            .find(|k| k.rid == self.device_id)
            .ok_or_else(|| anyhow::anyhow!("no key for device {}", self.device_id))?;

        let ciphertext_message = if key.prekey {
            CiphertextMessage::PreKeySignalMessage(
                PreKeySignalMessage::try_from(key.data.as_slice())
                    .map_err(|e| anyhow::anyhow!("invalid prekey message: {e}"))?,
            )
        } else {
            CiphertextMessage::SignalMessage(
                SignalMessage::try_from(key.data.as_slice())
                    .map_err(|e| anyhow::anyhow!("invalid signal message: {e}"))?,
            )
        };

        let remote_address = ProtocolAddress::new(sender_jid.to_string(), sender_device_id.into());

        // Clone the store for each trait-object slot that message_decrypt needs.
        // InMemSignalProtocolStore is not splittable into independent mutable
        // borrows, so we clone per parameter and merge the mutation we care
        // about (the consumed pre-key) back via whichever clone handles it.
        let mut s_session = self.store.clone();
        let mut s_identity = self.store.clone();
        let mut s_signed = self.store.clone();
        let dek_and_mac = message_decrypt(
            &ciphertext_message,
            &remote_address,
            &mut s_session,
            &mut s_identity,
            &mut self.store, // pre_key_store — consumes the prekey here
            &mut s_signed,
            &mut thread_rng(),
            None,
        )
        .now_or_never()
        .ok_or_else(|| anyhow::anyhow!("decrypt future did not resolve synchronously"))?
        .map_err(|e| anyhow::anyhow!("Signal decryption failed: {e}"))?;

        anyhow::ensure!(
            dek_and_mac.len() == KEY_SIZE + MAC_SIZE,
            "unexpected DEK+MAC length {}",
            dek_and_mac.len()
        );

        let dek = aes_gcm::Key::<Aes128Gcm>::from_slice(&dek_and_mac[..KEY_SIZE]);
        let mac = &dek_and_mac[KEY_SIZE..KEY_SIZE + MAC_SIZE];

        let payload = encrypted
            .payload
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no payload in encrypted element"))?;

        let mut payload_and_mac = Vec::with_capacity(payload.data.len() + MAC_SIZE);
        payload_and_mac.extend(&payload.data);
        payload_and_mac.extend(mac);

        let nonce = aes_gcm::Nonce::<<Aes128Gcm as AeadCore>::NonceSize>::from_slice(
            &encrypted.header.iv.data,
        );
        let cipher = Aes128Gcm::new(dek);
        let cleartext = cipher
            .decrypt(nonce, payload_and_mac.as_slice())
            .map_err(|_| anyhow::anyhow!("AES-GCM decryption failed"))?;

        String::from_utf8(cleartext).map_err(|e| anyhow::anyhow!("utf-8 error: {e}"))
    }
}

fn bundle_to_prekey_bundle(device_id: u32, bundle: &legacy_omemo::Bundle) -> PreKeyBundle {
    let spk = bundle.signed_pre_key_public.as_ref().expect("spk");
    let signed_pre_key_id = spk.signed_pre_key_id.expect("spk id");
    let signed_pre_key = PublicKey::deserialize(&spk.data).expect("spk pubkey");
    let sig = &bundle
        .signed_pre_key_signature
        .as_ref()
        .expect("spk sig")
        .data;
    let identity_key =
        IdentityKey::decode(&bundle.identity_key.as_ref().expect("identity key").data)
            .expect("identity key decode");
    let prekey = bundle
        .prekeys
        .as_ref()
        .expect("prekeys")
        .keys
        .first()
        .expect("at least one prekey");
    let pre_key_id = prekey.pre_key_id;
    let pre_key_pub = PublicKey::deserialize(&prekey.data).expect("prekey pubkey");

    PreKeyBundle::new(
        0,
        device_id.into(),
        Some((pre_key_id.into(), pre_key_pub)),
        signed_pre_key_id.into(),
        signed_pre_key,
        sig.to_vec(),
        identity_key,
    )
    .expect("prekey bundle")
}
