//! Integration tests for XEP-0333 Chat Markers (`<displayed>`) handling.
//!
//! Covers:
//! - A self-sent chat `<displayed>` marker clears the unread indicator in the win-bar.
//! - A self-sent groupchat `<displayed>` marker clears the MUC unread indicator.
//! - A marker only clears messages at or before its referenced timestamp (partial clear).
//! - A groupchat marker is ignored when the MUC has not announced XEP-0359 support.
//! - A displayed marker is sent to the MUC after MAM catchup when the window is current.
//! - No displayed marker is sent on join when the MUC does not announce XEP-0359 support.

mod common;

use std::thread;
use std::time::Duration;

use rstest::rstest;
use xmpp_parsers::muc::user::{Affiliation, Role};

use common::describe;
use common::grid_contains;
use common::xmpp_fixture::{
    chat_displayed_marker, chat_message, groupchat_displayed_marker, groupchat_message,
    groupchat_message_with_stanza_id, muc_join_presence, xmpp, xmpp_with_contact, XmppFixture,
};

const NICK: &str = "user";

const ROOM: &str = "dev@conference.localhost";
const BOUND_JID: &str = "user@localhost/aparte_test";

fn join_room(xmpp: &XmppFixture, room: &str, nick: &str) {
    xmpp.send_command(&format!("/join {room}"));
    thread::sleep(Duration::from_millis(400));
    xmpp.inject(muc_join_presence(
        &format!("{room}/{nick}"),
        BOUND_JID,
        Affiliation::Member,
        Role::Participant,
    ));
    thread::sleep(Duration::from_millis(400));
}

/// A self-sent chat `<displayed>` marker (XEP-0333) from another device clears the
/// unread indicator for that chat window in the win-bar.
#[rstest]
fn chat_marker_clears_unread_count(xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));

    // Inject a chat message while on the console window (don't switch away).
    xmpp_with_contact.inject(chat_message(
        "contact@localhost",
        "user@localhost/aparte_test",
        "chat-clr-msg-1",
        "Unread chat message",
    ));

    // Wait for the unread indicator to appear (chat messages are always "important",
    // so the win-bar shows "contact@localhost (1, 1)" — the "(1" prefix matches both formats).
    let highlighted = xmpp_with_contact.wait_for("contact@localhost (1", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        highlighted,
        "unread indicator did not appear in win-bar after incoming chat message\n{}",
        describe(parser.screen()),
    );

    // Another device sends a <displayed> marker referencing that message.
    // The marker is sent to the conversation partner (as it would appear after
    // carbon copy unwrapping: Device A sent it to contact@localhost).
    xmpp_with_contact.inject(chat_displayed_marker(
        "user@localhost/other-device",
        "contact@localhost",
        "chat-clr-msg-1",
    ));

    // The unread indicator must disappear.
    thread::sleep(Duration::from_millis(500));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        !grid_contains(parser.screen(), "contact@localhost ("),
        "unread indicator still present after chat <displayed> marker\n{}",
        describe(parser.screen()),
    );
}

/// A self-sent groupchat `<displayed>` marker clears the MUC window's unread
/// indicator. Requires the MUC to have announced XEP-0359 support so the marker's
/// referenced stanza-id can be resolved to a message timestamp.
#[test]
fn muc_marker_clears_unread_count() {
    let xmpp = XmppFixture::new_with_muc_sid(&[], &[ROOM]);
    thread::sleep(Duration::from_millis(300));
    join_room(&xmpp, ROOM, "user");
    // Switch back to console so the MUC message counts as unread.
    xmpp.switch_window("console");
    thread::sleep(Duration::from_millis(200));

    // Inject a groupchat message with an embedded stanza-id while on the console window.
    xmpp.inject(groupchat_message_with_stanza_id(
        &format!("{ROOM}/otheruser"),
        "user@localhost",
        "gc-clr-msg-1",
        "gc-clr-sid-1",
        "Unread MUC message",
    ));

    let highlighted = xmpp.wait_for(&format!("{ROOM} ("), Duration::from_secs(5));
    let parser = xmpp.snapshot();
    assert!(
        highlighted,
        "unread indicator did not appear in win-bar after groupchat message\n{}",
        describe(parser.screen()),
    );

    // Our own nick sends a <displayed> marker referencing the stanza-id.
    xmpp.inject(groupchat_displayed_marker(
        &format!("{ROOM}/user"),
        "user@localhost/aparte_test",
        "gc-clr-sid-1",
    ));

    thread::sleep(Duration::from_millis(600));
    let parser = xmpp.snapshot();
    assert!(
        !grid_contains(parser.screen(), &format!("{ROOM} (")),
        "unread indicator still present after MUC <displayed> marker\n{}",
        describe(parser.screen()),
    );
}

