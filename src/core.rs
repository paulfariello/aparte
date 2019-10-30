use futures::Sink;
use futures::unsync::mpsc::UnboundedSender;
use std::any::{Any, TypeId};
use std::cell::{RefCell, RefMut, Ref};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use std::io::Error as IoError;
use std::rc::Rc;
use std::string::FromUtf8Error;
use tokio_xmpp::Packet;
use xmpp_parsers::{Element, FullJid, BareJid, presence, iq};
use xmpp_parsers;

use crate::{contact, conversation};
use crate::message::Message;

#[derive(Debug, Clone)]
pub enum CommandOrMessage {
    Command(Command),
    Message(Message),
}

#[derive(Debug, Clone)]
pub struct Command {
    pub name: String,
    pub args: Vec<String>,
    pub cursor: usize,
}

impl Command {
    pub fn new(command: String, args: Vec<String>) -> Self {
        Self {
            name: command,
            cursor: args.len() + 1,
            args: args,
        }
    }

    pub fn parse_with_cursor(string: &str, cursor: usize) -> Result<Self, &'static str> {
        enum State {
            Initial,
            Delimiter,
            SimplyQuoted,
            DoublyQuoted,
            Unquoted,
            UnquotedEscaped,
            SimplyQuotedEscaped,
            DoublyQuotedEscaped,
        };

        use State::*;

        let mut string_cursor = cursor;
        let mut tokens: Vec<String> = Vec::new();
        let mut token = String::new();
        let mut state = Initial;
        let mut chars = string.chars();
        let mut token_cursor = None;

        loop {
            let c = chars.next();
            state = match state {
                Initial => match c {
                    Some('/') => Delimiter,
                    _ => return Err("Missing starting /"),
                },
                Delimiter => match c {
                    Some(' ') => Delimiter,
                    Some('\'') => SimplyQuoted,
                    Some('\"') => DoublyQuoted,
                    Some('\\') => UnquotedEscaped,
                    Some(c) => {
                        token.push(c);
                        Unquoted
                    },
                    None => {
                        break;
                    }
                },
                SimplyQuoted => match c {
                    Some('\'') => Unquoted,
                    Some('\\') => SimplyQuotedEscaped,
                    Some(c) => {
                        token.push(c);
                        SimplyQuoted
                    },
                    None => return Err("Missing closing quote"),
                },
                DoublyQuoted => match c {
                    Some('\"') => Unquoted,
                    Some('\\') => DoublyQuotedEscaped,
                    Some(c) => {
                        token.push(c);
                        DoublyQuoted
                    },
                    None => return Err("Missing closing quote"),
                },
                Unquoted => match c {
                    Some('\'') => SimplyQuoted,
                    Some('\"') => DoublyQuoted,
                    Some('\\') => UnquotedEscaped,
                    Some(' ') => {
                        tokens.push(token);
                        token = String::new();
                        Delimiter
                    },
                    Some(c) => {
                        token.push(c);
                        Unquoted
                    },
                    None => {
                        tokens.push(token);
                        token = String::new();
                        break;
                    }
                },
                UnquotedEscaped => match c {
                    Some(c) => {
                        token.push(c);
                        Unquoted
                    },
                    None => return Err("Missing escaped char"),
                },
                SimplyQuotedEscaped => match c {
                    Some(c) => {
                        token.push(c);
                        SimplyQuoted
                    },
                    None => return Err("Missing escaped char"),
                },
                DoublyQuotedEscaped => match c {
                    Some(c) => {
                        token.push(c);
                        DoublyQuoted
                    },
                    None => return Err("Missing escaped char"),
                }
            };

            if string_cursor == 0 {
                if token_cursor.is_none() {
                    token_cursor = match c {
                        Some(_) => Some(tokens.len()),
                        None => None,
                    }
                }
            } else {
                string_cursor -= 1;
            }
        }

        if token_cursor.is_none() {
            token_cursor = match state {
                Delimiter => Some(tokens.len()),
                _ => Some(tokens.len() - 1),
            };
        }

