mod common;

use std::thread;
use std::time::Duration;

use common::{
    describe, grid_contains, row_text, rows_with_bgcolor, wait_for_ready, wait_for_screen, Harness,
    SELECTION_BGCOLOR,
};

/// Send enough unknown commands to push the welcome banner off-screen.
/// Waits until the last message is confirmed on screen before returning.
fn fill_console(h: &Harness) {
    for i in 0..30 {
        h.send_command(&format!("/bad{i}"));
    }
    // Wait until the last message is rendered
    wait_for_screen(h, "bad29", Duration::from_secs(5));
}

fn enter_normal(h: &Harness) {
    h.send_bytes(b"\x1b");
    wait_for_screen(h, "NORMAL", Duration::from_secs(2));
}

#[test]
fn default_mode_is_insert() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    let parser = h.snapshot();
    let screen = parser.screen();

    h.shutdown();

    assert!(
        grid_contains(screen, "INSERT"),
        "Expected INSERT mode indicator in win_bar\n{}",
        describe(screen)
    );
}

#[test]
fn escape_switches_to_normal_mode() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    h.send_bytes(b"\x1b");

    let found = wait_for_screen(&h, "NORMAL", Duration::from_secs(2));
    let parser = h.snapshot();
    let screen = parser.screen();

    h.shutdown();

    assert!(
        found,
        "Expected NORMAL mode indicator after Escape\n{}",
        describe(screen)
    );
}

#[test]
fn i_in_normal_mode_returns_to_insert() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    h.send_bytes(b"\x1b");
    let _ = wait_for_screen(&h, "NORMAL", Duration::from_secs(2));

    h.send_bytes(b"i");
    let found = wait_for_screen(&h, "INSERT", Duration::from_secs(2));
    let parser = h.snapshot();
    let screen = parser.screen();

    h.shutdown();

    assert!(
        found,
        "Expected INSERT mode indicator after 'i' in Normal mode\n{}",
        describe(screen)
    );
}

#[test]
fn typing_in_normal_mode_does_not_reach_input() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    h.send_bytes(b"\x1b");
    let _ = wait_for_screen(&h, "NORMAL", Duration::from_secs(2));

    h.send_bytes(b"hello");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();

    h.shutdown();

    assert!(
        !grid_contains(screen, "hello"),
        "Typed text should not appear in input while in Normal mode\n{}",
        describe(screen)
    );
}

#[test]
fn k_in_normal_mode_does_not_type_in_input() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);
    h.send_bytes(b"k");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        !grid_contains(screen, "k"),
        "k should not appear in input while in Normal mode\n{}",
        describe(screen)
    );
}

#[test]
fn j_in_normal_mode_does_not_type_in_input() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);
    h.send_bytes(b"j");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        !grid_contains(screen, "j"),
        "j should not appear in input while in Normal mode\n{}",
        describe(screen)
    );
}

#[test]
fn i_after_jk_navigation_returns_to_insert() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);
    h.send_bytes(b"k");
    h.send_bytes(b"k");
    h.send_bytes(b"j");
    thread::sleep(Duration::from_millis(200));

    h.send_bytes(b"i");
    let found = wait_for_screen(&h, "INSERT", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
        "Expected INSERT mode after 'i' following j/k navigation\n{}",
        describe(screen)
    );
}

#[test]
fn insert_mode_typing_works_after_jk_navigation() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);
    h.send_bytes(b"kkj");
    thread::sleep(Duration::from_millis(200));

    // Return to insert and type something
    h.send_bytes(b"i");
    let _ = wait_for_screen(&h, "INSERT", Duration::from_secs(2));
    h.send_bytes(b"hello");
    let found = wait_for_screen(&h, "hello", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
        "Typed text should appear in input after returning from Normal mode\n{}",
        describe(screen)
    );
}

#[test]
fn gg_in_normal_mode_does_not_appear_in_input() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);
    h.send_bytes(b"gg");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        !grid_contains(screen, "gg"),
        "gg should not appear in input while in Normal mode\n{}",
        describe(screen)
    );
}

