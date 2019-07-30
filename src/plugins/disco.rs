use std::fmt;
use futures::Sink;
use futures::unsync::mpsc::SendError;
use tokio_xmpp::Packet;

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

impl<'a> super::Plugin for Disco<'a> {
    fn new() -> Disco<'a> {
        Disco { features: Vec::new() }
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
}

impl<'a> fmt::Display for Disco<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0030: Service Discovery")
    }
}
