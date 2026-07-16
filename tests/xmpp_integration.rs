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
use common::rows_with_bgcolor;
use common::xmpp_fixture::{
    carbon_received, carbon_received_groupchat, carbon_sent, carbon_sent_groupchat, chat_message,
    contact_offline_presence, contact_presence, corrected_chat_message, groupchat_message,
    muc_join_presence, xmpp, xmpp_with_contact, XmppFixture,
};
use common::{INPUT_ROW, ROWS, SELECTION_BGCOLOR};

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

/// Regression: pressing `i` directly after `Esc` (without first `k` to move
/// focus into the message frame) must NOT start an in-place edit, even if the
/// auto-select on NORMAL mode highlighted an outgoing message. Otherwise the
/// cursor stays on the input bar but the message also enters edit state,
/// keystrokes are typed into the input bar (never reaching the message), and
/// Enter sends a "correction" with the unmodified body — i.e. a no-op
/// correction that appears as a duplicate message to peers.
#[rstest]
fn esc_then_i_without_k_does_not_start_in_place_edit(mut xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    xmpp_with_contact.send_command("/msg contact@localhost");
    thread::sleep(Duration::from_millis(400));
    xmpp_with_contact.send_command("Hello world");
    assert!(
        xmpp_with_contact.wait_for("Hello world", Duration::from_secs(5)),
        "original message not visible",
    );

    // Drain the original send.
    let _ = xmpp_with_contact.recv_outgoing_message_matching(
        |msg| msg.bodies.values().any(|b| b == "Hello world"),
        Duration::from_secs(5),
    );

    // Esc → NORMAL; focus is still on the input bar (no `k` pressed).
    xmpp_with_contact.send_bytes(b"\x1b");
    assert!(xmpp_with_contact.wait_for("NORMAL", Duration::from_secs(2)));

    // Press 'i' — must enter INSERT on the input bar (not on the message).
    xmpp_with_contact.send_bytes(b"i");
    assert!(xmpp_with_contact.wait_for("INSERT", Duration::from_secs(2)));

    // Type a fresh body in the input bar.
    xmpp_with_contact.send_bytes(b"Brand new");
    assert!(
        xmpp_with_contact.wait_for("Brand new", Duration::from_secs(3)),
        "expected typed text to appear in the input bar",
    );
    xmpp_with_contact.send_bytes(b"\r");

    // The next outgoing stanza must be a *plain* message with "Brand new",
    // NOT a correction of "Hello world" with the same body.
    let next = xmpp_with_contact
        .recv_outgoing_message_matching(|_| true, Duration::from_secs(5))
        .expect("expected an outgoing stanza after Enter");

    let has_replace = next
        .payloads
        .iter()
        .any(|p| p.is("replace", "urn:xmpp:message-correct:0"));
    assert!(
        !has_replace,
        "Esc + i (focus still on input bar) must not produce a correction stanza",
    );

    let body = next
        .bodies
        .values()
        .next()
        .map(String::as_str)
        .unwrap_or("");
    assert_eq!(
        body, "Brand new",
        "expected a fresh message with the typed body, got: {body:?}",
    );
}

