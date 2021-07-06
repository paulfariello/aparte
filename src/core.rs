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
use crate::color;
use crate::command::{Command, CommandParser};
use crate::conversation::{Channel, Conversation};
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
    RawCommand(Option<Account>, String, String),
    Command(Command),
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
    Leave(Channel),
    Iq(Account, iq::Iq),
    Disco(Account),
    PubSub(Account, PubSubEvent),
    Presence(Account, presence::Presence),
    ReadPassword(Command),
    Win(String),
    Close(String),
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
        context: String,
        raw_buf: String,
        cursor: Cursor,
    },
    ResetCompletion,
    Completed(String, Cursor),
    ChangeWindow(String),
    Notification(String),
    Subject(Account, Jid, HashMap<String, String>),
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
    Correction(mods::correction::CorrectionMod),
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
from_mod!(Messages, mods::messages::MessagesMod);
from_mod!(Correction, mods::correction::CorrectionMod);

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
            Mod::Correction(r#mod) => r#mod.init(aparte),
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
            Mod::Correction(r#mod) => r#mod.on_event(aparte, event),
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
            Mod::Correction(r#mod) => {
                r#mod.can_handle_xmpp_message(aparte, account, message, delay)
            }
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
            Mod::Correction(r#mod) => r#mod.handle_xmpp_message(aparte, account, message, delay),
        }
    }
}

impl fmt::Debug for Mod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Mod::Completion(_) => f.write_str("Mod::Completion"),
            Mod::Carbons(_) => f.write_str("Mod::Carbons"),
            Mod::Contact(_) => f.write_str("Mod::Contact"),
            Mod::Conversation(_) => f.write_str("Mod::Conversation"),
            Mod::Disco(_) => f.write_str("Mod::Disco"),
            Mod::Bookmarks(_) => f.write_str("Mod::Bookmarks"),
            Mod::UI(_) => f.write_str("Mod::UI"),
            Mod::Mam(_) => f.write_str("Mod::Mam"),
            Mod::Messages(_) => f.write_str("Mod::Messages"),
            Mod::Correction(_) => f.write_str("Mod::Correction"),
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
            Mod::Correction(r#mod) => r#mod.fmt(f),
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
            let ui = aparte.get_mod::<mods::ui::UIMod>();
            ui.get_windows()
        })
    }
},
|aparte, _command| {
    aparte.schedule(Event::Win(window.clone()));
    Ok(())
});

command_def!(close,
r#"Usage: /close [<window>]

    window        Name of the window to close

Description:
    Close the current or a given window.

Examples:
    /close
    /close contact@server.tld"#,
{
    window: Option<String> = {
        completion: (|aparte, _command| {
            let ui = aparte.get_mod::<mods::ui::UIMod>();
            ui.get_windows()
        })
    }
},
|aparte, _command| {
    let current =  {
        let ui = aparte.get_mod::<mods::ui::UIMod>();
        ui.current_window().cloned()
    };
    let window = window.or(current).clone();
    if let Some(window) = window {
        // Close window
        aparte.schedule(Event::Close(window.clone()));
    }
    Ok(())
});

