//! Message Archive Management (XEP-0313) integration tests.

mod common;

use std::time::Duration;

use rstest::rstest;

use common::describe;
use common::xmpp_fixture::XmppFixture;

fn xmpp_with_contact_mam() -> XmppFixture {
    XmppFixture::new_with_mam(
        &["contact@localhost"],
        &[("contact@localhost", "user@localhost", "a1", "Archived message!")],
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