/// Press `k` to select a previously-sent outgoing message, `i` to start an
/// in-place edit, type more text, then Enter. The client must send a XEP-0308
/// correction stanza: a fresh `<message>` carrying
/// `<replace xmlns='urn:xmpp:message-correct:0' id='<original-id>'/>`
/// where the replace id is the original message id and the stanza id differs.
///
/// We send two messages. Without auto-select on NORMAL entry, the first `k`
/// selects the last message and the second `k` moves to "Original body".
/// A single message would cause the second `k` to bubble back to the input
/// bar — see `select_prev` handler in `src/mods/ui.rs`.
#[rstest]
fn enter_in_correction_edit_sends_replace_stanza(mut xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    xmpp_with_contact.send_command("/msg contact@localhost");
    thread::sleep(Duration::from_millis(400));
    xmpp_with_contact.send_command("Original body");
    xmpp_with_contact.send_command("Second message");
    assert!(
        xmpp_with_contact.wait_for("Second message", Duration::from_secs(5)),
        "second message not visible",
    );

    // Capture both outgoing stanzas and remember the id of the *first* one —
    // it's the one we'll correct.
    let original = xmpp_with_contact
        .recv_outgoing_message_matching(
            |msg| msg.bodies.values().any(|b| b == "Original body"),
            Duration::from_secs(5),
        )
        .expect("original outgoing stanza was not captured");
    let original_id = original
        .id
        .as_ref()
        .expect("original must carry an id")
        .0
        .clone();
    // Drain the "Second message" stanza so it doesn't get picked up below.
    let _ = xmpp_with_contact.recv_outgoing_message_matching(
        |msg| msg.bodies.values().any(|b| b == "Second message"),
        Duration::from_secs(5),
    );

    // Esc → NORMAL (no auto-select), then k twice: the first k selects the
    // last message ("Second message"), the second k moves up to "Original body".
    xmpp_with_contact.send_bytes(b"\x1b");
    assert!(xmpp_with_contact.wait_for("NORMAL", Duration::from_secs(2)));
    xmpp_with_contact.send_bytes(b"k");
    thread::sleep(Duration::from_millis(100));
    xmpp_with_contact.send_bytes(b"k");
    thread::sleep(Duration::from_millis(200));
    xmpp_with_contact.send_bytes(b"i");
    assert!(xmpp_with_contact.wait_for("INSERT", Duration::from_secs(2)));

    // Append " plus" to the in-place edit and commit. We wait for the edited
    // text to actually render before pressing Enter; otherwise the keys can
    // race the PTY/renderer and the correction is sent before the edit is
    // applied.
    xmpp_with_contact.send_bytes(b" plus");
    assert!(
        xmpp_with_contact.wait_for("Original body plus", Duration::from_secs(5)),
        "edited text did not render in place before Enter",
    );
    xmpp_with_contact.send_bytes(b"\r");

    let correction = xmpp_with_contact
        .recv_outgoing_message_matching(
            |msg| {
                msg.payloads
                    .iter()
                    .any(|p| p.is("replace", "urn:xmpp:message-correct:0"))
            },
            Duration::from_secs(5),
        )
        .expect("expected a <replace> correction stanza on the wire");

    let replace = correction
        .payloads
        .iter()
        .find(|p| p.is("replace", "urn:xmpp:message-correct:0"))
        .expect("correction stanza must contain a <replace> element");
    assert_eq!(
        replace.attr("id"),
        Some(original_id.as_str()),
        "<replace> id must equal the original message id",
    );

    let new_id = correction
        .id
        .as_ref()
        .expect("correction stanza must have its own id")
        .0
        .clone();
    assert_ne!(
        new_id, original_id,
        "the correction stanza must use a fresh id distinct from the original",
    );

    let body = correction
        .bodies
        .values()
        .next()
        .expect("correction must carry a body");
    assert!(
        body.contains("plus"),
        "new body should reflect the edit, got: {}",
        body,
    );
}

