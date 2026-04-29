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
