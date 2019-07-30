use xmpp_parsers::Jid;

pub struct Message {
    pub from: Jid,
    pub body: String,
}

impl Message {
    pub fn new(from: Jid, body: String) -> Message {
        Message { from: from, body: body }
    }
}
