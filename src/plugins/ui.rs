use bytes::{BytesMut, BufMut};
use futures::Sink;
use std::fmt;
use std::io::{Error as IoError, ErrorKind};
use std::io::{Write, Stdout};
use termion::raw::{IntoRawMode, RawTerminal};
use termion::screen::AlternateScreen;
use tokio::codec::FramedRead;
use tokio_codec::{Decoder};
use tokio_xmpp;
use xmpp_parsers::Jid;

use crate::core::Message;
use crate::core::{Command, CommandError};

pub struct UIPlugin {
    screen: AlternateScreen<RawTerminal<Stdout>>,
}

pub type CommandStream = FramedRead<tokio::reactor::PollEvented2<tokio_file_unix::File<std::fs::File>>, KeyCodec>;

impl UIPlugin {
    pub fn command_stream(&self) -> CommandStream {
        let file = tokio_file_unix::raw_stdin().unwrap();
        let file = tokio_file_unix::File::new_nb(file).unwrap();
        let file = file.into_io(&tokio::reactor::Handle::default()).unwrap();

        FramedRead::new(file, KeyCodec::new())
    }
}

impl super::Plugin for UIPlugin {
    fn new() -> UIPlugin {
        let stdout = std::io::stdout().into_raw_mode().unwrap();

        UIPlugin {
            screen: AlternateScreen::from(stdout)
        }
    }

    fn init(&mut self, _mgr: &super::PluginManager) -> Result<(), ()> {
        const VERSION: &'static str = env!("CARGO_PKG_VERSION");

        write!(self.screen, "Welcome to Aparté {}\n", VERSION);

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

pub struct KeyCodec {
    buf: BytesMut,
    queue: Vec<Command>,
}

impl KeyCodec {
    pub fn new() -> Self {
        Self {
            buf: BytesMut::new(),
            queue: Vec::new(),
        }
    }
}

impl Decoder for KeyCodec {
    type Item = Command;
    type Error = CommandError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let buf = buf.take();

        for i in 0..buf.len() {
            match buf[i] {
                0x20..=0x7E => {
                    self.buf.put(buf[i]);
                },
                0x0D => {
                    let vec = self.buf.take().to_vec();
                    let string = match String::from_utf8(vec) {
                        Ok(string) => string,
                        Err(err) => return Err(CommandError::Utf8(err)),
                    };

                    self.queue.push(Command::new(string));
                    self.buf.clear()
                },
                0x03 => return Err(CommandError::Io(IoError::new(ErrorKind::BrokenPipe, "ctrl+c"))),
                _ => { println!("{:?}", buf[i]); },
            }
        }

        match self.queue.pop() {
            Some(command) => Ok(Some(command)),
            None => Ok(None),
        }
    }
}
