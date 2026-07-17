mod common;

use std::thread;
use std::time::Duration;

use common::{
    describe, row_text, wait_for_ready, wait_for_screen, Harness, COMMAND_ROW, INPUT_ROW,
    STATUS_ROW,
};

fn enter_normal(h: &Harness) {
    h.send_bytes(b"\x1b");
    wait_for_screen(h, "NORMAL", Duration::from_secs(2));
    // Wait for crossterm's Esc-sequence timeout (~100 ms) so the next key is
    // not misread as Alt+<key> by the terminal parser.
    thread::sleep(Duration::from_millis(150));
}

fn type_draft(h: &Harness, draft: &str) {
    enter_normal(h);
    h.send_bytes(b"i");
    wait_for_screen(h, "INSERT", Duration::from_secs(2));
    h.send_bytes(draft.as_bytes());
    wait_for_screen(h, draft, Duration::from_secs(2));
}

#[test]
fn command_types_in_command_bar_while_draft_stays_in_input_bar() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    type_draft(&h, "hello draft");
    enter_normal(&h);
    h.send_bytes(b":win");
    wait_for_screen(&h, ":win", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        row_text(screen, COMMAND_ROW).contains(":win"),
        "Command being typed must render on the command bar (bottom row)\n{}",
        describe(screen)
    );
    assert_eq!(
        screen.cursor_position().0,
        COMMAND_ROW,
        "Cursor must sit on the command bar while typing a command\n{}",
        describe(screen)
    );
    assert!(
        row_text(screen, STATUS_ROW).contains("COMMAND"),
        "Status line (above the command bar) must show the mode\n{}",
        describe(screen)
    );
    assert!(
        row_text(screen, INPUT_ROW).contains("hello draft"),
        "Draft message must stay visible in the input bar while a command is typed\n{}",
        describe(screen)
    );
}

#[test]
fn draft_survives_command_escape_and_execution() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    type_draft(&h, "still here");

    // Cancelled command.
    enter_normal(&h);
    h.send_bytes(b":nope");
    wait_for_screen(&h, ":nope", Duration::from_secs(2));
    h.send_bytes(b"\x1b");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    assert!(
        row_text(screen, INPUT_ROW).contains("still here"),
        "Draft must survive a cancelled command\n{}",
        describe(screen)
    );
    assert!(
        !row_text(screen, COMMAND_ROW).contains(":nope"),
        "Command bar must be empty again after Esc\n{}",
        describe(screen)
    );

    // Executed command.
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b":win console\r");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        row_text(screen, INPUT_ROW).contains("still here"),
        "Draft must survive an executed command\n{}",
        describe(screen)
    );
}

/// The top row is a tab bar: every open window is listed in open order, with
/// the current one emphasized — not only windows with pending activity.
#[test]
fn tab_bar_lists_all_windows_with_current_emphasized() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    // Console is the only window at startup and must appear as a tab even
    // though it has no pending activity.
    let parser = h.snapshot();
    assert!(
        row_text(parser.screen(), 0).contains("console"),
        "Tab bar must show the console tab at startup\n{}",
        describe(parser.screen())
    );

    // Open a chat window (becomes current); both tabs must be visible.
    h.send_command("/inject_msg hello");
    wait_for_screen(&h, "test-contact", Duration::from_secs(5));

    let parser = h.snapshot();
    let screen = parser.screen();
    let tabs = row_text(screen, 0);
    let current_bold = bold_span(screen, 0, "test-contact@test.localhost");
    let console_bold = bold_span(screen, 0, "console");
    h.shutdown();

    assert!(
        tabs.contains("console") && tabs.contains("test-contact@test.localhost"),
        "Tab bar must list all open windows, got: {:?}",
        tabs
    );
    assert_eq!(
        current_bold,
        Some(true),
        "Current window tab must be emphasized (bold), tabs: {:?}",
        tabs
    );
    assert_eq!(
        console_bold,
        Some(false),
        "Non-current idle tab must not be emphasized, tabs: {:?}",
        tabs
    );
}

