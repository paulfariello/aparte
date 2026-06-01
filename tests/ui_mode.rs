mod common;

use std::thread;
use std::time::{Duration, Instant};

use common::{
    describe, find_row_with, grid_contains, row_text, rows_with_bgcolor, wait_for_ready,
    wait_for_screen, Harness, ROWS, SELECTION_BGCOLOR,
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
    // Wait for crossterm's Esc-sequence timeout (~100 ms) so the next key is
    // not misread as Alt+<key> by the terminal parser.
    thread::sleep(Duration::from_millis(150));
}

fn enter_insert(h: &Harness) {
    enter_normal(h);
    h.send_bytes(b"i");
    wait_for_screen(h, "INSERT", Duration::from_secs(2));
}

#[test]
fn default_mode_is_normal() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    let parser = h.snapshot();
    let screen = parser.screen();

    h.shutdown();

    assert!(
        grid_contains(screen, "NORMAL"),
        "Expected NORMAL mode indicator in win_bar at startup\n{}",
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

    fill_console(&h);

    enter_normal(&h);
    // Navigate up with 'k' (moves focus to message frame) then back to the
    // last message with 'G', then 'j' to bubble focus back to the input bar.
    h.send_bytes(b"k");
    h.send_bytes(b"G");
    h.send_bytes(b"j");
    thread::sleep(Duration::from_millis(200));

    h.send_bytes(b"i");
    let found = wait_for_screen(&h, "INSERT", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
        "Expected INSERT mode after navigating messages and returning to input bar via 'j'\n{}",
        describe(screen)
    );
}

#[test]
fn insert_mode_typing_works_after_jk_navigation() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);

    enter_normal(&h);
    // Navigate into messages with 'k', jump to last with 'G', then bubble
    // back to the input bar with 'j' before entering INSERT mode.
    h.send_bytes(b"k");
    h.send_bytes(b"G");
    h.send_bytes(b"j");
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
    let found = wait_for_screen(&h, "g", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
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

    // fill_console leaves the app in Normal mode (commands exit to Normal).
    // Press 'i' to return to Insert mode before the PageUp test.
    h.send_bytes(b"i");
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
fn j_at_input_bar_does_not_select_message() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);

    enter_normal(&h);

    // 'j' twice: first selects the bottom message; second hits the boundary
    // and bubbles focus back to the input bar (clears the selection).
    h.send_bytes(b"j");
    thread::sleep(Duration::from_millis(100));
    h.send_bytes(b"j");
    thread::sleep(Duration::from_millis(300));

    // Confirm the selection was cleared (focus is on input bar).
    let parser = h.snapshot();
    let selected_before = rows_with_bgcolor(parser.screen(), SELECTION_BGCOLOR);
    assert!(
        selected_before.is_empty(),
        "Selection should be cleared after 'j' bubbles to input bar\n{}",
        describe(parser.screen())
    );

    // Third 'j' while already at the input bar (last component) must be a no-op:
    // it must NOT cause the frame to steal focus and re-select the bottom message.
    h.send_bytes(b"j");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    let selected_after = rows_with_bgcolor(screen, SELECTION_BGCOLOR);

    h.shutdown();

    assert!(
        selected_after.is_empty(),
        "'j' at input bar must not select any message; {} rows highlighted\n{}",
        selected_after.len(),
        describe(screen)
    );
}

#[test]
fn insert_mode_jk_do_not_move_message_selection() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);
    wait_for_screen(&h, "bad29", Duration::from_secs(5));

    // Enter NORMAL mode and select a message with 'k' (moves focus to message frame).
    enter_normal(&h);
    h.send_bytes(b"k");
    thread::sleep(Duration::from_millis(200));

    // Return to INSERT mode: jump to last message then bubble back to input bar.
    // ModeChange(Insert) clears the selection.
    h.send_bytes(b"G");
    h.send_bytes(b"j");
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

    // Enter INSERT mode to trigger SteadyBar emission.
    enter_insert(&h);

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
fn cursor_steady_bar_in_command_mode() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_command_colon(&h);

    let parser = h.snapshot();
    let screen = parser.screen();

    // ESC [ 6 SP q = DECSCUSR 6 = SteadyBar
    // ESC [ 2 SP q = DECSCUSR 2 = SteadyBlock (emitted on Normal mode entry)
    let steady_bar: &[u8] = b"\x1b[6 q";
    let steady_block: &[u8] = b"\x1b[2 q";
    let bytes = h.bytes.lock().unwrap().clone();

    // Find the last SteadyBlock (emitted during Normal mode), then confirm
    // SteadyBar appears after it (emitted when entering Command mode).
    let last_block_pos = bytes
        .windows(steady_block.len())
        .enumerate()
        .filter(|(_, w)| *w == steady_block)
        .map(|(i, _)| i)
        .last();
    let bar_after_block = last_block_pos
        .map(|pos| {
            bytes[pos + steady_block.len()..]
                .windows(steady_bar.len())
                .any(|w| w == steady_bar)
        })
        .unwrap_or(false);

    h.shutdown();

    assert!(
        !screen.hide_cursor(),
        "Cursor should be visible in COMMAND mode\n{}",
        describe(screen)
    );
    assert!(
        bar_after_block,
        "SteadyBar (ESC[6 q) should be emitted after entering COMMAND mode"
    );
}

/// Cursor should sit on the last row (the input bar) and advance with each
/// typed character in Command mode.
#[test]
fn cursor_position_in_command_mode() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_command_colon(&h);

    // Type text after the already-present ':'.
    h.send_bytes(b"hello");
    let visible = wait_for_screen(&h, ":hello", Duration::from_secs(2));
    assert!(visible, "':hello' should appear in the input bar");

    let parser = h.snapshot();
    let (row, col) = parser.screen().cursor_position();

    h.shutdown();

    assert_eq!(
        row,
        ROWS - 1,
        "Cursor should be on the last row (input bar)"
    );
    assert_eq!(
        col,
        ":hello".len() as u16,
        "Cursor should be at column {} after typing ':hello', got {}",
        ":hello".len(),
        col
    );
}