command_def!(leave,
r#"Usage: /leave [<window>]

    window        Name of the channel to leave

Description:
    Close the current or a given channel.

Examples:
    /leave
    /leave channel@conversation.server.tld"#,
{
    window: Option<String> = {
        completion: (|aparte, _command| {
            let ui = aparte.get_mod::<mods::ui::UIMod>();
            ui.get_windows().iter().map(|window| {
                if let Some(account) = aparte.current_account() {
                    if let Ok(jid) = BareJid::from_str(&window) {
                        let conversation_mod = aparte.get_mod::<mods::conversation::ConversationMod>();
                        conversation_mod.get(&account, &jid).cloned()
                    } else {
                        None
                    }
                } else {
                    None
                }
            }).filter_map(|conversation| {
                if let Some(Conversation::Channel(channel)) = conversation {
                    Some(channel.jid.into())
                } else {
                    None
                }
            }).collect()
        })
    }
},
|aparte, _command| {
    let current =  {
        let ui = aparte.get_mod::<mods::ui::UIMod>();
        ui.current_window().cloned()
    };
    let window = window.or(current).clone();
    if let Some(window) = window {
        if let Some(account) = aparte.current_account() {
            if let Ok(jid) = BareJid::from_str(&window) {
                let conversation =  {
                    let conversation_mod = aparte.get_mod::<mods::conversation::ConversationMod>();
                    conversation_mod.get(&account, &jid).cloned()
                };
                if let Some(Conversation::Channel(channel)) = conversation {
                    aparte.schedule(Event::Leave(channel));
                }
            }
        }
    }
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
            let contact = aparte.get_mod::<mods::contact::ContactMod>();
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
            if let Some(body) = message {
                let mut bodies = HashMap::new();
                bodies.insert("".to_string(), body);
                let id = Uuid::new_v4().to_string();
                let from: Jid = account.clone().into();
                let timestamp = LocalTz::now();
                let message = Message::outgoing_chat(id, timestamp.into(), &from, &jid, &bodies);
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
            let bookmarks = aparte.get_mod::<mods::bookmarks::BookmarksMod>();
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
                let bookmarks = aparte.get_mod::<mods::bookmarks::BookmarksMod>();
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

mod me {
    use chrono::Local as LocalTz;
    use std::collections::HashMap;
    use std::str::FromStr;
    use uuid::Uuid;
    use xmpp_parsers::{BareJid, Jid};

    use crate::account::Account;
    use crate::command::*;
    use crate::conversation::Conversation;
    use crate::core::{Aparte, Event};
    use crate::message::Message;
    use crate::mods;

    fn parse(account: &Option<Account>, context: &str, buf: &str) -> Result<Command, String> {
        Ok(Command {
            account: account.clone(),
            context: context.to_string(),
            args: vec![buf.to_string()],
            cursor: 0,
        })
    }

    fn exec(aparte: &mut Aparte, command: Command) -> Result<(), String> {
        let account = command
            .account
            .ok_or("Can't use /me in non XMPP window".to_string())?;
        let jid = BareJid::from_str(&command.context)
            .map_err(|_| "Can't use /me in non XMPP window".to_string())?;
        let message = {
            let conversation = aparte.get_mod::<mods::conversation::ConversationMod>();
            if let Some(conversation) = conversation.get(&account, &jid) {
                match conversation {
                    Conversation::Chat(chat) => {
                        let account = &chat.account;
                        let us = account.clone().into();
                        let from: Jid = us;
                        let to: Jid = chat.contact.clone().into();
                        let id = Uuid::new_v4();
                        let timestamp = LocalTz::now().into();
                        let mut bodies = HashMap::new();
                        bodies.insert("".to_string(), command.args[0].clone());
                        Ok(Message::outgoing_chat(
                            id.to_string(),
                            timestamp,
                            &from,
                            &to,
                            &bodies,
                        ))
                    }
                    Conversation::Channel(channel) => {
                        let account = &channel.account;
                        let mut us = account.clone();
                        us.resource = channel.nick.clone();
                        let from: Jid = us.into();
                        let to: Jid = channel.jid.clone().into();
                        let id = Uuid::new_v4();
                        let timestamp = LocalTz::now().into();
                        let mut bodies = HashMap::new();
                        bodies.insert("".to_string(), command.args[0].clone());
                        Ok(Message::outgoing_channel(
                            id.to_string(),
                            timestamp,
                            &from,
                            &to,
                            &bodies,
                        ))
                    }
                }
            } else {
                Err(format!("Unknown context {}", command.context))
            }
        }?;
        aparte.schedule(Event::SendMessage(account, message));
        Ok(())
    }

    pub fn new() -> CommandParser {
        CommandParser {
            name: "me",
            help: r#"/me message

    message       Message to be sent

Description:
    Send a /me message

Examples:
    /me loves Aparté"#
                .to_string(),
            parse,
            exec,
            autocompletions: vec![],
        }
    }
}

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

        let mut aparte = Self {
            command_parsers: Rc::new(HashMap::new()),
            mods: Rc::new(HashMap::new()),
            connections: HashMap::new(),
            current_connection: None,
            event_queue: Vec::new(),
            send_queue: VecDeque::new(),
            event_channel: None,
            config: config,
        };

        aparte.add_mod(Mod::Completion(mods::completion::CompletionMod::new()));
        aparte.add_mod(Mod::Carbons(mods::carbons::CarbonsMod::new()));
        aparte.add_mod(Mod::Contact(mods::contact::ContactMod::new()));
        aparte.add_mod(Mod::Conversation(mods::conversation::ConversationMod::new()));
        aparte.add_mod(Mod::Disco(mods::disco::DiscoMod::new()));
        aparte.add_mod(Mod::Bookmarks(mods::bookmarks::BookmarksMod::new()));
        aparte.add_mod(Mod::UI(mods::ui::UIMod::new()));
        aparte.add_mod(Mod::Mam(mods::mam::MamMod::new()));
        aparte.add_mod(Mod::Messages(mods::messages::MessagesMod::new()));
        aparte.add_mod(Mod::Correction(mods::correction::CorrectionMod::new()));

        aparte
    }

    pub fn add_command(&mut self, command_parser: CommandParser) {
        let command_parsers = Rc::get_mut(&mut self.command_parsers).unwrap();
        command_parsers.insert(command_parser.name.to_string(), command_parser);
    }

    pub fn handle_raw_command(
        &mut self,
        account: &Option<Account>,
        context: &String,
        buf: &String,
    ) -> Result<(), String> {
        let command_name = Command::parse_name(&buf)?;

        let parser = {
            match self.command_parsers.get(command_name) {
                Some(parser) => parser.clone(),
                None => return Err(format!("Unknown command {}", command_name)),
            }
        };

        let command = (parser.parse)(account, context, buf)?;
        (parser.exec)(self, command)
    }

    pub fn handle_command(&mut self, command: Command) -> Result<(), String> {
        let parser = {
            match self.command_parsers.get(&command.args[0]) {
                Some(parser) => parser.clone(),
                None => return Err(format!("Unknown command {}", command.args[0])),
            }
        };

        (parser.exec)(self, command)
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
            Mod::Correction(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::correction::CorrectionMod>(),
                    RefCell::new(Mod::Correction(r#mod)),
                );
            }
        }
    }

    pub fn get_mod<'a, T>(&'a self) -> Ref<'a, T>
    where
        T: 'static,
        for<'b> &'b T: From<&'b Mod>,
    {
        match self.mods.get(&TypeId::of::<T>()) {
            Some(r#mod) => Ref::map(r#mod.borrow(), |m| m.into()),
            None => unreachable!(),
        }
    }

    pub fn get_mod_mut<T: 'static>(&self) -> RefMut<T>
    where
        T: 'static,
        for<'b> &'b mut T: From<&'b mut Mod>,
    {
        match self.mods.get(&TypeId::of::<T>()) {
            Some(r#mod) => RefMut::map(r#mod.borrow_mut(), |m| m.into()),
            None => unreachable!(),
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
        self.add_command(close::new());
        self.add_command(leave::new());
        self.add_command(msg::new());
        self.add_command(join::new());
        self.add_command(quit::new());
        self.add_command(me::new());

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
            let ui = self.get_mod::<mods::ui::UIMod>();
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
        self.log(color::rainbow(WELCOME));
        self.log(format!("Version: {}", VERSION));

        for (_, account) in self.config.accounts.clone() {
            if account.autoconnect {
                self.schedule(Event::RawCommand(
                    None,
                    "console".to_string(),
                    format!("/connect {}", account.jid),
                ));
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
                    .map(char::from)
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
                Event::Command(command) => match self.handle_command(command) {
                    Err(err) => self.log(err),
                    Ok(()) => {}
                },
                Event::RawCommand(account, context, buf) => {
                    match self.handle_raw_command(&account, &context, &buf) {
                        Err(err) => self.log(err),
                        Ok(()) => {}
                    }
                }
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
                Event::Leave(channel) => {
                    // Send presence in the channel
                    let mut presence = Presence::new(PresenceType::Unavailable);
                    presence = presence.with_to(channel.jid.clone());
                    presence = presence.with_from(channel.account.clone());
                    presence.add_payload(Muc::new());
                    self.send(&channel.account, presence.into());
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
            debug!("Handling xmpp message by {:?}", r#mod);
            r#mod
                .borrow_mut()
                .handle_xmpp_message(self, &account, &message, &delay);
        } else {
            info!("Don't know how to handle message: {:?}", message);
        }
    }
}
