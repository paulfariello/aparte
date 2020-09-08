/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use chrono::Utc;
use core::fmt::Debug;
use futures::unsync::mpsc::UnboundedSender;
use futures::{future, Future, Sink, Stream};
use std::any::{Any, TypeId};
use std::cell::{RefCell, RefMut, Ref};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use termion::event::Key;
use tokio::runtime::{Runtime as TokioRuntime, Handle as TokioHandle};
use tokio::io;
use tokio::signal::unix;
use tokio::sync::mpsc;
use tokio_xmpp::{AsyncClient as Client, Error as XmppError, Packet};
use uuid::Uuid;
use xmpp_parsers::iq::{Iq, IqType};
use xmpp_parsers::message::{Message as XmppParsersMessage, MessageType as XmppParsersMessageType};
use xmpp_parsers::muc::Muc;
use xmpp_parsers::presence::{Presence, Show as PresenceShow, Type as PresenceType};
use xmpp_parsers::{Element, Jid, FullJid, BareJid, presence, iq};
use xmpp_parsers;

use crate::{command_def, parse_command_args, generate_command_autocompletions, generate_arg_autocompletion};
use crate::command::{Command, CommandParser};
use crate::config::Config;
use crate::message::Message;
use crate::plugins;
use crate::terminus::ViewTrait;
use crate::{contact, conversation};

#[derive(Debug)]
pub enum Event {
    Start,
    Connect(FullJid, Password<String>),
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
    WindowChange,
    #[allow(unused)]
    LoadHistory(BareJid),
    Quit,
    Key(Key),
    AutoComplete(String, usize),
    ResetCompletion,
    Completed(String, usize),
    ChangeWindow(String),
}

impl Debug for dyn ViewTrait<Event> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ViewTrait<Event>")
    }
}

pub trait Plugin: fmt::Display {
    fn new() -> Self where Self: Sized;
    fn init(&mut self, aparte: &mut Aparte) -> Result<(), ()>;
    fn on_event(&mut self, aparte: &mut Aparte, event: &Event);
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

#[derive(Debug)]
pub struct Password<T: FromStr>(pub T);

impl<T: FromStr> FromStr for Password<T> {
    type Err = T::Err;