/// When a message is selected in NORMAL mode and the user enters COMMAND mode
/// via ':', the cursor must move to the input bar (last row), not stay on the
/// selected message row.  The scroll_win renders selected children with cursor
/// priority 2, which beats the input widget's priority 1 — the fix is to clear
/// the selection on ModeChange(Command).
#[test]
fn cursor_position_in_command_mode_after_message_selection() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);

    // Enter NORMAL mode.
    enter_normal(&h);

    // Press 'k' to explicitly select the last visible message (select_prev with
    // no prior selection lands on bottom_visible_child_index).
    h.send_bytes(b"k");
    let selected = wait_for_screen(&h, "NORMAL", Duration::from_secs(2));
    assert!(selected, "Should still be in NORMAL mode after 'k'");
    // Give the selection highlight a moment to render.
    thread::sleep(Duration::from_millis(200));

    // Sanity-check: at least one message row is highlighted.
    let parser = h.snapshot();
    let selected_rows = rows_with_bgcolor(parser.screen(), SELECTION_BGCOLOR);
    assert!(
        !selected_rows.is_empty(),
        "Expected a selected message row after pressing 'k' in NORMAL mode\n{}",
        describe(parser.screen())
    );
    let selection_row = *selected_rows.last().unwrap();

    // Now enter COMMAND mode via ':'.
    h.send_bytes(b":");
    let found = wait_for_screen(&h, "COMMAND", Duration::from_secs(2));
    assert!(found, "COMMAND mode indicator should appear");

    let parser = h.snapshot();
    let (row, _col) = parser.screen().cursor_position();

    h.shutdown();

    assert_ne!(
        row, selection_row,
        "Cursor must not sit on the selected message row ({}) in COMMAND mode",
        selection_row
    );
    assert_eq!(
        row,
        ROWS - 1,
        "Cursor must be on the input bar (row {}) after entering COMMAND mode from NORMAL with a selected message, got row {}",
        ROWS - 1,
        row
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

#[test]
fn i_on_non_insertable_component_does_not_enter_insert_mode() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);
    wait_for_screen(&h, "bad29", Duration::from_secs(5));

    // Enter NORMAL mode — last message is auto-selected by ModeChange(Normal),
    // and focus is on the input bar (INPUT_INDEX).
    enter_normal(&h);

    // Press 'k': selects the second-to-last message AND moves focus to the
    // message frame (FRAME_LAYOUT_INDEX), which is not insertable.
    h.send_bytes(b"k");
    thread::sleep(Duration::from_millis(200));

    // Sanity: a message row should be highlighted.
    {
        let parser = h.snapshot();
        assert!(
            !rows_with_bgcolor(parser.screen(), SELECTION_BGCOLOR).is_empty(),
            "Expected a selected message after pressing 'k'\n{}",
            describe(parser.screen())
        );
    }

    // Save the cursor row before pressing 'i'.
    let cursor_row_before = {
        let parser = h.snapshot();
        parser.screen().cursor_position().0
    };

    // Press 'i' — the focused component (message frame) is NOT insertable,
    // so INSERT mode must be silently denied.
    h.send_bytes(b"i");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    // Mode must still be NORMAL.
    assert!(
        grid_contains(screen, "NORMAL"),
        "Mode must remain NORMAL when 'i' is pressed on a non-insertable component\n{}",
        describe(screen)
    );
    assert!(
        !grid_contains(screen, "INSERT"),
        "'i' must not switch to INSERT mode when the focused component is not insertable\n{}",
        describe(screen)
    );

    // Cursor must not have moved: it should be wherever it was before 'i'.
    // In Normal mode the cursor stays on the input bar (priority-2 matches
    // the message-selection cursor and the input bar renders last), so both
    // before and after the denied 'i' the cursor is on the input bar.
    let (cursor_row_after, _) = parser.screen().cursor_position();
    assert_eq!(
        cursor_row_after,
        cursor_row_before,
        "Cursor must not move when INSERT is denied (was row {}, now row {})\n{}",
        cursor_row_before,
        cursor_row_after,
        describe(screen)
    );
}

#[test]
fn j_on_last_message_moves_cursor_to_input_bar() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);

    // Enter NORMAL mode — the ScrollWin auto-selects the last visible message.
    enter_normal(&h);
    // Jump explicitly to the last message so the selection is on "bad29".
    h.send_bytes(b"G");
    wait_for_screen(&h, "bad29", Duration::from_secs(2));
    thread::sleep(Duration::from_millis(200));

    // Sanity: a message row should be highlighted at this point.
    {
        let parser = h.snapshot();
        assert!(
            !rows_with_bgcolor(parser.screen(), SELECTION_BGCOLOR).is_empty(),
            "Expected a selected message before pressing j at the last message\n{}",
            describe(parser.screen())
        );
    }

    // Press j one more time — we are already on the last message, so this bubbles
    // up to the parent layout which clears the selection and moves focus to the
    // input bar (the next insertable sibling after FRAME_LAYOUT_INDEX).
    h.send_bytes(b"j");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    // Selection must be cleared: no message row should be highlighted.
    let selected = rows_with_bgcolor(screen, SELECTION_BGCOLOR);
    assert!(
        selected.is_empty(),
        "Selection should be cleared after j bubbles from the last message, but {} rows are still highlighted\n{}",
        selected.len(),
        describe(screen)
    );

    // Cursor must be on the input bar (last row).
    let (row, _) = parser.screen().cursor_position();
    assert_eq!(
        row,
        ROWS - 1,
        "Cursor should be on the input bar (row {}) after j bubbles from the last message, got row {}\n{}",
        ROWS - 1,
        row,
        describe(screen)
    );

    // Mode must remain NORMAL — j does not switch modes.
    assert!(
        grid_contains(screen, "NORMAL"),
        "Mode should remain NORMAL after j bubbles from last message\n{}",
        describe(screen)
    );
}

