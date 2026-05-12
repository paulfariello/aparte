mod common;

use std::time::Duration;

use common::{describe, wait_for_ready, wait_for_screen, Harness};

/// Pressing Tab after a partial `:win` argument completes it to the full window name.
#[test]
fn tab_completes_window_name() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    // Esc → Normal mode (wait for confirmation so crossterm processes Esc alone)
    h.send_bytes(b"\x1b");
    let in_normal = wait_for_screen(&h, "NORMAL", Duration::from_secs(2));
    assert!(in_normal, "Expected NORMAL mode after Esc");

    // ':' → Command mode, type partial window name without submitting
    h.send_bytes(b":win con");
    let typed = wait_for_screen(&h, ":win con", Duration::from_secs(2));
    assert!(typed, "partial command not visible in input bar");

    // Press Tab — should complete "con" → "console"
    h.send_bytes(b"\t");

    let found = wait_for_screen(&h, ":win console", Duration::from_secs(2));
    let parser = h.snapshot();
    h.shutdown();

    assert!(
        found,
        "Tab did not complete :win con → :win console\n{}",
        describe(parser.screen())
    );
}

/// Arg completion must work even when the command is given as a unique prefix.
/// `:h w<Tab>` should complete to `:h win` (help arg, with `h` resolving to `help`).
#[test]
fn tab_completes_arg_for_abbreviated_command() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    h.send_bytes(b"\x1b");
    let in_normal = wait_for_screen(&h, "NORMAL", Duration::from_secs(2));
    assert!(in_normal, "Expected NORMAL mode after Esc");

    h.send_bytes(b":h w");
    let typed = wait_for_screen(&h, ":h w", Duration::from_secs(2));
    assert!(typed, "partial command not visible in input bar");

    h.send_bytes(b"\t");

    let found = wait_for_screen(&h, ":h win", Duration::from_secs(2));
    let parser = h.snapshot();
    h.shutdown();

    assert!(
        found,
        "Tab did not complete :h w → :h win (abbreviated command arg completion)\n{}",
        describe(parser.screen())
    );
}