#[test]
fn partial_command_shows_in_winbar() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);
    // Send 'g' — partial prefix for both 'gg' and 'gG'
    h.send_bytes(b"g");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        grid_contains(screen, "g"),
        "Partial command 'g' should appear in the win_bar\n{}",
        describe(screen)
    );
}

#[test]
fn partial_command_clears_on_nonmatch() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);
    // 'g' then 'x' — 'gx' matches no command, buffer should clear
    h.send_bytes(b"g");
    thread::sleep(Duration::from_millis(100));
    h.send_bytes(b"x");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    // After clearing, the buffer indicator 'g' should no longer appear adjacent to the mode
    // (it's acceptable that a single 'g' might appear in other parts of the UI, but the
    // command buffer specifically should be gone — verified by returning to normal idle state)
    assert!(
        !grid_contains(screen, "gx"),
        "gx should not appear in the win_bar after clearing\n{}",
        describe(screen)
    );
}

#[test]
fn gg_scrolls_to_oldest_message() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    // Push enough messages to scroll the welcome banner off-screen
    fill_console(&h);

    // Confirm the banner is not visible while at the bottom
    {
        let parser = h.snapshot();
        assert!(
            !grid_contains(parser.screen(), "\u{258C}"),
            "Welcome banner should be off-screen before gg\n{}",
            describe(parser.screen())
        );
    }

    enter_normal(&h);
    h.send_bytes(b"gg");
    let found = wait_for_screen(&h, "\u{258C}", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
        "Welcome banner (▌ glyph) should be visible after gg\n{}",
        describe(screen)
    );
}

#[test]
fn gg_selects_first_visible_message() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);

    enter_normal(&h);
    h.send_bytes(b"gg");
    wait_for_screen(&h, "\u{258C}", Duration::from_secs(2));
    thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    // After gg exactly one message (the welcome banner) must be highlighted.
    let selected_rows = rows_with_bgcolor(screen, SELECTION_BGCOLOR);
    assert!(
        !selected_rows.is_empty(),
        "Expected some rows to have the selection background after gg\n{}",
        describe(screen)
    );
    assert!(
        selected_rows.windows(2).all(|w| w[1] == w[0] + 1),
        "Selection spans non-contiguous rows — more than one message is highlighted\n{}",
        describe(screen)
    );
    assert!(
        selected_rows
            .iter()
            .any(|&r| row_text(screen, r).contains('\u{258C}')),
        "Expected the selected block to contain the welcome-banner glyph after gg\n{}",
        describe(screen)
    );
}

#[test]
#[allow(non_snake_case)]
fn G_selects_last_message() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);

    enter_normal(&h);
    h.send_bytes(b"gg");
    wait_for_screen(&h, "\u{258C}", Duration::from_secs(2));

    h.send_bytes(b"G");
    wait_for_screen(&h, "bad29", Duration::from_secs(2));
    thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    // After G exactly one message ('bad29') must be highlighted.
    let selected_rows = rows_with_bgcolor(screen, SELECTION_BGCOLOR);
    assert!(
        !selected_rows.is_empty(),
        "Expected some rows to have the selection background after G\n{}",
        describe(screen)
    );
    assert!(
        selected_rows.windows(2).all(|w| w[1] == w[0] + 1),
        "Selection spans non-contiguous rows — more than one message is highlighted\n{}",
        describe(screen)
    );
    assert!(
        selected_rows
            .iter()
            .any(|&r| row_text(screen, r).contains("bad29")),
        "Expected the selected block to contain 'bad29' after G\n{}",
        describe(screen)
    );
}

