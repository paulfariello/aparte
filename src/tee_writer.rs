/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::fs::File;
use std::io::{self, Write};

use termion::raw::RawTerminal;
use termion::screen::AlternateScreen;

pub struct TeeWriter<W: Write> {
    inner: W,
    record_file: File,
    pub events_file: File,
    bytes_written: u64,
}

impl<W: Write> TeeWriter<W> {
    pub fn new(inner: W, record_file: File, events_file: File) -> Self {
        Self {
            inner,
            record_file,
            events_file,
            bytes_written: 0,
        }
    }

    pub fn annotate(&mut self, tag: &str) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let _ = writeln!(self.events_file, "{} {} {}", ts, self.bytes_written, tag);
    }
}

impl<W: Write> Write for TeeWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = self.record_file.write_all(buf);
        let n = self.inner.write(buf)?;
        self.bytes_written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = self.record_file.flush();
        self.annotate("FRAME");
        let _ = self.events_file.flush();
        self.inner.flush()
    }
}

pub type PlainScreen = AlternateScreen<RawTerminal<std::io::Stdout>>;
pub type RecordingScreen = TeeWriter<AlternateScreen<RawTerminal<std::io::Stdout>>>;

pub enum Screen {
    Plain(PlainScreen),
    Recording(RecordingScreen),
}

impl Screen {
    pub fn annotate(&mut self, tag: &str) {
        if let Screen::Recording(tee) = self {
            tee.annotate(tag);
        }
    }

    pub fn dump_buffer(&mut self, content: &str, frame: u64) {
        if let Screen::Recording(tee) = self {
            let _ = writeln!(tee.events_file, "--- BUFFER_DUMP frame {} ---", frame);
            let _ = tee.events_file.write_all(content.as_bytes());
        }
    }
}

impl Write for Screen {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Screen::Plain(s) => s.write(buf),
            Screen::Recording(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Screen::Plain(s) => s.flush(),
            Screen::Recording(s) => s.flush(),
        }
    }
}
