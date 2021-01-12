/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use chrono::Local as LocalTz;
use core::fmt::Debug;
use futures::stream::StreamExt;
use futures::sink::SinkExt;
use rand::{self, Rng};
use std::any::{Any, TypeId};
use std::cell::{RefCell, RefMut, Ref};
use std::collections::{HashMap, VecDeque};
use std::convert::TryFrom;
use std::fmt;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;
use termion::event::Key;
use tokio::runtime::Runtime as TokioRuntime;
use tokio::task;
use tokio::signal::unix;
use tokio::sync::mpsc;
use tokio_xmpp::{AsyncClient as TokioXmppClient, Error as XmppError, Event as XmppEvent, Packet as XmppPacket};
use uuid::Uuid;
use xmpp_parsers::iq::{Iq, IqType};
use xmpp_parsers::message::{Message as XmppParsersMessage, MessageType as XmppParsersMessageType};
use xmpp_parsers::muc::Muc;
use xmpp_parsers::presence::{Presence, Show as PresenceShow, Type as PresenceType};
use xmpp_parsers::{Element, Jid, FullJid, BareJid, presence, iq};
use xmpp_parsers;

use crate::{command_def, parse_command_args, generate_command_autocompletions, generate_arg_autocompletion, generate_help};
use crate::command::{Command, CommandParser};
use crate::config::Config;
use crate::message::Message;
use crate::plugins;
use crate::account::Account;
use crate::terminus::ViewTrait;
use crate::{contact, conversation};

const WELCOME: &str = r#"
▌ ▌   ▜               ▐      ▞▀▖         ▐   ▞
▌▖▌▞▀▖▐ ▞▀▖▞▀▖▛▚▀▖▞▀▖ ▜▀ ▞▀▖ ▙▄▌▛▀▖▝▀▖▙▀▖▜▀ ▞▀▖
▙▚▌▛▀ ▐ ▌ ▖▌ ▌▌▐ ▌▛▀  ▐ ▖▌ ▌ ▌ ▌▙▄▘▞▀▌▌  ▐ ▖▛▀
▘ ▘▝▀▘ ▘▝▀ ▝▀ ▘▝ ▘▝▀▘  ▀ ▝▀  ▘ ▘▌  ▝▀▘▘   ▀ ▝▀▘
"#;
const VERSION: &'static str = env!("CARGO_PKG_VERSION");

#[derive(Debug)]
pub enum Event {
    Start,
    Connect(Account, Password<String>),
    Connected(Jid),
    Disconnected(FullJid, String),
    AuthError(FullJid, String),
    Stanza(FullJid, Element),
    Command(Command),
    CommandError(String),
    SendMessage(Message),
    Message(Message),
    Chat(BareJid),
    Join(Jid, bool),
    Joined(FullJid, bool),
    Iq(iq::Iq),
    Disco,
    Presence(presence::Presence),
    ReadPassword(Command),
    Win(String),
    Contact(contact::Contact),
    ContactUpdate(contact::Contact),
    Bookmark(contact::Bookmark),
    DeletedBookmark(BareJid),
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
    pub sink: mpsc::Sender<Element>,
    pub account: FullJid,
}

