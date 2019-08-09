use std::io::Error as IoError;
use std::string::FromUtf8Error;
use xmpp_parsers::Jid;

#[derive(Debug, Clone)]
pub struct Message {
    pub from: Jid,
    pub body: String,
}

impl Message {
    pub fn new(from: Jid, body: String) -> Self {
        Message { from: from, body: body }
    }
}

#[derive(Debug)]
pub struct Command {
    pub command: String,
}

impl Command {
    pub fn new(command: String) -> Self {
        Self { command: command }
    }
}

#[derive(Debug, Error)]
pub enum CommandError {
    Io(IoError),
    Utf8(FromUtf8Error),
}
