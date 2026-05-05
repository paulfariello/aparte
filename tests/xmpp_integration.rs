//! XMPP integration tests with a mock TCP server.
//!
//! Each test:
//!   1. Binds a random TCP port and runs a minimal XMPP handshake server
//!      (SASL PLAIN → bind → stanza loop) in a Tokio runtime.
//!   2. Launches the real `aparte` binary inside a PTY with a config that
//!      points at that port, `APARTE_INSECURE_XMPP=1`, and `autoconnect=true`.
//!   3. Polls the vt100 screen until the expected text appears (or times out).

mod common;

use std::thread;
use std::time::Duration;

use rstest::rstest;
use tokio_xmpp::xmlstream::XmppStreamElement;
use xmpp_parsers::jid::Jid;
use xmpp_parsers::presence::Show;

use common::describe;
use common::grid_contains;
use common::row_text;
use common::xmpp_fixture::{
    carbon_received, carbon_received_groupchat, carbon_sent, carbon_sent_groupchat, chat_message,
    contact_offline_presence, contact_presence, corrected_chat_message, groupchat_message,
    muc_join_presence, xmpp, xmpp_with_contact, XmppFixture,
};
use common::ROWS;

/// Verify that carbon stanzas can be built and round-trip through xmpp-parsers.
#[test]
fn carbon_type_construction() {
    let stanza = carbon_received(
        "user@localhost",
        "user@localhost/aparte_test",
        "contact@localhost",
        "user@localhost",
        "c1",
        "Carbon message!",
    );
    match stanza {
        XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg)) => {
            assert!(
                !msg.payloads.is_empty(),
                "payloads should contain <received>"
            );
            assert_eq!(msg.from, Some(Jid::new("user@localhost").unwrap()));
        }
        _ => panic!("expected a Message stanza"),
    }
}

/// After a successful connection the bound JID appears in the console log.
#[rstest]
fn connection_shows_jid_in_console(xmpp: XmppFixture) {
    let found = xmpp.wait_for("Connected as", Duration::from_secs(1));
    let parser = xmpp.snapshot();
    assert!(
        found,
        "expected 'Connected as' in screen\n{}",
        describe(parser.screen()),
    );
}

/// An incoming chat message body appears in the UI.
#[rstest]
fn incoming_chat_message_appears_in_ui(xmpp: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    xmpp.inject(chat_message(
        "contact@localhost",
        "user@localhost/aparte_test",
        "m1",
        "Hello from mock!",
    ));
    xmpp.switch_window("contact@localhost");
    let found = xmpp.wait_for("Hello from mock!", Duration::from_secs(5));
    let parser = xmpp.snapshot();
    assert!(
        found,
        "expected 'Hello from mock!' in screen within 5s\n{}",
        describe(parser.screen()),
    );
}

/// An incoming carbon copy (XEP-0280 received) shows the forwarded body.
#[rstest]
fn incoming_carbon_appears_in_ui(xmpp: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    xmpp.inject(carbon_received(
        "user@localhost",
        "user@localhost/aparte_test",
        "contact@localhost",
        "user@localhost",
        "c1",
        "Carbon message!",
    ));
    xmpp.switch_window("contact@localhost");
    let found = xmpp.wait_for("Carbon message!", Duration::from_secs(5));
    let parser = xmpp.snapshot();
    assert!(
        found,
        "expected 'Carbon message!' in screen within 5s\n{}",
        describe(parser.screen()),
    );
}

/// After a successful connection the server's roster response populates the
/// contact list shown in the console window's right-hand panel.
#[rstest]
fn roster_contacts_displayed_in_ui(xmpp_with_contact: XmppFixture) {
    let found = xmpp_with_contact.wait_for("contact@localhost", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        found,
        "expected 'contact@localhost' in roster panel within 5s\n{}",
        describe(parser.screen()),
    );
}

/// /msg <contact> opens a chat window visible in the win-bar.
#[rstest]
fn msg_command_opens_chat_window(xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    xmpp_with_contact.send_command("/msg contact@localhost");
    let found = xmpp_with_contact.wait_for("contact@localhost", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        found,
        "/msg did not open a chat window\n{}",
        describe(parser.screen()),
    );
}

/// Typing a message in a chat window sends it and displays it locally.
#[rstest]
fn outgoing_chat_message_appears_in_ui(xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    xmpp_with_contact.send_command("/msg contact@localhost");
    thread::sleep(Duration::from_millis(400));
    xmpp_with_contact.send_command("Hello outgoing!");
    let found = xmpp_with_contact.wait_for("Hello outgoing!", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        found,
        "outgoing message not in UI\n{}",
        describe(parser.screen()),
    );
}

