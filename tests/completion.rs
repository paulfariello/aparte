mod common;

use std::time::Duration;

use common::{describe, wait_for_ready, wait_for_screen, Harness};

/// Pressing Tab after a partial `/win` argument completes it to the full window name.
#[test]
fn tab_completes_window_name() {
    let h = Harness::spawn("", &[]);
    wait_for_ready(&h);

    // Type a partial window name without submitting
    h.send_bytes(b"/win con");
    let typed = wait_for_screen(&h, "/win con", Duration::from_secs(2));
    assert!(typed, "partial command not visible in input bar");

    // Press Tab — should complete "con" → "console"
    h.send_bytes(b"\t");

    let found = wait_for_screen(&h, "/win console", Duration::from_secs(2));
    let parser = h.snapshot();
    h.shutdown();

    assert!(
        found,
        "Tab did not complete /win con → /win console\n{}",
        describe(parser.screen())
    );
}