        if tokens.len() > 0 {
            Ok(Command {
                name: tokens[0].clone(),
                args: tokens[1..].to_vec(),
                cursor: token_cursor.unwrap(),
            })
        } else {
            Ok(Command {
                name: "".to_string(),
                args: Vec::new(),
                cursor: token_cursor.unwrap(),
            })
        }
    }

    fn escape(arg: &str) -> String {
        let mut quote = None;
        let mut escaped = String::with_capacity(arg.len());
        for c in arg.chars() {
            escaped.extend(match c {
                '\\' => "\\\\".to_string(),
                ' ' => {
                    if quote.is_none() {
                        quote = Some(' ');
                    }
                    " ".to_string()
                },
                '\'' => {
                    match quote {
                        Some('\'') => "\\'".to_string(),
                        Some('"') => "'".to_string(),
                        Some(' ') | None => {
                            quote = Some('"');
                            "'".to_string()
                        },
                        Some(_) => unreachable!(),
                    }
                }
                '"' => {
                    match quote {
                        Some('\'') => "\"".to_string(),
                        Some('"') => "\\\"".to_string(),
                        Some(' ') | None => {
                            quote = Some('\'');
                            "\"".to_string()
                        },
                        Some(_) => unreachable!(),
                    }
                }
                c => c.to_string(),
            }.chars())
        }

        if quote == Some(' ') {
            quote = Some('"');
        }

        if quote.is_none() {
            return escaped;
        } else {
            return format!("{}{}{}", quote.unwrap(), escaped, quote.unwrap());
        }

    }

    pub fn assemble(&self) -> String {
        let mut command = "/".to_string();

        command.extend(self.name.chars());

        for arg in &self.args {
            command.push(' ');
            command.extend(Command::escape(arg).chars());
        }

        command
    }
}

impl TryFrom<&str> for Command {
    type Error = &'static str;

    fn try_from(string: &str) -> Result<Self, Self::Error> {
        Command::parse_with_cursor(string, string.len())
    }
}

pub struct CommandParser {
    pub name: &'static str,
    pub parser: Box<dyn Fn(Rc<Aparte>, Command) -> Result<(), String>>,
    pub completions: Vec<Option<Box<dyn Fn(&Aparte, Command) -> Vec<String>>>>,
}

#[derive(Debug, Error)]
pub enum CommandError {
    Io(IoError),
    Utf8(FromUtf8Error),
    Parse,
}

pub enum Event {
    Connected(FullJid),
    #[allow(dead_code)]
    Disconnected(FullJid),
    Message(Message),
    Chat(BareJid),
    Join(FullJid),
    Iq(iq::Iq),
    Presence(presence::Presence),
    ReadPassword(Command),
    Win(String),
    Contact(contact::Contact),
    ContactUpdate(contact::Contact),
    Occupant(conversation::Occupant),
    Signal(i32),
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
    commands: HashMap<String, CommandParser>,
    plugins: HashMap<TypeId, RefCell<Box<dyn AnyPlugin>>>,
    connections: RefCell<HashMap<String, Connection>>,
    current_connection: RefCell<Option<String>>,
    event_lock: RefCell<()>,
    event_queue: RefCell<Vec<Event>>,
}

