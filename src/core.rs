/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use futures::Sink;
use futures::unsync::mpsc::UnboundedSender;
use std::any::{Any, TypeId};
use std::cell::{RefCell, RefMut, Ref};
use std::collections::HashMap;
use std::fmt;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::PathBuf;
use std::rc::Rc;
use std::convert::TryFrom;
use tokio_xmpp::Packet;
use xmpp_parsers::{Element, FullJid, BareJid, presence, iq};
use xmpp_parsers;
use termion::event::Key;

use crate::{contact, conversation};
use crate::message::Message;
use crate::command::{Command, CommandParser};
use crate::config::Config;
use crate::terminus::ViewTrait;

pub enum Event {
    Connected(FullJid),
    #[allow(dead_code)]
    Disconnected(FullJid),
    Command(Command),
    CommandError(String),
    SendMessage(Message),
    Message(Message),
    Chat(BareJid),
    Join(FullJid),
    Iq(iq::Iq),
    Presence(presence::Presence),
    ReadPassword(Command),
    Win(String),
    Contact(contact::Contact),
    ContactUpdate(contact::Contact),
    Occupant{conversation: BareJid, occupant: conversation::Occupant},
    Signal(i32),
    LoadHistory(BareJid),
    Quit,
    Key(Key),
    Validate(Rc<RefCell<Option<(String, bool)>>>),
    Complete(Rc<RefCell<Option<(String, usize, bool)>>>),
    Completed(String),
    AddWindow(String, Option<Box<dyn ViewTrait<Event>>>),
    ChangeWindow(String),
}

pub trait Plugin: fmt::Display {
    fn new() -> Self where Self: Sized;
    fn init(&mut self, mgr: &Aparte) -> Result<(), ()>;
    fn on_event(&mut self, aparte: Rc<Aparte>, event: &Event);
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

pub struct Connection {
    pub sink: UnboundedSender<Packet>,
    pub account: FullJid,
}

pub struct Aparte {
    pub commands: HashMap<String, CommandParser>,
    plugins: HashMap<TypeId, RefCell<Box<dyn AnyPlugin>>>,
    connections: RefCell<HashMap<String, Connection>>,
    current_connection: RefCell<Option<String>>,
    event_lock: RefCell<()>,
    event_queue: RefCell<Vec<Event>>,
    pub config: Config,

}

impl Aparte {
    pub fn new(config_path: PathBuf) -> Self {
        let mut config_file = match OpenOptions::new().read(true).write(true).create(true).open(config_path) {
            Err(err) => panic!("Cannot read config file {}", err),
            Ok(config_file) => config_file,
        };

        let mut config_str = String::new();
        if let Err(e) = config_file.read_to_string(&mut config_str) {
            panic!("Cannot read config file {}", e);
        }

        let config = match config_str.len() {
            0 => Config { accounts: HashMap::new() },
            _ => match toml::from_str(&config_str) {
                Err(err) => panic!("Cannot read config file {}", err),
                Ok(config) => config,
            },
        };

        Self {
            commands: HashMap::new(),
            plugins: HashMap::new(),
            connections: RefCell::new(HashMap::new()),
            current_connection: RefCell::new(None),
            event_lock: RefCell::new(()),
            event_queue: RefCell::new(Vec::new()),
            config: config,
        }
    }

    pub fn add_command(&mut self, command: CommandParser) {
        self.commands.insert(command.name.to_string(), command);
    }

    pub fn parse_command(self: Rc<Self>, command: Command) -> Result<(), String> {
        match Rc::clone(&self).commands.get(&command.args[0]) {
            Some(parser) => (parser.parser)(self, command),
            None => Err(format!("Unknown command {}", command.args[0])),
        }
    }

    pub fn autocomplete(&self, command: Command) -> Vec<String> {
        if command.cursor == 0 {
            self.commands.iter().map(|c| c.0.to_string()).collect()
        } else {
            if let Some(parser) = self.commands.get(&command.args[0]) {
                if command.cursor - 1 < parser.completions.len() {
                    if let Some(completion) = &parser.completions[command.cursor - 1] {
                        completion(self, command)
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        }
    }

    pub fn add_plugin<T: 'static + fmt::Display + Plugin>(&mut self, plugin: T) {
        info!("Add plugin `{}`", plugin);
        self.plugins.insert(TypeId::of::<T>(), RefCell::new(Box::new(plugin)));
    }

    pub fn get_plugin<T: 'static>(&self) -> Option<Ref<T>> {
        let rc = match self.plugins.get(&TypeId::of::<T>()) {
            Some(rc) => rc,
            None => return None,
        };

        let any_plugin = rc.borrow();
        /* Calling unwrap here on purpose as we expect panic if plugin is not of the right type */
        Some(Ref::map(any_plugin, |p| p.as_any().downcast_ref::<T>().unwrap()))
    }

    pub fn get_plugin_mut<T: 'static>(&self) -> Option<RefMut<T>> {
        let rc = match self.plugins.get(&TypeId::of::<T>()) {
            Some(rc) => rc,
            None => return None,
        };

        let any_plugin = rc.borrow_mut();
        /* Calling unwrap here on purpose as we expect panic if plugin is not of the right type */
        Some(RefMut::map(any_plugin, |p| p.as_any_mut().downcast_mut::<T>().unwrap()))
    }

