//! Integration tests for the date-change separator line between chat messages.

mod common;

use std::thread;
use std::time::Duration;

use rstest::rstest;

use common::xmpp_fixture::{chat_message_with_delay, xmpp_with_contact, XmppFixture};
use common::{
    describe, find_row_with, grid_contains, row_has_bgcolor, row_text, SELECTION_BGCOLOR,
};

const CONTACT: &str = "contact@localhost";
const BOUND_JID: &str = "user@localhost/aparte_test";

/// Two messages arriving from different calendar days must be separated by a ─ line.
///
/// Both stamps are set to noon UTC so the local date is stable across all timezones
/// (UTC-12 through UTC+14 all land on the same calendar date for noon-UTC stamps).
#[rstest]
fn date_separator_appears_between_messages_from_different_days(xmpp_with_contact: XmppFixture) {
    xmpp_with_contact.send_command(&format!("/msg {CONTACT}"));
    xmpp_with_contact.switch_window(CONTACT);

    xmpp_with_contact.inject(chat_message_with_delay(
        CONTACT,
        BOUND_JID,
        "day1-msg",
        "Day one message",
        "2024-06-01T12:00:00Z",
    ));
    xmpp_with_contact.inject(chat_message_with_delay(
        CONTACT,
        BOUND_JID,
        "day2-msg",
        "Day two message",
        "2024-06-02T12:00:00Z",
    ));

    let _ = xmpp_with_contact.wait_for("Day one message", Duration::from_secs(5));
    let _ = xmpp_with_contact.wait_for("Day two message", Duration::from_secs(5));

    let sep_found = xmpp_with_contact.wait_for("─", Duration::from_secs(5));
    let parser = xmpp_with_contact.snapshot();
    let screen = parser.screen();

    assert!(
        sep_found,
        "Date separator line (─) not found between messages from different days\n{}",
        describe(screen),
    );

    let row1 = find_row_with(screen, "Day one message");
    let row_sep = find_row_with(screen, "─");
    let row2 = find_row_with(screen, "Day two message");

    assert!(
        row1.is_some() && row2.is_some(),
        "Both message bodies must be visible\n{}",
        describe(screen),
    );
    assert!(
        row1 < row_sep && row_sep < row2,
        "Separator must sit between the two message rows: msg1={row1:?}, sep={row_sep:?}, msg2={row2:?}\n{}",
        describe(screen),
    );
}

/// Two messages arriving on the same calendar day must not have a separator between them.
#[rstest]
fn no_date_separator_for_same_day_messages(xmpp_with_contact: XmppFixture) {
    xmpp_with_contact.send_command(&format!("/msg {CONTACT}"));
    xmpp_with_contact.switch_window(CONTACT);

    xmpp_with_contact.inject(chat_message_with_delay(
        CONTACT,
        BOUND_JID,
        "same-day-msg1",
        "First same-day message",
        "2024-06-01T12:00:00Z",
    ));
    xmpp_with_contact.inject(chat_message_with_delay(
        CONTACT,
        BOUND_JID,
        "same-day-msg2",
        "Second same-day message",
        "2024-06-01T13:00:00Z",
    ));

    let _ = xmpp_with_contact.wait_for("First same-day message", Duration::from_secs(5));
    let _ = xmpp_with_contact.wait_for("Second same-day message", Duration::from_secs(5));
    // Let the UI settle; a separator would have appeared by now if it were going to.
    thread::sleep(Duration::from_millis(300));

    let parser = xmpp_with_contact.snapshot();
    let screen = parser.screen();

    let row1 = find_row_with(screen, "First same-day message").expect("msg1 on screen");
    let row2 = find_row_with(screen, "Second same-day message").expect("msg2 on screen");

    let sep_between = (row1 + 1..row2).any(|r| row_text(screen, r).contains('─'));

    assert!(
        !sep_between,
        "No date separator should appear between same-day messages (rows {row1}..{row2})\n{}",
        describe(screen),
    );

    // Also confirm no stray separator anywhere on screen
    assert!(
        !grid_contains(screen, "─"),
        "Unexpected ─ separator found on screen for same-day messages\n{}",
        describe(screen),
    );
}

/// Selecting a message that owns a day-barrier must not highlight the barrier row.
///
/// When the second message (from a different day) is selected in Normal mode,
/// only its content row should carry the selection background; the ─ separator
/// line that belongs to the same MessageView must keep its themed background.
#[rstest]
fn selecting_message_with_day_barrier_does_not_highlight_barrier_row(
    xmpp_with_contact: XmppFixture,
) {
    xmpp_with_contact.send_command(&format!("/msg {CONTACT}"));
    xmpp_with_contact.switch_window(CONTACT);

    xmpp_with_contact.inject(chat_message_with_delay(
        CONTACT,
        BOUND_JID,
        "sel-day1-msg",
        "Selection day one",
        "2024-07-01T12:00:00Z",
    ));
    xmpp_with_contact.inject(chat_message_with_delay(
        CONTACT,
        BOUND_JID,
        "sel-day2-msg",
        "Selection day two",
        "2024-07-02T12:00:00Z",
    ));

    let _ = xmpp_with_contact.wait_for("Selection day one", Duration::from_secs(5));
    let _ = xmpp_with_contact.wait_for("Selection day two", Duration::from_secs(5));
    let _ = xmpp_with_contact.wait_for("─", Duration::from_secs(5));

    // Enter Normal mode and jump to the last message (which has the separator).
    xmpp_with_contact.send_bytes(b"\x1b");
    xmpp_with_contact.wait_for("NORMAL", Duration::from_secs(2));
    xmpp_with_contact.send_bytes(b"G");
    thread::sleep(Duration::from_millis(300));

    let parser = xmpp_with_contact.snapshot();
    let screen = parser.screen();

    let row_sep = find_row_with(screen, "─").expect("date separator (─) must be visible on screen");
    let row_msg = find_row_with(screen, "Selection day two")
        .expect("second message must be visible on screen");

    assert!(
        row_sep < row_msg,
        "separator row ({row_sep}) must appear above message row ({row_msg})\n{}",
        describe(screen),
    );
    assert!(
        row_has_bgcolor(screen, row_msg, SELECTION_BGCOLOR),
        "message content row {row_msg} must carry the selection background\n{}",
        describe(screen),
    );
    assert!(
        !row_has_bgcolor(screen, row_sep, SELECTION_BGCOLOR),
        "day-barrier row {row_sep} must NOT carry the selection background\n{}",
        describe(screen),
    );
}