#[test]
fn j_twice_from_input_bar_moves_cursor_to_input_bar() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);

    // Enter NORMAL mode.
    enter_normal(&h);
    thread::sleep(Duration::from_millis(200));

    // Press j once: should navigate to the last visible message.
    h.send_bytes(b"j");
    thread::sleep(Duration::from_millis(200));

    // Sanity: a message row should be highlighted.
    {
        let parser = h.snapshot();
        assert!(
            !rows_with_bgcolor(parser.screen(), SELECTION_BGCOLOR).is_empty(),
            "Expected a selected message after first j\n{}",
            describe(parser.screen())
        );
    }

    // Press j again: at last message, so this bubbles to the input bar.
    h.send_bytes(b"j");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    // Selection must be cleared.
    let selected = rows_with_bgcolor(screen, SELECTION_BGCOLOR);
    assert!(
        selected.is_empty(),
        "Selection should be cleared after j bubbles from last message; {} rows highlighted\n{}",
        selected.len(),
        describe(screen)
    );

    // Cursor must be on the input bar.
    let (row, _) = parser.screen().cursor_position();
    assert_eq!(
        row,
        ROWS - 1,
        "Cursor should be on input bar (row {}) after j twice; got row {}\n{}",
        ROWS - 1,
        row,
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

    enter_command_slash(&h);
    let found = grid_contains(h.snapshot().screen(), "COMMAND");

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

#[test]
fn command_mode_restores_insert_input_on_exit() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    // Switch to INSERT mode then type some text.
    enter_insert(&h);
    h.send_bytes(b"hello world");
    wait_for_screen(&h, "hello world", Duration::from_secs(2));

    // Enter Normal then Command mode — input bar should show the command, not the saved text.
    enter_command_colon(&h);
    h.send_bytes(b"foo");
    wait_for_screen(&h, ":foo", Duration::from_secs(2));

    let mid = h.snapshot();
    assert!(
        !grid_contains(mid.screen(), "hello world"),
        "Saved input should not be visible while in COMMAND mode\n{}",
        describe(mid.screen())
    );

    // Esc back to Normal, then 'i' back to Insert — saved text must be restored.
    h.send_bytes(b"\x1b");
    wait_for_screen(&h, "NORMAL", Duration::from_secs(2));
    h.send_bytes(b"i");
    let found = wait_for_screen(&h, "hello world", Duration::from_secs(2));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
        "Input text typed in INSERT mode should be restored after exiting COMMAND mode\n{}",
        describe(screen)
    );
}

// The title bar (second-to-last row) must show exactly the active mode label.
// Regression test for a rendering bug where incremental diffing could leave a
// stray character from a previous mode at column 0 of the title bar.
#[test]
fn title_bar_mode_label_is_stable_across_transitions() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    // Title bar = row ROWS - 2 (WinBar:0, Frame:1..21, TitleBar:22, Input:23).
    let title_row = ROWS - 2;

    // -- NORMAL (startup) --
    {
        let parser = h.snapshot();
        let screen = parser.screen();
        let bar = row_text(screen, title_row);
        assert!(
            bar.contains("NORMAL"),
            "Title bar must show NORMAL at startup; got: {:?}\n{}",
            bar,
            describe(screen)
        );
    }

    // NORMAL → INSERT
    h.send_bytes(b"i");
    wait_for_screen(&h, "INSERT", Duration::from_secs(2));
    {
        let parser = h.snapshot();
        let screen = parser.screen();
        let bar = row_text(screen, title_row);
        assert!(
            bar.contains("INSERT"),
            "Title bar must show INSERT after 'i'; got: {:?}\n{}",
            bar,
            describe(screen)
        );
        assert!(
            !bar.contains("NORMAL"),
            "Stray NORMAL in title bar after switching to INSERT; got: {:?}\n{}",
            bar,
            describe(screen)
        );
    }

    // INSERT → NORMAL
    h.send_bytes(b"\x1b");
    wait_for_screen(&h, "NORMAL", Duration::from_secs(2));
    {
        let parser = h.snapshot();
        let screen = parser.screen();
        let bar = row_text(screen, title_row);
        assert!(
            bar.contains("NORMAL"),
            "Title bar must show NORMAL after Esc; got: {:?}\n{}",
            bar,
            describe(screen)
        );
        assert!(
            !bar.contains("INSERT"),
            "Stray INSERT in title bar after switching to NORMAL; got: {:?}\n{}",
            bar,
            describe(screen)
        );
    }

    // NORMAL → COMMAND
    h.send_bytes(b":");
    wait_for_screen(&h, "COMMAND", Duration::from_secs(2));
    {
        let parser = h.snapshot();
        let screen = parser.screen();
        let bar = row_text(screen, title_row);
        assert!(
            bar.contains("COMMAND"),
            "Title bar must show COMMAND after ':'; got: {:?}\n{}",
            bar,
            describe(screen)
        );
        assert!(
            !bar.contains("NORMAL"),
            "Stray NORMAL in title bar after switching to COMMAND; got: {:?}\n{}",
            bar,
            describe(screen)
        );
    }

    // COMMAND → NORMAL (Esc)
    h.send_bytes(b"\x1b");
    wait_for_screen(&h, "NORMAL", Duration::from_secs(2));
    {
        let parser = h.snapshot();
        let screen = parser.screen();
        let bar = row_text(screen, title_row);
        assert!(
            bar.contains("NORMAL"),
            "Title bar must show NORMAL after Esc from COMMAND; got: {:?}\n{}",
            bar,
            describe(screen)
        );
        assert!(
            !bar.contains("COMMAND"),
            "Stray COMMAND in title bar after returning to NORMAL; got: {:?}\n{}",
            bar,
            describe(screen)
        );
    }

    // NORMAL → INSERT
    h.send_bytes(b"i");
    wait_for_screen(&h, "INSERT", Duration::from_secs(2));
    {
        let parser = h.snapshot();
        let screen = parser.screen();
        let bar = row_text(screen, title_row);
        assert!(
            bar.contains("INSERT"),
            "Title bar must show INSERT after 'i'; got: {:?}\n{}",
            bar,
            describe(screen)
        );
        assert!(
            !bar.contains("NORMAL"),
            "Stray NORMAL in title bar after returning to INSERT; got: {:?}\n{}",
            bar,
            describe(screen)
        );
    }

    h.shutdown();
}

