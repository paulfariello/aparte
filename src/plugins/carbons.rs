use futures::Sink;
use futures::unsync::mpsc::SendError;
use std::fmt;
use tokio_xmpp::Packet;
use uuid::Uuid;
use xmpp_parsers::Element;
use xmpp_parsers::carbons;
use xmpp_parsers::iq::Iq;

use crate::plugins::disco;

#[allow(non_camel_case_types)]
pub struct CarbonsPlugin {
}

impl CarbonsPlugin {
    fn enable(&self) -> Element {
        let id = Uuid::new_v4().to_hyphenated().to_string();
        let iq = Iq::from_set(id, carbons::Enable);
        iq.into()
    }
}

impl super::Plugin for CarbonsPlugin {
    fn new() -> CarbonsPlugin {
        CarbonsPlugin { }
    }

    fn init(&self, mgr: &super::PluginManager) -> Result<(), ()> {
        let mut disco = mgr.get::<disco::Disco>().unwrap();
        disco.add_feature("urn:xmpp:carbons:2")
    }

    fn on_connect(&self, sink: &mut dyn Sink<SinkItem=Packet, SinkError=SendError<Packet>>) -> Result<(), ()> {
        let iq = self.enable();

        debug!("SEND: {}", String::from(&iq));
        sink.start_send(Packet::Stanza(iq)).unwrap();

        Ok(())
    }

    fn on_disconnect(&self) -> Result<(), ()> {
        Ok(())
    }
}

impl fmt::Display for CarbonsPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0280: Message Carbons")
    }
}
