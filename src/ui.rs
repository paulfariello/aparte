use xmpp_parsers::Jid;

use crate::core::Message;

pub fn receive_message(message: Message) {
    match message.from {
        Jid::Bare(from) => println!("{}: {}", from, message.body),
        Jid::Full(from) => println!("{}: {}", from, message.body),
    }
}
