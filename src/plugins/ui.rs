use bytes::{BytesMut, BufMut};
use futures::Sink;
use std::fmt;
use std::io::{Error as IoError, ErrorKind};
use std::io::{Write, Stdout};
use std::rc::Rc;
use termion::raw::{IntoRawMode, RawTerminal};
use termion::screen::AlternateScreen;
use termion::cursor::DetectCursorPos;
use tokio::codec::FramedRead;
use tokio_codec::{Decoder};
use tokio_xmpp;
use xmpp_parsers::Jid;

use crate::core::Message;
use crate::core::{Command, CommandError};

pub type CommandStream<'a> = FramedRead<tokio::reactor::PollEvented2<tokio_file_unix::File<std::fs::File>>, KeyCodec<'a>>;

pub struct UIPlugin {
    screen: AlternateScreen<RawTerminal<Stdout>>,
}

impl UIPlugin {
    pub fn command_stream(&mut self) -> CommandStream {
        let file = tokio_file_unix::raw_stdin().unwrap();
        let file = tokio_file_unix::File::new_nb(file).unwrap();
        let file = file.into_io(&tokio::reactor::Handle::default()).unwrap();

        FramedRead::new(file, KeyCodec::new(self))
    }
}

impl super::Plugin for UIPlugin {
    fn new() -> Self {
        let stdout = std::io::stdout().into_raw_mode().unwrap();

        Self {
            screen: AlternateScreen::from(stdout),
        }
    }

    fn init(&mut self, _mgr: &super::PluginManager) -> Result<(), ()> {
        const VERSION: &'static str = env!("CARGO_PKG_VERSION");

        write!(self.screen, "{}", termion::clear::All);
        write!(self.screen, "{}", termion::cursor::Goto(1,1));
        write!(self.screen, "Welcome to Aparté {}\n", VERSION);
        let (_x, y) = termion::terminal_size().unwrap();
        write!(self.screen, "{}", termion::cursor::Goto(0, y));
        self.screen.flush();

        Ok(())
    }

    fn on_connect(&mut self, _sink: &mut dyn Sink<SinkItem=tokio_xmpp::Packet, SinkError=tokio_xmpp::Error>) {
    }

    fn on_disconnect(&mut self) {
    }

    fn on_message(&mut self, message: &mut Message) {
        let result = match & message.from {
            Jid::Bare(from) => write!(self.screen, "{}: {}\n", from, message.body),
            Jid::Full(from) => write!(self.screen, "{}: {}\n", from, message.body),
        };


        self.screen.flush();
    }
}

impl fmt::Display for UIPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Aparté UI")
    }
}

pub struct KeyCodec<'a> {
    buf: String,
    queue: Vec<Command>,
    ui: &'a mut UIPlugin,
}

impl<'a> KeyCodec<'a> {
    pub fn new(ui: &'a mut UIPlugin) -> Self {
        Self {
            buf: String::new(),
            queue: Vec::new(),
            ui: ui,
        }
    }
}

impl<'a> Decoder for KeyCodec<'a> {
    type Item = Command;
    type Error = CommandError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let copy = buf.clone();
        let string = match std::str::from_utf8(&copy) {
            Ok(string) => {
                buf.clear();
                string
            },
            Err(err) => {
                let index = err.valid_up_to();
                buf.advance(index);
                std::str::from_utf8(&copy[..index]).unwrap()
            }
        };

        let mut chars = string.chars();
        while let Some(c) = chars.next() {
            if !c.is_control() {
                self.buf.push(c);
                write!(self.ui.screen, "{}", c);
                self.ui.screen.flush();
            } else {
                match c {
                    '\r' => {
                        self.queue.push(Command::new(self.buf.clone()));
                        self.buf.clear();
                    },
                    '\x7f' => {
                        write!(self.ui.screen, "{} {}", termion::cursor::Left(1), termion::cursor::Left(1));
                        self.buf.pop();
                        self.ui.screen.flush();
                    },
                    '\x03' => return Err(CommandError::Io(IoError::new(ErrorKind::BrokenPipe, "ctrl+c"))),
                    '\x1b' => {
                        match chars.next() {
                            Some('[') => {
                                match chars.next() {
                                    Some('C') => {
                                        let (x, _y) = self.ui.screen.cursor_pos().unwrap();
                                        if x as usize <= self.buf.len() {
                                            write!(self.ui.screen, "{}", termion::cursor::Right(1));
                                            self.ui.screen.flush();
                                        }
                                    },
                                    Some('D') => {
                                        write!(self.ui.screen, "{}", termion::cursor::Left(1));
                                        self.ui.screen.flush();
                                    },
                                    Some(_) => {}
                                    None => {},
                                }
                            },
                            Some(_) => {},
                            None => {},
                        }
                    },
                    _ => {
                        write!(self.ui.screen, "^{:x}", c as u8);
                        self.ui.screen.flush();
                    },
                }
            }
        }

        match self.queue.pop() {
            Some(command) => Ok(Some(command)),
            None => Ok(None),
        }
    }
}