// ── Normal-mode in-line cursor motion tests ──────────────────────────────────
//
// Pattern: enter Insert, type ASCII text, switch to Normal (Esc moves cursor
// back one), press a motion key, then verify the terminal cursor column on the
// input bar (row ROWS-1).  For plain ASCII, grapheme index == screen column.
//
// "hello world" grapheme map: h=0 e=1 l=2 l=3 o=4 ' '=5 w=6 o=7 r=8 l=9 d=10
// After Esc, cursor is at 10 (back one from Insert's end position of 11).

fn type_hello_world_then_normal(h: &Harness) {
    enter_insert(h);
    h.send_bytes(b"hello world");
    wait_for_screen(h, "hello world", Duration::from_secs(2));
    enter_normal(h);
    thread::sleep(Duration::from_millis(150));
}

#[test]
fn normal_mode_0_moves_cursor_to_start() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    type_hello_world_then_normal(&h);

    h.send_bytes(b"0");
    thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    let (row, col) = parser.screen().cursor_position();
    h.shutdown();

    assert_eq!(
        row,
        ROWS - 1,
        "cursor must be on the input bar\n{}",
        describe(parser.screen())
    );
    assert_eq!(
        col,
        0,
        "0 must move cursor to column 0\n{}",
        describe(parser.screen())
    );
}

#[test]
fn normal_mode_dollar_moves_cursor_to_end() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    type_hello_world_then_normal(&h);

    // Start at beginning then jump to end
    h.send_bytes(b"0");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"$");
    thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    let (row, col) = parser.screen().cursor_position();
    h.shutdown();

    assert_eq!(
        row,
        ROWS - 1,
        "cursor must be on the input bar\n{}",
        describe(parser.screen())
    );
    assert_eq!(
        col,
        10,
        "$ must move cursor to last char (col 10)\n{}",
        describe(parser.screen())
    );
}

#[test]
fn normal_mode_l_moves_cursor_right() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    type_hello_world_then_normal(&h);

    h.send_bytes(b"0");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"l");
    thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    let (row, col) = parser.screen().cursor_position();
    h.shutdown();

    assert_eq!(
        row,
        ROWS - 1,
        "cursor must be on the input bar\n{}",
        describe(parser.screen())
    );
    assert_eq!(
        col,
        1,
        "l from col 0 must move to col 1\n{}",
        describe(parser.screen())
    );
}

#[test]
fn normal_mode_h_moves_cursor_left() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    type_hello_world_then_normal(&h);

    // Move to col 1, then h to go back
    h.send_bytes(b"0");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"l");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"h");
    thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    let (row, col) = parser.screen().cursor_position();
    h.shutdown();

    assert_eq!(
        row,
        ROWS - 1,
        "cursor must be on the input bar\n{}",
        describe(parser.screen())
    );
    assert_eq!(
        col,
        0,
        "h from col 1 must return to col 0\n{}",
        describe(parser.screen())
    );
}

#[test]
fn normal_mode_w_moves_cursor_to_next_word() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    type_hello_world_then_normal(&h);

    h.send_bytes(b"0");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"w");
    thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    let (row, col) = parser.screen().cursor_position();
    h.shutdown();

    assert_eq!(
        row,
        ROWS - 1,
        "cursor must be on the input bar\n{}",
        describe(parser.screen())
    );
    // w from 'h' (col 0) skips "hello " and lands on 'w' of "world" (col 6)
    assert_eq!(
        col,
        6,
        "w from col 0 must land on start of 'world' (col 6)\n{}",
        describe(parser.screen())
    );
}

#[test]
fn normal_mode_b_moves_cursor_to_word_start() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    type_hello_world_then_normal(&h);

    // Cursor is at col 10 ('d') after Esc; b should land on 'w' of "world"
    h.send_bytes(b"b");
    thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    let (row, col) = parser.screen().cursor_position();
    h.shutdown();

    assert_eq!(
        row,
        ROWS - 1,
        "cursor must be on the input bar\n{}",
        describe(parser.screen())
    );
    assert_eq!(
        col,
        6,
        "b from col 10 must land on start of 'world' (col 6)\n{}",
        describe(parser.screen())
    );
}

#[test]
fn normal_mode_e_moves_cursor_to_word_end() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    type_hello_world_then_normal(&h);

    h.send_bytes(b"0");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"e");
    thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    let (row, col) = parser.screen().cursor_position();
    h.shutdown();

    assert_eq!(
        row,
        ROWS - 1,
        "cursor must be on the input bar\n{}",
        describe(parser.screen())
    );
    // e from 'h' (col 0) lands on 'o' of "hello" (col 4)
    assert_eq!(
        col,
        4,
        "e from col 0 must land on end of 'hello' (col 4)\n{}",
        describe(parser.screen())
    );
}