/// After `k` selects a sent outgoing message and `i` starts the in-place edit,
/// the terminal cursor must sit on the message row (steady bar), NOT on the
/// input bar (row ROWS-1). Typing must mutate the message text, not the input
/// bar; the input bar must remain empty.
#[rstest]
fn i_on_outgoing_message_puts_cursor_at_message_not_input_bar(xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    xmpp_with_contact.send_command("/msg contact@localhost");
    thread::sleep(Duration::from_millis(400));
    // Two messages so the first `k` selects the older one without bubbling to
    // the input bar.
    xmpp_with_contact.send_command("First message");
    xmpp_with_contact.send_command("Second message");
    assert!(
        xmpp_with_contact.wait_for("Second message", Duration::from_secs(5)),
        "second message not visible on screen",
    );

    // Esc → NORMAL; the last message is auto-selected.
    xmpp_with_contact.send_bytes(b"\x1b");
    assert!(xmpp_with_contact.wait_for("NORMAL", Duration::from_secs(2)));

    // k — select "First message" (the older one) and move focus into the
    // message frame.
    xmpp_with_contact.send_bytes(b"k");
    thread::sleep(Duration::from_millis(200));

    // Record which row is highlighted as the selection.
    let selection_row = {
        let parser = xmpp_with_contact.snapshot();
        let selected = rows_with_bgcolor(parser.screen(), SELECTION_BGCOLOR);
        assert!(
            !selected.is_empty(),
            "expected a selected message row after 'k'\n{}",
            describe(parser.screen()),
        );
        *selected.first().unwrap()
    };

    // Press 'i' — focus is on the message frame (FRAME_LAYOUT_INDEX), which is
    // insertable because the selected message is outgoing. The cursor must jump
    // to the selected message row with a steady-bar shape, NOT to the input
    // bar.
    xmpp_with_contact.send_bytes(b"i");
    assert!(
        xmpp_with_contact.wait_for("INSERT", Duration::from_secs(2)),
        "expected INSERT mode after pressing 'i' on an outgoing message",
    );

    let parser = xmpp_with_contact.snapshot();
    let screen = parser.screen();
    let (cursor_row, _cursor_col) = screen.cursor_position();

    assert_ne!(
        cursor_row,
        INPUT_ROW,
        "cursor must NOT be on the input bar (row {}) after pressing 'i' on a selected outgoing message; got row {}\n{}",
        INPUT_ROW,
        cursor_row,
        describe(screen),
    );
    assert_eq!(
        cursor_row,
        selection_row,
        "cursor must sit on the selected message row ({}) in edit mode, not row {}\n{}",
        selection_row,
        cursor_row,
        describe(screen),
    );
}

/// Pressing `j` to navigate from an older message BACK to the last (most
/// recent) message and then pressing `i` must start an in-place edit on the
/// last message, not switch focus to the input bar.
///
/// Regression: `j` on the last message bubbles focus to the input bar. When
/// the user navigates `k` → last-1, then `j` → last, the focus is correctly
/// on FRAME_LAYOUT_INDEX. But if the user is already on the last message and
/// presses `j` one extra time focus escapes to INPUT_INDEX. The fix must
/// ensure that when navigating back to the last message (via `j`) while focus
/// is on the frame, `i` still triggers in-place edit.
#[rstest]
fn i_on_last_message_after_j_navigation_starts_in_place_edit(xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    xmpp_with_contact.send_command("/msg contact@localhost");
    thread::sleep(Duration::from_millis(400));
    xmpp_with_contact.send_command("First message");
    xmpp_with_contact.send_command("Last message");
    assert!(
        xmpp_with_contact.wait_for("Last message", Duration::from_secs(5)),
        "last message not visible",
    );

    // Esc → NORMAL (no auto-select).
    xmpp_with_contact.send_bytes(b"\x1b");
    assert!(xmpp_with_contact.wait_for("NORMAL", Duration::from_secs(2)));

    // First k → selects "Last message" (bottom visible, no prior selection).
    // Second k → moves to "First message".
    // j → moves back to "Last message"; focus must remain on the message frame.
    xmpp_with_contact.send_bytes(b"k");
    thread::sleep(Duration::from_millis(100));
    xmpp_with_contact.send_bytes(b"k");
    thread::sleep(Duration::from_millis(200));
    xmpp_with_contact.send_bytes(b"j");
    thread::sleep(Duration::from_millis(200));

    // The last message row must be highlighted.
    let selection_row = {
        let parser = xmpp_with_contact.snapshot();
        let selected = rows_with_bgcolor(parser.screen(), SELECTION_BGCOLOR);
        assert!(
            !selected.is_empty(),
            "expected a selected message row after k+j\n{}",
            describe(parser.screen()),
        );
        *selected.last().unwrap()
    };

    // i must start an in-place edit on the last message — cursor on the
    // message row, NOT on the input bar.
    xmpp_with_contact.send_bytes(b"i");
    assert!(
        xmpp_with_contact.wait_for("INSERT", Duration::from_secs(2)),
        "expected INSERT mode after pressing 'i' on the last message",
    );

    let parser = xmpp_with_contact.snapshot();
    let screen = parser.screen();
    let (cursor_row, _) = screen.cursor_position();

    assert_ne!(
        cursor_row,
        INPUT_ROW,
        "cursor must NOT jump to the input bar (row {}) when pressing 'i' on the last message; got row {}\n{}",
        INPUT_ROW,
        cursor_row,
        describe(screen),
    );
    assert_eq!(
        cursor_row,
        selection_row,
        "cursor must sit on the last message row ({}) after pressing 'i', not row {}\n{}",
        selection_row,
        cursor_row,
        describe(screen),
    );
}

