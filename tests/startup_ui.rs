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

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

const ROWS: u16 = 24;
const COLS: u16 = 80;
const FIRST_FRAME_MS: u64 = 1500;

struct Harness {
    bytes: Arc<Mutex<Vec<u8>>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    _tmp: tempfile::TempDir,
}

impl Harness {
    fn spawn() -> Self {
        // Use a tmpdir for both config and data so the test is hermetic.
        let tmp = tempfile::Builder::new()
            .prefix("aparte-startup-ui")
            .tempdir()
            .expect("tmpdir");
        // Allow overriding the data dir (via env) so the aparte.log file
        // survives test teardown while debugging.
        let config_dir = tmp.path().join("config");
        let data_dir = match std::env::var_os("APARTE_TEST_DATA_DIR") {
            Some(p) => std::path::PathBuf::from(p),
            None => tmp.path().join("data"),
        };
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        // Empty config -> no accounts, no autoconnect.
        std::fs::write(config_dir.join("config.toml"), "").unwrap();

        let pty = NativePtySystem::default();
        let pair = pty
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty");

        let exe = env!("CARGO_BIN_EXE_aparte");
        let mut cmd = CommandBuilder::new(exe);
        cmd.arg("--config");
        cmd.arg(config_dir.join("config.toml"));
        cmd.arg("--shared");
        cmd.arg(&data_dir);
        // Minimise log noise and ensure deterministic terminal behaviour.
        cmd.env(
            "RUST_LOG",
            std::env::var("APARTE_TEST_LOG").unwrap_or_else(|_| "error".into()),
        );
        cmd.env("TERM", "xterm-256color");
        cmd.env("HOME", tmp.path());

        let child = pair.slave.spawn_command(cmd).expect("spawn aparte");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("clone reader");
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(pair.master.take_writer().expect("take writer")));

        let bytes = Arc::new(Mutex::new(Vec::<u8>::new()));
        {
            let bytes = Arc::clone(&bytes);
            let writer = Arc::clone(&writer);
            thread::spawn(move || {
                // Track live cursor position via a shadow vt100 parser so
                // we can answer cursor-position queries (DSR 6n) from the
                // child. In debug builds, `terminus::rendering::render_line`
                // issues these queries after every line to verify its
                // widechar width accounting. If nobody replies, the call
                // blocks for ~2s per line, which slows the UI to a crawl
                // in this test harness.
                let mut shadow = vt100::Parser::new(ROWS, COLS, 0);
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let chunk = &buf[..n];
                            bytes.lock().unwrap().extend_from_slice(chunk);
                            shadow.process(chunk);
                            // Count DSR (ESC[6n) queries in the chunk;
                            // reply once per query with a CPR report.
                            let queries = count_dsr_queries(chunk);
                            if queries > 0 {
                                let (row, col) = shadow.screen().cursor_position();
                                let reply = format!("\x1b[{};{}R", row + 1, col + 1);
                                let mut w = writer.lock().unwrap();
                                for _ in 0..queries {
                                    let _ = w.write_all(reply.as_bytes());
                                }
                                let _ = w.flush();
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        Harness {
            bytes,
            child,
            writer,
            _tmp: tmp,
        }
    }

    /// Build a vt100 screen from everything captured so far.
    fn snapshot(&self) -> vt100::Parser {
        let data = self.bytes.lock().unwrap().clone();
        let mut parser = vt100::Parser::new(ROWS, COLS, 0);
        parser.process(&data);
        parser
    }

    fn shutdown(mut self) {
        // Try a graceful quit first.
        {
            let mut w = self.writer.lock().unwrap();
            let _ = w.write_all(b"/quit\n");
            let _ = w.flush();
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Count occurrences of the cursor-position-request DSR sequence (ESC [ 6 n)
/// in a byte stream. Does not handle the sequence being split across reads
/// — acceptable because termion writes it whole via a single write!().
fn count_dsr_queries(bytes: &[u8]) -> usize {
    const NEEDLE: &[u8] = b"\x1b[6n";
    let mut count = 0;
    let mut i = 0;
    while i + NEEDLE.len() <= bytes.len() {
        if &bytes[i..i + NEEDLE.len()] == NEEDLE {
            count += 1;
            i += NEEDLE.len();
        } else {
            i += 1;
        }
    }
    count
}

fn row_text(screen: &vt100::Screen, r: u16) -> String {
    let mut s = String::new();
    for c in 0..COLS {
        if let Some(cell) = screen.cell(r, c) {
            let contents = cell.contents();
            if !contents.is_empty() {
                s.push_str(&contents);
            }
        }
    }
    s
}

fn grid_contains(screen: &vt100::Screen, needle: &str) -> bool {
    (0..ROWS).any(|r| row_text(screen, r).contains(needle))
}

fn describe(screen: &vt100::Screen) -> String {
    let mut s = String::new();
    for r in 0..ROWS {
        s.push_str(&format!("{:02}: {:?}\n", r, row_text(screen, r)));
    }
    s
}

#[test]
fn first_frame_contains_welcome_and_input_line() {
    let h = Harness::spawn();

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
