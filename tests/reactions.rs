//! Integration tests for XEP-0444 Message Reactions.
//!
//! Covers:
//! - A chat reaction appears as an emoji summary line below the original message.
//! - A groupchat reaction referencing a stanza-id appears below the MUC message.
//! - A reaction update from the same sender replaces the previous emoji set.

mod common;

use std::thread;
use std::time::Duration;

use rstest::rstest;
use xmpp_parsers::muc::user::{Affiliation, Role};

use common::describe;
use common::grid_contains;
use common::xmpp_fixture::{
    chat_message, chat_reaction, groupchat_message_with_stanza_id, groupchat_reaction,
    muc_join_presence, xmpp_with_contact, XmppFixture,
};

const MAM_FROM: &str = "contact@localhost";
const MAM_TO: &str = "user@localhost/aparte_test";

const MAM_MSG_ID: &str = "mam-archived-1";

const ROOM: &str = "dev@conference.localhost";
const BOUND_JID: &str = "user@localhost/aparte_test";
const NICK: &str = "user";

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

/// A chat reaction (XEP-0444) from a contact appears as an emoji count line
/// directly below the original message in the chat window.
#[rstest]
fn chat_reaction_appears_below_message(xmpp_with_contact: XmppFixture) {
    // Open the chat window so messages are visible.
    xmpp_with_contact.send_command("/msg contact@localhost");
    thread::sleep(Duration::from_millis(300));

    // Inject a chat message.
    xmpp_with_contact.inject(chat_message(
        "contact@localhost",
        "user@localhost/aparte_test",
        "react-msg-1",
        "Hello there",
    ));

    let found = xmpp_with_contact.wait_for("Hello there", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        found,
        "original message did not appear in chat window\n{}",
        describe(parser.screen()),
    );

    // Contact reacts with 👍.
    xmpp_with_contact.inject(chat_reaction(
        "contact@localhost",
        "user@localhost/aparte_test",
        "react-msg-1",
        &["👍"],
    ));

    let reacted = xmpp_with_contact.wait_for("👍", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        reacted,
        "reaction emoji did not appear after injecting reaction\n{}",
        describe(parser.screen()),
    );
}

/// In a MUC, a reaction that references a stanza-id (XEP-0359) resolves to
/// the correct message and the emoji appears below it.
#[test]
fn groupchat_reaction_via_stanza_id_appears() {
    let xmpp = XmppFixture::new_with_muc_sid(&[], &[ROOM]);
    thread::sleep(Duration::from_millis(300));
    join_room(&xmpp, ROOM, NICK);

    // Inject a groupchat message that carries a stanza-id.
    xmpp.inject(groupchat_message_with_stanza_id(
        &format!("{ROOM}/otheruser"),
        "user@localhost",
        "gc-react-msg-1",
        "gc-react-sid-1",
        "MUC message",
    ));

    let found = xmpp.wait_for("MUC message", Duration::from_secs(5));
    let parser = xmpp.snapshot();
    assert!(
        found,
        "original MUC message did not appear\n{}",
        describe(parser.screen()),
    );

    // Another occupant reacts using the stanza-id as the reference id.
    xmpp.inject(groupchat_reaction(
        &format!("{ROOM}/reactor"),
        "user@localhost",
        "gc-react-sid-1",
        &["🎉"],
    ));

    let reacted = xmpp.wait_for("🎉", Duration::from_secs(5));
    let parser = xmpp.snapshot();
    assert!(
        reacted,
        "reaction emoji did not appear for MUC message with stanza-id\n{}",
        describe(parser.screen()),
    );
}

/// When a contact sends a new reaction set to the same message, their previous
/// emoji is replaced. The old emoji count must disappear and the new one appear.
#[rstest]
fn reaction_update_replaces_previous(xmpp_with_contact: XmppFixture) {
    xmpp_with_contact.send_command("/msg contact@localhost");
    thread::sleep(Duration::from_millis(300));

    xmpp_with_contact.inject(chat_message(
        "contact@localhost",
        "user@localhost/aparte_test",
        "react-upd-msg-1",
        "Update test",
    ));

    xmpp_with_contact.wait_for("Update test", Duration::from_secs(5));

    // First reaction: 👎
    xmpp_with_contact.inject(chat_reaction(
        "contact@localhost",
        "user@localhost/aparte_test",
        "react-upd-msg-1",
        &["👎"],
    ));

    let thumbs_down = xmpp_with_contact.wait_for("👎", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        thumbs_down,
        "initial 👎 reaction did not appear\n{}",
        describe(parser.screen()),
    );

    // Update: replace with 👍 — the 👎 must disappear.
    xmpp_with_contact.inject(chat_reaction(
        "contact@localhost",
        "user@localhost/aparte_test",
        "react-upd-msg-1",
        &["👍"],
    ));

    let thumbs_up = xmpp_with_contact.wait_for("👍", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        thumbs_up,
        "updated 👍 reaction did not appear\n{}",
        describe(parser.screen()),
    );

    // 👎 must be gone from the screen now.
    assert!(
        !grid_contains(parser.screen(), "👎"),
        "old 👎 reaction still visible after update\n{}",
        describe(parser.screen()),
    );
}

/// Both the original message and its reaction come from the MAM archive.
/// The reaction MAM result is processed while the original message's Event::Message
/// is still in the queue (not yet stored). The waiting-reaction flush must work
/// regardless of the HashMap iteration order of on_event callbacks.
#[test]
fn reaction_and_message_both_from_mam_appears() {
    let xmpp = XmppFixture::new_with_mam_and_reaction(
        &["contact@localhost"],
        &[(MAM_FROM, MAM_TO, MAM_MSG_ID, "Archived message")],
        &[(MAM_FROM, MAM_TO, MAM_MSG_ID, "👍")],
    );

    xmpp.send_command("/msg contact@localhost");

    let found = xmpp.wait_for("Archived message", Duration::from_secs(5));
    let parser = xmpp.snapshot();
    assert!(
        found,
        "MAM message did not appear in chat window\n{}",
        describe(parser.screen()),
    );

    let reacted = xmpp.wait_for("👍", Duration::from_secs(5));
    let parser = xmpp.snapshot();
    assert!(
        reacted,
        "reaction from MAM archive did not appear below the message\n{}",
        describe(parser.screen()),
    );
}

/// A live reaction to a message that was loaded from MAM history must appear
/// below that message in the chat window.
#[test]
fn reaction_to_mam_message_appears() {
    let xmpp = XmppFixture::new_with_mam(
        &["contact@localhost"],
        &[(
            "contact@localhost",
            "user@localhost/aparte_test",
            MAM_MSG_ID,
            "Archived hello",
        )],
    );

    // Open chat window — this triggers MAM load.
    xmpp.send_command("/msg contact@localhost");

    // Wait until the archived message has been rendered in the chat window.
    let found = xmpp.wait_for("Archived hello", Duration::from_secs(5));
    let parser = xmpp.snapshot();
    assert!(
        found,
        "MAM message did not appear in chat window\n{}",
        describe(parser.screen()),
    );

    // Inject a live reaction referencing the archived message's id.
    xmpp.inject(chat_reaction(
        "contact@localhost",
        "user@localhost/aparte_test",
        MAM_MSG_ID,
        &["👍"],
    ));

    let reacted = xmpp.wait_for("👍", Duration::from_secs(5));
    let parser = xmpp.snapshot();
    assert!(
        reacted,
        "reaction to MAM-loaded message did not appear\n{}",
        describe(parser.screen()),
    );
}