/// Pressing Escape from INSERT mode while typing in the chat input bar must
/// leave the cursor on the input bar in NORMAL mode. No chat message should
/// be auto-selected and the cursor must NOT jump to a message row.
#[rstest]
fn escape_from_insert_in_chat_stays_on_input_bar(xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    xmpp_with_contact.send_command("/msg contact@localhost");
    thread::sleep(Duration::from_millis(400));
    // Send a message so the chat window has content that could be auto-selected.
    xmpp_with_contact.send_command("Hello world");
    assert!(
        xmpp_with_contact.wait_for("Hello world", Duration::from_secs(5)),
        "sent message not visible",
    );

    // After send_command the app is still in INSERT mode (Enter does not change
    // mode). Explicitly switch to NORMAL first, then back to INSERT — this
    // puts us in the "typing in the input bar" state we want to test.
    xmpp_with_contact.send_bytes(b"\x1b");
    assert!(xmpp_with_contact.wait_for("NORMAL", Duration::from_secs(2)));
    xmpp_with_contact.send_bytes(b"i");
    assert!(xmpp_with_contact.wait_for("INSERT", Duration::from_secs(2)));
    xmpp_with_contact.send_bytes(b"draft");
    assert!(
        xmpp_with_contact.wait_for("draft", Duration::from_secs(2)),
        "typed text not visible in input bar",
    );

    // Press Escape — must stay on input bar in NORMAL mode.
    xmpp_with_contact.send_bytes(b"\x1b");
    assert!(xmpp_with_contact.wait_for("NORMAL", Duration::from_secs(2)));

    let parser = xmpp_with_contact.snapshot();
    let screen = parser.screen();

    // No chat message row must be highlighted.
    let selected = rows_with_bgcolor(screen, SELECTION_BGCOLOR);
    assert!(
        selected.is_empty(),
        "no message should be selected after Escape from the input bar, but {} rows are highlighted\n{}",
        selected.len(),
        describe(screen),
    );

    // Cursor must stay on the input bar (last row).
    let (row, _) = screen.cursor_position();
    assert_eq!(
        row,
        INPUT_ROW,
        "cursor must remain on the input bar (row {}) after Escape from INSERT mode, got row {}\n{}",
        INPUT_ROW,
        row,
        describe(screen),
    );
}

/// Regression: when multiple XMPP messages arrive in Normal mode with
/// follow_bottom=true (typical after opening a chat with MAM history),
/// each new message must cancel the previous selection's edit cursor before
/// calling start_cursor on the new last message.
///
/// Without the fix, all arriving messages accumulate orphaned priority-3
/// edit cursors.  After k→G→j, the earliest message's cursor overrides the
/// input bar's priority-1 cursor and the terminal cursor never reaches the
/// input bar row.
///
/// Uses a MAM archive so all three messages arrive asynchronously without
/// any intervening Escape/ModeChange cleanup that would mask the bug.
#[test]
fn mam_messages_do_not_leave_orphaned_cursors_after_k_g_j() {
    let xmpp = XmppFixture::new_with_mam(
        &["contact@localhost"],
        &[
            ("contact@localhost", "user@localhost", "m1", "first message"),
            (
                "contact@localhost",
                "user@localhost",
                "m2",
                "second message",
            ),
            ("contact@localhost", "user@localhost", "m3", "third message"),
        ],
    );

    xmpp.send_command("/msg contact@localhost");
    xmpp.switch_window("contact@localhost");
    let visible = xmpp.wait_for("third message", Duration::from_secs(10));
    assert!(visible, "all three MAM messages must appear on screen");

    xmpp.send_bytes(b"k");
    thread::sleep(Duration::from_millis(200));
    xmpp.send_bytes(b"G");
    thread::sleep(Duration::from_millis(200));
    xmpp.send_bytes(b"j");
    thread::sleep(Duration::from_millis(300));

    let parser = xmpp.snapshot();
    let screen = parser.screen();

    let selected = rows_with_bgcolor(screen, SELECTION_BGCOLOR);
    assert!(
        selected.is_empty(),
        "selection must be cleared after j bubbles from last message, but {} rows highlighted\n{}",
        selected.len(),
        describe(screen)
    );

    let (row, _) = screen.cursor_position();
    assert_eq!(
        row,
        INPUT_ROW,
        "cursor must be on the input bar (row {}) after k->G->j with MAM messages, got row {}\n{}",
        INPUT_ROW,
        row,
        describe(screen)
    );
}

