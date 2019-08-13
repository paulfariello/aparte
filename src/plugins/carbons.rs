use futures::Sink;
use std::fmt;
use tokio_xmpp;
use uuid::Uuid;
use xmpp_parsers::Element;
use xmpp_parsers::carbons;
use xmpp_parsers::iq::Iq;

use crate::core::{Plugin, PluginManager, Message};
use crate::plugins::disco;

pub struct CarbonsPlugin {
}

impl CarbonsPlugin {
    fn enable(&self) -> Element {
        let id = Uuid::new_v4().to_hyphenated().to_string();
        let iq = Iq::from_set(id, carbons::Enable);
        iq.into()
    }
}

impl Plugin for CarbonsPlugin {
    fn new() -> CarbonsPlugin {
        CarbonsPlugin { }
    }

    fn init(&mut self, mgr: &PluginManager) -> Result<(), ()> {
        let mut disco = mgr.get_mut::<disco::Disco>().unwrap();
        disco.add_feature("urn:xmpp:carbons:2")
    }

    fn on_connect(&mut self, sink: &mut dyn Sink<SinkItem=tokio_xmpp::Packet, SinkError=tokio_xmpp::Error>) {
        let iq = self.enable();

        trace!("SEND: {}", String::from(&iq));
        sink.start_send(tokio_xmpp::Packet::Stanza(iq)).unwrap();
    }

    fn on_disconnect(&mut self) {
    }

    fn on_message(&mut self, _message: &mut Message) {
    }
}

impl fmt::Display for CarbonsPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0280: Message Carbons")
    }
}