/// /win switches to a different window.
#[rstest]
fn win_command_switches_window(xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    xmpp_with_contact.send_command("/msg contact@localhost");
    thread::sleep(Duration::from_millis(400));
    xmpp_with_contact.send_command("/win console");
    let found = xmpp_with_contact.wait_for("Connected as", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        found,
        "/win console did not switch to console window\n{}",
        describe(parser.screen()),
    );
}

/// Contact coming online keeps the contact visible in the roster panel.
#[rstest]
fn contact_presence_available(xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    xmpp_with_contact.inject(contact_presence(
        "contact@localhost/mobile",
        "user@localhost",
        None,
        None,
    ));
    let found = xmpp_with_contact.wait_for("contact@localhost", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        found,
        "contact disappeared from UI after available presence\n{}",
        describe(parser.screen()),
    );
}

/// Contact going away keeps the contact visible in the roster panel.
#[rstest]
fn contact_presence_away(xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    xmpp_with_contact.inject(contact_presence(
        "contact@localhost/mobile",
        "user@localhost",
        Some(Show::Away),
        Some("Out for lunch"),
    ));
    let found = xmpp_with_contact.wait_for("contact@localhost", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        found,
        "contact disappeared from UI after away presence\n{}",
        describe(parser.screen()),
    );
}

/// Contact going offline keeps the contact visible in the roster panel.
#[rstest]
fn contact_presence_offline(xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    xmpp_with_contact.inject(contact_presence(
        "contact@localhost/mobile",
        "user@localhost",
        None,
        None,
    ));
    thread::sleep(Duration::from_millis(200));
    xmpp_with_contact.inject(contact_offline_presence(
        "contact@localhost/mobile",
        "user@localhost",
    ));
    let found = xmpp_with_contact.wait_for("contact@localhost", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        found,
        "contact disappeared from UI after offline presence\n{}",
        describe(parser.screen()),
    );
}

/// Receiving a corrected message is processed without crashing the app.
/// The correction is applied to the in-memory model; the chat window remains
/// visible (the UI re-render of an updated message is a known limitation of
/// the BTreeSet-based ScrollWin).
#[rstest]
fn incoming_message_correction_is_processed(xmpp: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    xmpp.inject(chat_message(
        "contact@localhost",
        "user@localhost/aparte_test",
        "orig1",
        "Original text",
    ));
    xmpp.switch_window("contact@localhost");
    assert!(
        xmpp.wait_for("Original text", Duration::from_secs(5)),
        "original message not visible",
    );
    // Inject the correction — must not crash
    xmpp.inject(corrected_chat_message(
        "contact@localhost",
        "user@localhost/aparte_test",
        "corr1",
        "orig1",
        "Corrected text",
    ));
    thread::sleep(Duration::from_millis(500));
    // The app should still be running with the chat window open
    let found = xmpp.wait_for("contact@localhost", Duration::from_secs(3));
    let parser = xmpp.snapshot();
    assert!(
        found,
        "chat window disappeared after processing correction\n{}",
        describe(parser.screen()),
    );
}

/// Non-regression: wide-char (emoji) right-half cell must not persist as a ghost
/// after switching windows. The bug: apply_diff only updated reference_screen[P]
/// for a w=2 emoji at position P, leaving reference_screen[P+1] stale. When
/// switching back to console, compute_diff saw reference[P+1] == buffer[P+1]
/// (coincidental match) and skipped the cell, leaving the emoji right-half on
/// screen and corrupting text at that column.
#[rstest]
fn wide_char_right_half_cleared_on_window_switch(xmpp: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    // Emoji lands at col 30 (11-char timestamp + 19-char "contact@localhost: "
    // prefix), placing the right-half at col 31 — the same column as 'c' in
    // "localhost" from the console's "Connected as user@localhost/aparte_test".
    // With the bug, col 31 stays as the emoji right-half continuation after
    // switching back, breaking "localhost" → "loalhost" in the vt100 view.
    xmpp.inject(chat_message(
        "contact@localhost",
        "user@localhost/aparte_test",
        "wc1",
        "🤣 regression check",
    ));
    xmpp.switch_window("contact@localhost");
    thread::sleep(Duration::from_millis(400));
    xmpp.switch_window("console");
    let found = xmpp.wait_for("localhost", Duration::from_secs(5));
    let parser = xmpp.snapshot();
    assert!(
        found,
        "emoji right-half cell persisted after window switch, corrupting console\n{}",
        describe(parser.screen()),
    );
}