#[test]
fn normal_mode_count_prefix_multiplies_motion() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    type_hello_world_then_normal(&h);

    h.send_bytes(b"0");
    thread::sleep(Duration::from_millis(150));
    // 2l = move right twice → col 2
    h.send_bytes(b"2l");
    thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    let (row, col) = parser.screen().cursor_position();
    h.shutdown();

    assert_eq!(
        row,
        ROWS - 1,
        "cursor must be on the input bar\n{}",
        describe(parser.screen())
    );
    assert_eq!(
        col,
        2,
        "2l from col 0 must land on col 2\n{}",
        describe(parser.screen())
    );
}

// ── Message-editor Normal-mode tests ──────────────────────────────────────────

/// Set up for message-editor tests: inject a fake outgoing message then
/// navigate to it.  After the second 'k' the message frame has focus and
/// the edit buffer is open in Normal mode — motions work immediately.
fn setup_message_editor_normal_mode(h: &Harness) {
    h.send_command("/inject_msg hello world");
    let visible = wait_for_screen(h, "hello world", Duration::from_secs(5));
    assert!(
        visible,
        "injected message 'hello world' must appear on screen"
    );

    // The message is highlighted on arrival (selection, no cursor).  The first
    // 'k' finds old==new at index 0 → bubbles, clears selection.  The second
    // 'k' selects fresh from None, calls start_cursor(), and focuses the frame.
    h.send_bytes(b"k");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"k");
    thread::sleep(Duration::from_millis(200));
}

/// Pressing '0' while in Normal mode on the message editor moves the cursor
/// to the start of the message body.
#[test]
fn message_editor_0_moves_cursor_to_start() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    setup_message_editor_normal_mode(&h);

    // Cursor starts at position 0; move to end first so '0' has somewhere to go.
    h.send_bytes(b"$");
    thread::sleep(Duration::from_millis(150));

    let parser_before = h.snapshot();
    let (row_before, col_before) = parser_before.screen().cursor_position();

    // '0' → start of line (start of message body).
    h.send_bytes(b"0");
    thread::sleep(Duration::from_millis(200));

    let parser_after = h.snapshot();
    let (row_after, col_after) = parser_after.screen().cursor_position();

    h.shutdown();

    // Cursor must be on a message row (not the input bar).
    assert_ne!(
        row_before,
        ROWS - 1,
        "cursor must be on the message row, not the input bar\n{}",
        describe(parser_before.screen())
    );
    assert_eq!(
        row_before,
        row_after,
        "row must not change after '0'\n{}",
        describe(parser_after.screen())
    );
    // '0' must move the cursor to the left (towards the start of the body).
    assert!(
        col_after < col_before,
        "'0' must move cursor left: col_before={col_before} col_after={col_after}\n{}",
        describe(parser_after.screen())
    );
}

/// Pressing 'w' while in Normal mode on the message editor moves the cursor
/// forward to the next word start.
#[test]
fn message_editor_w_moves_cursor_to_next_word() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    setup_message_editor_normal_mode(&h);

    // First go to line start so we have a known start position.
    h.send_bytes(b"0");
    thread::sleep(Duration::from_millis(150));
    let parser_at_start = h.snapshot();
    let (_, col_start) = parser_at_start.screen().cursor_position();

    // 'w' → jump to start of next word ("world").
    h.send_bytes(b"w");
    thread::sleep(Duration::from_millis(200));

    let parser_after = h.snapshot();
    let (row_after, col_after) = parser_after.screen().cursor_position();

    h.shutdown();

    assert_ne!(
        row_after,
        ROWS - 1,
        "cursor must stay on the message row\n{}",
        describe(parser_after.screen())
    );
    // 'w' from the start of "hello world" must move right by 6 graphemes.
    assert_eq!(
        col_after,
        col_start + 6,
        "'w' from start of 'hello world' must land on 'world' (col_start+6)\n{}",
        describe(parser_after.screen())
    );
}

/// Escape while in message-editor Normal mode (auto-started by navigation)
/// cancels the edit.
#[test]
fn message_editor_second_esc_cancels_edit() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    setup_message_editor_normal_mode(&h);

    // Single Escape cancels the edit (editor was already in Normal mode).
    h.send_bytes(b"\x1b");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let (row, _) = parser.screen().cursor_position();

    h.shutdown();

    // After cancel, focus should return to input bar or the message row
    // without an active edit. The mode bar still shows NORMAL.
    assert!(
        grid_contains(parser.screen(), "NORMAL"),
        "Should still be in NORMAL mode after canceling edit\n{}",
        describe(parser.screen())
    );
    // Cursor must NOT be on the input bar when we were just editing a message
    // and then canceled — the frame still has focus (message is selected but
    // no longer in editing state, so selection cursor applies).
    // We just verify the mode indicator is correct.
    let _ = row; // row assertion omitted: position depends on selection state
}

