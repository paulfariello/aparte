//! Multi-user chat (XEP-0045) integration tests.

mod common;

use std::thread;
use std::time::Duration;

use rstest::rstest;
use xmpp_parsers::muc::user::{Affiliation, Role};

use common::describe;
use common::xmpp_fixture::{
    groupchat_message, muc_join_presence, room_subject_message, xmpp, XmppFixture,
};

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
    thread::sleep(Duration::from_millis(300));
}

/// /join opens a MUC window visible in the win-bar.
#[rstest]
fn muc_join_creates_window(xmpp: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    join_room(&xmpp, ROOM, "user");
    let found = xmpp.wait_for(ROOM, Duration::from_secs(5));
    let parser = xmpp.snapshot();
    assert!(
        found,
        "MUC room not shown in UI after join\n{}",
        describe(parser.screen()),
    );
}

/// An incoming groupchat message appears in the MUC window.
#[rstest]
fn muc_incoming_groupchat_message(xmpp: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    join_room(&xmpp, ROOM, "user");
    xmpp.switch_window(ROOM);
    thread::sleep(Duration::from_millis(200));
    xmpp.inject(groupchat_message(
        &format!("{ROOM}/otheruser"),
        "user@localhost",
        "gcm1",
        "Hello MUC!",
    ));
    let found = xmpp.wait_for("Hello MUC!", Duration::from_secs(5));
    let parser = xmpp.snapshot();
    assert!(
        found,
        "groupchat message not visible in MUC window\n{}",
        describe(parser.screen()),
    );
}

/// The room subject appears in the UI after the server sends it.
#[rstest]
fn muc_room_subject_shown_in_ui(xmpp: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    join_room(&xmpp, ROOM, "user");
    xmpp.switch_window(ROOM);
    thread::sleep(Duration::from_millis(200));
    xmpp.inject(room_subject_message(
        &format!("{ROOM}/owner"),
        "user@localhost",
        "subj1",
        "Development channel",
    ));
    let found = xmpp.wait_for("Development channel", Duration::from_secs(5));
    let parser = xmpp.snapshot();
    assert!(
        found,
        "room subject not visible in UI\n{}",
        describe(parser.screen()),
    );
}

/// After emitting a grapheme containing U+200D (ZWJ), the renderer must
/// re-anchor the cursor with an absolute CUP before any further printable
/// output. Some terminals render unrecognised ZWJ sequences wider than
/// `unicode_display_width` reports, so relying on natural cursor advance
/// shifts everything after the emoji on the same row (see ADR-0006).
#[rstest]
fn muc_zwj_message_reanchors_cursor_before_next_printable(xmpp: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    join_room(&xmpp, ROOM, "user");
    xmpp.switch_window(ROOM);
    thread::sleep(Duration::from_millis(200));
    xmpp.inject(muc_join_presence(
        &format!("{ROOM}/alice"),
        BOUND_JID,
        Affiliation::Admin,
        Role::Moderator,
    ));
    xmpp.wait_for("Moderator", Duration::from_secs(5));
    xmpp.inject(groupchat_message(
        &format!("{ROOM}/alice"),
        "user@localhost",
        "zwj1",
        "look \u{1F642}\u{200D}\u{2194}\u{FE0F} shake",
    ));
    assert!(
        xmpp.wait_for("shake", Duration::from_secs(5)),
        "ZWJ message not visible in MUC window\n{}",
        describe(xmpp.snapshot().screen()),
    );

    let bytes = xmpp.raw_bytes();
    let zwj_emoji = "\u{1F642}\u{200D}\u{2194}\u{FE0F}".as_bytes();
    let mut occurrences = 0;
    let mut i = 0;
    while i + zwj_emoji.len() <= bytes.len() {
        if &bytes[i..i + zwj_emoji.len()] != zwj_emoji {
            i += 1;
            continue;
        }
        occurrences += 1;
        let after = i + zwj_emoji.len();
        assert!(
            cup_before_next_printable(&bytes[after..]),
            "occurrence {occurrences} of ZWJ emoji at byte {i} is followed by \
             printable output without a cursor re-anchor (CUP): {:?}",
            String::from_utf8_lossy(&bytes[after..(after + 40).min(bytes.len())]),
        );
        i = after;
    }
    assert!(occurrences > 0, "ZWJ emoji never emitted to the terminal");
}

/// Scans forward skipping ANSI escape sequences; returns true if a CUP
/// (ESC [ ... H) appears before any printable character.
fn cup_before_next_printable(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            if bytes.get(i + 1) == Some(&b'[') {
                let mut j = i + 2;
                while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                    j += 1;
                }
                if bytes.get(j) == Some(&b'H') {
                    return true;
                }
                i = j + 1;
            } else {
                i += 2;
            }
        } else if bytes[i] < 0x20 {
            i += 1;
        } else {
            return false;
        }
    }
    true
}

/// An occupant joining the room appears in the occupant list.
#[rstest]
fn muc_occupant_appears_in_occupant_list(xmpp: XmppFixture) {
    thread::sleep(Duration::from_millis(300));
    join_room(&xmpp, ROOM, "user");
    xmpp.switch_window(ROOM);
    thread::sleep(Duration::from_millis(200));
    xmpp.inject(muc_join_presence(
        &format!("{ROOM}/alice"),
        BOUND_JID,
        Affiliation::Member,
        Role::Participant,
    ));
    let found = xmpp.wait_for("alice", Duration::from_secs(5));
    let parser = xmpp.snapshot();
    assert!(
        found,
        "occupant 'alice' not visible in MUC window\n{}",
        describe(parser.screen()),
    );
}