/// Non-regression: tab characters in /help output must not corrupt the screen after
/// switching windows. The bug: \t has display_width()=1 but terminals advance the
/// cursor to the next tab stop (column multiple of 8). textwrap::indent in
/// generate_sub_help! prepends \t to each sub-command help line. This caused a
/// divergence between reference_screen (thinks next char is at col N+1) and the
/// terminal (wrote it at the tab stop). On window switch, compute_diff skipped cells
/// where reference_screen coincidentally matched the new buffer, leaving ghost help
/// text visible in the chat window.
#[rstest]
fn help_tab_chars_do_not_corrupt_screen_after_window_switch(xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    xmpp_with_contact.send_command("/help bookmark");
    // Wait for a keyword from the tab-indented sub-command help to appear
    assert!(
        xmpp_with_contact.wait_for("autojoin", Duration::from_secs(5)),
        "help output did not appear",
    );
    // Open a chat window — the delta render must correctly overwrite all console cells
    xmpp_with_contact.send_command("/msg contact@localhost");
    thread::sleep(Duration::from_millis(600));
    // With the bug, tab-shifted help text leaks as ghost chars into the chat window
    let parser = xmpp_with_contact.snapshot();
    assert!(
        !grid_contains(parser.screen(), "autojoin"),
        "help text leaked into chat window (tab rendering bug)\n{}",
        describe(parser.screen()),
    );
}

/// A carbon copy of a message sent from another device appears in the chat window.
#[rstest]
fn outgoing_carbon_sent_appears_in_ui(xmpp: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    xmpp.inject(carbon_sent(
        "user@localhost",
        "user@localhost/aparte_test",
        "user@localhost/other-device",
        "contact@localhost",
        "cs1",
        "Sent from other device",
    ));
    xmpp.switch_window("contact@localhost");
    let found = xmpp.wait_for("Sent from other device", Duration::from_secs(5));
    let parser = xmpp.snapshot();
    assert!(
        found,
        "carbon-sent message not in UI\n{}",
        describe(parser.screen()),
    );
}

/// A carbons::Sent copy of a message we sent on this device must not duplicate it in the UI.
/// Regression: before the fix, the carbon copy was re-processed and could cause a decryption
/// error or show the message a second time.
#[test]
fn carbon_sent_own_chat_message_deduplicated() {
    let (xmpp, mut capture) = XmppFixture::new_with_omemo(&["contact@localhost"]);
    thread::sleep(Duration::from_millis(300));

    xmpp.send_command("/msg contact@localhost");
    thread::sleep(Duration::from_millis(400));
    xmpp.send_command("dedup-chat-9x7z");

    // Capture the outgoing stanza to get the message ID assigned by aparte.
    let sent = capture
        .recv_message_matching(
            |m| m.bodies.values().any(|b| b.contains("dedup-chat-9x7z")),
            Duration::from_secs(5),
        )
        .expect("outgoing stanza not captured — is the mock server connected?");
    let msg_id = sent.id.as_ref().expect("sent stanza has no id").0.clone();

    assert!(
        xmpp.wait_for("dedup-chat-9x7z", Duration::from_secs(5)),
        "outgoing message did not appear in UI"
    );

    // Server echoes the message back as carbons::Sent with the same stanza ID.
    xmpp.inject(carbon_sent(
        "user@localhost",
        "user@localhost/aparte_test",
        "user@localhost/aparte_test",
        "contact@localhost",
        &msg_id,
        "dedup-chat-9x7z",
    ));
    thread::sleep(Duration::from_millis(500));

    let parser = xmpp.snapshot();
    let count = (0..ROWS)
        .filter(|&r| row_text(parser.screen(), r).contains("dedup-chat-9x7z"))
        .count();
    assert_eq!(
        count,
        1,
        "message appeared {count} times; expected once (carbon copy was not deduplicated)\n{}",
        describe(parser.screen()),
    );
}

/// A carbons::Sent copy of a groupchat message we sent must not be re-processed.
/// This is the primary bug: OMEMO-encrypted MUC messages can't be decrypted from
/// their own carbon copy, because the crypto engine is keyed by room JID but the
/// carbon's inner message has from=our_jid.
#[test]
fn carbon_sent_own_muc_message_deduplicated() {
    let (xmpp, mut capture) = XmppFixture::new_with_omemo(&[]);
    thread::sleep(Duration::from_millis(300));

    let room = "dev@conference.localhost";
    xmpp.send_command(&format!("/join {room}"));
    thread::sleep(Duration::from_millis(400));
    xmpp.inject(muc_join_presence(
        &format!("{room}/user"),
        "user@localhost/aparte_test",
        xmpp_parsers::muc::user::Affiliation::Member,
        xmpp_parsers::muc::user::Role::Participant,
    ));
    thread::sleep(Duration::from_millis(300));

    xmpp.switch_window(room);
    xmpp.send_command("dedup-muc-4k2q");

    // Capture the outgoing groupchat stanza to get its ID.
    let sent = capture
        .recv_message_matching(
            |m| m.bodies.values().any(|b| b.contains("dedup-muc-4k2q")),
            Duration::from_secs(5),
        )
        .expect("outgoing MUC stanza not captured");
    let msg_id = sent.id.as_ref().expect("sent stanza has no id").0.clone();

    assert!(
        xmpp.wait_for("dedup-muc-4k2q", Duration::from_secs(5)),
        "outgoing MUC message did not appear in UI"
    );

    // Server echoes back a carbons::Sent wrapping the groupchat message.
    xmpp.inject(carbon_sent_groupchat(
        "user@localhost",
        "user@localhost/aparte_test",
        "user@localhost/aparte_test",
        room,
        &msg_id,
        "dedup-muc-4k2q",
    ));
    thread::sleep(Duration::from_millis(500));

    let parser = xmpp.snapshot();
    let count = (0..ROWS)
        .filter(|&r| row_text(parser.screen(), r).contains("dedup-muc-4k2q"))
        .count();
    assert_eq!(
        count, 1,
        "MUC message appeared {count} times; expected once (groupchat carbon copy was not deduplicated)\n{}",
        describe(parser.screen()),
    );
}

