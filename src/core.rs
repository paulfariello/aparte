/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use chrono::{DateTime, FixedOffset, Local as LocalTz};
use core::fmt::Debug;
use futures::sink::SinkExt;
use futures::stream::StreamExt;
use rand::{self, Rng};
use std::any::TypeId;
use std::cell::{Ref, RefCell, RefMut};
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
use tokio::signal::unix;
use tokio::sync::mpsc;
use tokio::task;
use tokio_xmpp::{
    AsyncClient as TokioXmppClient, Error as XmppError, Event as XmppEvent, Packet as XmppPacket,
};
use uuid::Uuid;
use xmpp_parsers;
use xmpp_parsers::delay::Delay;
use xmpp_parsers::iq::{Iq, IqType};
use xmpp_parsers::message::Message as XmppParsersMessage;
use xmpp_parsers::muc::Muc;
use xmpp_parsers::presence::{Presence, Show as PresenceShow, Type as PresenceType};
use xmpp_parsers::pubsub::event::PubSubEvent;
use xmpp_parsers::{iq, presence, BareJid, Element, FullJid, Jid};

use crate::account::{Account, ConnectionInfo};
use crate::command::{Command, CommandParser};
use crate::config::Config;
use crate::cursor::Cursor;
use crate::message::Message;
use crate::mods;
use crate::{
    command_def, generate_arg_autocompletion, generate_command_autocompletions, generate_help,
    parse_command_args,
};
use crate::{contact, conversation};

const WELCOME: &str = r#"
▌ ▌   ▜               ▐      ▞▀▖         ▐   ▞
▌▖▌▞▀▖▐ ▞▀▖▞▀▖▛▚▀▖▞▀▖ ▜▀ ▞▀▖ ▙▄▌▛▀▖▝▀▖▙▀▖▜▀ ▞▀▖
▙▚▌▛▀ ▐ ▌ ▖▌ ▌▌▐ ▌▛▀  ▐ ▖▌ ▌ ▌ ▌▙▄▘▞▀▌▌  ▐ ▖▛▀
▘ ▘▝▀▘ ▘▝▀ ▝▀ ▘▝ ▘▝▀▘  ▀ ▝▀  ▘ ▘▌  ▝▀▘▘   ▀ ▝▀▘
"#;
const VERSION: &'static str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub enum Event {
    Start,
    Connect(ConnectionInfo, Password<String>),
    Connected(Account, Jid),
    Disconnected(Account, String),
    AuthError(Account, String),
    Stanza(Account, Element),
    RawMessage(Account, XmppParsersMessage, Option<Delay>),
    Command(Command),
    CommandError(String),
    SendMessage(Account, Message),
    Message(Option<Account>, Message),
    Chat {
        account: Account,
        contact: BareJid,
    },
    Join {
        account: FullJid,
        channel: Jid,
        user_request: bool,
    },
    Joined {
        account: FullJid,
        channel: FullJid,
        user_request: bool,
    },
    Iq(Account, iq::Iq),
    Disco(Account),
    PubSub(Account, PubSubEvent),
    Presence(Account, presence::Presence),
    ReadPassword(Command),
    Win(String),
    Contact(Account, contact::Contact),
    ContactUpdate(Account, contact::Contact),
    Bookmark(contact::Bookmark),
    DeletedBookmark(BareJid),
    Occupant {
        account: Account,
        conversation: BareJid,
        occupant: conversation::Occupant,
    },
    WindowChange,
    LoadChannelHistory {
        account: Account,
        jid: BareJid,
        from: Option<DateTime<FixedOffset>>,
    },
    LoadChatHistory {
        account: Account,
        contact: BareJid,
        from: Option<DateTime<FixedOffset>>,
    },
    Quit,
    Key(Key),
    AutoComplete {
        account: Option<Account>,
        conversation: Option<BareJid>,
        raw_buf: String,
        cursor: Cursor,
    },
    ResetCompletion,
    Completed(String, Cursor),
    ChangeWindow(String),
    Notification(String),
}

pub enum Mod {
    Messages(mods::messages::MessagesMod),
    Completion(mods::completion::CompletionMod),
    Carbons(mods::carbons::CarbonsMod),
    Contact(mods::contact::ContactMod),
    Conversation(mods::conversation::ConversationMod),
    Disco(mods::disco::DiscoMod),
    Bookmarks(mods::bookmarks::BookmarksMod),
    UI(mods::ui::UIMod),
    Mam(mods::mam::MamMod),
}

