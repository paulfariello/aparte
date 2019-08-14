use futures::Sink;
use std::fmt;
use std::rc::Rc;
use tokio_xmpp;
use uuid::Uuid;
use xmpp_parsers::Element;
use xmpp_parsers::carbons;
use xmpp_parsers::iq::Iq;

use crate::core::{Plugin, Aparte, Message};
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

    fn init(&mut self, aparte: &Aparte) -> Result<(), ()> {
        let mut disco = aparte.get_plugin_mut::<disco::Disco>().unwrap();
        disco.add_feature("urn:xmpp:carbons:2")
    }

    fn on_connect(&mut self, aparte: Rc<Aparte>) {
        aparte.send(self.enable());
    }

    fn on_disconnect(&mut self, _aparte: Rc<Aparte>) {
    }

    fn on_message(&mut self, _aparte: Rc<Aparte>, _message: &mut Message) {
    }
}

impl fmt::Display for CarbonsPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0280: Message Carbons")
    }
}
