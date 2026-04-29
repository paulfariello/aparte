//! Message Archive Management (XEP-0313) integration tests.

mod common;

use std::time::Duration;

use rstest::rstest;

use common::describe;
use common::xmpp_fixture::XmppFixture;

fn xmpp_with_contact_mam() -> XmppFixture {
    XmppFixture::new_with_mam(
        &["contact@localhost"],
        &[(
            "contact@localhost",
            "user@localhost",
            "a1",
            "Archived message!",
        )],
    )
}

/// Archived messages from the server appear when a chat window is opened.
#[rstest]
fn mam_archived_messages_appear_on_window_open() {
    let xmpp = xmpp_with_contact_mam();
    xmpp.send_command("/msg contact@localhost");
    xmpp.switch_window("contact@localhost");
    let found = xmpp.wait_for("Archived message!", Duration::from_secs(10));
    let parser = xmpp.snapshot();
    assert!(
        found,
        "MAM archived message not visible after opening chat window\n{}",
        describe(parser.screen()),
    );
}

/// MAM history fetch is triggered only once when reaching the top, not on every PageUp.
#[rstest]
fn mam_pageup_triggers_only_once_at_top() {
    let mut xmpp = XmppFixture::new_with_mam(&["contact@localhost"], &[]);
    xmpp.send_command("/msg contact@localhost");
    xmpp.switch_window("contact@localhost");

    // Wait for the initial MAM query (triggered on chat open) to complete, then drain it.
    std::thread::sleep(Duration::from_millis(500));
    xmpp.drain_mam_queries();

    // First PageUp at top — should trigger exactly one MAM history fetch.
    xmpp.send_bytes(b"\x1b[5~");
    std::thread::sleep(Duration::from_millis(300));
    let count = xmpp.drain_mam_queries();
    assert_eq!(count, 1, "expected 1 MAM query on first PageUp at top");

    // Second PageUp while still at top — should trigger no further MAM fetch.
    xmpp.send_bytes(b"\x1b[5~");
    std::thread::sleep(Duration::from_millis(300));
    let count = xmpp.drain_mam_queries();
    assert_eq!(count, 0, "expected no MAM query on repeated PageUp at top");
}
