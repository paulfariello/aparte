use futures::Sink;
use std::any::{Any, TypeId};
use std::cell::{RefCell, RefMut};
use std::collections::HashMap;
use std::fmt;
use tokio_xmpp;

use crate::core::Message;

pub mod disco;
pub mod carbons;
pub mod ui;

pub trait Plugin: fmt::Display {
    fn new() -> Self where Self: Sized;
    fn init(&mut self, mgr: &PluginManager) -> Result<(), ()>;
    fn on_connect(&mut self, sink: &mut dyn Sink<SinkItem=tokio_xmpp::Packet, SinkError=tokio_xmpp::Error>);
    fn on_disconnect(&mut self);
    fn on_message(&mut self, message: &mut Message);
}

pub trait AnyPlugin: Any + Plugin {
    fn as_any(&mut self) -> &mut dyn Any;
    fn as_plugin(&mut self) -> &mut dyn Plugin;
}

impl<T> AnyPlugin for T where T: Any + Plugin {
    fn as_any(&mut self) -> &mut dyn Any {
        self
    }

    fn as_plugin(&mut self) -> &mut dyn Plugin {
        self
    }
}


pub struct PluginManager {
    plugins: HashMap<TypeId, RefCell<Box<dyn AnyPlugin>>>,
}

impl PluginManager {
    pub fn new() -> PluginManager {
        PluginManager { plugins: HashMap::new() }
    }

    pub fn add<T: 'static>(&mut self, plugin: Box<dyn AnyPlugin>) -> Result<(), ()> {
        info!("Add plugin `{}`", plugin);
        self.plugins.insert(TypeId::of::<T>(), RefCell::new(plugin));
        Ok(())
    }

    pub fn get<T: 'static>(&self) -> Option<RefMut<T>> {
        let rc = match self.plugins.get(&TypeId::of::<T>()) {
            Some(rc) => rc,
            None => return None,
        };

        let any_plugin = rc.borrow_mut();
        /* Calling unwrap here on purpose as we expect panic if plugin is not of the right type */
        Some(RefMut::map(any_plugin, |p| p.as_any().downcast_mut::<T>().unwrap()))
    }

    pub fn init(&mut self) -> Result<(), ()> {
        for (_, plugin) in self.plugins.iter() {
            if let Err(err) = plugin.borrow_mut().as_plugin().init(&self) {
                return Err(err);
            }
        }

        Ok(())
    }

    pub fn on_connect(&self, sink: &mut dyn Sink<SinkItem=tokio_xmpp::Packet, SinkError=tokio_xmpp::Error>) {
        for (_, plugin) in self.plugins.iter() {
            plugin.borrow_mut().as_plugin().on_connect(sink);
        }
    }

    pub fn on_message(&self, message: &mut Message) {
        for (_, plugin) in self.plugins.iter() {
            plugin.borrow_mut().as_plugin().on_message(message);
        }
    }
}
