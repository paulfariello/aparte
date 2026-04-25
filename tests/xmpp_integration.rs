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

use common::describe;
use common::xmpp_fixture::{carbon_received, chat_message, xmpp, xmpp_with_contact, XmppFixture};

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
