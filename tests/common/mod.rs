#![allow(dead_code)]

pub mod omemo;
pub mod xmpp_fixture;

use std::io::{Read, Write};
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
        {
            let bytes = Arc::clone(&bytes);
            let writer = Arc::clone(&writer);
            thread::spawn(move || {
                let mut shadow = vt100::Parser::new(ROWS, COLS, 0);
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
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

    pub fn snapshot(&self) -> vt100::Parser {
        let data = self.bytes.lock().unwrap().clone();
        let mut parser = vt100::Parser::new(ROWS, COLS, 0);
        parser.process(&data);
        parser
    }

    pub fn send_command(&self, cmd: &str) {
        let mut w = self.writer.lock().unwrap();
        let _ = w.write_all(cmd.as_bytes());
        let _ = w.write_all(b"\n");
        let _ = w.flush();
    }

    pub fn send_bytes(&self, bytes: &[u8]) {
        let mut w = self.writer.lock().unwrap();
        let _ = w.write_all(bytes);
        let _ = w.flush();
    }

    pub fn shutdown(mut self) {
        {
            let mut w = self.writer.lock().unwrap();
            let _ = w.write_all(b"/quit\n");
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

pub fn wait_for_screen(h: &Harness, needle: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let parser = h.snapshot();
        if grid_contains(parser.screen(), needle) {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}