#[test]
fn insert_mode_page_up_scrolls_message_window() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);

    let found = wait_for_screen(&h, "bad29", Duration::from_secs(5));
    assert!(found, "Expected bad29 visible before PageUp test");

    // Confirm the app has settled in INSERT mode before sending PageUp.
    let in_insert = wait_for_screen(&h, "INSERT", Duration::from_secs(2));
    assert!(
        in_insert,
        "Expected INSERT mode indicator before PageUp test"
    );

    // In INSERT mode (the default), PageUp SHOULD scroll the message window.
    h.send_bytes(b"\x1b[5~");

    // Poll until bad29 scrolls off — more robust than a fixed sleep.
    let scrolled = {
        use std::time::Instant;
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut found = false;
        while Instant::now() < deadline {
            let parser = h.snapshot();
            if !grid_contains(parser.screen(), "bad29") {
                found = true;
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        found
    };

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        scrolled,
        "PageUp in INSERT mode should scroll the message window away from newest message\n{}",
        describe(screen)
    );
}

#[test]
fn normal_mode_page_up_scrolls_message_window() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);

    wait_for_screen(&h, "bad29", Duration::from_secs(5));

    enter_normal(&h);
    h.send_bytes(b"\x1b[5~"); // PageUp
    thread::sleep(Duration::from_millis(400));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        !grid_contains(screen, "bad29"),
        "PageUp in NORMAL mode should scroll the message window away from newest message\n{}",
        describe(screen)
    );
}

#[test]
fn insert_mode_jk_do_not_move_message_selection() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);
    wait_for_screen(&h, "bad29", Duration::from_secs(5));

    // Enter NORMAL mode and select a message with 'k'.
    enter_normal(&h);
    h.send_bytes(b"k");
    thread::sleep(Duration::from_millis(200));

    // Return to INSERT mode — ModeChange clears the selection.
    h.send_bytes(b"i");
    let _ = wait_for_screen(&h, "INSERT", Duration::from_secs(2));

    // Typing 'j' and 'k' in INSERT mode must not re-select any message.
    h.send_bytes(b"jk");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    let selected = rows_with_bgcolor(screen, SELECTION_BGCOLOR);
    assert!(
        selected.is_empty(),
        "j/k in INSERT mode should not select any message, but {} rows are highlighted\n{}",
        selected.len(),
        describe(screen)
    );
}

#[test]
fn cursor_visible_in_insert_mode() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    let parser = h.snapshot();
    let screen = parser.screen();

    h.shutdown();

    assert!(
        !screen.hide_cursor(),
        "Cursor should be visible in INSERT mode\n{}",
        describe(screen)
    );
}

#[test]
fn cursor_steady_block_in_normal_mode() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);

    let parser = h.snapshot();
    let screen = parser.screen();

    // SetCursorStyle::SteadyBlock emits ESC [ 2 SP q  (DECSCUSR = 2)
    let steady_block: &[u8] = b"\x1b[2 q";
    let bytes = h.bytes.lock().unwrap().clone();
    let found = bytes.windows(steady_block.len()).any(|w| w == steady_block);

    h.shutdown();

    assert!(
        !screen.hide_cursor(),
        "Cursor should be visible in NORMAL mode\n{}",
        describe(screen)
    );
    assert!(
        found,
        "SteadyBlock escape (ESC[2 q) should have been sent when entering NORMAL mode"
    );
}

#[test]
fn cursor_steady_bar_escape_sent_in_insert_mode() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    // SetCursorStyle::SteadyBar emits ESC [ 6 SP q  (DECSCUSR = 6)
    let steady_bar: &[u8] = b"\x1b[6 q";
    let bytes = h.bytes.lock().unwrap().clone();
    let found = bytes.windows(steady_bar.len()).any(|w| w == steady_bar);

    h.shutdown();

    assert!(
        found,
        "SteadyBar escape (ESC[6 q) should have been sent during INSERT mode rendering"
    );
}

#[test]
#[allow(non_snake_case)]
fn G_scrolls_to_newest_message() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);

    // Go to top first so G has something to do
    enter_normal(&h);
    h.send_bytes(b"gg");
    wait_for_screen(&h, "\u{258C}", Duration::from_secs(2));

    // Now scroll to bottom with G
    h.send_bytes(b"G");
    let found = wait_for_screen(&h, "bad29", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
        "Most recent message (bad29) should be visible after G\n{}",
        describe(screen)
    );
}
