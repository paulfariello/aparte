mod common;

use std::thread;
use std::time::Duration;

use tokio_xmpp::xmlstream::XmppStreamElement;
use xmpp_parsers::{
    jid::{BareJid, Jid},
    message::{Id, Message},
};

use common::omemo::ContactKeys;
use common::xmpp_fixture::XmppFixture;

fn message_has_omemo(msg: &xmpp_parsers::message::Message) -> bool {
    msg.payloads
        .iter()
        .any(|p| p.is("encrypted", "eu.siacs.conversations.axolotl"))
}

/// `/omemo fingerprint` shows the own device's fingerprint in the UI.
#[test]
fn omemo_fingerprint_shown() {
    let (fixture, _capture) = XmppFixture::new_with_omemo(&[]);
    // configure() stores the identity synchronously before the spawn,
    // so the fingerprint is readable immediately after connection.
    fixture.send_command("/omemo fingerprint");
    let found = fixture.wait_for("\u{1f6e1}", Duration::from_secs(5));
    if !found {
        let screen = fixture.snapshot();
        eprintln!("{}", common::describe(screen.screen()));
    }
    assert!(found, "fingerprint emoji not found in UI");
}

/// Sending a message to an OMEMO-enabled contact produces an encrypted stanza on the wire.
#[test]
fn omemo_outgoing_encrypted() {
    let contact = ContactKeys::generate();
    let contact_jid = "contact@localhost";

    let (fixture, mut capture) =
        XmppFixture::new_with_omemo_contact(contact_jid, contact.device_id, contact.bundle);

    // Wait for aparte to publish its own bundle (confirm configure finished)
    let _ = capture.recv_bundle(Duration::from_secs(15));

    fixture.send_command(&format!("/msg {contact_jid}"));
    fixture.send_command(&format!("/omemo enable {contact_jid}"));
    // start_session is async; give it time to download the bundle and set up a session
    thread::sleep(Duration::from_secs(3));

    fixture.send_command("hello encrypted world");

    let encrypted_msg = capture.recv_message_matching(message_has_omemo, Duration::from_secs(5));

    if encrypted_msg.is_none() {
        let screen = fixture.snapshot();
        eprintln!("{}", common::describe(screen.screen()));
    }
    assert!(encrypted_msg.is_some(), "no OMEMO-encrypted message stanza was sent");

    // The outgoing message displayed in the UI must carry the lock emoji.
    let lock_shown = fixture.wait_for("\u{1f512}", Duration::from_secs(5));
    if !lock_shown {
        let screen = fixture.snapshot();
        eprintln!("{}", common::describe(screen.screen()));
    }
    assert!(lock_shown, "🔒 not shown for outgoing OMEMO message");
}

/// Injecting an OMEMO-encrypted message from a contact results in decrypted plaintext in the UI.
#[test]
fn omemo_incoming_decrypted() {
    let mut contact = ContactKeys::generate();
    let contact_jid = "contact@localhost";

    let (fixture, mut capture) =
        XmppFixture::new_with_omemo_contact(contact_jid, contact.device_id, contact.bundle.clone());

    // Capture aparte's own bundle so we can encrypt for its device
    let (aparte_device_id, aparte_bundle) = capture.recv_bundle(Duration::from_secs(15));

    fixture.send_command(&format!("/msg {contact_jid}"));
    fixture.send_command(&format!("/omemo enable {contact_jid}"));
    // Wait for start_session to download contact's bundle + establish Signal session
    thread::sleep(Duration::from_secs(3));

    // Build an OMEMO-encrypted message from contact to aparte
    let aparte_jid = BareJid::new("user@localhost").expect("bare jid");
    let encrypted = contact.encrypt_for(&aparte_jid, aparte_device_id, &aparte_bundle, "secret text");

    let mut msg = Message::chat(Some(Jid::new("user@localhost").expect("jid")));
    msg.from = Some(Jid::new(&format!("{contact_jid}/desktop")).expect("from jid"));
    msg.id = Some(Id("omemo-test-1".into()));
    msg.payloads.push(
        xmpp_parsers::minidom::Element::from(encrypted),
    );

    fixture.inject(XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg)));

    let found = fixture.wait_for("secret text", Duration::from_secs(5));
    if !found {
        let screen = fixture.snapshot();
        eprintln!("{}", common::describe(screen.screen()));
    }
    assert!(found, "decrypted message body not found in UI");

    // Encrypted incoming message must show the lock emoji in the header.
    let lock_shown = fixture.wait_for("\u{1f512}", Duration::from_secs(5));
    if !lock_shown {
        let screen = fixture.snapshot();
        eprintln!("{}", common::describe(screen.screen()));
    }
    assert!(lock_shown, "🔒 not shown for incoming OMEMO message");
}