macro_rules! from_mod {
    ($enum:ident, $type:path) => {
        impl<'a> From<&'a Mod> for &'a $type {
            fn from(r#mod: &'a Mod) -> &'a $type {
                match r#mod {
                    Mod::$enum(r#mod) => r#mod,
                    _ => unreachable!(),
                }
            }
        }

        impl<'a> From<&'a mut Mod> for &'a mut $type {
            fn from(r#mod: &'a mut Mod) -> &'a mut $type {
                match r#mod {
                    Mod::$enum(r#mod) => r#mod,
                    _ => unreachable!(),
                }
            }
        }
    };
}

from_mod!(Completion, mods::completion::CompletionMod);
from_mod!(Carbons, mods::carbons::CarbonsMod);
from_mod!(Contact, mods::contact::ContactMod);
from_mod!(Conversation, mods::conversation::ConversationMod);
from_mod!(Disco, mods::disco::DiscoMod);
from_mod!(Bookmarks, mods::bookmarks::BookmarksMod);
from_mod!(UI, mods::ui::UIMod);
from_mod!(Mam, mods::mam::MamMod);

pub trait ModTrait: fmt::Display {
    fn init(&mut self, aparte: &mut Aparte) -> Result<(), ()>;
    fn on_event(&mut self, aparte: &mut Aparte, event: &Event);
    /// Return weither this message can be handled
    /// 0 means no, 1 mean definitely yes
    fn can_handle_xmpp_message(
        &mut self,
        _aparte: &mut Aparte,
        _account: &Account,
        _message: &XmppParsersMessage,
        _delay: &Option<Delay>,
    ) -> f64 {
        0f64
    }

    /// Handle message
    fn handle_xmpp_message(
        &mut self,
        _aparte: &mut Aparte,
        _account: &Account,
        _message: &XmppParsersMessage,
        _delay: &Option<Delay>,
    ) {
    }
}