/// A chat `<displayed>` marker only clears unread messages at or before the
/// referenced message's timestamp. Messages that arrived later remain unread.
#[rstest]
fn chat_marker_partial_clear(xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));

    // Inject three live messages with small sleeps so each gets a strictly later
    // wall-clock timestamp (delayed messages with <delay> would be suppressed).
    xmpp_with_contact.inject(chat_message(
        "contact@localhost",
        "user@localhost/aparte_test",
        "partial-msg-1",
        "First message",
    ));
    thread::sleep(Duration::from_millis(20));
    xmpp_with_contact.inject(chat_message(
        "contact@localhost",
        "user@localhost/aparte_test",
        "partial-msg-2",
        "Second message",
    ));
    thread::sleep(Duration::from_millis(20));
    xmpp_with_contact.inject(chat_message(
        "contact@localhost",
        "user@localhost/aparte_test",
        "partial-msg-3",
        "Third message",
    ));

    // All three should be unread: "(3" matches both "(3)" and "(3, 3)" formats.
    let found_3 = xmpp_with_contact.wait_for("contact@localhost (3", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        found_3,
        "expected 3 unread messages in win-bar\n{}",
        describe(parser.screen()),
    );

    // Mark only up to the second message as read (partial marker).
    xmpp_with_contact.inject(chat_displayed_marker(
        "user@localhost/other-device",
        "contact@localhost",
        "partial-msg-2",
    ));

    // Exactly one message (the third) must remain unread.
    let found_1 = xmpp_with_contact.wait_for("contact@localhost (1", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        found_1,
        "expected exactly 1 unread message to remain after partial <displayed> marker\n{}",
        describe(parser.screen()),
    );
    assert!(
        !grid_contains(parser.screen(), "contact@localhost (3"),
        "3-unread indicator still visible after partial clear\n{}",
        describe(parser.screen()),
    );
}

/// A groupchat `<displayed>` marker from our own nick must be silently ignored
/// when the MUC has NOT announced XEP-0359 support. The unread count must persist.
#[rstest]
fn muc_marker_ignored_without_xep359_support(xmpp: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    join_room(&xmpp, ROOM, "user");
    // Switch back to console so the MUC message counts as unread.
    xmpp.switch_window("console");
    thread::sleep(Duration::from_millis(200));

    // Inject a plain groupchat message (no stanza-id element, no disco for this room).
    xmpp.inject(groupchat_message(
        &format!("{ROOM}/otheruser"),
        "user@localhost",
        "no-sid-msg-1",
        "MUC message without SID",
    ));

    let highlighted = xmpp.wait_for(&format!("{ROOM} ("), Duration::from_secs(5));
    let parser = xmpp.snapshot();
    assert!(
        highlighted,
        "unread indicator did not appear\n{}",
        describe(parser.screen()),
    );

    // Our nick sends a <displayed> marker — must be ignored (MUC lacks XEP-0359).
    xmpp.inject(groupchat_displayed_marker(
        &format!("{ROOM}/user"),
        "user@localhost/aparte_test",
        "no-sid-msg-1",
    ));

    thread::sleep(Duration::from_millis(500));
    let parser = xmpp.snapshot();
    assert!(
        grid_contains(parser.screen(), &format!("{ROOM} (")),
        "unread indicator was cleared despite no XEP-0359 support — marker should be ignored\n{}",
        describe(parser.screen()),
    );
}

/// After MAM catchup on MUC join with the window currently visible, aparte sends
/// a `<displayed>` groupchat marker to the MUC for the most recent non-self message.
#[test]
fn muc_join_sends_displayed_marker_when_window_current() {
    let mut xmpp = XmppFixture::new_with_muc_sid_and_gc_mam(
        &[],
        &[ROOM],
        &[(
            "sid-gc-1",
            &format!("{ROOM}/otheruser"),
            "msg-gc-1",
            "Hello",
        )],
    );
    thread::sleep(Duration::from_millis(300));

    // Join the room — the MUC window is automatically focused on join.
    join_room(&xmpp, ROOM, NICK);

    // MAM catchup completes → window is visible → marker must be sent.
    let sent = xmpp.recv_outgoing_displayed_marker_for(ROOM, Duration::from_secs(5));
    assert!(
        sent,
        "expected aparte to send a <displayed> marker to {ROOM} after MAM catchup",
    );
}

/// No displayed marker is sent when the MUC does not support XEP-0359 (Stanza IDs).
/// The pending marker must be discarded when disco confirms the absence of the feature.
#[test]
fn muc_join_no_marker_without_xep0359_on_join() {
    let mut xmpp = XmppFixture::new_with_gc_mam_no_sid(
        &[],
        &[(
            "sid-no-xep-1",
            &format!("{ROOM}/otheruser"),
            "msg-no-xep-1",
            "Hello",
        )],
    );
    thread::sleep(Duration::from_millis(300));

    join_room(&xmpp, ROOM, NICK);

    // Wait long enough for MAM and disco to complete, then verify no marker.
    thread::sleep(Duration::from_millis(800));
    let sent = xmpp.recv_outgoing_displayed_marker_for(ROOM, Duration::from_millis(200));
    assert!(
        !sent,
        "marker was sent even though the MUC does not support XEP-0359",
    );
}

/// When all MAM results were sent by ourselves, no displayed marker is emitted
/// (XEP-0333 forbids marking self-sent messages as displayed).
#[test]
fn muc_join_no_marker_when_all_mam_messages_are_self() {
    let mut xmpp = XmppFixture::new_with_muc_sid_and_gc_mam(
        &[],
        &[ROOM],
        // Message from our own nick — must not be marked.
        &[(
            "sid-self-1",
            &format!("{ROOM}/{NICK}"),
            "msg-self-1",
            "I said this",
        )],
    );
    thread::sleep(Duration::from_millis(300));

    join_room(&xmpp, ROOM, NICK);

    // Wait long enough for MAM to complete, then verify no marker was sent.
    thread::sleep(Duration::from_millis(800));
    let sent = xmpp.recv_outgoing_displayed_marker_for(ROOM, Duration::from_millis(200));
    assert!(
        !sent,
        "marker was sent even though all MAM messages were from ourselves",
    );
}