/// On first visit to a chat window that already has messages loaded (e.g. via
/// MAM or an incoming message), the cursor must still be on the input bar —
/// not on the last message.
#[rstest]
fn new_window_with_history_cursor_on_input_bar(xmpp: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    // Inject a message before the user opens the window — simulates MAM loading.
    xmpp.inject(chat_message(
        "contact@localhost",
        "user@localhost/aparte_test",
        "m_hist",
        "Historical message",
    ));
    // Switch to the chat window for the first time.
    xmpp.switch_window("contact@localhost");
    assert!(
        xmpp.wait_for("Historical message", Duration::from_secs(5)),
        "message not visible in chat window",
    );
    thread::sleep(Duration::from_millis(300));

    let parser = xmpp.snapshot();
    let screen = parser.screen();

    let selected = rows_with_bgcolor(screen, SELECTION_BGCOLOR);
    assert!(
        selected.is_empty(),
        "first visit with history: no message should be selected, got rows {:?}\n{}",
        selected,
        describe(screen),
    );

    let (row, _) = screen.cursor_position();
    assert_eq!(
        row,
        INPUT_ROW,
        "first visit with history: cursor should be on input bar (row {}), got row {}\n{}",
        INPUT_ROW,
        row,
        describe(screen),
    );
}

/// Opening a chat window for the first time puts the cursor on the input bar
/// in Normal mode — no automatic mode switch, no message selected.
#[rstest]
fn new_window_first_visit_cursor_on_input_bar(xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    // Open chat window for the first time.
    xmpp_with_contact.send_command("/msg contact@localhost");
    assert!(
        xmpp_with_contact.wait_for("contact@localhost", Duration::from_secs(5)),
        "chat window did not open",
    );
    thread::sleep(Duration::from_millis(300));

    let parser = xmpp_with_contact.snapshot();
    let screen = parser.screen();

    // No message should be selected.
    let selected = rows_with_bgcolor(screen, SELECTION_BGCOLOR);
    assert!(
        selected.is_empty(),
        "first visit: no message should be selected, got selected rows {:?}\n{}",
        selected,
        describe(screen),
    );

    // Cursor must be on the input bar (last row).
    let (row, _) = screen.cursor_position();
    assert_eq!(
        row,
        INPUT_ROW,
        "first visit: cursor should be on input bar (row {}), got row {}\n{}",
        INPUT_ROW,
        row,
        describe(screen),
    );

    // Mode must remain NORMAL — no automatic mode switch.
    assert!(
        grid_contains(screen, "NORMAL"),
        "first visit: mode must remain NORMAL\n{}",
        describe(screen),
    );
}

