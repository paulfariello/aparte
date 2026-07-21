mod common;

use std::time::Duration;

use common::{describe, row_text, wait_for_ready, wait_for_screen, Harness, INPUT_ROW};

#[test]
fn prompt_visible_in_chat_window() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    h.send_command("/inject_msg hello");
    wait_for_screen(&h, "test-contact", Duration::from_secs(5));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        row_text(screen, INPUT_ROW).starts_with("> "),
        "Chat window input bar must show '> ' prompt\n{}",
        describe(screen)
    );
}

#[test]
fn console_has_no_input_prompt() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        !row_text(screen, INPUT_ROW).starts_with("> "),
        "Console must not show '> ' prompt on the input row\n{}",
        describe(screen)
    );
}

#[test]
fn draft_persists_across_window_switch() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    h.send_command("/inject_msg hello");
    wait_for_screen(&h, "test-contact", Duration::from_secs(5));

    // Type a draft in the chat window.
    h.send_bytes(b"\x1b");
    wait_for_screen(&h, "NORMAL", Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"i");
    wait_for_screen(&h, "INSERT", Duration::from_secs(2));
    h.send_bytes(b"my draft");
    wait_for_screen(&h, "my draft", Duration::from_secs(2));

    // Switch to console.
    h.send_bytes(b"\x1b");
    wait_for_screen(&h, "NORMAL", Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(150));
    h.send_bytes(b":win console\r");
    wait_for_screen(&h, "console", Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(200));

    // Switch back to the chat window.
    h.send_bytes(b":win test-contact@test.localhost\r");
    wait_for_screen(&h, "test-contact", Duration::from_secs(3));
    std::thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        row_text(screen, INPUT_ROW).contains("my draft"),
        "Draft must survive a window switch\n{}",
        describe(screen)
    );
}