    pub fn add_connection(&self, account: FullJid, sink: UnboundedSender<Packet>) {
        let connection = Connection {
            account: account,
            sink: sink,
        };

        let account = connection.account.to_string();

        self.connections.borrow_mut().insert(account.clone(), connection);
        self.current_connection.replace(Some(account.clone()));
    }

    pub fn current_connection(&self) -> Option<FullJid> {
        let current_connection = self.current_connection.borrow();
        match &*current_connection {
            Some(current_connection) => {
                let connections = self.connections.borrow_mut();
                let connection = connections.get(&current_connection.clone()).unwrap();
                Some(connection.account.clone())
            },
            None => None,
        }
    }

    pub fn init(&mut self) -> Result<(), ()> {
        for (_, plugin) in self.plugins.iter() {
            if let Err(err) = plugin.borrow_mut().as_plugin().init(&self) {
                return Err(err);
            }
        }

        Ok(())
    }

    pub fn send(&self, element: Element) {
        debug!("SEND: {:?}", element);
        let packet = Packet::Stanza(element);
        // TODO use correct connection
        let mut connections = self.connections.borrow_mut();
        let current_connection = connections.iter_mut().next().unwrap().1;
        let mut sink = &current_connection.sink;
        if let Err(e) = sink.start_send(packet) {
            warn!("Cannot send packet: {}", e);
        }
    }

    pub fn event(self: Rc<Self>, event: Event) {
        self.event_queue.borrow_mut().push(event);
        if let Ok(_lock) = self.event_lock.try_borrow_mut() {
            while self.event_queue.borrow().len() > 0 {
                let event = self.event_queue.borrow_mut().remove(0);
                for (_, plugin) in self.plugins.iter() {
                    plugin.borrow_mut().as_plugin().on_event(Rc::clone(&self), &event);
                }

                match event {
                    Event::Command(command) => {
                        match Rc::clone(&self).parse_command(command) {
                            Err(err) => Rc::clone(&self).log(err),
                            Ok(()) => {},
                        }
                    },
                    Event::SendMessage(message) => {
                        Rc::clone(&self).event(Event::Message(message.clone()));
                        if let Ok(xmpp_message) = Element::try_from(message) {
                            self.send(xmpp_message);
                        }
                    },
                    _ => {},
                }
            }
        }
    }

