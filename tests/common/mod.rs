#![allow(dead_code)]

pub mod omemo;
pub mod xmpp_fixture;

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

pub const ROWS: u16 = 24;
pub const COLS: u16 = 80;

pub struct Harness {
    pub bytes: Arc<Mutex<Vec<u8>>>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// Set to true by the reader thread when the child's PTY reaches EOF (process exited).
    pub exited: Arc<AtomicBool>,
    pub _tmp: tempfile::TempDir,
}

impl Harness {
    pub fn spawn(config_toml: &str, extra_env: &[(&str, &str)]) -> Self {
        let tmp = tempfile::Builder::new()
            .prefix("aparte-integ")
            .tempdir()
            .expect("tmpdir");

        let config_dir = tmp.path().join("config");
        let data_dir = match std::env::var_os("APARTE_TEST_DATA_DIR") {
            Some(p) => std::path::PathBuf::from(p),
            None => tmp.path().join("data"),
        };
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(config_dir.join("config.toml"), config_toml).unwrap();

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
        cmd.env(
            "RUST_LOG",
            std::env::var("APARTE_TEST_LOG").unwrap_or_else(|_| "error".into()),
        );
        cmd.env("TERM", "xterm-256color");
        cmd.env("HOME", tmp.path());
        for (k, v) in extra_env {
            cmd.env(k, v);
        }

        let child = pair.slave.spawn_command(cmd).expect("spawn aparte");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("clone reader");
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(pair.master.take_writer().expect("take writer")));

        let bytes = Arc::new(Mutex::new(Vec::<u8>::new()));
        let exited = Arc::new(AtomicBool::new(false));
        {
            let bytes = Arc::clone(&bytes);
            let writer = Arc::clone(&writer);
            let exited = Arc::clone(&exited);
            thread::spawn(move || {
                let mut shadow = vt100::Parser::new(ROWS, COLS, 0);
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => {
                            exited.store(true, Ordering::Relaxed);
                            break;
                        }
                        Ok(n) => {
                            let chunk = &buf[..n];
                            bytes.lock().unwrap().extend_from_slice(chunk);
                            shadow.process(chunk);
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
                    }
                }
            });
        }

        Harness {
            bytes,
            child,
            writer,
            exited,
            _tmp: tmp,
        }
    }

    pub fn snapshot(&self) -> vt100::Parser {
        let data = self.bytes.lock().unwrap().clone();
        let mut parser = vt100::Parser::new(ROWS, COLS, 0);
        parser.process(&data);
        parser
    }

    pub fn send_command(&self, cmd: &str) {
        // Send bare Esc first, then wait for Normal mode before sending the rest.
        // This ensures crossterm processes Esc alone (not as an Alt+<x> escape prefix).
        // When already in Normal mode, wait_for_screen returns immediately; the handlers
        // for 'i' and ':' use `..` patterns that ignore Alt modifiers, so even if \x1b<char>
        // is parsed as Alt+<char>, the correct mode transition still fires.
        self.send_bytes(b"\x1b");
        wait_for_screen(self, "NORMAL", Duration::from_secs(5));
        if cmd.starts_with('/') && !cmd.starts_with("/me") {
            // ':' → Command mode, type without '/', Enter → Normal
            let mut w = self.writer.lock().unwrap();
            let _ = w.write_all(b":");
            let _ = w.write_all(cmd[1..].as_bytes());
            let _ = w.write_all(b"\r");
            let _ = w.flush();
        } else {
            // 'i' → Insert mode, type verbatim (/me or plain text), Enter
            let mut w = self.writer.lock().unwrap();
            let _ = w.write_all(b"i");
            let _ = w.write_all(cmd.as_bytes());
            let _ = w.write_all(b"\r");
            let _ = w.flush();
        }
    }

    pub fn send_bytes(&self, bytes: &[u8]) {
        let mut w = self.writer.lock().unwrap();
        let _ = w.write_all(bytes);
        let _ = w.flush();
    }

    pub fn shutdown(mut self) {
        {
            // Send bare Esc, then wait for crossterm's escape timeout to fire before ':quit'.
            let mut w = self.writer.lock().unwrap();
            let _ = w.write_all(b"\x1b");
            let _ = w.flush();
        }
        thread::sleep(Duration::from_millis(150));
        {
            let mut w = self.writer.lock().unwrap();
            let _ = w.write_all(b":quit\r");
            let _ = w.flush();
        }
        let deadline = Instant::now() + Duration::from_millis(800);
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

pub fn count_dsr_queries(bytes: &[u8]) -> usize {
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

pub fn row_text(screen: &vt100::Screen, r: u16) -> String {
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

pub fn grid_contains(screen: &vt100::Screen, needle: &str) -> bool {
    (0..ROWS).any(|r| row_text(screen, r).contains(needle))
}

pub fn describe(screen: &vt100::Screen) -> String {
    let mut s = String::new();
    for r in 0..ROWS {
        s.push_str(&format!("{:02}: {:?}\n", r, row_text(screen, r)));
    }
    s
}

/// Default selection highlight color (matches config default `selected_message`).
pub const SELECTION_BGCOLOR: vt100::Color = vt100::Color::Rgb(49, 50, 68);

pub fn find_row_with(screen: &vt100::Screen, needle: &str) -> Option<u16> {
    (0..ROWS).find(|&r| row_text(screen, r).contains(needle))
}

pub fn row_has_bgcolor(screen: &vt100::Screen, r: u16, color: vt100::Color) -> bool {
    (0..COLS).any(|c| {
        screen
            .cell(r, c)
            .map(|cell| cell.bgcolor() == color)
            .unwrap_or(false)
    })
}

/// Returns the set of rows that contain at least one cell with `color` as background.
pub fn rows_with_bgcolor(screen: &vt100::Screen, color: vt100::Color) -> Vec<u16> {
    (0..ROWS)
        .filter(|&r| row_has_bgcolor(screen, r, color))
        .collect()
}

/// Poll until `needle` appears on screen or the timeout elapses.
/// Returns `false` on timeout. Panics immediately if the child process exits unexpectedly.
pub fn wait_for_screen(h: &Harness, needle: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if h.exited.load(Ordering::Relaxed) {
            let parser = h.snapshot();
            panic!(
                "Child process exited unexpectedly while waiting for {:?}\n{}",
                needle,
                describe(parser.screen())
            );
        }
        let parser = h.snapshot();
        if grid_contains(parser.screen(), needle) {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Poll until the app reaches NORMAL mode (ready to accept input).
/// Fails immediately if the process exits, or after 60 s on a loaded machine.
pub fn wait_for_ready(h: &Harness) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if h.exited.load(Ordering::Relaxed) {
            let parser = h.snapshot();
            panic!(
                "App exited before reaching NORMAL mode — likely a startup crash\n{}",
                describe(parser.screen())
            );
        }
        let parser = h.snapshot();
        if grid_contains(parser.screen(), "NORMAL") {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "App did not reach NORMAL mode within 60s — startup hung\n{}",
            describe(parser.screen())
        );
        thread::sleep(Duration::from_millis(100));
    }
}
