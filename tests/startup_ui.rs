//! PTY-based integration test for aparte's first on-screen frame.
//!
//! Spawns the aparte binary inside a pseudoterminal with a throwaway
//! XDG_CONFIG_HOME / XDG_DATA_HOME pointing at a tmpdir with an empty
//! config (so no XMPP connection is attempted). Bytes written by
//! aparte to the PTY are captured and replayed into a `vt100::Parser`
//! at specific instants, so we can inspect what a real terminal would
//! show at each moment.
//!
//! The bug under test: after commit 794ca7b the first visible frame
//! is blank / partial — the rainbow WELCOME banner and the bottom
//! input line only appear after the first real event batch completes.

mod common;

use std::time::Duration;

use common::{describe, wait_for_screen, Harness};

const FIRST_FRAME_MS: u64 = 1500;

#[test]
fn first_frame_contains_welcome_and_input_line() {
    let h = Harness::spawn("", &[]);

    // "First frame" — what the user sees shortly after launch. We allow
    // 1500ms of slack: the UI renderer task ticks every 16ms, so by now
    // there has been ample opportunity to flush a frame containing the
    // welcome banner that Aparte::start() logs. The original regression
    // (widechar cursor check adding ~2.4 s/frame) would still fail here.
    // Poll rather than sleep so we catch the frame as soon as it renders.
    let banner_present = wait_for_screen(&h, "\u{258C}", Duration::from_millis(FIRST_FRAME_MS));
    let version_present = wait_for_screen(&h, "Version", Duration::from_millis(FIRST_FRAME_MS));
    let parser = h.snapshot();
    let screen = parser.screen();

    // Settled control: verify banner eventually appears (confirms test infra works).
    let settled_ok = banner_present || wait_for_screen(&h, "\u{258C}", Duration::from_secs(5));
    let settled_parser = h.snapshot();
    let settled = settled_parser.screen();

    h.shutdown();

    // Control: if the settled frame is missing the banner, the test
    // infra is broken rather than the bug under study.
    assert!(
        settled_ok,
        "settled frame (t=~6s) missing WELCOME banner — test infra or binary broken\n--- first frame ---\n{}\n--- settled ---\n{}",
        describe(screen),
        describe(settled)
    );

    assert!(
        banner_present,
        "first frame (t={}ms) missing WELCOME banner glyph — regression reproduced\n--- first frame ---\n{}\n--- settled ---\n{}",
        FIRST_FRAME_MS,
        describe(screen),
        describe(settled)
    );
    assert!(
        version_present,
        "first frame (t={}ms) missing Version line — regression reproduced\n--- first frame ---\n{}\n--- settled ---\n{}",
        FIRST_FRAME_MS,
        describe(screen),
        describe(settled)
    );
}
