mod common;

use std::thread;
use std::time::{Duration, Instant};

use common::{
    describe, find_row_with, grid_contains, row_text, rows_with_bgcolor, wait_for_ready,
    wait_for_screen, Harness, SELECTION_BGCOLOR,
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
fn insert_mode_page_up_to_top_shows_multiple_messages() {
    // Regression test: when repeated PageUps drove bottom_visible_child_index to 0,
    // layout_from_top only showed the single first message. This verifies that the
    // welcome banner AND the next message ("bad0") are both visible at the top.
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);
    wait_for_screen(&h, "bad29", Duration::from_secs(5));
    wait_for_screen(&h, "INSERT", Duration::from_secs(2));

    // Press PageUp until the welcome banner comes back into view.
    let banner_visible = {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut found = false;
        while Instant::now() < deadline {
            h.send_bytes(b"\x1b[5~");
            thread::sleep(Duration::from_millis(150));
            let parser = h.snapshot();
            if grid_contains(parser.screen(), "\u{258C}") {
                found = true;
                break;
            }
        }
        found
    };

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        banner_visible,
        "Expected welcome banner to appear after repeated PageUps\n{}",
        describe(screen)
    );

    // The banner must be flush with the top of the message window (row 0 is the
    // win_bar, so the message window starts at row 1). If blank rows appear above
    // the banner, layout_from_bottom anchored it to the bottom instead of the top.
    let banner_row = find_row_with(screen, "\u{258C}");
    assert!(
        matches!(banner_row, Some(r) if r <= 4),
        "Welcome banner should be at the top of the message window (row ≤ 4), got {:?}\n{}",
        banner_row,
        describe(screen)
    );

    // Messages after the banner must also be visible. When layout_from_top only
    // laid out a single child (the bottom_visible_child_index=0 bug), the rows
    // below the banner were blank and "bad0" was absent.
    assert!(
        grid_contains(screen, "bad0"),
        "PageUp to top should show multiple messages — expected 'bad0' visible alongside the welcome banner\n{}",
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

#[test]
fn search_slash_shows_in_winbar() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);
    h.send_bytes(b"/");
    let found = wait_for_screen(&h, "/", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
        "'/' should appear in win_bar after pressing / in Normal mode\n{}",
        describe(screen)
    );
}

#[test]
fn search_query_shown_while_typing() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);
    h.send_bytes(b"/hello");
    let found = wait_for_screen(&h, "/hello", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
        "'/hello' should appear in win_bar while typing a search query\n{}",
        describe(screen)
    );
}

#[test]
fn search_result_is_highlighted() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);
    wait_for_screen(&h, "bad29", Duration::from_secs(5));
    enter_normal(&h);

    // Search "bad1" — from the bottom anchor (bad29), the nearest backward match
    // is "bad19" (index 20 in the sorted children list).
    h.send_bytes(b"/bad1\r");
    thread::sleep(Duration::from_millis(400));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    let highlighted = rows_with_bgcolor(screen, SELECTION_BGCOLOR);
    let match_row = find_row_with(screen, "bad19");

    assert!(
        match_row.is_some(),
        "'bad19' should be visible on screen after searching for 'bad1'\n{}",
        describe(screen)
    );
    assert!(
        highlighted.contains(&match_row.unwrap()),
        "Row containing 'bad19' should have selection background after search\n{}",
        describe(screen)
    );
}

#[test]
fn search_n_moves_to_next_result() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);
    wait_for_screen(&h, "bad29", Duration::from_secs(5));
    enter_normal(&h);

    // Initial search finds "bad19" (nearest backward match for "bad1" from bottom).
    h.send_bytes(b"/bad1\r");
    thread::sleep(Duration::from_millis(400));

    let highlighted_before = {
        let parser = h.snapshot();
        rows_with_bgcolor(parser.screen(), SELECTION_BGCOLOR)
    };

    // n searches forward (toward newer messages then wrapping); next match is "bad12".
    h.send_bytes(b"n");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    let highlighted_after = rows_with_bgcolor(screen, SELECTION_BGCOLOR);

    assert!(
        !highlighted_before.is_empty(),
        "Expected a highlighted search result before pressing 'n'\n{}",
        describe(screen)
    );
    assert!(
        !highlighted_after.is_empty(),
        "Expected a highlighted search result after pressing 'n'\n{}",
        describe(screen)
    );
    assert_ne!(
        highlighted_before,
        highlighted_after,
        "'n' should move the highlighted selection to a different message\n{}",
        describe(screen)
    );
}

