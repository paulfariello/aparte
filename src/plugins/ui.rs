use bytes::BytesMut;
use futures::Sink;
use std::fmt;
use std::io::Error as IoError;
use std::io::{Write, stdout, Stdout};
use std::string::FromUtf8Error;
use termion::screen::AlternateScreen;
use tokio_codec::{Decoder};
use tokio_xmpp;
use xmpp_parsers::Jid;

use crate::core::Message;
use crate::core::Command;

pub struct UIPlugin {
    screen: AlternateScreen<Stdout>,
}

impl UIPlugin {
}

impl super::Plugin for UIPlugin {
    fn new() -> UIPlugin {
        UIPlugin { screen: AlternateScreen::from(stdout()) }
    }

    fn init(&mut self, _mgr: &super::PluginManager) -> Result<(), ()> {
        const VERSION: &'static str = env!("CARGO_PKG_VERSION");

        write!(self.screen, "Welcome to Aparté {}\n", VERSION);

        Ok(())
    }

    fn on_connect(&mut self, _sink: &mut dyn Sink<SinkItem=tokio_xmpp::Packet, SinkError=tokio_xmpp::Error>) -> Result<(), ()> {
        Ok(())
    }

    fn on_disconnect(&mut self) -> Result<(), ()> {
        Ok(())
    }

    fn on_message(&mut self, message: &mut Message) -> Result<(), ()> {
        match & message.from {
            Jid::Bare(from) => write!(self.screen, "{}: {}\n", from, message.body),
            Jid::Full(from) => write!(self.screen, "{}: {}\n", from, message.body),
        };

        self.screen.flush();

        Ok(())
    }
}

impl fmt::Display for UIPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Aparté UI")
    }
}

pub struct CommandCodec {
}

impl CommandCodec {
    pub fn new() -> Self {
        Self { }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    Io(IoError),
    Utf8(FromUtf8Error),
}

impl Decoder for CommandCodec {
    type Item = Command;
    type Error = Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Command>, Error> {
        if buf.is_empty() {
            return Ok(None)
        } else {
            let vec = buf.take().to_vec();
            let string = match String::from_utf8(vec) {
                Ok(string) => string,
                Err(err) => return Err(Error::Utf8(err)),
            };

            let command = Command::new(string);
            debug!("Received command {:?}", command);

            Ok(Some(command))
        }
    }
}
