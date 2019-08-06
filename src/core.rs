use xmpp_parsers::Jid;

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