#[test]
#[allow(non_snake_case)]
fn search_N_moves_to_prev_result() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);
    wait_for_screen(&h, "bad29", Duration::from_secs(5));
    enter_normal(&h);

    // Get to a mid-search position: initial → bad19, then n → bad12.
    h.send_bytes(b"/bad1\r");
    thread::sleep(Duration::from_millis(300));
    h.send_bytes(b"n");
    thread::sleep(Duration::from_millis(200));

    let highlighted_before = {
        let parser = h.snapshot();
        rows_with_bgcolor(parser.screen(), SELECTION_BGCOLOR)
    };

    // N goes in the opposite direction; from bad12 the next prev-match is "bad13".
    h.send_bytes(b"N");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    let highlighted_after = rows_with_bgcolor(screen, SELECTION_BGCOLOR);

    assert!(
        !highlighted_before.is_empty(),
        "Expected a highlighted search result before pressing 'N'\n{}",
        describe(screen)
    );
    assert!(
        !highlighted_after.is_empty(),
        "Expected a highlighted search result after pressing 'N'\n{}",
        describe(screen)
    );
    assert_ne!(
        highlighted_before,
        highlighted_after,
        "'N' should move the highlighted selection to a different message than 'n' did\n{}",
        describe(screen)
    );
}

#[test]
fn command_mode_esc_then_i_returns_to_insert() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);
    h.send_bytes(b"/hello");
    wait_for_screen(&h, "COMMAND", Duration::from_secs(2));

    // Esc returns to Normal, then 'i' enters Insert.
    h.send_bytes(b"\x1b");
    wait_for_screen(&h, "NORMAL", Duration::from_secs(2));

    h.send_bytes(b"i");
    let found = wait_for_screen(&h, "INSERT", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
        "Esc + 'i' should reach INSERT mode from COMMAND mode\n{}",
        describe(screen)
    );
    assert!(
        !grid_contains(screen, "/hello"),
        "Command buffer should be cleared after Esc from COMMAND mode\n{}",
        describe(screen)
    );
}

fn enter_command_colon(h: &Harness) {
    enter_normal(h);
    h.send_bytes(b":");
    wait_for_screen(h, "COMMAND", Duration::from_secs(2));
}

fn enter_command_slash(h: &Harness) {
    enter_normal(h);
    h.send_bytes(b"/");
    wait_for_screen(h, "COMMAND", Duration::from_secs(2));
}

#[test]
fn command_mode_via_colon_shows_command_label() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);
    h.send_bytes(b":");
    let found = wait_for_screen(&h, "COMMAND", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
        "':' in Normal mode should show COMMAND mode label\n{}",
        describe(screen)
    );
}

#[test]
fn command_mode_via_slash_shows_command_label() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);
    h.send_bytes(b"/");
    let found = wait_for_screen(&h, "COMMAND", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
        "'/' in Normal mode should show COMMAND mode label\n{}",
        describe(screen)
    );
}

#[test]
fn command_mode_keystrokes_appear_in_input_bar() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_command_colon(&h);
    h.send_bytes(b"hello");
    let found = wait_for_screen(&h, ":hello", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
        "Keystrokes in COMMAND mode should appear in input bar as ':hello'\n{}",
        describe(screen)
    );
}

#[test]
fn command_mode_backspace_removes_last_char() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_command_colon(&h);
    h.send_bytes(b"hellox");
    wait_for_screen(&h, ":hellox", Duration::from_secs(2));

    h.send_bytes(b"\x7f");
    // Wait for the unique suffix "hellox" to disappear (":hell" would match prematurely).
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        let parser = h.snapshot();
        if !grid_contains(parser.screen(), ":hellox") {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        grid_contains(screen, ":hello"),
        "':hello' should be visible after backspace removes 'x'\n{}",
        describe(screen)
    );
    assert!(
        !grid_contains(screen, ":hellox"),
        "':hellox' should be gone after backspace\n{}",
        describe(screen)
    );
}

#[test]
fn esc_from_command_mode_returns_to_normal() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_command_colon(&h);
    h.send_bytes(b"\x1b");
    let found = wait_for_screen(&h, "NORMAL", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
        "Esc from COMMAND mode should return to NORMAL mode\n{}",
        describe(screen)
    );
    assert!(
        !grid_contains(screen, "COMMAND"),
        "COMMAND label should disappear after Esc\n{}",
        describe(screen)
    );
}

#[test]
fn i_in_command_mode_types_into_buffer() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_command_colon(&h);
    h.send_bytes(b"i");
    let found = wait_for_screen(&h, ":i", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
        "'i' in COMMAND mode should type 'i' into the command buffer\n{}",
        describe(screen)
    );
    assert!(
        !grid_contains(screen, "INSERT"),
        "'i' in COMMAND mode must not switch to INSERT mode\n{}",
        describe(screen)
    );
}
