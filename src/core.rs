use std::io::Error as IoError;
use std::string::FromUtf8Error;
use xmpp_parsers::Jid;

#[derive(Debug, Clone)]
pub enum MessageType {
    IN,
    OUT,
    LOG
}

#[derive(Debug, Clone)]
pub struct Message {
    pub kind: MessageType,
    pub from: Option<Jid>,
    pub body: String,
}

impl Message {
    pub fn incoming(from: Jid, body: String) -> Self {
        Message { kind: MessageType::IN, from: Some(from), body: body }
    }

    pub fn outgoing(body: String) -> Self {
        Message { kind: MessageType::OUT, from: None, body: body }
    }

    pub fn log(msg: String) -> Self {
        Message { kind: MessageType::LOG, from: None, body: msg }
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
