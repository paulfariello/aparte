//! Integration tests for message delivery status (XEP-0184 / XEP-0333).
//!
//! Covers:
//! - Outgoing chat message shows ✓ after a XEP-0184 delivery receipt.
//! - Outgoing chat message shows ✓✓ after a XEP-0333 displayed marker.
//! - Displayed (✓✓) upgrades a previously delivered (✓) message.

mod common;

use std::thread;
use std::time::Duration;

use rstest::rstest;

use common::describe;
use common::xmpp_fixture::{
    chat_displayed_marker, delivery_receipt, xmpp_with_contact, XmppFixture,
};

const CONTACT: &str = "contact@localhost/mobile";
const USER: &str = "user@localhost/aparte_test";

/// After a XEP-0184 receipt the outgoing message header shows ✓.
#[rstest]
fn delivery_receipt_shows_checkmark(mut xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));

    xmpp_with_contact.send_command("/msg contact@localhost");
    thread::sleep(Duration::from_millis(400));
    xmpp_with_contact.send_command("receipt-test-abc1");

    let appeared = xmpp_with_contact.wait_for("receipt-test-abc1", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        appeared,
        "outgoing message did not appear\n{}",
        describe(parser.screen()),
    );

    // Capture the outgoing stanza so we know its id.
    let sent = xmpp_with_contact
        .recv_outgoing_message_matching(
            |m| m.bodies.values().any(|b| b.contains("receipt-test-abc1")),
            Duration::from_secs(5),
        )
        .expect("outgoing stanza not captured");
    let msg_id = sent.id.as_ref().expect("sent stanza has no id").0.clone();

    // Contact sends back a XEP-0184 receipt.
    xmpp_with_contact.inject(delivery_receipt(CONTACT, USER, &msg_id));

    let delivered = xmpp_with_contact.wait_for("✓", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        delivered,
        "delivery checkmark ✓ did not appear after receipt\n{}",
        describe(parser.screen()),
    );
}

/// After a XEP-0333 displayed marker the outgoing message header shows ✓✓.
#[rstest]
fn displayed_marker_shows_double_checkmark(mut xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));

    xmpp_with_contact.send_command("/msg contact@localhost");
    thread::sleep(Duration::from_millis(400));
    xmpp_with_contact.send_command("displayed-test-xyz9");

    let appeared = xmpp_with_contact.wait_for("displayed-test-xyz9", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        appeared,
        "outgoing message did not appear\n{}",
        describe(parser.screen()),
    );

    let sent = xmpp_with_contact
        .recv_outgoing_message_matching(
            |m| m.bodies.values().any(|b| b.contains("displayed-test-xyz9")),
            Duration::from_secs(5),
        )
        .expect("outgoing stanza not captured");
    let msg_id = sent.id.as_ref().expect("sent stanza has no id").0.clone();

    // Contact sends a XEP-0333 <displayed> marker.
    xmpp_with_contact.inject(chat_displayed_marker(CONTACT, USER, &msg_id));

    let displayed = xmpp_with_contact.wait_for("✓✓", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        displayed,
        "double checkmark ✓✓ did not appear after displayed marker\n{}",
        describe(parser.screen()),
    );
}

/// Receiving a XEP-0333 <displayed> after a XEP-0184 receipt upgrades ✓ to ✓✓.
#[rstest]
fn displayed_upgrades_delivered_status(mut xmpp_with_contact: XmppFixture) {
    thread::sleep(Duration::from_millis(300));

    xmpp_with_contact.send_command("/msg contact@localhost");
    thread::sleep(Duration::from_millis(400));
    xmpp_with_contact.send_command("upgrade-test-q7r2");

    let appeared = xmpp_with_contact.wait_for("upgrade-test-q7r2", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        appeared,
        "outgoing message did not appear\n{}",
        describe(parser.screen()),
    );

    let sent = xmpp_with_contact
        .recv_outgoing_message_matching(
            |m| m.bodies.values().any(|b| b.contains("upgrade-test-q7r2")),
            Duration::from_secs(5),
        )
        .expect("outgoing stanza not captured");
    let msg_id = sent.id.as_ref().expect("sent stanza has no id").0.clone();

    // First a XEP-0184 receipt → single ✓.
    xmpp_with_contact.inject(delivery_receipt(CONTACT, USER, &msg_id));
    let delivered = xmpp_with_contact.wait_for("✓", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        delivered,
        "delivery checkmark ✓ did not appear\n{}",
        describe(parser.screen()),
    );

    // Then a XEP-0333 <displayed> → upgrades to ✓✓.
    xmpp_with_contact.inject(chat_displayed_marker(CONTACT, USER, &msg_id));
    let upgraded = xmpp_with_contact.wait_for("✓✓", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    assert!(
        upgraded,
        "double checkmark ✓✓ did not appear after upgrade from delivered\n{}",
        describe(parser.screen()),
    );
}
