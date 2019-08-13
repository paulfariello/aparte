use futures::Sink;
use shell_words::ParseError;
use std::any::{Any, TypeId};
use std::cell::{RefCell, RefMut, Ref};
use std::collections::HashMap;
use std::fmt;
use std::io::Error as IoError;
use std::rc::Rc;
use std::string::FromUtf8Error;
use tokio_xmpp;
use xmpp_parsers::Jid;

#[derive(Debug, Clone)]
pub enum MessageType {
    IN,
    OUT,
    LOG
}

#[derive(Debug, Clone)]
pub struct Message {
    pub kind: MessageType,
    pub from: Option<Jid>,
    pub body: String,
}

impl Message {
    pub fn incoming(from: Jid, body: String) -> Self {
        Message { kind: MessageType::IN, from: Some(from), body: body }
    }

    pub fn outgoing(body: String) -> Self {
        Message { kind: MessageType::OUT, from: None, body: body }
    }

    pub fn log(msg: String) -> Self {
        Message { kind: MessageType::LOG, from: None, body: msg }
    }
}

pub enum CommandOrMessage {
    Command(Command),
    Message(Message),
}

#[derive(Debug, Clone)]
pub struct Command {
    pub name: String,
    pub args: Vec<String>,
}

impl Command {
    pub fn new(command: String, args: Vec<String>) -> Self {
        Self {
            name: command,
            args: args,
        }
    }
}

#[derive(Debug, Error)]
pub enum CommandError {
    Io(IoError),
    Utf8(FromUtf8Error),
    Parse(ParseError),
}

pub trait CommandParser {
    fn parse(&self, plugins: Rc<PluginManager>, command: &Command) -> Result<(), ()>;
}

pub struct CommandManager<'a> {
    pub commands: HashMap<String, &'a Fn(Rc<PluginManager>, &Command) -> Result<(), ()>>,
}

impl<'a> CommandManager<'a> {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new()
        }
    }

    pub fn add_command(&mut self, name: &str, command: &'a Fn(Rc<PluginManager>, &Command) -> Result<(), ()>) {
        self.commands.insert(name.to_string(), command);
    }

    pub fn parse(&self, plugins: Rc<PluginManager>, command: &Command) -> Result<(), ()> {
        match self.commands.get(&command.name) {
            Some(parser) => parser(plugins, command),
            None => Err(()),
        }
    }
}

pub trait Plugin: fmt::Display {
    fn new() -> Self where Self: Sized;
    fn init(&mut self, mgr: &PluginManager) -> Result<(), ()>;
    fn on_connect(&mut self, sink: &mut dyn Sink<SinkItem=tokio_xmpp::Packet, SinkError=tokio_xmpp::Error>);
    fn on_disconnect(&mut self);
    fn on_message(&mut self, message: &mut Message);
}

pub trait AnyPlugin: Any + Plugin {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn as_plugin(&mut self) -> &mut dyn Plugin;
}

impl<T> AnyPlugin for T where T: Any + Plugin {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
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

    pub fn get<T: 'static>(&self) -> Option<Ref<T>> {
        let rc = match self.plugins.get(&TypeId::of::<T>()) {
            Some(rc) => rc,
            None => return None,
        };

        let any_plugin = rc.borrow();
        /* Calling unwrap here on purpose as we expect panic if plugin is not of the right type */
        Some(Ref::map(any_plugin, |p| p.as_any().downcast_ref::<T>().unwrap()))
    }

    pub fn get_mut<T: 'static>(&self) -> Option<RefMut<T>> {
        let rc = match self.plugins.get(&TypeId::of::<T>()) {
            Some(rc) => rc,
            None => return None,
        };

        let any_plugin = rc.borrow_mut();
        /* Calling unwrap here on purpose as we expect panic if plugin is not of the right type */
        Some(RefMut::map(any_plugin, |p| p.as_any_mut().downcast_mut::<T>().unwrap()))
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

    pub fn log(&self, message: String) {
        let mut message = Message::log(message);
        self.on_message(&mut message);
    }
}
