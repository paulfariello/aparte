//! Integration tests for MessageStoreMod (XEP-384 cleartext persistence).
//!
//! OMEMO uses the Double Ratchet algorithm: each message is decrypted once and the
//! ratchet advances, preventing re-decryption. When MAM replays encrypted stanzas
//! after a restart, they can no longer be decrypted. MessageStoreMod persists the
//! cleartext on first receipt and looks it up on MAM replay.
//!
//! These tests cover:
//!   1. An incoming OMEMO 1:1 message is stored and restored when the same stanza
//!      is replayed with a failing decryption (pre-key consumed).
//!   2. A sent OMEMO MUC message whose server echo arrives encrypted (decryption
//!      fails — no key for our own device) is not displayed a second time.
//!   3. A sent OMEMO 1:1 message is stored at send time and restored when MAM
//!      replays the encrypted stanza (simulating history fetch after restart).

mod common;

use std::thread;
use std::time::{Duration, Instant};

use xmpp_parsers::{
    jid::BareJid,
    muc::user::{Affiliation, Role},
};

use common::omemo::ContactKeys;
use common::xmpp_fixture::{
    muc_join_presence_with_jid, omemo_encrypted_chat_message, omemo_encrypted_chat_replay,
    omemo_encrypted_groupchat_echo, room_subject_message, XmppFixture,
};
use common::{describe, row_text, ROWS};

const BOUND_JID: &str = "user@localhost/aparte_test";
const CONTACT_JID: &str = "contact@localhost";
const ROOM: &str = "room@conference.localhost";

fn message_has_omemo(msg: &xmpp_parsers::message::Message) -> bool {
    msg.payloads
        .iter()
        .any(|p| p.is("encrypted", "eu.siacs.conversations.axolotl"))
}

/// An incoming OMEMO 1:1 message is decrypted, persisted to the cleartext store,
/// and re-displayed when the same stanza is replayed (pre-key consumed → decryption
/// fails on second pass, so MessageStoreMod must supply the body from the DB).
///
/// The original message is injected with a past `<delay>` timestamp so the
/// replayed message — timestamped to `now` — gets a different BTreeSet key and
/// appears as a second entry in the chat window.
#[test]
fn message_store_incoming_omemo_restored_on_replay() {
    let mut contact = ContactKeys::generate();

    let (fixture, mut capture) =
        XmppFixture::new_with_omemo_contact(CONTACT_JID, contact.device_id, contact.bundle.clone());

    // Wait for aparte to publish its own bundle (configure() has finished).
    let (aparte_device_id, aparte_bundle) = capture.recv_bundle(Duration::from_secs(15));

    fixture.send_command(&format!("/msg {CONTACT_JID}"));
    fixture.send_command(&format!("/omemo enable {CONTACT_JID}"));
    // Allow start_session() to download the contact bundle and set up the Signal session.
    thread::sleep(Duration::from_secs(3));

    // ── Step 1: inject the original encrypted message (decryption succeeds) ─────
    // Use a fixed past timestamp so the replayed stanza gets a different clock-based
    // timestamp → two distinct BTreeSet keys → both visible in the chat window.
    let aparte_bare = BareJid::new("user@localhost").expect("bare jid");
    let encrypted = contact.encrypt_for(
        &aparte_bare,
        aparte_device_id,
        &aparte_bundle,
        "mstore-body-k9r2",
    );
    let encrypted_elem = xmpp_parsers::minidom::Element::from(encrypted);

    fixture.inject(omemo_encrypted_chat_message(
        &format!("{CONTACT_JID}/desktop"),
        "user@localhost",
        "mstore-orig-id-k9r2",
        encrypted_elem.clone(),
        Some("2020-06-01T12:00:00Z"),
    ));

    assert!(
        fixture.wait_for("mstore-body-k9r2", Duration::from_secs(5)),
        "original OMEMO message was not decrypted and displayed"
    );

    // ── Step 2: replay the same stanza (pre-key already consumed → decryption fails) ─
    // MessageStoreMod::handle_xmpp_message must look up "mstore-orig-id-k9r2" in the
    // cleartext store and re-emit Event::Message with the stored body.
    // The replayed message has no <delay> so its timestamp = LocalTz::now() ≠ the
    // original 2020 timestamp; the ScrollWin BTreeSet treats them as distinct entries.
    fixture.inject(omemo_encrypted_chat_message(
        &format!("{CONTACT_JID}/desktop"),
        "user@localhost",
        "mstore-orig-id-k9r2",
        encrypted_elem,
        None, // timestamp = now (differs from 2020 timestamp above)
    ));

    // Give aparte time to process the replay.
    thread::sleep(Duration::from_millis(800));

    let parser = fixture.snapshot();
    let count = (0..ROWS)
        .filter(|&r| row_text(parser.screen(), r).contains("mstore-body-k9r2"))
        .count();
    assert!(
        count >= 2,
        "expected the stored cleartext to appear at least twice (original + restored), \
         got {count}\n{}",
        describe(parser.screen()),
    );
}