/// Password prompts render on the command bar; Esc cancels the prompt,
/// hands focus back to the input bar (Insert mode must work again), and the
/// aborted partial password is never submitted.
#[test]
fn password_prompt_on_command_bar_esc_cancels() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);
    h.send_bytes(b":connect esc-test@localhost\r");
    let prompted = wait_for_screen(&h, "password:", Duration::from_secs(5));
    let parser = h.snapshot();
    assert!(
        prompted,
        "password prompt must appear\n{}",
        describe(parser.screen())
    );
    assert!(
        row_text(parser.screen(), COMMAND_ROW).contains("password:"),
        "password prompt must render on the command bar\n{}",
        describe(parser.screen())
    );

    // Type part of a password, then abort with Esc.
    h.send_bytes(b"secr");
    thread::sleep(Duration::from_millis(200));
    h.send_bytes(b"\x1b");
    wait_for_screen(&h, "NORMAL", Duration::from_secs(2));
    thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    assert!(
        !row_text(parser.screen(), COMMAND_ROW).contains("password:"),
        "password prompt must clear on Esc\n{}",
        describe(parser.screen())
    );

    // Focus must be back on the input bar: Insert mode works again.
    h.send_bytes(b"i");
    let insert = wait_for_screen(&h, "INSERT", Duration::from_secs(2));
    let parser = h.snapshot();
    assert!(
        insert,
        "'i' must re-enter INSERT after a cancelled password prompt\n{}",
        describe(parser.screen())
    );
    h.send_bytes(b"after prompt");
    let typed = wait_for_screen(&h, "after prompt", Duration::from_secs(2));
    let parser = h.snapshot();
    assert!(
        typed && row_text(parser.screen(), INPUT_ROW).contains("after prompt"),
        "typing must reach the input bar after a cancelled password prompt\n{}",
        describe(parser.screen())
    );

    // Enter must not submit the aborted password (and must not crash).
    h.send_bytes(b"\r");
    thread::sleep(Duration::from_millis(300));
    assert!(
        !h.exited.load(std::sync::atomic::Ordering::Relaxed),
        "app must survive Enter after a cancelled password prompt"
    );
    h.shutdown();
}

/// A failed command echoes its error on the command bar (also logged to the
/// console window) and the echo clears on the next keypress.
#[test]
fn command_error_echoes_in_command_bar_until_keypress() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);
    h.send_bytes(b":nosuchcmd\r");

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut echoed = false;
    while std::time::Instant::now() < deadline {
        let parser = h.snapshot();
        if row_text(parser.screen(), COMMAND_ROW).contains("Unknown command nosuchcmd") {
            echoed = true;
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let parser = h.snapshot();
    assert!(
        echoed,
        "Command error must echo on the command bar\n{}",
        describe(parser.screen())
    );

    // Any keypress clears the echo.
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"i");
    wait_for_screen(&h, "INSERT", Duration::from_secs(2));
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut cleared = false;
    while std::time::Instant::now() < deadline {
        let parser = h.snapshot();
        if !row_text(parser.screen(), COMMAND_ROW).contains("Unknown command") {
            cleared = true;
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let parser = h.snapshot();
    h.shutdown();
    assert!(
        cleared,
        "Command error echo must clear on the next keypress\n{}",
        describe(parser.screen())
    );
}

/// Whether every cell of `needle`'s span on `row` is bold.
/// `None` when the needle is not on that row.
fn bold_span(screen: &vt100::Screen, row: u16, needle: &str) -> Option<bool> {
    let text = row_text(screen, row);
    let start = text.find(needle)? as u16;
    #[allow(clippy::cast_possible_truncation)]
    let end = start + needle.len() as u16;
    Some((start..end).all(|c| screen.cell(row, c).is_some_and(vt100::Cell::bold)))
}

#[test]
fn search_types_in_command_bar() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);
    h.send_bytes(b"/needle");
    wait_for_screen(&h, "/needle", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        row_text(screen, COMMAND_ROW).contains("/needle"),
        "Search being typed must render on the command bar\n{}",
        describe(screen)
    );
}
