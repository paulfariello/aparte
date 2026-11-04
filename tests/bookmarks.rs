//! Bookmarks (XEP-0048) integration tests.

mod common;

use std::thread;
use std::time::Duration;

use rstest::rstest;
use xmpp_parsers::muc::user::{Affiliation, Role};

use common::describe;
use common::xmpp_fixture::{
    bookmarks_v1_push_event, muc_join_presence, xmpp_with_disco, XmppFixture,
};

const ROOM: &str = "dev@conference.localhost";
const ROOM_NAME: &str = "Dev Room";
const BOUND_JID: &str = "user@localhost/aparte_test";

/// A bookmarked room with autojoin=true is automatically joined when the server pushes it.
#[rstest]
fn bookmarks_autojoin_on_push(xmpp_with_disco: XmppFixture) {
    // Allow init IQs (pubsub create/subscribe) to be acked before pushing bookmarks
    thread::sleep(Duration::from_millis(800));
    xmpp_with_disco.inject(bookmarks_v1_push_event(
        "user@localhost",
        "user@localhost/aparte_test",
        &[(ROOM, ROOM_NAME, true)],
    ));
    let found = xmpp_with_disco.wait_for(ROOM, Duration::from_secs(10));
    let parser = xmpp_with_disco.snapshot();
    assert!(
        found,
        "auto-join room not visible after bookmark push\n{}",
        describe(parser.screen()),
    );
}

/// After joining a bookmarked room, the win bar shows the bookmark name instead of the JID.
#[rstest]
fn bookmarked_room_shows_display_name_in_win_bar(xmpp_with_disco: XmppFixture) {
    thread::sleep(Duration::from_millis(800));
    xmpp_with_disco.inject(bookmarks_v1_push_event(
        "user@localhost",
        BOUND_JID,
        &[(ROOM, ROOM_NAME, true)],
    ));
    thread::sleep(Duration::from_millis(500));
    xmpp_with_disco.inject(muc_join_presence(
        &format!("{ROOM}/user"),
        BOUND_JID,
        Affiliation::Member,
        Role::Participant,
    ));

    let found = xmpp_with_disco.wait_for(ROOM_NAME, Duration::from_secs(10));
    let parser = xmpp_with_disco.snapshot();
    assert!(
        found,
        "bookmark display name not visible in win bar after join\n{}",
        describe(parser.screen()),
    );
}

/// /win <display-name> switches to a MUC window using its bookmark name.
#[rstest]
fn win_command_accepts_bookmark_name(xmpp_with_disco: XmppFixture) {
    thread::sleep(Duration::from_millis(800));
    xmpp_with_disco.inject(bookmarks_v1_push_event(
        "user@localhost",
        BOUND_JID,
        &[(ROOM, ROOM_NAME, true)],
    ));
    thread::sleep(Duration::from_millis(500));
    xmpp_with_disco.inject(muc_join_presence(
        &format!("{ROOM}/user"),
        BOUND_JID,
        Affiliation::Member,
        Role::Participant,
    ));
    thread::sleep(Duration::from_millis(400));

    // Switch away so we can verify the /win switch brings us back
    xmpp_with_disco.switch_window("console");
    thread::sleep(Duration::from_millis(400));

    // Switch back using the display name
    xmpp_with_disco.send_command(&format!("/win {ROOM_NAME}"));

    let found = xmpp_with_disco.wait_for(ROOM_NAME, Duration::from_secs(5));
    let parser = xmpp_with_disco.snapshot();
    assert!(
        found,
        "/win with bookmark display name did not switch to MUC window\n{}",
        describe(parser.screen()),
    );
}