/// The MUC server echoes back an OMEMO-encrypted groupchat message. Since the stanza
/// is encrypted only for room members' devices (not for our own device in the test
/// setup), decryption fails and MessageStoreMod::handle_xmpp_message claims the echo.
/// With the sent_muc_ids guard the echo must be discarded — no duplicate display.
#[test]
fn message_store_muc_omemo_echo_not_duplicated() {
    let contact = ContactKeys::generate();

    let (fixture, mut capture) =
        XmppFixture::new_with_omemo_contact(CONTACT_JID, contact.device_id, contact.bundle.clone());

    let (_aparte_device_id, _aparte_bundle) = capture.recv_bundle(Duration::from_secs(15));

    // ── Join the MUC and set up OMEMO ────────────────────────────────────────
    fixture.send_command(&format!("/join {ROOM}"));
    thread::sleep(Duration::from_millis(400));

    // Self-presence (real JID required so conversation.rs can build the Channel).
    fixture.inject(muc_join_presence_with_jid(
        &format!("{ROOM}/user"),
        BOUND_JID,
        Affiliation::Member,
        Role::Participant,
        "user@localhost/aparte",
    ));
    // Contact's presence with their real JID (needed for OMEMO member lookup).
    fixture.inject(muc_join_presence_with_jid(
        &format!("{ROOM}/contact"),
        BOUND_JID,
        Affiliation::Member,
        Role::Participant,
        &format!("{CONTACT_JID}/desktop"),
    ));
    // Room subject marks the join complete.
    fixture.inject(room_subject_message(
        &format!("{ROOM}/user"),
        BOUND_JID,
        "subj-mstore",
        "Test room",
    ));
    thread::sleep(Duration::from_millis(300));

    // Switch to the room window before enabling OMEMO so we can watch for 🔒.
    fixture.switch_window(ROOM);
    fixture.send_command(&format!("/omemo enable {ROOM}"));

    // Wait until the 🔒 appears in the title bar — this confirms that
    // start_session() completed and the Signal session is established.
    // A fixed sleep is not reliable under parallel test load.
    let title_row = ROWS - 2;
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut omemo_ready = false;
    while Instant::now() < deadline {
        if row_text(fixture.snapshot().screen(), title_row).contains('\u{1f512}') {
            omemo_ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    assert!(
        omemo_ready,
        "🔒 did not appear in room title bar within 15s"
    );

    fixture.send_command("mstore-muc-echo-4f9z");

    // ── Capture the encrypted MUC stanza ────────────────────────────────────
    let sent = capture
        .recv_message_matching(message_has_omemo, Duration::from_secs(5))
        .expect("no OMEMO MUC stanza captured");
    let msg_id = sent.id.as_ref().expect("sent stanza has no id").0.clone();

    assert!(
        fixture.wait_for("mstore-muc-echo-4f9z", Duration::from_secs(5)),
        "outgoing MUC OMEMO message did not appear in UI"
    );

    // ── Inject the MUC server echo ───────────────────────────────────────────
    // The echo carries the same encrypted payload (from=room/user). Because the
    // test fixture's OMEMO setup does not include a key for aparte's own device,
    // decryption fails → MessageStoreMod claims the stanza (0.005 confidence).
    // The cleartext WAS saved by save_if_encrypted when Event::SendMessage fired,
    // so without the sent_muc_ids guard MessageStoreMod would restore it and
    // cause a duplicate display.
    fixture.inject(omemo_encrypted_groupchat_echo(
        &format!("{ROOM}/user"),
        BOUND_JID,
        &msg_id,
        sent.payloads.clone(),
    ));
    thread::sleep(Duration::from_millis(600));

    let parser = fixture.snapshot();
    let count = (0..ROWS)
        .filter(|&r| row_text(parser.screen(), r).contains("mstore-muc-echo-4f9z"))
        .count();
    assert_eq!(
        count,
        1,
        "MUC OMEMO message appeared {count} times; expected once \
         (MessageStoreMod::sent_muc_ids must suppress the echo)\n{}",
        describe(parser.screen()),
    );
}

/// Sent OMEMO 1:1 messages are saved to the cleartext store when sent, and restored
/// from the store when MAM replays the encrypted stanza (decryption fails because
/// the ratchet advanced after restart).
///
/// The replay is injected with a past `<delay>` timestamp so the BTreeSet sees two
/// distinct entries (original at T=now, restore at T=2020) → both visible in the UI.
#[test]
fn message_store_sent_omemo_restored_on_mam_replay() {
    let contact = ContactKeys::generate();

    let (fixture, mut capture) =
        XmppFixture::new_with_omemo_contact(CONTACT_JID, contact.device_id, contact.bundle.clone());

    let (_aparte_device_id, _aparte_bundle) = capture.recv_bundle(Duration::from_secs(15));

    fixture.send_command(&format!("/msg {CONTACT_JID}"));
    fixture.send_command(&format!("/omemo enable {CONTACT_JID}"));
    thread::sleep(Duration::from_secs(3));

    // ── Step 1: send an OMEMO message and capture the encrypted stanza ───────
    fixture.send_command("mstore-sent-k7p4");

    let sent = capture
        .recv_message_matching(message_has_omemo, Duration::from_secs(5))
        .expect("no OMEMO stanza from sent chat message");
    let msg_id = sent.id.as_ref().expect("sent stanza has no id").0.clone();

    assert!(
        fixture.wait_for("mstore-sent-k7p4", Duration::from_secs(5)),
        "sent OMEMO message was not displayed in UI"
    );

    // ── Step 2: inject a MAM replay ──────────────────────────────────────────
    // Same encrypted payloads, from=our JID, past timestamp so the BTreeSet
    // produces a second distinct entry (timestamp 2020 ≠ original send time).
    // Decryption is impossible (pre-key consumed), so MessageStoreMod must look
    // up the cleartext saved at send time and re-emit Event::Message.
    fixture.inject(omemo_encrypted_chat_replay(
        "user@localhost",
        CONTACT_JID,
        &msg_id,
        sent.payloads.clone(),
        Some("2020-06-01T12:00:00Z"),
    ));

    thread::sleep(Duration::from_millis(800));

    let parser = fixture.snapshot();
    let count = (0..ROWS)
        .filter(|&r| row_text(parser.screen(), r).contains("mstore-sent-k7p4"))
        .count();
    assert!(
        count >= 2,
        "expected sent OMEMO cleartext to appear at least twice (original + MAM restore), \
         got {count}\n{}",
        describe(parser.screen()),
    );
}
