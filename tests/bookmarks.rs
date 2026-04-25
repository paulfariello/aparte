//! Bookmarks (XEP-0048) integration tests.

mod common;

use std::thread;
use std::time::Duration;

use rstest::rstest;

use common::describe;
use common::xmpp_fixture::{bookmarks_v1_push_event, xmpp_with_disco, XmppFixture};

/// A bookmarked room with autojoin=true is automatically joined when the server pushes it.
#[rstest]
fn bookmarks_autojoin_on_push(xmpp_with_disco: XmppFixture) {
    // Allow init IQs (pubsub create/subscribe) to be acked before pushing bookmarks
    thread::sleep(Duration::from_millis(800));
    xmpp_with_disco.inject(bookmarks_v1_push_event(
        "user@localhost",
        "user@localhost/aparte_test",
        &[("dev@conference.localhost", "Dev Room", true)],
    ));
    let found = xmpp_with_disco.wait_for("dev@conference.localhost", Duration::from_secs(10));
    let parser = xmpp_with_disco.snapshot();
    assert!(
        found,
        "auto-join room not visible after bookmark push\n{}",
        describe(parser.screen()),
    );
}
