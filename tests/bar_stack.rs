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