impl ModTrait for Mod {
    fn init(&mut self, aparte: &mut Aparte) -> Result<(), ()> {
        match self {
            Mod::Completion(r#mod) => r#mod.init(aparte),
            Mod::Carbons(r#mod) => r#mod.init(aparte),
            Mod::Contact(r#mod) => r#mod.init(aparte),
            Mod::Conversation(r#mod) => r#mod.init(aparte),
            Mod::Disco(r#mod) => r#mod.init(aparte),
            Mod::Bookmarks(r#mod) => r#mod.init(aparte),
            Mod::UI(r#mod) => r#mod.init(aparte),
            Mod::Mam(r#mod) => r#mod.init(aparte),
            Mod::Messages(r#mod) => r#mod.init(aparte),
        }
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match self {
            Mod::Completion(r#mod) => r#mod.on_event(aparte, event),
            Mod::Carbons(r#mod) => r#mod.on_event(aparte, event),
            Mod::Contact(r#mod) => r#mod.on_event(aparte, event),
            Mod::Conversation(r#mod) => r#mod.on_event(aparte, event),
            Mod::Disco(r#mod) => r#mod.on_event(aparte, event),
            Mod::Bookmarks(r#mod) => r#mod.on_event(aparte, event),
            Mod::UI(r#mod) => r#mod.on_event(aparte, event),
            Mod::Mam(r#mod) => r#mod.on_event(aparte, event),
            Mod::Messages(r#mod) => r#mod.on_event(aparte, event),
        }
    }

    fn can_handle_xmpp_message(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        delay: &Option<Delay>,
    ) -> f64 {
        match self {
            Mod::Completion(r#mod) => {
                r#mod.can_handle_xmpp_message(aparte, account, message, delay)
            }
            Mod::Carbons(r#mod) => r#mod.can_handle_xmpp_message(aparte, account, message, delay),
            Mod::Contact(r#mod) => r#mod.can_handle_xmpp_message(aparte, account, message, delay),
            Mod::Conversation(r#mod) => {
                r#mod.can_handle_xmpp_message(aparte, account, message, delay)
            }
            Mod::Disco(r#mod) => r#mod.can_handle_xmpp_message(aparte, account, message, delay),
            Mod::Bookmarks(r#mod) => r#mod.can_handle_xmpp_message(aparte, account, message, delay),
            Mod::UI(r#mod) => r#mod.can_handle_xmpp_message(aparte, account, message, delay),
            Mod::Mam(r#mod) => r#mod.can_handle_xmpp_message(aparte, account, message, delay),
            Mod::Messages(r#mod) => r#mod.can_handle_xmpp_message(aparte, account, message, delay),
        }
    }

    fn handle_xmpp_message(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        delay: &Option<Delay>,
    ) {
        match self {
            Mod::Completion(r#mod) => r#mod.handle_xmpp_message(aparte, account, message, delay),
            Mod::Carbons(r#mod) => r#mod.handle_xmpp_message(aparte, account, message, delay),
            Mod::Contact(r#mod) => r#mod.handle_xmpp_message(aparte, account, message, delay),
            Mod::Conversation(r#mod) => r#mod.handle_xmpp_message(aparte, account, message, delay),
            Mod::Disco(r#mod) => r#mod.handle_xmpp_message(aparte, account, message, delay),
            Mod::Bookmarks(r#mod) => r#mod.handle_xmpp_message(aparte, account, message, delay),
            Mod::UI(r#mod) => r#mod.handle_xmpp_message(aparte, account, message, delay),
            Mod::Mam(r#mod) => r#mod.handle_xmpp_message(aparte, account, message, delay),
            Mod::Messages(r#mod) => r#mod.handle_xmpp_message(aparte, account, message, delay),
        }
    }
}

impl fmt::Display for Mod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Mod::Completion(r#mod) => r#mod.fmt(f),
            Mod::Carbons(r#mod) => r#mod.fmt(f),
            Mod::Contact(r#mod) => r#mod.fmt(f),
            Mod::Conversation(r#mod) => r#mod.fmt(f),
            Mod::Disco(r#mod) => r#mod.fmt(f),
            Mod::Bookmarks(r#mod) => r#mod.fmt(f),
            Mod::UI(r#mod) => r#mod.fmt(f),
            Mod::Mam(r#mod) => r#mod.fmt(f),
            Mod::Messages(r#mod) => r#mod.fmt(f),
        }
    }
}

#[derive(Debug, Clone)]
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
    mods: Rc<HashMap<TypeId, RefCell<Mod>>>,
    connections: HashMap<Account, Connection>,
    current_connection: Option<Account>,
    event_queue: Vec<Event>,
    send_queue: VecDeque<(Account, Element)>,
    event_channel: Option<mpsc::Sender<Event>>,
    /// Aparté main configuration
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
        } else if !account_name.contains("@") {
            return Err(format!("Unknown account or invalid jid {}", account_name));
        } else if let Ok(jid) = Jid::from_str(&account_name) {
            ConnectionInfo {
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
            let ui = aparte.get_mod::<mods::ui::UIMod>().unwrap();
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
            let contact = aparte.get_mod::<mods::contact::ContactMod>().unwrap();
            contact.contacts.iter().map(|(_, contact)| contact.jid.to_string()).collect()
        })
    },
    message: Option<String>
},
|aparte, _command| {
    let account = aparte.current_account().ok_or(format!("No connection found"))?;
    match Jid::from_str(&contact.clone()) {
        Ok(jid) => {
            let to = match jid.clone() {
                Jid::Bare(jid) => jid,
                Jid::Full(jid) => jid.into(),
            };
            aparte.schedule(Event::Chat { account: account.clone(), contact: to });
            if message.is_some() {
                let id = Uuid::new_v4().to_string();
                let from: Jid = account.clone().into();
                let timestamp = LocalTz::now();
                let message = Message::outgoing_chat(id, timestamp.into(), &from, &jid, &message.unwrap());
                aparte.schedule(Event::Message(Some(account.clone()), message.clone()));

                aparte.send(&account, Element::try_from(message).unwrap());
            }
            Ok(())
        },
        Err(err) => {
            Err(format!("Invalid JID {}: {}", contact, err))
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
            let bookmarks = aparte.get_mod::<mods::bookmarks::BookmarksMod>().unwrap();
            bookmarks.bookmarks_by_name.iter().map(|(a, _)| a.clone()).chain(bookmarks.bookmarks_by_jid.iter().map(|(a, _)| a.to_string())).collect()
        })
    },
},
|aparte, _command| {
    let account = aparte.current_account().ok_or(format!("No connection found"))?;
    match Jid::from_str(&muc) {
        Ok(jid) => {
            aparte.schedule(Event::Join {
                account,
                channel: jid,
                user_request: true
            });
            Ok(())
        },
        Err(_) => {
            let jid = {
                let bookmarks = aparte.get_mod::<mods::bookmarks::BookmarksMod>().unwrap();
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
                    aparte.schedule(Event::Join {
                        account,
                        channel: jid,
                        user_request: true
                    });
                    Ok(())
                },
                Err(e) => Err(e),
            }
        }
    }
});

command_def!(
    quit,
    r#"/quit

Description:
    Quit Aparté.

Example:
    /quit"#,
    {},
    |aparte, _command| {
        aparte.schedule(Event::Quit);

        Ok(())
    }
);

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
        let help = match aparte.command_parsers.get(&cmd) {
            Some(command) => Ok(command.help.to_string()),
            None => Err(format!("Unknown command {}", cmd)),
        }?;

        aparte.log(help);
        Ok(())
    } else {
        aparte.log(format!("Available commands: {}", aparte.command_parsers.iter().map(|c| c.0.to_string()).collect::<Vec<String>>().join(", ")));
        Ok(())
    }
});

impl Aparte {
    pub fn new(config_path: PathBuf) -> Self {
        let mut config_file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(config_path)
        {
            Err(err) => panic!("Cannot read config file {}", err),
            Ok(config_file) => config_file,
        };

        let mut config_str = String::new();
        if let Err(e) = config_file.read_to_string(&mut config_str) {
            panic!("Cannot read config file {}", e);
        }

        let config = match config_str.len() {
            0 => Config {
                accounts: HashMap::new(),
            },
            _ => match toml::from_str(&config_str) {
                Err(err) => {
                    error!("Malformed config file: {}", err);
                    Config {
                        accounts: HashMap::new(),
                    }
                }
                Ok(config) => config,
            },
        };

        Self {
            command_parsers: Rc::new(HashMap::new()),
            mods: Rc::new(HashMap::new()),
            connections: HashMap::new(),
            current_connection: None,
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

    pub fn add_mod(&mut self, r#mod: Mod) {
        info!("Add mod `{}`", r#mod);
        let mods = Rc::get_mut(&mut self.mods).unwrap();
        // TODO ensure mod is not inserted twice
        match r#mod {
            Mod::Completion(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::completion::CompletionMod>(),
                    RefCell::new(Mod::Completion(r#mod)),
                );
            }
            Mod::Carbons(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::carbons::CarbonsMod>(),
                    RefCell::new(Mod::Carbons(r#mod)),
                );
            }
            Mod::Contact(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::contact::ContactMod>(),
                    RefCell::new(Mod::Contact(r#mod)),
                );
            }
            Mod::Conversation(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::conversation::ConversationMod>(),
                    RefCell::new(Mod::Conversation(r#mod)),
                );
            }
            Mod::Disco(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::disco::DiscoMod>(),
                    RefCell::new(Mod::Disco(r#mod)),
                );
            }
            Mod::Bookmarks(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::bookmarks::BookmarksMod>(),
                    RefCell::new(Mod::Bookmarks(r#mod)),
                );
            }
            Mod::UI(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::ui::UIMod>(),
                    RefCell::new(Mod::UI(r#mod)),
                );
            }
            Mod::Mam(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::mam::MamMod>(),
                    RefCell::new(Mod::Mam(r#mod)),
                );
            }
            Mod::Messages(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::messages::MessagesMod>(),
                    RefCell::new(Mod::Messages(r#mod)),
                );
            }
        }
    }

    pub fn get_mod<'a, T>(&'a self) -> Option<Ref<'a, T>>
    where
        T: 'static,
        for<'b> &'b T: From<&'b Mod>,
    {
        match self.mods.get(&TypeId::of::<T>()) {
            Some(r#mod) => Some(Ref::map(r#mod.borrow(), |m| m.into())),
            None => None,
        }
    }

    pub fn get_mod_mut<T: 'static>(&self) -> Option<RefMut<T>>
    where
        T: 'static,
        for<'b> &'b mut T: From<&'b mut Mod>,
    {
        match self.mods.get(&TypeId::of::<T>()) {
            Some(r#mod) => Some(RefMut::map(r#mod.borrow_mut(), |m| m.into())),
            None => None,
        }
    }

    pub fn add_connection(&mut self, account: Account, sink: mpsc::Sender<Element>) {
        let connection = Connection {
            account: account.clone(),
            sink,
        };

        self.connections.insert(account.clone(), connection);
        self.current_connection = Some(account.clone());
    }

    pub fn current_account(&self) -> Option<Account> {
        self.current_connection.clone()
    }

    pub fn init(&mut self) -> Result<(), ()> {
        self.add_command(help::new());
        self.add_command(connect::new());
        self.add_command(win::new());
        self.add_command(msg::new());
        self.add_command(join::new());
        self.add_command(quit::new());

        let mods = Rc::clone(&self.mods);
        for (_, r#mod) in mods.iter() {
            if let Err(err) = r#mod.borrow_mut().init(self) {
                return Err(err);
            }
        }

        Ok(())
    }

    pub fn run(mut self) {
        let mut input_event_stream = {
            let ui = self.get_mod::<mods::ui::UIMod>().unwrap();
            ui.event_stream()
        };

        let (tx, mut rx) = mpsc::channel(32);
        let tx_for_signal = tx.clone();
        let tx_for_event = tx.clone();
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
                    }
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
                    cursor: 0,
                }));
            }
        }
    }

    pub fn send(&mut self, account: &Account, stanza: Element) {
        self.send_queue.push_back((account.clone(), stanza));
    }

    async fn send_loop(&mut self) {
        for (account, stanza) in self.send_queue.drain(..) {
            let mut raw = Vec::<u8>::new();
            stanza.write_to(&mut raw).unwrap();
            debug!("SEND: {}", String::from_utf8(raw).unwrap());
            match self.connections.get_mut(&account) {
                Some(connection) => {
                    if let Err(e) = connection.sink.send(stanza).await {
                        warn!("Cannot send stanza: {}", e);
                    }
                }
                None => {
                    warn!("No connection found for {}", account);
                }
            }
        }
    }

    pub async fn connect(&mut self, connection_info: &ConnectionInfo, password: Password<String>) {
        let account: Account = match Jid::from_str(&connection_info.jid) {
            Ok(Jid::Full(jid)) => jid,
            Ok(Jid::Bare(jid)) => {
                let rand_string: String = rand::thread_rng()
                    .sample_iter(&rand::distributions::Alphanumeric)
                    .take(5)
                    .collect();
                jid.with_resource(format!("aparte_{}", rand_string))
            }
            Err(err) => {
                self.log(format!(
                    "Cannot connect as {}: {}",
                    connection_info.jid, err
                ));
                return;
            }
        };

        self.log(format!("Connecting as {}", account));
        let mut client = match TokioXmppClient::new(&account.to_string(), &password.0) {
            Ok(client) => client,
            Err(err) => {
                self.log(format!("Cannot connect as {}: {}", account, err));
                return;
            }
        };

        client.set_reconnect(true);

        let (connection_channel, mut rx) = mpsc::channel(32);

        self.add_connection(account.clone(), connection_channel);

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

        let event_channel = match &self.event_channel {
            Some(event_channel) => event_channel.clone(),
            None => unreachable!(),
        };

        let reconnect = true;
        task::spawn_local(async move {
            while let Some(event) = reader.next().await {
                debug!("XMPP Event: {:?}", event);
                match event {
                    XmppEvent::Disconnected(XmppError::Auth(e)) => {
                        if let Err(err) = event_channel
                            .send(Event::AuthError(account.clone(), format!("{}", e)))
                            .await
                        {
                            error!("Cannot send event to internal channel: {}", err);
                        };
                        break;
                    }
                    XmppEvent::Disconnected(e) => {
                        if let Err(err) = event_channel
                            .send(Event::Disconnected(account.clone(), format!("{}", e)))
                            .await
                        {
                            error!("Cannot send event to internal channel: {}", err);
                        };
                        if !reconnect {
                            break;
                        }
                    }
                    XmppEvent::Online {
                        bound_jid: jid,
                        resumed: true,
                    } => {
                        debug!("Reconnected to {}", jid);
                    }
                    XmppEvent::Online {
                        bound_jid: jid,
                        resumed: false,
                    } => {
                        if let Err(err) = event_channel
                            .send(Event::Connected(account.clone(), jid))
                            .await
                        {
                            error!("Cannot send event to internal channel: {}", err);
                            break;
                        }
                    }
                    XmppEvent::Stanza(stanza) => {
                        debug!("RECV: {}", String::from(&stanza));
                        if let Err(err) = event_channel
                            .send(Event::Stanza(account.clone(), stanza))
                            .await
                        {
                            error!("Cannot send stanza to internal channel: {}", err);
                            break;
                        }
                    }
                }
            }
        });
    }

    pub async fn event_loop(&mut self) -> Result<(), ()> {
        while self.event_queue.len() > 0 {
            let event = self.event_queue.remove(0);
            debug!("Event: {:?}", event);
            {
                let mods = Rc::clone(&self.mods);
                for (_, r#mod) in mods.iter() {
                    r#mod.borrow_mut().on_event(self, &event);
                }
                self.send_loop().await;
            }

            match event {
                Event::Start => {
                    self.start();
                }
                Event::Command(command) => match self.parse_command(command) {
                    Err(err) => self.log(err),
                    Ok(()) => {}
                },
                Event::SendMessage(account, message) => {
                    self.schedule(Event::Message(Some(account.clone()), message.clone()));
                    if let Ok(xmpp_message) = Element::try_from(message) {
                        self.send(&account, xmpp_message);
                    }
                }
                Event::Connect(account, password) => {
                    self.connect(&account, password).await;
                }
                Event::Connected(account, _) => {
                    self.log(format!("Connected as {}", account));
                    let mut presence = Presence::new(PresenceType::None);
                    presence.show = Some(PresenceShow::Chat);

                    self.send(&account, presence.into());
                }
                Event::Disconnected(account, err) => {
                    self.log(format!("Connection lost for {}: {}", account, err));
                }
                Event::AuthError(account, err) => {
                    self.log(format!("Authentication error for {}: {}", account, err));
                }
                Event::Stanza(account, stanza) => {
                    self.handle_stanza(account, stanza);
                }
                Event::RawMessage(account, message, delay) => {
                    self.handle_xmpp_message(account, message, delay);
                }
                Event::Join {
                    account,
                    channel,
                    user_request,
                } => {
                    let to = match channel.clone() {
                        Jid::Full(jid) => jid,
                        Jid::Bare(jid) => {
                            let node = account.node.clone().unwrap();
                            jid.with_resource(node)
                        }
                    };
                    let from: Jid = account.clone().into();

                    // Send presence in the channel
                    let mut presence = Presence::new(PresenceType::None);
                    presence = presence.with_to(Jid::Full(to.clone()));
                    presence = presence.with_from(from);
                    presence.add_payload(Muc::new());
                    self.send(&account, presence.into());

                    // Successful join
                    self.log(format!("Joined {}", channel));
                    self.schedule(Event::Joined {
                        account: account.clone(),
                        channel: to,
                        user_request,
                    });
                }
                Event::Quit => {
                    return Err(());
                }
                _ => {}
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
        self.schedule(Event::Message(None, message));
    }

    fn handle_stanza(&mut self, account: Account, stanza: Element) {
        if let Ok(message) = XmppParsersMessage::try_from(stanza.clone()) {
            self.handle_xmpp_message(account, message, None);
        } else if let Ok(iq) = Iq::try_from(stanza.clone()) {
            if let IqType::Error(stanza) = iq.payload.clone() {
                if let Some(text) = stanza.texts.get("en") {
                    let message = Message::log(text.clone());
                    self.schedule(Event::Message(Some(account.clone()), message));
                }
            }
            self.schedule(Event::Iq(account, iq));
        } else if let Ok(presence) = Presence::try_from(stanza.clone()) {
            self.schedule(Event::Presence(account, presence));
        }
    }

    fn handle_xmpp_message(
        &mut self,
        account: Account,
        message: XmppParsersMessage,
        delay: Option<Delay>,
    ) {
        let mut best_match = 0f64;
        let mut matched_mod = None;

        let mods = Rc::clone(&self.mods);
        for (_, r#mod) in mods.iter() {
            let message_match = r#mod
                .borrow_mut()
                .can_handle_xmpp_message(self, &account, &message, &delay);
            if message_match > best_match {
                matched_mod = Some(r#mod);
                best_match = message_match;
            }
        }

        if let Some(r#mod) = matched_mod {
            r#mod
                .borrow_mut()
                .handle_xmpp_message(self, &account, &message, &delay);
        } else {
            info!("Unknown message: {:?}", message);
        }
    }
}