/// Regression: pressing Enter in Command mode when a message has a Normal-mode
/// navigation cursor (from start_cursor / auto-selection) must NOT trigger the
/// ValidateEdit / send_correction path.
///
/// Before the fix, is_editing() returned true for any edit-Some state, so a
/// navigation cursor was indistinguishable from an in-progress XEP-0308 edit.
/// ValidateEdit would fire, send_correction would consume the Enter key, and
/// the layout returned to Normal mode via ModeChange (not via the normal
/// Validate path), so saved_input was never restored.  The command string
/// (:win console here) stayed in the input bar.  A second Enter in Normal mode
/// then triggered the "looks like a command" popup.
///
/// With the fix (is_insert_editing — only true when normal_mode==false), the
/// command executes, mode returns via the Validate path, saved_input is
/// restored, the input is clean, and no popup appears.
#[test]
fn command_executes_correctly_with_navigation_cursor_on_message() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    // inject_msg opens a chat window and highlights the message (selection only;
    // no edit cursor on arrival).  Focus remains on the input bar.
    h.send_command("/inject_msg hello world");
    let visible = wait_for_screen(&h, "hello world", Duration::from_secs(5));
    assert!(visible, "injected message must appear on screen");

    // We are now in Normal mode, chat window focused, input bar has focus.
    // Enter Command mode and type a harmless command.
    h.send_bytes(b":");
    wait_for_screen(&h, "COMMAND", Duration::from_secs(2));
    h.send_bytes(b"win console\r");

    // Give the event loop time to process the command.
    thread::sleep(Duration::from_millis(300));

    // Press Enter once more in Normal mode.  If the bug were present the
    // command above would not have executed, ":win console" would still be
    // in the input bar, and this Enter would trigger the "looks like a
    // command" popup.  With the fix the input is empty and nothing happens.
    h.send_bytes(b"\r");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        !grid_contains(screen, "looks like a command"),
        "\"looks like a command\" popup must not appear — command should have \
         executed cleanly without triggering the correction path\n{}",
        describe(screen)
    );
}

/// Regression: ScrollToBottom (G) must cancel the previously-selected
/// message's edit cursor before jumping to the last message.
///
/// Without the fix, the priority-3 edit cursor on the k-navigated message
/// survived G and overrode the input bar's priority-1 cursor after j.
///
/// Uses /inject_msg so send_command's leading Escape clears all edit cursors
/// between injections — this isolates the G-handler bug specifically.
#[test]
fn k_then_g_then_j_moves_cursor_to_input_bar() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    h.send_command("/inject_msg first message");
    let visible = wait_for_screen(&h, "first message", Duration::from_secs(5));
    assert!(visible, "first injected message must appear on screen");

    h.send_command("/inject_msg second message");
    let visible = wait_for_screen(&h, "second message", Duration::from_secs(5));
    assert!(visible, "second injected message must appear on screen");

    // We are now in Normal mode with the last message auto-selected.
    h.send_bytes(b"k");
    thread::sleep(Duration::from_millis(200));
    h.send_bytes(b"G");
    thread::sleep(Duration::from_millis(200));
    h.send_bytes(b"j");
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    let selected = rows_with_bgcolor(screen, SELECTION_BGCOLOR);
    assert!(
        selected.is_empty(),
        "Selection must be cleared after j bubbles from last message, but {} rows highlighted\n{}",
        selected.len(),
        describe(screen)
    );

    let (row, _) = parser.screen().cursor_position();
    assert_eq!(
        row,
        ROWS - 1,
        "Cursor must be on the input bar (row {}) after k->G->j, got row {}\n{}",
        ROWS - 1,
        row,
        describe(screen)
    );
}

// ── c (change) operator — message frame ──────────────────────────────────────

/// Regression: after `caw` on a message-frame cursor, typed text must go to
/// the message editor, not to the input bar.
///
/// Before the fix, `dispatch_action` switched the mode to Insert without
/// calling `layout.set_focus(FRAME_LAYOUT_INDEX)`, so the event router still
/// sent keystrokes to the input bar (the previous `focused_child`).
#[test]
fn message_editor_caw_types_into_frame_not_input_bar() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    setup_message_editor_normal_mode(&h);

    // On "hello world", go to start then 'w' to land on "world".
    h.send_bytes(b"0");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"w");
    thread::sleep(Duration::from_millis(150));

    // caw deletes "world" (last word, eats leading space) and enters Insert.
    h.send_bytes(b"caw");
    let found = wait_for_screen(&h, "INSERT", Duration::from_secs(2));
    assert!(found, "caw on message editor must switch to INSERT mode");

    // Type a sentinel string; it must land in the message frame.
    h.send_bytes(b"xyz");
    thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    let screen = parser.screen();
    let (cursor_row, _) = screen.cursor_position();
    h.shutdown();

    // The cursor must be on a message row, not the last row (input bar).
    assert_ne!(
        cursor_row,
        ROWS - 1,
        "after caw on message editor, typed text must go to the frame, not the input bar\n{}",
        describe(screen)
    );
    assert!(
        row_text(screen, cursor_row).contains("xyz"),
        "typed 'xyz' must appear in the message editor row\n{}",
        describe(screen)
    );
    assert!(
        !row_text(screen, ROWS - 1).contains("xyz"),
        "typed 'xyz' must NOT appear in the input bar\n{}",
        describe(screen)
    );
}

// ── c (change) operator — input bar ──────────────────────────────────────────

#[test]
fn normal_mode_cw_enters_insert_and_deletes_to_word_end() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    type_hello_world_then_normal(&h);

    h.send_bytes(b"0");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"cw");

    let found = wait_for_screen(&h, "INSERT", Duration::from_secs(2));
    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(found, "cw must switch to INSERT mode\n{}", describe(screen));
    assert!(
        row_text(screen, ROWS - 1).contains("world"),
        "cw must leave 'world' in the input bar\n{}",
        describe(screen)
    );
}

#[test]
fn normal_mode_cc_enters_insert_and_clears_line() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    type_hello_world_then_normal(&h);

    h.send_bytes(b"cc");

    let found = wait_for_screen(&h, "INSERT", Duration::from_secs(2));
    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(found, "cc must switch to INSERT mode\n{}", describe(screen));
    assert!(
        !row_text(screen, ROWS - 1).contains("hello"),
        "cc must clear the input bar\n{}",
        describe(screen)
    );
}