/// After navigating to a message in a window, switching away and back
/// restores the selection exactly where it was.
#[rstest]
fn return_to_window_restores_selection(xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    // Inject a message so there is something to select.
    xmpp_with_contact.inject(chat_message(
        "contact@localhost",
        "user@localhost/aparte_test",
        "msg1",
        "Hello from contact",
    ));
    xmpp_with_contact.switch_window("contact@localhost");
    assert!(
        xmpp_with_contact.wait_for("Hello from contact", Duration::from_secs(5)),
        "message not visible in chat window",
    );

    // Navigate to the message (G = go to last message).
    xmpp_with_contact.send_bytes(b"G");
    thread::sleep(Duration::from_millis(300));

    let selection_before: Vec<u16> = {
        let p = xmpp_with_contact.snapshot();
        rows_with_bgcolor(p.screen(), SELECTION_BGCOLOR)
    };
    assert!(
        !selection_before.is_empty(),
        "expected a selected message after G",
    );

    // Switch to console, then return to the chat window.
    xmpp_with_contact.switch_window("console");
    xmpp_with_contact.wait_for("Connected as", Duration::from_secs(3));
    xmpp_with_contact.switch_window("contact@localhost");
    assert!(
        xmpp_with_contact.wait_for("Hello from contact", Duration::from_secs(3)),
        "chat window did not re-appear",
    );
    thread::sleep(Duration::from_millis(300));

    let parser = xmpp_with_contact.snapshot();
    let screen = parser.screen();
    let selection_after = rows_with_bgcolor(screen, SELECTION_BGCOLOR);

    assert_eq!(
        selection_before,
        selection_after,
        "return visit: selection must be preserved\n{}",
        describe(screen),
    );
}

/// Pressing `i` (Insert mode) in the active window must not clear the
/// selection in a background window.
#[rstest]
fn insert_mode_in_active_window_preserves_inactive_selection(xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    // Inject a message so there is something to select in the chat window.
    xmpp_with_contact.inject(chat_message(
        "contact@localhost",
        "user@localhost/aparte_test",
        "msg2",
        "Background message",
    ));
    xmpp_with_contact.switch_window("contact@localhost");
    assert!(
        xmpp_with_contact.wait_for("Background message", Duration::from_secs(5)),
        "message not visible",
    );

    // Navigate to the message in the chat window.
    xmpp_with_contact.send_bytes(b"G");
    thread::sleep(Duration::from_millis(300));

    let selection_in_chat: Vec<u16> = {
        let p = xmpp_with_contact.snapshot();
        rows_with_bgcolor(p.screen(), SELECTION_BGCOLOR)
    };
    assert!(
        !selection_in_chat.is_empty(),
        "expected a selected message in chat window",
    );

    // Switch to console and enter Insert mode there.
    xmpp_with_contact.switch_window("console");
    xmpp_with_contact.wait_for("Connected as", Duration::from_secs(3));
    xmpp_with_contact.send_bytes(b"i");
    assert!(
        xmpp_with_contact.wait_for("INSERT", Duration::from_secs(2)),
        "did not enter INSERT mode in console",
    );

    // Return to Normal mode and go back to the chat window.
    xmpp_with_contact.send_bytes(b"\x1b");
    assert!(
        xmpp_with_contact.wait_for("NORMAL", Duration::from_secs(2)),
        "did not return to NORMAL mode",
    );
    xmpp_with_contact.switch_window("contact@localhost");
    assert!(
        xmpp_with_contact.wait_for("Background message", Duration::from_secs(3)),
        "chat window did not re-appear",
    );
    thread::sleep(Duration::from_millis(300));

    let parser = xmpp_with_contact.snapshot();
    let screen = parser.screen();
    let selection_after = rows_with_bgcolor(screen, SELECTION_BGCOLOR);

    assert_eq!(
        selection_in_chat,
        selection_after,
        "Insert mode in console must not clear the chat window's selection\n{}",
        describe(screen),
    );
}

