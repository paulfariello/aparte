mod common;

use std::convert::TryFrom;
use std::thread;
use std::time::{Duration, Instant};

use tokio_xmpp::xmlstream::XmppStreamElement;
use xmpp_parsers::{
    jid::{BareJid, Jid},
    legacy_omemo,
    message::{Id, Message},
    muc::user::{Affiliation, Role},
};

use common::omemo::ContactKeys;
use common::xmpp_fixture::{muc_join_presence_with_jid, room_subject_message, XmppFixture};
use common::{describe, row_text, INPUT_ROW, ROWS, TITLE_ROW};

const BOUND_JID: &str = "user@localhost/aparte_test";

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
    assert!(
        encrypted_msg.is_some(),
        "no OMEMO-encrypted message stanza was sent"
    );

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
    let encrypted =
        contact.encrypt_for(&aparte_jid, aparte_device_id, &aparte_bundle, "secret text");

    let mut msg = Message::chat(Some(Jid::new("user@localhost").expect("jid")));
    msg.from = Some(Jid::new(&format!("{contact_jid}/desktop")).expect("from jid"));
    msg.id = Some(Id("omemo-test-1".into()));
    msg.payloads
        .push(xmpp_parsers::minidom::Element::from(encrypted));

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

/// Sending an OMEMO message in a MUC produces an encrypted stanza that includes a key
/// for each occupant's device and that the occupant can actually decrypt.
#[test]
fn omemo_muc_outgoing_encrypted() {
    let mut contact = ContactKeys::generate();
    let contact_jid_str = "contact@localhost";

    let (fixture, mut capture) = XmppFixture::new_with_omemo_contact(
        contact_jid_str,
        contact.device_id,
        contact.bundle.clone(),
    );

    // Wait for aparte to publish its own bundle so we know configure() finished.
    let (aparte_device_id, _aparte_bundle) = capture.recv_bundle(Duration::from_secs(15));

    let room = "room@conference.localhost";

    // Join the MUC.
    fixture.send_command(&format!("/join {room}"));
    thread::sleep(Duration::from_millis(400));

    // Self-presence (real JID must be a full JID so conversation.rs converts it to bare).
    fixture.inject(muc_join_presence_with_jid(
        &format!("{room}/user"),
        BOUND_JID,
        Affiliation::Member,
        Role::Participant,
        "user@localhost/aparte",
    ));

    // Contact's presence with their real JID — this is what OMEMO uses to look up their bundle.
    fixture.inject(muc_join_presence_with_jid(
        &format!("{room}/contact"),
        BOUND_JID,
        Affiliation::Member,
        Role::Participant,
        &format!("{contact_jid_str}/desktop"),
    ));

    // Room subject marks the join as complete in the UI.
    fixture.inject(room_subject_message(
        &format!("{room}/user"),
        BOUND_JID,
        "subj-1",
        "Test room",
    ));
    thread::sleep(Duration::from_millis(300));

    // Enable OMEMO — this triggers start_session() for each occupant with a known real JID.
    fixture.send_command(&format!("/omemo enable {room}"));
    // Give start_session() time to fetch the contact's device list + bundle and build a session.
    thread::sleep(Duration::from_secs(4));

    fixture.switch_window(room);
    fixture.send_command("hello encrypted muc");

    let encrypted_msg = capture.recv_message_matching(message_has_omemo, Duration::from_secs(5));

    if encrypted_msg.is_none() {
        let screen = fixture.snapshot();
        eprintln!("{}", common::describe(screen.screen()));
    }
    assert!(
        encrypted_msg.is_some(),
        "no OMEMO-encrypted groupchat message was sent"
    );

    let msg = encrypted_msg.unwrap();
    let encrypted = msg
        .payloads
        .iter()
        .find_map(|p| legacy_omemo::Encrypted::try_from(p.clone()).ok())
        .expect("no <encrypted> element in the groupchat stanza");

    // The encrypted message must include a key for the contact's device.
    assert!(
        encrypted
            .header
            .keys
            .iter()
            .any(|k| k.rid == contact.device_id),
        "encrypted groupchat message has no key for contact device {}; keys present: {:?}",
        contact.device_id,
        encrypted
            .header
            .keys
            .iter()
            .map(|k| k.rid)
            .collect::<Vec<_>>(),
    );

    // The contact must be able to decrypt the message end-to-end.
    let aparte_bare = BareJid::new("user@localhost").expect("bare jid");
    let plaintext = contact
        .decrypt_from(&aparte_bare, aparte_device_id, &encrypted)
        .expect("contact could not decrypt the MUC message");
    assert_eq!(
        plaintext, "hello encrypted muc",
        "decrypted MUC message body mismatch"
    );
}