    pub fn log(self: Rc<Self>, message: String) {
        let message = Message::log(message);
        self.event(Event::Message(message));
    }
}

#[macro_export]
macro_rules! parse_command_args {
    ($aparte:ident, $command:ident, $index:ident) => ();
    ($aparte:ident, $command:ident, $index:ident, (password) $arg:ident) => (
        if $command.args.len() <= $index {
            Rc::clone(&$aparte).event(Event::ReadPassword($command.clone()));
            return Ok(())
        }

        let $arg = $command.args[$index].clone();
    );
    ($aparte:ident, $command:ident, $index:ident, (optional) $arg:ident) => (
        let $arg = {
            if $command.args.len() > $index {
                Some($command.args[$index].clone())
            } else {
                None
            }
        };
    );
    ($aparte:ident, $command:ident, $index:ident, $arg:ident) => (
        if $command.args.len() <= $index {
            return Err(format!("Missing {} argument", stringify!($arg)))
        }
        let $arg = $command.args[$index].clone();
    );
    ($aparte:ident, $command:ident, $index:ident, (optional) $arg:ident, $($(($attr:ident))? $args:ident),+) => (
        let $arg = {
            if $command.args.len() > $index {
                Some($command.args[$index].clone())
            } else {
                None
            }
        };

        $index += 1;

        parse_command_args!($command, $index, $($(($attr))? $args),*);
    );
    ($aparte:ident, $command:ident, $index:ident, $arg:ident, $($(($attr:ident))? $args:ident),+) => (
        if $command.args.len() <= $index {
            return Err(format!("Missing {} argument", stringify!($arg)))
        }

        let $arg = $command.args[$index].clone();

        $index += 1;

        parse_command_args!($aparte, $command, $index, $($(($attr))? $args),*);
    );
}

#[macro_export]
macro_rules! generate_command_completions {
    ($completions:ident) => ();
    ($completions:ident, $argname:ident) => (
        $completions.push(None);
    );
    ($completions:ident, $argname:ident: { completion: |$aparte:ident, $command:ident| $completion:block }) => (
        $completions.push(Some(Box::new(|$aparte: &Aparte, $command: Command| -> Vec<String> { $completion })));
    );
    ($completions:ident, $argname:ident, $($argnames:ident$(: $args:tt)?),+) => (
        $completions.push(None);
        generate_command_completions!($completions, $($argnames$(: $args)?),*);
    );
    ($completions:ident, $argname:ident: { completion: |$aparte:ident, $command:ident| $completion:block }, $($argnames:ident$(: $args:tt)?),+) => (
        $completions.push(Some(Box::new(|$aparte: &Aparte, $command: Command| -> Vec<String> { $completion })));
        generate_command_completions!($completions, $($argnames$(: $args)?),*);
    );
}

#[macro_export]
macro_rules! command_def {
    ($name:ident, $help: tt, |$aparte:ident, $command:ident| $body:block) => (
        fn $name() -> CommandParser {
            let completions = Vec::<Option<Box<dyn Fn(&Aparte, Command) -> Vec<String>>>>::new();

            CommandParser {
                name: stringify!($name),
                help: $help,
                parser: Box::new(|$aparte: Rc<Aparte>, $command: Command| -> Result<(), String> {
                    #[allow(unused_mut)]
                    $body
                }),
                completions: completions,
            }
        }
    );
    ($name:ident, $help: tt, $($(($attr:ident))? $argnames:ident$(: $args:tt)?),*, |$aparte:ident, $command:ident| $body:block) => (
        fn $name() -> CommandParser {
            let mut completions = Vec::<Option<Box<dyn Fn(&Aparte, Command) -> Vec<String>>>>::new();

            generate_command_completions!(completions, $($argnames$(: $args)?),*);

            CommandParser {
                name: stringify!($name),
                help: $help,
                parser: Box::new(|$aparte: Rc<Aparte>, $command: Command| -> Result<(), String> {
                    #[allow(unused_mut)]
                    let mut index = 1;
                    parse_command_args!($aparte, $command, index, $($(($attr))? $argnames),*);
                    $body
                }),
                completions: completions,
            }
        }
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    command_def!{
        no_args,
        "help",
        |_aparte, _command| {
            Ok(())
        }
    }

    #[test]
    fn test_command_without_args() {
        let cmd = no_args();

        assert_eq!(cmd.name, "no_args");
        assert_eq!(cmd.help, "help");
    }

    command_def!{
        one_arg,
        "help",
        _first_arg,
        |_aparte, _command| {
            Ok(())
        }
    }

    #[test]
    fn test_command_with_one_arg() {
        let cmd = one_arg();

        assert_eq!(cmd.name, "one_arg");
        assert_eq!(cmd.help, "help");
    }

    command_def!{
        one_arg_completion,
        "help",
        _first_arg: {
            completion: |_aparte, _command| {
                Vec::new()
            }
        },
        |_aparte, _command| {
            Ok(())
        }
    }

    #[test]
    fn test_command_with_one_arg_with_completion() {
        let cmd = one_arg_completion();

        assert_eq!(cmd.name, "one_arg_completion");
        assert_eq!(cmd.help, "help");
        assert_eq!(cmd.completions.len(), 1);
    }

    command_def!{
        two_args,
        "help",
        _first_arg,
        _second_arg,
        |_aparte, _command| {
            Ok(())
        }
    }

    #[test]
    fn test_command_with_two_args() {
        let cmd = two_args();

        assert_eq!(cmd.name, "two_args");
        assert_eq!(cmd.help, "help");
        assert_eq!(cmd.completions.len(), 2);
    }

    command_def!{
        two_args_completion,
        "help",
        _first_arg: {
            completion: |_aparte, _command| {
                Vec::new()
            }
        },
        _second_arg,
        |_aparte, _command| {
            Ok(())
        }
    }

    #[test]
    fn test_command_with_two_args_with_completion() {
        let cmd = two_args_completion();

        assert_eq!(cmd.name, "two_args_completion");
        assert_eq!(cmd.help, "help");
        assert_eq!(cmd.completions.len(), 2);
    }
}