/// The MUC server echoes back your own groupchat message (from room/nick). When the echo
/// carries the same stanza ID as the message we sent, it must be suppressed — we already
/// showed it immediately via SendMessage.
#[test]
fn muc_own_message_echo_not_duplicated() {
    let (xmpp, mut capture) = XmppFixture::new_with_omemo(&[]);
    thread::sleep(Duration::from_millis(300));

    let room = "dev@conference.localhost";
    xmpp.send_command(&format!("/join {room}"));
    thread::sleep(Duration::from_millis(400));
    xmpp.inject(muc_join_presence(
        &format!("{room}/user"),
        "user@localhost/aparte_test",
        xmpp_parsers::muc::user::Affiliation::Member,
        xmpp_parsers::muc::user::Role::Participant,
    ));
    thread::sleep(Duration::from_millis(300));

    xmpp.switch_window(room);
    xmpp.send_command("echo-dedup-7z3x");

    // Capture the outgoing stanza to learn the stanza ID assigned by aparte.
    let sent = capture
        .recv_message_matching(
            |m| m.bodies.values().any(|b| b.contains("echo-dedup-7z3x")),
            Duration::from_secs(5),
        )
        .expect("outgoing MUC stanza not captured");
    let msg_id = sent.id.as_ref().expect("sent stanza has no id").0.clone();

    assert!(
        xmpp.wait_for("echo-dedup-7z3x", Duration::from_secs(5)),
        "outgoing MUC message did not appear in UI"
    );

    // Inject the MUC server echo with the same stanza ID — this must be suppressed.
    xmpp.inject(groupchat_message(
        &format!("{room}/user"),
        "user@localhost/aparte_test",
        &msg_id,
        "echo-dedup-7z3x",
    ));
    thread::sleep(Duration::from_millis(500));

    let parser = xmpp.snapshot();
    let count = (0..ROWS)
        .filter(|&r| row_text(parser.screen(), r).contains("echo-dedup-7z3x"))
        .count();
    assert_eq!(
        count, 1,
        "MUC message appeared {count} times; expected once (server echo with same ID must be suppressed)\n{}",
        describe(parser.screen()),
    );
}

/// A carbons::Received wrapping a groupchat message must be silently ignored per
/// XEP-0280 — the MUC server delivers it directly to all resources anyway.
#[test]
fn carbon_received_muc_message_ignored() {
    let xmpp = XmppFixture::new(&[]);
    thread::sleep(Duration::from_millis(300));

    let room = "dev@conference.localhost";
    xmpp.send_command(&format!("/join {room}"));
    thread::sleep(Duration::from_millis(400));
    xmpp.inject(muc_join_presence(
        &format!("{room}/user"),
        "user@localhost/aparte_test",
        xmpp_parsers::muc::user::Affiliation::Member,
        xmpp_parsers::muc::user::Role::Participant,
    ));
    thread::sleep(Duration::from_millis(300));
    xmpp.switch_window(room);

    // Inject a carbons::Received wrapping a groupchat message. This must be dropped.
    xmpp.inject(carbon_received_groupchat(
        "user@localhost",
        "user@localhost/aparte_test",
        &format!("{room}/contact"),
        "user@localhost/aparte_test",
        "carbon-muc-id-9f1w",
        "carbon-muc-body-9f1w",
    ));
    thread::sleep(Duration::from_millis(500));

    let parser = xmpp.snapshot();
    assert!(
        !(0..ROWS).any(|r| row_text(parser.screen(), r).contains("carbon-muc-body-9f1w")),
        "groupchat carbons::Received should be ignored but message appeared in UI\n{}",
        describe(parser.screen()),
    );
}