    fn from_str(s: &str) -> Result<Self, T::Err> {
        match T::from_str(s) {
            Err(e) => Err(e),
            Ok(inner) => Ok(Password(inner)),
        }
    }
}

pub struct Connection {
    pub sink: UnboundedSender<Packet>,
    pub account: FullJid,
}

pub struct Aparte {
    pub command_parsers: Rc<HashMap<String, CommandParser>>,
    plugins: Rc<HashMap<TypeId, RefCell<Box<dyn AnyPlugin>>>>,
    connections: RefCell<HashMap<String, Connection>>,
    current_connection: RefCell<Option<String>>,
    event_queue: Vec<Event>,
    pub config: Config,

}

command_def!(connect,
r#"/connect <account>

    account       Account to connect to

Description:
    Connect to the given account.

Examples:
    /connect myaccount
    /connect account@server.tld
    /connect account@server.tld/resource
    /connect account@server.tld:5223
"#,
{
    account: String = {
        completion: (|aparte, _command| {
            aparte.config.accounts.iter().map(|(name, _)| name.clone()).collect()
        })
    },
    password: Password<String>
},
|aparte, _command| {
    let jid;

    if let Some((_, config)) = aparte.config.accounts.iter().find(|(name, _)| *name == &account) {
        if let Ok(jid_config) = Jid::from_str(&config.jid) {
            jid = jid_config;
        } else {
            return Err(format!("Invalid account jid {}", config.jid));
        }
    } else if let Ok(jid_param) = Jid::from_str(&account) {
        jid = jid_param;
    } else {
        return Err(format!("Unknown account {}", account));
    }

    let full_jid = match jid {
        Jid::Full(jid) => jid,
        Jid::Bare(jid) => jid.with_resource("aparte"),
    };


    aparte.schedule(Event::Connect(full_jid, password));

    Ok(())
});

command_def!(win,
r#"Usage: /win <window>

    window        Name of the window to switch to

Description:
    Switch to a given window.

Examples:
    /win console
    /win contact@server.tld"#,
{
    window: String = {
        completion: (|aparte, _command| {
            let ui = aparte.get_plugin::<plugins::ui::UIPlugin>().unwrap();
            ui.get_windows()
        })
    }
},
|aparte, _command| {
    aparte.schedule(Event::Win(window.clone()));
    Ok(())
});

command_def!(msg,
r#"/msg <contact> [<message>]

    contact       Contact to send a message to
    message       Optionnal message to be sent

Description:
    Open a window for a private discussion with a given contact and optionnaly
    send a message.

Example:
    /msg contact@server.tld
    /msg contact@server.tld "Hi there!"
"#,
{
    contact: String = {
        completion: (|aparte, _command| {
            let contact = aparte.get_plugin::<plugins::contact::ContactPlugin>().unwrap();
            contact.contacts.iter().map(|c| c.0.to_string()).collect()
        })
    },
    message: Option<String>
},
|aparte, _command| {
    match aparte.current_connection() {
        Some(connection) => {
            match Jid::from_str(&contact.clone()) {
                Ok(jid) => {
                    let to = match jid.clone() {
                        Jid::Bare(jid) => jid,
                        Jid::Full(jid) => jid.into(),
                    };
                    aparte.schedule(Event::Chat(to));
                    if message.is_some() {
                        let id = Uuid::new_v4().to_string();
                        let from: Jid = connection.into();
                        let timestamp = Utc::now();
                        let message = Message::outgoing_chat(id, timestamp, &from, &jid, &message.unwrap());
                        aparte.schedule(Event::Message(message.clone()));

                        aparte.send(Element::try_from(message).unwrap());
                    }
                    Ok(())
                },
                Err(err) => {
                    Err(format!("Invalid JID {}: {}", contact, err))
                }
            }
        },
        None => {
            Err(format!("No connection found"))
        }
    }
});

command_def!(join,
r#"/join <channel>

    channel       Channel JID to join
Description:
    Open a window and join a given channel.

Example:
    /join channel@conference.server.tld"#,
{
    muc: String
},
|aparte, _command| {
    match aparte.current_connection() {
        Some(connection) => {
            match Jid::from_str(&muc) {
                Ok(jid) => {
                    let to = match jid {
                        Jid::Full(jid) => jid,
                        Jid::Bare(jid) => {
                            let node = connection.node.clone().unwrap();
                            jid.with_resource(node)
                        }
                    };
                    let from: Jid = connection.into();

                    let mut presence = Presence::new(PresenceType::None);
                    presence = presence.with_to(Jid::Full(to.clone()));
                    presence = presence.with_from(from);
                    presence.add_payload(Muc::new());
                    aparte.send(presence.into());
                    aparte.schedule(Event::Join(to.clone()));

                    Ok(())
                },
                Err(err) => {
                    Err(format!("Invalid JID {}: {}", muc, err))
                }
            }
        },
        None => {
            Err(format!("No connection found"))
        }
    }
});

command_def!(quit,
r#"/quit

Description:
    Quit Aparté.

Example:
    /quit"#,
{ },
|aparte, _command| {
    aparte.schedule(Event::Quit);

    Ok(())
});

command_def!(help,
r#"/help <command>

    command       Name of command

Description:
    Print help of a given command.

Examples:
    /help help
    /help win"#,
{
    cmd: String = {
        completion: (|aparte, _command| {
            aparte.command_parsers.iter().map(|c| c.0.to_string()).collect()
        })
    }
},
|aparte, _command| {
    let log = match aparte.command_parsers.get(&cmd) {
        Some(command) => command.help.to_string(),
        None => format!("Unknown command {}", cmd),
    };

    aparte.log(log);

    Ok(())
});

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
            command_parsers: Rc::new(HashMap::new()),
            plugins: Rc::new(HashMap::new()),
            connections: RefCell::new(HashMap::new()),
            current_connection: RefCell::new(None),
            event_queue: Vec::new(),
            config: config,
        }
    }

    pub fn add_command(&mut self, command_parser: CommandParser) {
        let command_parsers = Rc::get_mut(&mut self.command_parsers).unwrap();
        command_parsers.insert(command_parser.name.to_string(), command_parser);
    }

    pub fn parse_command(&mut self, command: Command) -> Result<(), String> {
        let parser = {
            match self.command_parsers.get(&command.args[0]) {
                Some(parser) => parser.clone(),
                None => return Err(format!("Unknown command {}", command.args[0])),
            }
        };

        (parser.parser)(self, command)
    }

    pub fn add_plugin<T: 'static + fmt::Display + Plugin>(&mut self, plugin: T) {
        info!("Add plugin `{}`", plugin);
        let plugins = Rc::get_mut(&mut self.plugins).unwrap();
        plugins.insert(TypeId::of::<T>(), RefCell::new(Box::new(plugin)));
    }

    pub fn get_plugin<T: 'static>(&self) -> Option<Ref<T>> {
        let plugin = match self.plugins.get(&TypeId::of::<T>()) {
            Some(plugin) => plugin,
            None => return None,
        };

        let any_plugin = plugin.borrow();
        /* Calling unwrap here on purpose as we expect panic if plugin is not of the right type */
        Some(Ref::map(any_plugin, |p| p.as_any().downcast_ref::<T>().unwrap()))
    }

    pub fn get_plugin_mut<T: 'static>(&self) -> Option<RefMut<T>> {
        let plugin = match self.plugins.get(&TypeId::of::<T>()) {
            Some(plugin) => plugin,
            None => return None,
        };

        let any_plugin = plugin.borrow_mut();
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
        self.add_command(help::new());
        self.add_command(connect::new());
        self.add_command(win::new());
        self.add_command(msg::new());
        self.add_command(join::new());
        self.add_command(quit::new());

        let plugins = Rc::clone(&self.plugins);
        for (_, plugin) in plugins.iter() {
            if let Err(err) = plugin.borrow_mut().as_plugin().init(self) {
                return Err(err);
            }
        }

        Ok(())
    }

    pub fn run(mut self) {
        const VERSION: &'static str = env!("CARGO_PKG_VERSION");

        self.log(r#"
▌ ▌   ▜               ▐      ▞▀▖         ▐   ▞
▌▖▌▞▀▖▐ ▞▀▖▞▀▖▛▚▀▖▞▀▖ ▜▀ ▞▀▖ ▙▄▌▛▀▖▝▀▖▙▀▖▜▀ ▞▀▖
▙▚▌▛▀ ▐ ▌ ▖▌ ▌▌▐ ▌▛▀  ▐ ▖▌ ▌ ▌ ▌▙▄▘▞▀▌▌  ▐ ▖▛▀
▘ ▘▝▀▘ ▘▝▀ ▝▀ ▘▝ ▘▝▀▘  ▀ ▝▀  ▘ ▘▌  ▝▀▘▘   ▀ ▝▀▘
"#.to_string());
        self.log(format!("Version: {}", VERSION));

        let mut rt = TokioRuntime::new().unwrap();
        let mut event_stream = {
            let ui = self.get_plugin::<plugins::ui::UIPlugin>().unwrap();
            ui.event_stream()
        };

        let aparte = Arc::new(Mutex::new(self));
        let (mut tx, mut rx) = mpsc::channel(32);

        let mut tx_for_signal = tx.clone();
        rt.spawn(async move {
            let mut sigwinch = unix::signal(unix::SignalKind::window_change()).unwrap();
            loop {
                sigwinch.recv().await;
                tx_for_signal.send(Event::WindowChange).await;
            }
        });

        Aparte::ex_schedule(aparte.clone(), Event::Start);
        Aparte::event_loop(aparte.clone()).unwrap();

        let mut tx_for_event = tx.clone();
        rt.spawn(async move {
            loop {
                match event_stream.read_event().await {
                    Ok(event) => {
                        tx_for_event.send(event).await;
                    },
                    Err(err) => {
                        error!("Input error: {}", err);
                        tx_for_event.send(Event::Quit).await;
                        break;
                    }
                }
            }
        });

        let aparte_for_event = aparte.clone();

        rt.block_on(async move {
            while let Some(event) = rx.recv().await {

                let quit = match event {
                    Event::Quit => true,
                    _  => false,
                };

                Aparte::ex_schedule(aparte_for_event.clone(), event);
                if Aparte::event_loop(aparte_for_event.clone()).is_err() {
                    break;
                }
            }
        });
    }

    pub fn start(&mut self) {
        for (_, account) in self.config.accounts.clone() {
            if account.autoconnect {
                self.schedule(Event::Command(Command {
                    args: vec!["connect".to_string(), account.jid.clone()],
                    cursor: 0
                }));
            }
        }
    }

    pub fn send(&self, element: Element) {
        let mut raw = Vec::<u8>::new();
        element.write_to(&mut raw).unwrap();
        debug!("SEND: {}", String::from_utf8(raw).unwrap());
        let packet = Packet::Stanza(element);
        // TODO use correct connection
        let mut connections = self.connections.borrow_mut();
        let current_connection = connections.iter_mut().next().unwrap().1;
        let mut sink = &current_connection.sink;
        if let Err(e) = sink.start_send(packet) {
            warn!("Cannot send packet: {}", e);
        }
    }

    pub fn connect(this: Arc<Mutex<Self>>, jid: FullJid, password: Password<String>) {
        //let mut aparte = this.lock().unwrap();
        //aparte.log(format!("Connecting as {}", jid));
        //let client = Client::new(&jid.to_string(), &password.0).unwrap();

        //let (sink, stream) = io::split(client);
        //let (tx, rx) = futures::unsync::mpsc::unbounded();

        //aparte.add_connection(jid.clone(), tx);

        //TokioHandle::current().spawn(
        //    rx.forward(
        //        sink.sink_map_err(|_| panic!("Pipe"))
        //        ).map(|(rx, mut sink)| {
        //        drop(rx);
        //        let _ = sink.close();
        //    }).map_err(|e| {
        //            panic!("Send error: {:?}", e);
        //        })
        //    );

        //let aparte_for_event = this.clone();
        //let client = stream.for_each(move |event| {
        //    debug!("XMPP Event: {:?}", event);
        //    if event.is_online() {
        //        Aparte::ex_log(aparte_for_event.clone(), format!("Connected as {}", jid));
        //        Aparte::ex_schedule(aparte_for_event.clone(), Event::Connected(jid.clone()));

        //        {
        //            let aparte = aparte_for_event.lock().unwrap();
        //            let mut presence = Presence::new(PresenceType::None);
        //            presence.show = Some(PresenceShow::Chat);

        //            aparte.send(presence.into());
        //        }
        //    } else if let Some(stanza) = event.into_stanza() {
        //        debug!("RECV: {}", String::from(&stanza));

        //        let mut aparte = aparte_for_event.lock().unwrap();
        //        aparte.handle_stanza(stanza);
        //    }

        //    Aparte::event_loop(aparte_for_event.clone());
        //    future::ok(())
        //});

        //let aparte_for_error = this.clone();
        //let client = client.map_err(move |error| {
        //    match error {
        //        XmppError::Auth(auth) => {
        //            Aparte::ex_log(aparte_for_error.clone(), format!("Authentication failed {}", auth));
        //        },
        //        error => {
        //            Aparte::ex_log(aparte_for_error.clone(), format!("Connection error {:?}", error));
        //        },
        //    }
        //    Aparte::event_loop(aparte_for_error.clone());
        //});

        //TokioHandle::current().spawn(client);
    }

    pub fn event_loop(this: Arc<Mutex<Self>>) -> Result<(), ()> {
        while this.lock().unwrap().event_queue.len() > 0 {
            let event = this.lock().unwrap().event_queue.remove(0);
            debug!("Event: {:?}", event);
            {
                let mut aparte = this.lock().unwrap();
                let plugins = Rc::clone(&aparte.plugins);
                for (_, plugin) in plugins.iter() {
                    plugin.borrow_mut().as_plugin().on_event(&mut aparte, &event);
                }
            }

            match event {
                Event::Start => {
                    let mut aparte = this.lock().unwrap();
                    aparte.start();
                }
                Event::Command(command) => {
                    let mut aparte = this.lock().unwrap();
                    match aparte.parse_command(command) {
                        Err(err) => aparte.log(err),
                        Ok(()) => {},
                    }
                },
                Event::SendMessage(message) => {
                    let mut aparte = this.lock().unwrap();
                    aparte.schedule(Event::Message(message.clone()));
                    if let Ok(xmpp_message) = Element::try_from(message) {
                        aparte.send(xmpp_message);
                    }
                },
                Event::Connect(jid, password) => {
                    Aparte::connect(this.clone(), jid, password);
                },
                Event::Quit => {
                    return Err(());
                }
                _ => {},
            }
        }

        Ok(())
    }

    pub fn schedule(&mut self, event: Event) {
        self.event_queue.push(event);
    }

    fn ex_schedule(this: Arc<Mutex<Self>>, event: Event) {
        let mut aparte = this.lock().unwrap();
        aparte.schedule(event);
    }

    pub fn log(&mut self, message: String) {
        let message = Message::log(message);
        self.schedule(Event::Message(message));
    }

    fn ex_log(this: Arc<Mutex<Self>>, message: String) {
        let mut aparte = this.lock().unwrap();
        aparte.log(message);
    }

    pub fn handle_stanza(&mut self, stanza: Element) {
        if let Some(message) = XmppParsersMessage::try_from(stanza.clone()).ok() {
            self.handle_message(message);
        } else if let Some(iq) = Iq::try_from(stanza.clone()).ok() {
            if let IqType::Error(stanza) = iq.payload.clone() {
                if let Some(text) = stanza.texts.get("en") {
                  let message = Message::log(text.clone());
                  self.schedule(Event::Message(message));
                }
            }
            self.schedule(Event::Iq(iq));
        } else if let Some(presence) = Presence::try_from(stanza.clone()).ok() {
            self.schedule(Event::Presence(presence));
        }
    }

    fn handle_message(&mut self, message: XmppParsersMessage) {
        if let (Some(from), Some(to)) = (message.from, message.to) {
            if let Some(ref body) = message.bodies.get("") {
                match message.type_ {
                    XmppParsersMessageType::Error => {},
                    XmppParsersMessageType::Chat => {
                        let id = message.id.unwrap_or_else(|| Uuid::new_v4().to_string());
                        let timestamp = Utc::now();
                        let message = Message::incoming_chat(id, timestamp, &from, &to, &body.0);
                        self.schedule(Event::Message(message));
                    },
                    XmppParsersMessageType::Groupchat => {
                        let id = message.id.unwrap_or_else(|| Uuid::new_v4().to_string());
                        let timestamp = Utc::now();
                        let message = Message::incoming_groupchat(id, timestamp, &from, &to, &body.0);
                        self.schedule(Event::Message(message));
                    },
                    _ => {},
                }
            }

            for payload in message.payloads {
                if let Some(received) = xmpp_parsers::carbons::Received::try_from(payload).ok() {
                    if let Some(ref original) = received.forwarded.stanza {
                        if original.type_ != XmppParsersMessageType::Error {
                            if let (Some(from), Some(to)) = (original.from.as_ref(), original.to.as_ref()) {
                                if let Some(body) = original.bodies.get("") {
                                    let id = original.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
                                    let timestamp = Utc::now();
                                    let message = Message::incoming_chat(id, timestamp, &from, &to, &body.0);
                                    self.schedule(Event::Message(message));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