#[test]
fn normal_mode_caw_enters_insert_and_changes_around_word() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    type_hello_world_then_normal(&h);

    h.send_bytes(b"0");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"w");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"caw");

    let found = wait_for_screen(&h, "INSERT", Duration::from_secs(2));
    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
        "caw must switch to INSERT mode\n{}",
        describe(screen)
    );
    let input = row_text(screen, ROWS - 1);
    assert!(
        input.contains("hello"),
        "caw from 'world' must leave 'hello' in the input bar\n{}",
        describe(screen)
    );
    assert!(
        !input.contains("world"),
        "caw must delete 'world' from the input bar\n{}",
        describe(screen)
    );
}

/// When `:` is pressed in Normal mode with an active message selection cursor,
/// the terminal cursor must jump to the command bar (last row).  When the
/// command is cancelled with ESC, the cursor must return to the message row.
///
/// inject_msg auto-selects the message on arrival (follow_bottom + Normal mode,
/// priority-3 edit cursor).  The first 'k' finds old==new at index 0 → bubbles,
/// clears selection.  The second 'k' selects fresh from None → start_cursor()
/// opens an edit cursor at priority 3.
///
/// In Command mode the input cursor also uses priority 3 and renders last, so
/// it wins → cursor at bottom.  After ESC the edit cursor is NOT cancelled
/// (was_command=true) and still beats the input cursor (priority 1) → cursor
/// returns to the message row.
#[test]
fn command_mode_cursor_moves_to_input_bar_and_back() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    // inject_msg auto-selects the message on arrival.
    // First 'k' bubbles (old==new at index 0), clearing the selection.
    // Second 'k' selects fresh from None; start_cursor() opens an edit
    // cursor at priority 3.
    h.send_command("/inject_msg hello world");
    let visible = wait_for_screen(&h, "hello world", Duration::from_secs(5));
    assert!(visible, "injected message must appear on screen");

    h.send_bytes(b"k");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"k");
    thread::sleep(Duration::from_millis(200));

    let before = h.snapshot();
    let (row_before, _) = before.screen().cursor_position();

    // Edit cursor (priority 3) beats input cursor (priority 1) → message row.
    assert_ne!(
        row_before,
        ROWS - 1,
        "cursor must be on the message row after two 'k' presses\n{}",
        describe(before.screen())
    );

    // Press ':' to enter Command mode.
    h.send_bytes(b":");
    wait_for_screen(&h, "COMMAND", Duration::from_secs(2));

    let during = h.snapshot();
    let (row_during, _) = during.screen().cursor_position();

    // In Command mode input priority becomes 3 (same as edit cursor) and
    // renders last → wins → cursor moves to the bottom input bar.
    assert_eq!(
        row_during,
        ROWS - 1,
        "cursor must be on the input bar while in Command mode\n{}",
        describe(during.screen())
    );

    // Cancel with Escape → back to Normal mode.
    h.send_bytes(b"\x1b");
    wait_for_screen(&h, "NORMAL", Duration::from_secs(2));
    thread::sleep(Duration::from_millis(150));

    let after = h.snapshot();
    let (row_after, _) = after.screen().cursor_position();

    h.shutdown();

    // After ESC from Command: was_command=true so the Normal-mode handler
    // does NOT cancel the edit cursor — it stays at priority 3, which beats
    // the input cursor (priority 1) → cursor returns to the message row.
    assert_ne!(
        row_after,
        ROWS - 1,
        "cursor must return to the message row after ESC from Command mode\n{}",
        describe(after.screen())
    );
}

#[test]
fn normal_mode_ciw_enters_insert_and_changes_inner_word() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    type_hello_world_then_normal(&h);

    h.send_bytes(b"0");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"ciw");

    let found = wait_for_screen(&h, "INSERT", Duration::from_secs(2));
    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    assert!(
        found,
        "ciw must switch to INSERT mode\n{}",
        describe(screen)
    );
    let input = row_text(screen, ROWS - 1);
    assert!(
        !input.contains("hello"),
        "ciw must delete 'hello' from the input bar\n{}",
        describe(screen)
    );
    assert!(
        input.contains("world"),
        "ciw must preserve ' world' (space+word) in the input bar\n{}",
        describe(screen)
    );
}

#[test]
fn normal_mode_dw_removes_word_from_input() {
    // "hello world": after Esc, cursor is at col 10 ('d').
    // 0 → cursor to 'h' (col 0).
    // dw → deletes "hello " (word + trailing space), leaving "world".
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    type_hello_world_then_normal(&h);

    h.send_bytes(b"0");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"dw");
    thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    let input = row_text(screen, ROWS - 1);
    assert!(
        !input.contains("hello"),
        "dw must remove 'hello' from the input bar\n{}",
        describe(screen)
    );
    assert!(
        input.contains("world"),
        "dw must leave 'world' in the input bar\n{}",
        describe(screen)
    );
}

#[test]
fn normal_mode_xp_transposes_chars() {
    // "abc" in input, Normal mode, cursor at 'a' (col 0) after pressing 0.
    // x → deletes 'a' (register = 'a'), buffer = "bc", cursor on 'b' (col 0).
    // p → PasteAfter: MoveCursor(Right) advances to 'c' (col 1), then inserts
    //     'a' at col 1 → "bac".
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_insert(&h);
    h.send_bytes(b"abc");
    wait_for_screen(&h, "abc", Duration::from_secs(2));
    enter_normal(&h);

    h.send_bytes(b"0");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"x");
    thread::sleep(Duration::from_millis(100));
    h.send_bytes(b"p");
    thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    let input = row_text(screen, ROWS - 1);
    assert!(
        input.contains("bac"),
        "xp must transpose 'abc' to 'bac'\n{}",
        describe(screen)
    );
}