/// After `/omemo enable`, the title bar (row ROWS-2) shows 🔒 for that conversation.
#[test]
fn omemo_enabled_shows_lock_in_titlebar() {
    let contact_jid = "contact@localhost";
    let contact = ContactKeys::generate();

    let (fixture, mut capture) =
        XmppFixture::new_with_omemo_contact(contact_jid, contact.device_id, contact.bundle);

    // Wait for aparte to publish its own bundle — confirms configure() has run.
    let _ = capture.recv_bundle(Duration::from_secs(15));

    // Open the chat window with the contact, then enable OMEMO.
    fixture.send_command(&format!("/msg {contact_jid}"));
    fixture.send_command(&format!("/omemo enable {contact_jid}"));

    // Poll the title-bar row (ROWS-2) until 🔒 appears, up to 10 seconds.
    // start_session() is async so the OmemoEvent::Enabled fires after
    // the bundle fetch completes, not immediately.
    let title_row = TITLE_ROW;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut found = false;
    while Instant::now() < deadline {
        let parser = fixture.snapshot();
        if row_text(parser.screen(), title_row).contains("🔒") {
            found = true;
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }

    if !found {
        let parser = fixture.snapshot();
        eprintln!("{}", describe(parser.screen()));
    }
    assert!(found, "🔒 not shown in title bar after /omemo enable");
}

/// 🔒 must persist in the title bar after entering COMMAND mode (`:` key).
/// COMMAND mode uses a 9-char label vs the 8-char INSERT/NORMAL labels, which
/// shifts the title one column right.  A stale wide-char continuation in the
/// reference screen used to cause a false match so 🔒 was never re-emitted.
#[test]
fn omemo_lock_persists_in_command_mode() {
    let contact_jid = "contact@localhost";
    let contact = ContactKeys::generate();

    let (fixture, mut capture) =
        XmppFixture::new_with_omemo_contact(contact_jid, contact.device_id, contact.bundle);

    let _ = capture.recv_bundle(Duration::from_secs(15));

    fixture.send_command(&format!("/msg {contact_jid}"));
    fixture.send_command(&format!("/omemo enable {contact_jid}"));

    let title_row = TITLE_ROW;

    // Wait for 🔒 in INSERT mode.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut found = false;
    while Instant::now() < deadline {
        if row_text(fixture.snapshot().screen(), title_row).contains("🔒") {
            found = true;
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    assert!(found, "🔒 not shown in INSERT mode — prerequisite failed");

    // Press ':' → COMMAND mode (mode label expands from 8 to 9 chars, shifting
    // every subsequent column right by one, including the 🔒 position).
    fixture.send_bytes(b"\x1b");
    let normal_shown = fixture.wait_for("NORMAL", Duration::from_secs(3));
    assert!(normal_shown, "did not enter NORMAL mode before ':'");
    fixture.send_bytes(b":");
    let cmd_shown = fixture.wait_for("COMMAND", Duration::from_secs(3));
    assert!(cmd_shown, "did not enter COMMAND mode");

    let parser = fixture.snapshot();
    let bar = row_text(parser.screen(), title_row);
    assert!(
        bar.contains("🔒"),
        "🔒 disappeared from title bar after entering COMMAND mode; got: {:?}\n{}",
        bar,
        describe(parser.screen())
    );
}

/// 🔒 must persist in the title bar after switching to NORMAL mode (Escape).
#[test]
fn omemo_lock_persists_in_normal_mode() {
    let contact_jid = "contact@localhost";
    let contact = ContactKeys::generate();

    let (fixture, mut capture) =
        XmppFixture::new_with_omemo_contact(contact_jid, contact.device_id, contact.bundle);

    let _ = capture.recv_bundle(Duration::from_secs(15));

    fixture.send_command(&format!("/msg {contact_jid}"));
    fixture.send_command(&format!("/omemo enable {contact_jid}"));

    let title_row = TITLE_ROW;

    // Wait for 🔒 to appear in the title bar while still in INSERT mode.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut found = false;
    while Instant::now() < deadline {
        if row_text(fixture.snapshot().screen(), title_row).contains("🔒") {
            found = true;
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    assert!(found, "🔒 not shown in INSERT mode — prerequisite failed");

    // Press Escape → NORMAL mode.
    fixture.send_bytes(b"\x1b");
    // Wait until the title bar shows NORMAL.
    let normal_shown = fixture.wait_for("NORMAL", Duration::from_secs(3));
    assert!(normal_shown, "did not enter NORMAL mode");

    // 🔒 must still be visible in the title bar.
    let parser = fixture.snapshot();
    let bar = row_text(parser.screen(), title_row);
    assert!(
        bar.contains("🔒"),
        "🔒 disappeared from title bar after entering NORMAL mode; got: {:?}\n{}",
        bar,
        describe(parser.screen())
    );
}