impl Aparte {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
            plugins: HashMap::new(),
            connections: RefCell::new(HashMap::new()),
            current_connection: RefCell::new(None),
            event_lock: RefCell::new(()),
            event_queue: RefCell::new(Vec::new()),
        }
    }

    pub fn add_command(&mut self, command: CommandParser) {
        self.commands.insert(command.name.to_string(), command);
    }

    pub fn parse_command(self: Rc<Self>, command: Command) -> Result<(), String> {
        match Rc::clone(&self).commands.get(&command.name) {
            Some(parser) => (parser.parser)(self, command),
            None => Err(format!("Unknown command {}", command.name)),
        }
    }

    pub fn autocomplete(&self, command: Command) -> Vec<String> {
        if command.cursor == 0 {
            self.commands.iter().filter_map(|c| {
                if c.0.starts_with(&command.name) {
                    Some(c.0.to_string())
                } else {
                    None
                }
            }).collect()
        } else {
            if let Some(parser) = self.commands.get(&command.name) {
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
    ($completions:ident, $argname:ident: { completion: |$aparte:ident, $command:ident| => $completion:block }, $($argnames:ident$(: $args:tt)?),+) => (
        $completions.push(Box::new(|$aparte: &Aparte, $command: Command| -> Vec<String> { $completion }));
        generate_command_completions!($completions, $($argnames$(: $args)?),*);
    );
}

#[macro_export]
macro_rules! command_def {
    ($name:ident, $($(($attr:ident))? $argnames:ident$(: $args:tt)?),*, |$aparte:ident, $command:ident| $body:block) => (
        fn $name() -> CommandParser {
            let mut completions = Vec::<Option<Box<dyn Fn(&Aparte, Command) -> Vec<String>>>>::new();

            generate_command_completions!(completions, $($argnames$(: $args)?),*);

            CommandParser {
                name: stringify!($name),
                parser: Box::new(|$aparte: Rc<Aparte>, $command: Command| -> Result<(), String> {
                    #[allow(unused_mut)]
                    let mut index = 0;
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

    #[test]
    fn test_simple_command_parsing() {
        let command = Command::try_from("/test command");
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.name, "test");
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0], "command");
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_multiple_args_command_parsing() {
        let command = Command::try_from("/test command with args");
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.name, "test");
        assert_eq!(command.args.len(), 3);
        assert_eq!(command.args[0], "command");
        assert_eq!(command.args[1], "with");
        assert_eq!(command.args[2], "args");
        assert_eq!(command.cursor, 3);
    }

    #[test]
    fn test_doubly_quoted_arg_command_parsing() {
        let command = Command::try_from("/test \"command with arg\"");
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.name, "test");
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0], "command with arg");
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_simply_quoted_arg_command_parsing() {
        let command = Command::try_from("/test 'command with arg'");
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.name, "test");
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0], "command with arg");
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_mixed_quote_arg_command_parsing() {
        let command = Command::try_from("/test 'command with \" arg'");
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.name, "test");
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0], "command with \" arg");
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_missing_closing_quote() {
        let command = Command::try_from("/test \"command with arg");
        assert!(command.is_err());
        assert_eq!(command.err(), Some("Missing closing quote"));
    }

    #[test]
    fn test_command_args_parsing_with_cursor() {
        let command = Command::parse_with_cursor("/test command with args", 10);
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.name, "test");
        assert_eq!(command.args.len(), 3);
        assert_eq!(command.args[0], "command");
        assert_eq!(command.args[1], "with");
        assert_eq!(command.args[2], "args");
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_command_parsing_with_cursor() {
        let command = Command::parse_with_cursor("/te", 3);
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.name, "te");
        assert_eq!(command.args.len(), 0);
        assert_eq!(command.cursor, 0);
    }

    #[test]
    fn test_command_end_with_space_parsing_with_cursor() {
        let command = Command::parse_with_cursor("/test ", 6);
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.name, "test");
        assert_eq!(command.args.len(), 0);
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_no_command_parsing_with_cursor() {
        let command = Command::parse_with_cursor("/", 1);
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.name, "");
        assert_eq!(command.args.len(), 0);
        assert_eq!(command.cursor, 0);
    }

    #[test]
    fn test_command_assemble() {
        let command = Command {
            name: "test".to_string(),
            args: vec!["foo".to_string(), "bar".to_string()],
            cursor: 0,
        };

        assert_eq!(command.assemble(), "/test foo bar");
    }

    #[test]
    fn test_command_with_double_quote_assemble() {
        let command = Command {
            name: "test".to_string(),
            args: vec!["fo\"o".to_string(), "bar".to_string()],
            cursor: 0,
        };

        assert_eq!(command.assemble(), "/test 'fo\"o' bar");
    }

    #[test]
    fn test_command_with_simple_quote_assemble() {
        let command = Command {
            name: "test".to_string(),
            args: vec!["fo'o".to_string(), "bar".to_string()],
            cursor: 0,
        };

        assert_eq!(command.assemble(), "/test \"fo'o\" bar");
    }

    #[test]
    fn test_command_with_space_assemble() {
        let command = Command {
            name: "test".to_string(),
            args: vec!["foo bar".to_string()],
            cursor: 0,
        };

        assert_eq!(command.assemble(), "/test \"foo bar\"");
    }

    #[test]
    fn test_command_with_space_and_quote_assemble() {
        let command = Command {
            name: "test".to_string(),
            args: vec!["foo bar\"".to_string()],
            cursor: 0,
        };

        assert_eq!(command.assemble(), "/test 'foo bar\"'");
    }
}
