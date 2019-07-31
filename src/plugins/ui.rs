use futures::Sink;
use futures::unsync::mpsc::SendError;
use std::fmt;
use tokio_xmpp::Packet;
use xmpp_parsers::Jid;

use crate::core::Message;

pub struct UIPlugin {
}

impl UIPlugin {
}

impl super::Plugin for UIPlugin {
    fn new() -> UIPlugin {
        UIPlugin { }
    }

    fn init(&self, _mgr: &super::PluginManager) -> Result<(), ()> {
        Ok(())
    }

    fn on_connect(&self, _sink: &mut dyn Sink<SinkItem=Packet, SinkError=SendError<Packet>>) -> Result<(), ()> {
        Ok(())
    }

    fn on_disconnect(&self) -> Result<(), ()> {
        Ok(())
    }

    fn on_message(&self, message: &mut Message) -> Result<(), ()> {
        match & message.from {
            Jid::Bare(from) => println!("{}: {}", from, message.body),
            Jid::Full(from) => println!("{}: {}", from, message.body),
        }

        Ok(())
    }
}

impl fmt::Display for UIPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Aparté UI")
    }
}
