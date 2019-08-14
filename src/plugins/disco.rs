use futures::Sink;
use std::fmt;
use std::rc::Rc;
use tokio_xmpp;

use crate::core::{Plugin, Aparte, Message};

#[allow(non_camel_case_types)]
pub struct Disco<'a> {
    features: Vec<&'a str>,
}

impl<'a> Disco<'a> {
    pub fn add_feature(&mut self, feature: &'a str) -> Result<(), ()> {
        debug!("Adding `{}` feature", feature);
        self.features.push(feature);

        Ok(())
    }
}

impl<'a> Plugin for Disco<'a> {
    fn new() -> Disco<'a> {
        Disco { features: Vec::new() }
    }

    fn init(&mut self, _aparte: &Aparte) -> Result<(), ()> {
        Ok(())
    }

    fn on_connect(&mut self, _aparte: Rc<Aparte>) {
    }

    fn on_disconnect(&mut self, _aparte: Rc<Aparte>) {
    }

    fn on_message(&mut self, _aparte: Rc<Aparte>, _message: &mut Message) {
    }
}

impl<'a> fmt::Display for Disco<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0030: Service Discovery")
    }
}