/// After selecting a message with j/k in Normal mode, pressing ':' and then Esc
/// must return the cursor to the selected message at the start of message content
/// (after the header), never at column 0 (start of line).
#[rstest]
fn selected_message_cursor_returns_to_message_start_after_command_escape(xmpp: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    for i in 0..5 {
        xmpp.inject(chat_message(
            "contact@localhost",
            "user@localhost/aparte_test",
            &format!("msg{i}"),
            &format!("Message {i}"),
        ));
    }

    xmpp.switch_window("contact@localhost");
    let found = xmpp.wait_for("Message 4", Duration::from_secs(5));
    assert!(found, "Chat messages should be visible");

    // Ensure Normal mode then select a message.
    xmpp.send_bytes(b"\x1b");
    xmpp.wait_for("NORMAL", Duration::from_secs(2));
    xmpp.send_bytes(b"k");
    thread::sleep(Duration::from_millis(200));

    let parser = xmpp.snapshot();
    let screen = parser.screen();
    let selected_rows = rows_with_bgcolor(screen, SELECTION_BGCOLOR);
    assert!(
        !selected_rows.is_empty(),
        "A message should be selected after 'k'\n{}",
        describe(screen)
    );
    let message_row = *selected_rows.last().unwrap();
    let (_, col_before) = screen.cursor_position();

    assert_ne!(
        col_before, 0,
        "Cursor must be at start of message content (after header) even before :+Esc, got col 0\n{}",
        describe(screen)
    );

    // Enter then immediately exit Command mode.
    xmpp.send_bytes(b":");
    xmpp.wait_for("COMMAND", Duration::from_secs(2));
    xmpp.send_bytes(b"\x1b");
    xmpp.wait_for("NORMAL", Duration::from_secs(2));
    thread::sleep(Duration::from_millis(200));

    let parser = xmpp.snapshot();
    let screen = parser.screen();
    let (row_after, col_after) = screen.cursor_position();

    assert_eq!(
        row_after,
        message_row,
        "Cursor must return to the selected message row after Esc from command mode\n{}",
        describe(screen)
    );
    assert_ne!(
        col_after, 0,
        "Cursor must be at start of message content (col > 0) after :+Esc, not at start of line\n{}",
        describe(screen)
    );
    assert_eq!(
        col_after,
        col_before,
        "Cursor column must be preserved precisely through the :+Esc roundtrip\n{}",
        describe(screen)
    );
}

/// Mid-message cursor position must survive a :+Esc roundtrip unchanged.
/// After pressing 'l' to advance the cursor inside the message body,
/// ':' then Esc must return to that exact column — not reset it to the
/// start of the message.
#[rstest]
fn mid_message_cursor_column_preserved_through_command_escape(xmpp: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    for i in 0..5 {
        xmpp.inject(chat_message(
            "contact@localhost",
            "user@localhost/aparte_test",
            &format!("mid{i}"),
            &format!("Message {i}"),
        ));
    }

    xmpp.switch_window("contact@localhost");
    let found = xmpp.wait_for("Message 4", Duration::from_secs(5));
    assert!(found, "Chat messages should be visible");

    // Select a message and move cursor to start of message body.
    xmpp.send_bytes(b"\x1b");
    xmpp.wait_for("NORMAL", Duration::from_secs(2));
    xmpp.send_bytes(b"k");
    thread::sleep(Duration::from_millis(200));

    // Advance cursor inside the message body with 'l' (move right).
    xmpp.send_bytes(b"l");
    thread::sleep(Duration::from_millis(150));
    xmpp.send_bytes(b"l");
    thread::sleep(Duration::from_millis(150));

    let parser = xmpp.snapshot();
    let screen = parser.screen();
    let (message_row, col_mid) = screen.cursor_position();

    assert_ne!(
        message_row,
        INPUT_ROW,
        "Cursor must be on a message row before entering command mode\n{}",
        describe(screen)
    );

    // Enter then immediately exit Command mode.
    xmpp.send_bytes(b":");
    xmpp.wait_for("COMMAND", Duration::from_secs(2));
    xmpp.send_bytes(b"\x1b");
    xmpp.wait_for("NORMAL", Duration::from_secs(2));
    thread::sleep(Duration::from_millis(200));

    let parser = xmpp.snapshot();
    let screen = parser.screen();
    let (row_after, col_after) = screen.cursor_position();

    assert_eq!(
        row_after,
        message_row,
        "Cursor must return to the same message row after :+Esc\n{}",
        describe(screen)
    );
    assert_eq!(
        col_after,
        col_mid,
        "Cursor column inside message must be preserved through :+Esc (was {col_mid}, got {col_after})\n{}",
        describe(screen)
    );
}