pub struct Aparte {
    pub command_parsers: Rc<HashMap<String, CommandParser>>,
    plugins: Rc<HashMap<TypeId, RefCell<Box<dyn AnyPlugin>>>>,
    connections: RefCell<HashMap<String, Connection>>,
    current_connection: RefCell<Option<String>>,
    event_queue: Vec<Event>,
    send_queue: VecDeque<Element>,
    event_channel: Option<mpsc::Sender<Event>>,
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
    account_name: String = {
        completion: (|aparte, _command| {
            aparte.config.accounts.iter().map(|(name, _)| name.clone()).collect()
        })
    },
    password: Password<String>
},
|aparte, _command| {
    let account = {
        if let Some((_, account)) = aparte.config.accounts.iter().find(|(name, _)| *name == &account_name) {
            account.clone()
        } else if let Ok(jid) = Jid::from_str(&account_name) {
            Account {
                jid: jid.to_string(),
                server: None,
                port: None,
                autoconnect: false,
            }
        } else {
            return Err(format!("Unknown account or invalid jid {}", account_name));
        }
    };

    aparte.schedule(Event::Connect(account, password));

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
                        let timestamp = LocalTz::now();
                        let message = Message::outgoing_chat(id, timestamp.into(), &from, &jid, &message.unwrap());
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
    muc: String = {
        completion: (|aparte, _command| {
            let bookmarks = aparte.get_plugin::<plugins::bookmarks::BookmarksPlugin>().unwrap();
            bookmarks.bookmarks_by_name.iter().map(|(a, _)| a.clone()).chain(bookmarks.bookmarks_by_jid.iter().map(|(a, _)| a.to_string())).collect()
        })
    },
},
|aparte, _command| {
    match Jid::from_str(&muc) {
        Ok(jid) => {
            aparte.schedule(Event::Join(jid, true));
            Ok(())
        },
        Err(_) => {
            let jid = {
                let bookmarks = aparte.get_plugin::<plugins::bookmarks::BookmarksPlugin>().unwrap();
                match bookmarks.get_by_name(&muc) {
                    Some(bookmark) => {
                        match bookmark.nick {
                            Some(nick) => Ok(Jid::Full(bookmark.jid.with_resource(nick))),
                            None => Ok(Jid::Bare(bookmark.jid.clone())),
                        }
                    },
                    None => match Jid::from_str(&muc) {
                        Ok(jid) => Ok(jid),
                        Err(e) => Err(e.to_string()),
                    }
                }
            };

            match jid {
                Ok(jid) => {
                    aparte.schedule(Event::Join(jid, true));
                    Ok(())
                },
                Err(e) => Err(e),
            }
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
r#"/help [command]

    command       Name of command

Description:
    Print help of a given command.

Examples:
    /help win"#,
{
    cmd: Option<String> = {
        completion: (|aparte, _command| {
            aparte.command_parsers.iter().map(|c| c.0.to_string()).collect()
        })
    }
},
|aparte, _command| {
    if let Some(cmd) = cmd {
        match aparte.command_parsers.get(&cmd) {
            Some(command) => {
                aparte.log(command.help.to_string());
                Ok(())
            },
            None => Err(format!("Unknown command {}", cmd)),
        }
    } else {
        aparte.log(format!("Available commands: {}", aparte.command_parsers.iter().map(|c| c.0.to_string()).collect::<Vec<String>>().join(", ")));
        Ok(())
    }
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
            send_queue: VecDeque::new(),
            event_channel: None,
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

    pub fn add_connection(&self, account: FullJid, sink: mpsc::Sender<Element>) {
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

        let mut input_event_stream = {
            let ui = self.get_plugin::<plugins::ui::UIPlugin>().unwrap();
            ui.event_stream()
        };

        let (tx, mut rx) = mpsc::channel(32);
        let mut tx_for_signal = tx.clone();
        let mut tx_for_event = tx.clone();
        self.event_channel = Some(tx);

        let mut rt = TokioRuntime::new().unwrap();

        rt.spawn(async move {
            let mut sigwinch = unix::signal(unix::SignalKind::window_change()).unwrap();
            loop {
                sigwinch.recv().await;
                if let Err(err) = tx_for_signal.send(Event::WindowChange).await {
                    error!("Cannot send signal to internal channel: {}", err);
                    break;
                }
            }
        });


        rt.spawn(async move {
            loop {
                match input_event_stream.next().await {
                    Some(event) => {
                        if let Err(err) = tx_for_event.send(event).await {
                            error!("Cannot send event to internal channel: {}", err);
                            break;
                        }
                    },
                    None => {
                        if let Err(err) = tx_for_event.send(Event::Quit).await {
                            error!("Cannot send Quit event to internal channel: {}", err);
                        }
                        break;
                    }
                }
            }
        });

        let local_set = tokio::task::LocalSet::new();
        local_set.block_on(&mut rt, async move {
            self.schedule(Event::Start);
            if self.event_loop().await.is_err() {
                // Quit event return err
                return;
            }

            while let Some(event) = rx.recv().await {
                self.schedule(event);
                if self.event_loop().await.is_err() {
                    // Quit event return err
                    break;
                }
            }
        });
    }

    pub fn start(&mut self) {
        self.log(WELCOME.to_string());
        self.log(format!("Version: {}", VERSION));

        for (_, account) in self.config.accounts.clone() {
            if account.autoconnect {
                self.schedule(Event::Command(Command {
                    args: vec!["connect".to_string(), account.jid.clone()],
                    cursor: 0
                }));
            }
        }
    }

    pub fn send(&mut self, stanza: Element) {
        self.send_queue.push_back(stanza);
    }

    async fn send_loop(&mut self) {
        for stanza in self.send_queue.drain(..) {
            let mut raw = Vec::<u8>::new();
            stanza.write_to(&mut raw).unwrap();
            debug!("SEND: {}", String::from_utf8(raw).unwrap());
            // TODO use correct connection
            let mut connections = self.connections.borrow_mut();
            let current_connection = connections.iter_mut().next().unwrap().1;
            if let Err(e) = current_connection.sink.send(stanza).await {
                warn!("Cannot send stanza: {}", e);
            }
        }
    }

    pub async fn connect(&mut self, account: &Account, password: Password<String>) {
        let jid = match Jid::from_str(&account.jid) {
            Ok(Jid::Full(jid)) => jid,
            Ok(Jid::Bare(jid)) => {
                let rand_string: String = rand::thread_rng()
                    .sample_iter(&rand::distributions::Alphanumeric)
                    .take(5).collect();
                jid.with_resource(format!("aparte_{}", rand_string))
            },
            Err(err) => {
                self.log(format!("Cannot connect as {}: {}", account.jid, err));
                return;
            }
        };

        self.log(format!("Connecting as {}", jid));
        let mut client = match TokioXmppClient::new(&jid.to_string(), &password.0) {
            Ok(client) => client,
            Err(err) => {
                self.log(format!("Cannot connect as {}: {}", jid, err));
                return;
            }
        };

        client.set_reconnect(true);

        let (connection_channel, mut rx) = mpsc::channel(32);

        self.add_connection(jid.clone(), connection_channel);

        let (mut writer, mut reader) = client.split();
        // XXX could use self.rt.spawn if client was impl Send
        task::spawn_local(async move {
            while let Some(element) = rx.recv().await {
                if let Err(err) = writer.send(XmppPacket::Stanza(element)).await {
                    error!("cannot send Stanza to internal channel: {}", err);
                    break;
                }
            }
        });

        let mut event_channel = match &self.event_channel {
            Some(event_channel) => event_channel.clone(),
            None => unreachable!(),
        };

        let reconnect = true;
        task::spawn_local(async move {
            while let Some(event) = reader.next().await {
                debug!("XMPP Event: {:?}", event);
                match event {
                    XmppEvent::Disconnected(XmppError::Auth(e)) => {
                        if let Err(err) = event_channel.send(Event::AuthError(jid.clone(), format!("{}", e))).await {
                            error!("Cannot send event to internal channel: {}", err);
                        };
                        break;
                    },
                    XmppEvent::Disconnected(e) => {
                        if let Err(err) = event_channel.send(Event::Disconnected(jid.clone(), format!("{}", e))).await {
                            error!("Cannot send event to internal channel: {}", err);
                        };
                        if !reconnect {
                            break;
                        }
                    },
                    XmppEvent::Online{bound_jid: jid, resumed: true} => {
                        debug!("Reconnected to {}", jid);
                    },
                    XmppEvent::Online{bound_jid: jid, resumed: false} => {
                        if let Err(err) = event_channel.send(Event::Connected(jid.clone())).await {
                            error!("Cannot send event to internal channel: {}", err);
                            break;
                        }
                    }
                    XmppEvent::Stanza(stanza) => {
                        debug!("RECV: {}", String::from(&stanza));
                        if let Err(err) = event_channel.send(Event::Stanza(jid.clone(), stanza)).await {
                            error!("Cannot send stanza to internal channel: {}", err);
                            break;
                        }
                    },
                }
            }
        });
    }

    pub async fn event_loop(&mut self) -> Result<(), ()> {
        while self.event_queue.len() > 0 {
            let event = self.event_queue.remove(0);
            debug!("Event: {:?}", event);
            {
                let plugins = Rc::clone(&self.plugins);
                for (_, plugin) in plugins.iter() {
                    plugin.borrow_mut().as_plugin().on_event(self, &event);
                }
                self.send_loop().await;
            }

            match event {
                Event::Start => {
                    self.start();
                }
                Event::Command(command) => {
                    match self.parse_command(command) {
                        Err(err) => self.log(err),
                        Ok(()) => {},
                    }
                },
                Event::SendMessage(message) => {
                    self.schedule(Event::Message(message.clone()));
                    if let Ok(xmpp_message) = Element::try_from(message) {
                        self.send(xmpp_message);
                    }
                },
                Event::Connect(account, password) => {
                    self.connect(&account, password).await;
                },
                Event::Connected(jid) => {
                    self.log(format!("Connected as {}", jid));
                    let mut presence = Presence::new(PresenceType::None);
                    presence.show = Some(PresenceShow::Chat);

                    self.send(presence.into());
                },
                Event::Disconnected(jid, err) => {
                    self.log(format!("Connection lost for {}: {}", jid, err));
                },
                Event::AuthError(jid, err) => {
                    self.log(format!("Authentication error for {}: {}", jid, err));
                },
                Event::Stanza(_jid, stanza) => {
                    // TODO propagate jid?
                    self.handle_stanza(stanza);
                },
                Event::Join(jid, change_window) => {
                    match self.current_connection() {
                        Some(connection) => {
                            let to = match jid.clone() {
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
                            self.send(presence.into());

                            self.log(format!("Joined {}", jid));
                            self.schedule(Event::Joined(to, change_window));
                        },
                        None => {
                            self.log(format!("No connection found"));
                        }
                    }
                },
                Event::Quit => {
                    return Err(());
                }
                _ => {},
            }
            self.send_loop().await;
        }

        Ok(())
    }

    pub fn schedule(&mut self, event: Event) {
        self.event_queue.push(event);
    }

    pub fn log(&mut self, message: String) {
        let message = Message::log(message);
        self.schedule(Event::Message(message));
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
        let id = message.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
        if let (Some(from), Some(to)) = (message.from.clone(), message.to.clone()) {
            if let Some((_, ref body)) = message.get_best_body(vec![]) {
                match message.type_ {
                    XmppParsersMessageType::Error => {},
                    XmppParsersMessageType::Chat => {
                        let mut timestamp = None;
                        for payload in message.payloads.iter().cloned() {
                            if let Some(delay) = xmpp_parsers::delay::Delay::try_from(payload).ok() {
                                timestamp = Some(delay.stamp.0);
                            }
                        }
                        let message = Message::incoming_chat(id, timestamp.unwrap_or(LocalTz::now().into()), &from, &to, &body.0);
                        self.schedule(Event::Message(message));
                    },
                    XmppParsersMessageType::Groupchat => {
                        let mut timestamp = None;
                        for payload in message.payloads.iter().cloned() {
                            if let Some(delay) = xmpp_parsers::delay::Delay::try_from(payload).ok() {
                                timestamp = Some(delay.stamp.0);
                            }
                        }
                        let message = Message::incoming_groupchat(id, timestamp.unwrap_or(LocalTz::now().into()), &from, &to, &body.0);
                        self.schedule(Event::Message(message));
                    },
                    _ => {},
                }
            }

            for payload in message.payloads.iter().cloned() {
                if let Some(received) = xmpp_parsers::carbons::Received::try_from(payload).ok() {
                    if let Some(ref original) = received.forwarded.stanza {
                        if original.type_ != XmppParsersMessageType::Error {
                            if let (Some(from), Some(to)) = (original.from.as_ref(), original.to.as_ref()) {
                                if let Some(body) = original.bodies.get("") {
                                    let id = original.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
                                    let timestamp = LocalTz::now().into();
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
