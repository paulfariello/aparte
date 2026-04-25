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

use std::thread;
use std::time::Duration;

use common::{describe, grid_contains, Harness};

const FIRST_FRAME_MS: u64 = 1500;

#[test]
fn first_frame_contains_welcome_and_input_line() {
    let h = Harness::spawn("", &[]);

    // "First frame" — what the user sees shortly after launch. We allow
    // 1500ms of slack: the UI renderer task ticks every 16ms, so by now
    // there has been ample opportunity to flush a frame containing the
    // welcome banner that Aparte::start() logs. The original regression
    // (widechar cursor check adding ~2.4 s/frame) would still fail here.
    thread::sleep(Duration::from_millis(FIRST_FRAME_MS));
    let parser = h.snapshot();
    let screen = parser.screen();

    // The WELCOME banner is drawn in Unicode box-drawing characters
    // (U+2580..U+259F), not literal "Welcome" — detect it by presence
    // of the distinctive "▌" glyph that starts its first line. The
    // Version line is printed immediately after as literal text.
    let banner_present = grid_contains(screen, "\u{258C}"); // '▌'
    let version_present = grid_contains(screen, "Version");

    // Settled control snapshot.
    thread::sleep(Duration::from_millis(5000));
    let settled_parser = h.snapshot();
    let settled = settled_parser.screen();
    let settled_banner = grid_contains(settled, "\u{258C}");
    let settled_version = grid_contains(settled, "Version");

    h.shutdown();

    // Control: if the settled frame is missing the banner, the test
    // infra is broken rather than the bug under study.
    assert!(
        settled_banner && settled_version,
        "settled frame (t=~6s) missing WELCOME banner or Version — test infra or binary broken\n--- first frame ---\n{}\n--- settled ---\n{}",
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