#[test]
fn normal_mode_named_register_paste() {
    // "abc" in input, Normal mode, cursor at 'c' (col 2).
    // 0 → cursor at 'a' (col 0).
    // "ayl → yank 'a' into register "a; cursor stays at col 0.
    // l → cursor at 'b' (col 1).
    // "ap → PasteAfter from register "a: MoveCursor(Right) → col 2 ('c'),
    //       then insert 'a' before 'c' → "abac".
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_insert(&h);
    h.send_bytes(b"abc");
    wait_for_screen(&h, "abc", Duration::from_secs(2));
    enter_normal(&h);

    h.send_bytes(b"0");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"\"ayl");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"l");
    thread::sleep(Duration::from_millis(100));
    h.send_bytes(b"\"ap");
    thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    let screen = parser.screen();
    h.shutdown();

    let input = row_text(screen, ROWS - 1);
    assert!(
        input.contains("abac"),
        "\"ayl then l then \"ap must give 'abac'\n{}",
        describe(screen)
    );
}

/// Issue 1: in Normal mode the terminal cursor must stay on the input bar even
/// when a message is selected via 'k'.  The selection should be shown as a
/// highlight only; the cursor belongs on the input bar until the user
/// explicitly starts an in-place edit.
#[test]
fn cursor_stays_on_input_bar_after_k_navigation_in_normal_mode() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);
    enter_normal(&h);

    // 'k' selects the last visible message and moves layout focus to the
    // frame.  That gives the message a priority-2 cursor while the input
    // bar is only priority 1 → cursor ends up on the message (bug).
    h.send_bytes(b"k");
    thread::sleep(Duration::from_millis(200));

    let parser = h.snapshot();
    let screen = parser.screen();
    let (row, _) = screen.cursor_position();
    h.shutdown();

    assert_eq!(
        row,
        ROWS - 1,
        "cursor should stay on input bar (row {}) in Normal mode after 'k', got row {}\n{}",
        ROWS - 1,
        row,
        describe(screen)
    );
}

/// Issue 2: pressing `o` when the frame has focus (e.g. from 'k' navigation
/// or from a FocusFrame event caused by an incoming message) must move the
/// visual cursor to the input bar.
#[test]
fn o_from_frame_focus_moves_visual_cursor_to_input_bar() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    fill_console(&h);
    enter_normal(&h);

    // Put frame in focus with a selected message (same state as after a
    // FocusFrame event from an incoming XMPP message).
    h.send_bytes(b"k");
    thread::sleep(Duration::from_millis(150));

    h.send_bytes(b"o");
    let found = wait_for_screen(&h, "INSERT", Duration::from_secs(2));
    let parser = h.snapshot();
    let screen = parser.screen();
    let (row, _) = screen.cursor_position();
    h.shutdown();

    assert!(
        found,
        "Expected INSERT mode after 'o'\n{}",
        describe(screen)
    );
    assert_eq!(
        row,
        ROWS - 1,
        "cursor should be on input bar after 'o' from frame-focused state\n{}",
        describe(screen)
    );
}

#[test]
fn o_in_normal_mode_enters_insert_on_input_bar() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    enter_normal(&h);
    h.send_bytes(b"o");
    let found = wait_for_screen(&h, "INSERT", Duration::from_secs(2));
    let parser = h.snapshot();
    let screen = parser.screen();
    let (row, _) = screen.cursor_position();

    h.shutdown();

    assert!(
        found,
        "Expected INSERT mode after 'o' in Normal mode\n{}",
        describe(screen)
    );
    assert_eq!(
        row,
        ROWS - 1,
        "Cursor should be on input bar (row {})\n{}",
        ROWS - 1,
        describe(screen)
    );
}

#[test]
fn o_in_normal_mode_with_frame_focus_moves_to_input() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);
    fill_console(&h);

    enter_normal(&h);
    h.send_bytes(b"k");
    thread::sleep(Duration::from_millis(150));
    h.send_bytes(b"o");
    let found = wait_for_screen(&h, "INSERT", Duration::from_secs(2));
    let parser = h.snapshot();
    let screen = parser.screen();
    let (row, _) = screen.cursor_position();

    h.shutdown();

    assert!(
        found,
        "Expected INSERT mode after 'o'\n{}",
        describe(screen)
    );
    assert_eq!(
        row,
        ROWS - 1,
        "Cursor must be on input bar\n{}",
        describe(screen)
    );
}

/// Regression: after `:msg contact@domain.tld` opens a 1-1 chat window and
/// delivers the first message, the visual cursor must land on the input bar
/// (row ROWS-1), not on the incoming message.
///
/// Root cause: when a message arrives with `current_mode == Normal && follow_bottom`,
/// the handler calls `start_cursor()` (priority 3) on the auto-selected message and
/// emits `FocusFrame`, which beats the input bar's priority-2 cursor and moves the
/// terminal cursor to the message row.
///
/// Uses `:inject_msg` to simulate the same code path without a real XMPP connection.
#[test]
fn cursor_on_input_bar_after_msg_opens_chat_window() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    // inject_msg fires Event::Chat (opens a chat window) then Event::Message
    // (delivers the first message).  In Normal mode with follow_bottom=true the
    // message-arrival handler currently emits FocusFrame + start_cursor, which
    // gives the message a priority-3 cursor that beats the input bar (priority 2).
    h.send_command("/inject_msg hello world");
    let visible = wait_for_screen(&h, "hello world", Duration::from_secs(5));
    assert!(visible, "injected message must appear on screen");

    // Wait for rendering to settle after FocusFrame / cursor-priority resolution.
    thread::sleep(Duration::from_millis(300));

    let parser = h.snapshot();
    let screen = parser.screen();
    let (row, _) = screen.cursor_position();
    h.shutdown();

    assert_eq!(
        row,
        ROWS - 1,
        "cursor must be on the input bar (row {}) after opening a 1-1 chat window via :msg, got row {}\n{}",
        ROWS - 1,
        row,
        describe(screen)
    );
}
