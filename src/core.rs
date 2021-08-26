/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use chrono::{DateTime, FixedOffset, Local as LocalTz};
use core::fmt::Debug;
use futures::sink::SinkExt;
use futures::stream::StreamExt;
use linked_hash_map::LinkedHashMap;
use rand::{self, Rng};
use std::any::TypeId;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use std::future::Future;
use std::task::Waker;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc};
use termion::event::Key;
use tokio::runtime::Runtime as TokioRuntime;
use tokio::signal::unix;
use tokio::sync::{mpsc, RwLock, RwLockReadGuard, RwLockWriteGuard, RwLockMappedWriteGuard};
use tokio::task;
use tokio_xmpp::{
    AsyncClient as TokioXmppClient, Error as XmppError, Event as XmppEvent, Packet as XmppPacket,
};
use uuid::Uuid;

use xmpp_parsers::hashes as xmpp_hashes;
use xmpp_parsers::caps::{self, Caps};
use xmpp_parsers::delay::Delay;
use xmpp_parsers::iq::{Iq, IqType};
use xmpp_parsers::message::Message as XmppParsersMessage;
use xmpp_parsers::muc::Muc;
use xmpp_parsers::presence::{Presence, Show as PresenceShow, Type as PresenceType};
use xmpp_parsers::pubsub::event::PubSubEvent;
use xmpp_parsers::stanza_error::StanzaError;
use xmpp_parsers::{iq, presence, BareJid, Element, FullJid, Jid};

use crate::account::{Account, ConnectionInfo};
use crate::async_iq::IqFuture;
use crate::color;
use crate::command::{Command, CommandParser};
use crate::config::Config;
use crate::conversation::{Channel, Conversation};
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
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub enum Event {
    Start,
    Connect(ConnectionInfo, Password<String>),
    Connected(Account, Jid),
    Disconnected(Account, String),
    AuthError(Account, String),
    Stanza(Account, Element),
    RawMessage {
        account: Account,
        message: XmppParsersMessage,
        delay: Option<Delay>,
        archive: bool,
    },
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
    IqResult { account: Account, uuid: Uuid, from: Option<Jid>, payload: Option<Element> },
    IqError { account: Account, uuid: Uuid, from: Option<Jid>, payload: StanzaError },
    Disco(Account),
    PubSub {
        account: Account,
        from: Option<Jid>,
        event: PubSubEvent
    },
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
    Notification {
        conversation: conversation::Conversation,
        important: bool,
    },
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
    Omemo(mods::omemo::OmemoMod),
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
        _archive: bool,
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
            Mod::Omemo(r#mod) => r#mod.init(aparte),
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
            Mod::Omemo(r#mod) => r#mod.on_event(aparte, event),
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
            Mod::Omemo(r#mod) => r#mod.can_handle_xmpp_message(aparte, account, message, delay),
        }
    }

    fn handle_xmpp_message(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        delay: &Option<Delay>,
        archive: bool,
    ) {
        match self {
            Mod::Completion(r#mod) => {
                r#mod.handle_xmpp_message(aparte, account, message, delay, archive)
            }
            Mod::Carbons(r#mod) => {
                r#mod.handle_xmpp_message(aparte, account, message, delay, archive)
            }
            Mod::Contact(r#mod) => {
                r#mod.handle_xmpp_message(aparte, account, message, delay, archive)
            }
            Mod::Conversation(r#mod) => {
                r#mod.handle_xmpp_message(aparte, account, message, delay, archive)
            }
            Mod::Disco(r#mod) => {
                r#mod.handle_xmpp_message(aparte, account, message, delay, archive)
            }
            Mod::Bookmarks(r#mod) => {
                r#mod.handle_xmpp_message(aparte, account, message, delay, archive)
            }
            Mod::UI(r#mod) => r#mod.handle_xmpp_message(aparte, account, message, delay, archive),
            Mod::Mam(r#mod) => r#mod.handle_xmpp_message(aparte, account, message, delay, archive),
            Mod::Messages(r#mod) => {
                r#mod.handle_xmpp_message(aparte, account, message, delay, archive)
            }
            Mod::Correction(r#mod) => {
                r#mod.handle_xmpp_message(aparte, account, message, delay, archive)
            }
            Mod::Omemo(r#mod) => {
                r#mod.handle_xmpp_message(aparte, account, message, delay, archive),
            }
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
            Mod::Omemo(_) => f.write_str("Mod::Omemo"),
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
            Mod::Omemo(r#mod) => r#mod.fmt(f),
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
            aparte.config.accounts.keys().cloned().collect()
        })
    },
    password: Password<String>
},
|aparte, _command| {
    let account = {
        if let Some((_, account)) = aparte.config.accounts.iter().find(|(name, _)| *name == &account_name) {
            account.clone()
        } else if !account_name.contains('@') {
            return Err(format!("Unknown account or invalid jid {account_name}"));
        } else if let Ok(jid) = Jid::from_str(&account_name) {
            ConnectionInfo {
                jid: jid.to_string(),
                server: None,
                port: None,
                autoconnect: false,
            }
        } else {
            return Err(format!("Unknown account or invalid jid {account_name}"));
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
    aparte.schedule(Event::Win(window));
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
    let window = window.or(current);
    if let Some(window) = window {
        // Close window
        aparte.schedule(Event::Close(window));
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
            let conversation_mod = aparte.get_mod::<mods::conversation::ConversationMod>();
            ui.get_windows().iter().map(|window| {
                if let Some(account) = aparte.current_account() {
                    if let Ok(jid) = BareJid::from_str(&window) {
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
    let window = window.or(current);
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
            contact.contacts.values().map(|contact| contact.jid.to_string()).collect()
        })
    },
    message: Option<String>
},
|aparte, _command| {
    let account = aparte.current_account().ok_or("No connection found".to_string())?;
    match Jid::from_str(&contact) {
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
                let message = Message::outgoing_chat(id, timestamp.into(), &from, &jid, &bodies, false);
                aparte.schedule(Event::Message(Some(account.clone()), message.clone()));

                aparte.send(&account, Element::try_from(message).unwrap());
            }
            Ok(())
        },
        Err(err) => {
            Err(format!("Invalid JID {contact}: {err}"))
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
            bookmarks.bookmarks_by_name.keys().cloned().chain(bookmarks.bookmarks_by_jid.keys().map(|a| a.to_string())).collect()
        })
    },
},
|aparte, _command| {
    let account = aparte.current_account().ok_or("No connection found".to_string())?;
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
            None => Err(format!("Unknown command {cmd}")),
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
                            false,
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
                            false,
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

pub struct Aparte {
    pub command_parsers: Arc<HashMap<String, CommandParser>>,
    mods: Arc<HashMap<TypeId, RwLock<Mod>>>,
    connections: HashMap<Account, Connection>,
    current_connection: Option<Account>,
    event_tx: mpsc::Sender<Event>,
    event_rx: Option<mpsc::Receiver<Event>>,
    send_tx: mpsc::Sender<(Account, Element)>,
    send_rx: Option<mpsc::Receiver<(Account, Element)>>,
    pending_iq: HashMap<Uuid, Waker>,
    /// Aparté main configuration
    pub config: Config,
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

        let config: Config = match config_str.len() {
            0 => Default::default(),
            _ => match toml::from_str(&config_str) {
                Err(err) => {
                    log::error!("Malformed config file: {}", err);
                    Default::default()
                }
                Ok(config) => config,
            },
        };

        let (event_tx, event_rx) = mpsc::channel(4096);
        let (send_tx, send_rx) = mpsc::channel(4096);

        let mut aparte = Self {
            command_parsers: Arc::new(HashMap::new()),
            mods: Arc::new(HashMap::new()),
            connections: HashMap::new(),
            current_connection: None,
            event_tx,
            event_rx: Some(event_rx),
            send_tx,
            send_rx: Some(send_rx),
            config,
            pending_iq: HashMap::new(),
        };

        aparte.add_mod(Mod::Completion(mods::completion::CompletionMod::new()));
        aparte.add_mod(Mod::Carbons(mods::carbons::CarbonsMod::new()));
        aparte.add_mod(Mod::Contact(mods::contact::ContactMod::new()));
        aparte.add_mod(Mod::Conversation(mods::conversation::ConversationMod::new()));
        aparte.add_mod(Mod::Disco(mods::disco::DiscoMod::new("client", "console", "Aparté", "en")));
        aparte.add_mod(Mod::Bookmarks(mods::bookmarks::BookmarksMod::new()));
        aparte.add_mod(Mod::UI(mods::ui::UIMod::new()));
        aparte.add_mod(Mod::Mam(mods::mam::MamMod::new()));
        aparte.add_mod(Mod::Messages(mods::messages::MessagesMod::new()));
        aparte.add_mod(Mod::Correction(mods::correction::CorrectionMod::new()));
        aparte.add_mod(Mod::Omemo(mods::omemo::OmemoMod::new()));

        aparte
    }

    pub fn handle_raw_command(
        &mut self,
        account: &Option<Account>,
        context: &String,
        buf: &String,
    ) -> Result<(), String> {
        let command_name = Command::parse_name(buf)?;

        let parser = {
            match self.command_parsers.get(command_name) {
                Some(parser) => parser,
                None => return Err(format!("Unknown command {command_name}")),
            }
        };

        let command = (parser.parse)(account, context, buf)?;
        (parser.exec)(self, command)
    }

    pub fn handle_command(&mut self, command: Command) -> Result<(), String> {
        let parser = {
            match self.command_parsers.get(&command.args[0]) {
                Some(parser) => parser,
                None => return Err(format!("Unknown command {}", command.args[0])),
            }
        };

        (parser.exec)(self, command)
    }

    pub fn add_mod(&mut self, r#mod: Mod) {
        log::info!("Add mod `{}`", r#mod);
        let mods = Arc::get_mut(&mut self.mods).unwrap();
        // TODO ensure mod is not inserted twice
        match r#mod {
            Mod::Completion(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::completion::CompletionMod>(),
                    RwLock::new(Mod::Completion(r#mod)),
                );
            }
            Mod::Carbons(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::carbons::CarbonsMod>(),
                    RwLock::new(Mod::Carbons(r#mod)),
                );
            }
            Mod::Contact(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::contact::ContactMod>(),
                    RwLock::new(Mod::Contact(r#mod)),
                );
            }
            Mod::Conversation(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::conversation::ConversationMod>(),
                    RwLock::new(Mod::Conversation(r#mod)),
                );
            }
            Mod::Disco(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::disco::DiscoMod>(),
                    RwLock::new(Mod::Disco(r#mod)),
                );
            }
            Mod::Bookmarks(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::bookmarks::BookmarksMod>(),
                    RwLock::new(Mod::Bookmarks(r#mod)),
                );
            }
            Mod::UI(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::ui::UIMod>(),
                    RwLock::new(Mod::UI(r#mod)),
                );
            }
            Mod::Mam(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::mam::MamMod>(),
                    RwLock::new(Mod::Mam(r#mod)),
                );
            }
            Mod::Messages(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::messages::MessagesMod>(),
                    RwLock::new(Mod::Messages(r#mod)),
                );
            }
            Mod::Correction(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::correction::CorrectionMod>(),
                    RwLock::new(Mod::Correction(r#mod)),
                );
            }
            Mod::Omemo(r#mod) => {
                mods.insert(
                    TypeId::of::<mods::omemo::OmemoMod>(),
                    RwLock::new(Mod::Omemo(r#mod)),
                );
            }
        }
    }

    pub fn add_connection(&mut self, account: Account, sink: mpsc::Sender<Element>) {
        let connection = Connection {
            account: account.clone(),
            sink,
        };

        self.connections.insert(account.clone(), connection);
        self.current_connection = Some(account);
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

        let mods = self.mods.clone();
        for (_, r#mod) in mods.iter() {
            r#mod.try_write().unwrap().init(self)?;
        }

        Ok(())
    }

    pub fn run(mut self) {
        let mut input_event_stream = {
            let ui = self.get_mod::<mods::ui::UIMod>();
            ui.event_stream()
        };

        let mut rt = TokioRuntime::new().unwrap();

        let tx_for_signal = self.event_tx.clone();
        rt.spawn(async move {
            let mut sigwinch = unix::signal(unix::SignalKind::window_change()).unwrap();
            loop {
                sigwinch.recv().await;
                if let Err(err) = tx_for_signal.send(Event::WindowChange).await {
                    log::error!("Cannot send signal to internal channel: {}", err);
                    break;
                }
            }
        });

        let tx_for_event = self.event_tx.clone();
        rt.spawn(async move {
            loop {
                match input_event_stream.next().await {
                    Some(event) => {
                        if let Err(err) = tx_for_event.send(event).await {
                            log::error!("Cannot send event to internal channel: {}", err);
                            break;
                        }
                    }
                    None => {
                        if let Err(err) = tx_for_event.send(Event::Quit).await {
                            log::error!("Cannot send Quit event to internal channel: {}", err);
                        }
                        break;
                    }
                }
            }
        });

        let local_set = tokio::task::LocalSet::new();
        local_set.block_on(&mut rt, async move {
            self.schedule(Event::Start);
            let mut event_rx = self.event_rx.take().unwrap();
            let mut send_rx = self.send_rx.take().unwrap();

            loop {
                tokio::select! {
                    event = event_rx.recv() => match event {
                        Some(event) => if let Err(_) = self.handle_event(event) {
                            break;
                        },
                        None => {
                            debug!("Broken event channel");
                            break;
                        }
                    },
                    account_and_stanza = send_rx.recv() => match account_and_stanza {
                        Some((account, stanza)) => self.send_stanza(account, stanza).await,
                        None => {
                            debug!("Broken send channel");
                            break;
                        }
                    }
                };
            }
        });
    }

    pub fn start(&mut self) {
        self.log(color::rainbow(WELCOME));
        self.log(format!("Version: {VERSION}"));

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

    async fn send_stanza(&mut self, account: Account, stanza: Element) {
        let mut raw = Vec::<u8>::new();
        stanza.write_to(&mut raw).unwrap();
        log::debug!("SEND: {}", String::from_utf8(raw).unwrap());
        match self.connections.get_mut(&account) {
            Some(connection) => {
                if let Err(e) = connection.sink.send(stanza).await {
                    log::warn!("Cannot send stanza: {}", e);
                }
            }
            None => {
                log::warn!("No connection found for {}", account);
            }
        }
    }

    pub fn connect(&mut self, connection_info: &ConnectionInfo, password: Password<String>) {
        let account: Account = match Jid::from_str(&connection_info.jid) {
            Ok(Jid::Full(jid)) => jid,
            Ok(Jid::Bare(jid)) => {
                let rand_string: String = rand::thread_rng()
                    .sample_iter(&rand::distributions::Alphanumeric)
                    .take(5)
                    .map(char::from)
                    .collect();
                jid.with_resource(format!("aparte_{rand_string}"))
            }
            Err(err) => {
                self.log(format!(
                    "Cannot connect as {}: {}",
                    connection_info.jid, err
                ));
                return;
            }
        };

        self.log(format!("Connecting as {account}"));
        let mut client = match TokioXmppClient::new(&account.to_string(), password.0) {
            Ok(client) => client,
            Err(err) => {
                self.log(format!("Cannot connect as {account}: {err}"));
                return;
            }
        };

        client.set_reconnect(true);

        let (connection_channel, mut rx) = mpsc::channel(256);

        self.add_connection(account.clone(), connection_channel);

        let (mut writer, mut reader) = client.split();
        // XXX could use self.rt.spawn if client was impl Send
        task::spawn_local(async move {
            while let Some(element) = rx.recv().await {
                if let Err(err) = writer.send(XmppPacket::Stanza(element)).await {
                    log::error!("cannot send Stanza to internal channel: {}", err);
                    break;
                }
            }
        });

        let event_tx = self.event_tx.clone();

        let reconnect = true;
        task::spawn_local(async move {
            while let Some(event) = reader.next().await {
                log::debug!("XMPP Event: {:?}", event);
                match event {
                    XmppEvent::Disconnected(XmppError::Auth(e)) => {
                        if let Err(err) = event_tx
                            .send(Event::AuthError(account.clone(), format!("{e}")))
                            .await
                        {
                            log::error!("Cannot send event to internal channel: {}", err);
                        };
                        break;
                    }
                    XmppEvent::Disconnected(e) => {
                        if let Err(err) = event_tx
                            .send(Event::Disconnected(account.clone(), format!("{e}")))
                            .await
                        {
                            log::error!("Cannot send event to internal channel: {}", err);
                        };
                        if !reconnect {
                            break;
                        }
                    }
                    XmppEvent::Online {
                        bound_jid: jid,
                        resumed: true,
                    } => {
                        log::debug!("Reconnected to {}", jid);
                    }
                    XmppEvent::Online {
                        bound_jid: jid,
                        resumed: false,
                    } => {
                        if let Err(err) = event_tx
                            .send(Event::Connected(account.clone(), jid))
                            .await
                        {
                            log::error!("Cannot send event to internal channel: {}", err);
                            break;
                        }
                    }
                    XmppEvent::Stanza(stanza) => {
                        log::debug!("RECV: {}", String::from(&stanza));
                        if let Err(err) = event_tx
                            .send(Event::Stanza(account.clone(), stanza))
                            .await
                        {
                            log::error!("Cannot send stanza to internal channel: {}", err);
                            break;
                        }
                    }
                }
            }
        });
    }

    pub fn handle_event(&mut self, event: Event) -> Result<(), ()> {
        log::debug!("Event: {:?}", event);
        {
            let mods = self.mods.clone();
            for (_, r#mod) in mods.iter() {
                r#mod.try_write().unwrap().on_event(self, &event);
            }
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
                self.connect(&account, password);
            }
            Event::Connected(account, _) => {
                self.log(format!("Connected as {}", account));
                let mut presence = Presence::new(PresenceType::None);
                presence.show = Some(PresenceShow::Chat);

                let disco = self.get_mod::<mods::disco::DiscoMod>().get_disco();
                let disco = caps::compute_disco(&disco);
                let verification_string = caps::hash_caps(&disco, xmpp_hashes::Algo::Blake2b_512).unwrap();
                let caps = Caps::new("aparté", verification_string);
                presence.add_payload(caps);

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
                self.handle_xmpp_message(account, message, delay, archive);
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

        Ok(())
    }

    fn handle_stanza(&mut self, account: Account, stanza: Element) {
        if let Ok(message) = XmppParsersMessage::try_from(stanza.clone()) {
            self.handle_xmpp_message(account, message, None, false)
        } else if let Ok(iq) = Iq::try_from(stanza.clone()) {
            self.handle_iq(account, iq);
        } else if let Ok(presence) = Presence::try_from(stanza.clone()) {
            self.schedule(Event::Presence(account, presence));
        }
    }

    fn handle_xmpp_message(
        &mut self,
        account: Account,
        message: XmppParsersMessage,
        delay: Option<Delay>,
        archive: bool,
    ) {
        let mut best_match = 0f64;
        let mut matched_mod = None;

        let mods = self.mods.clone();
        for (_, r#mod) in mods.iter() {
            let message_match = r#mod
                .try_write().unwrap()
                .can_handle_xmpp_message(self, &account, &message, &delay);
            if message_match > best_match {
                matched_mod = Some(r#mod);
                best_match = message_match;
            }
        }

        if let Some(r#mod) = matched_mod {
            log::debug!("Handling xmpp message by {:?}", r#mod);
            r#mod
                .try_write().unwrap()
                .handle_xmpp_message(self, &account, &message, &delay, archive);
        } else {
            log::info!("Don't know how to handle message: {:?}", message);
        }
    }

    fn handle_iq(&mut self, account: Account, iq: Iq) {
        match iq.payload {
            IqType::Error(payload) => {
                if let Ok(uuid) = Uuid::from_str(&iq.id) {
                    if let Some(_mod_id) = self.pending_iq.remove(&uuid) {
                        todo!();
                    }
                } else {
                    if let Some(text) = payload.texts.get("en") {
                        let message = Message::log(text.clone());
                        self.schedule(Event::Message(Some(account.clone()), message));
                    }
                }
            }
            IqType::Result(_payload) => {
                if let Ok(uuid) = Uuid::from_str(&iq.id) {
                    if let Some(_mod_id) = self.pending_iq.remove(&uuid) {
                        todo!();
                    }
                }
            }
            _ => {
                self.schedule(Event::Iq(account, iq));
            }
        }
    }

    // TODO maybe use From<>
    pub fn proxy(&self) -> AparteAsync {
        AparteAsync {
            current_connection: self.current_connection.clone(),
            mods: self.mods.clone(),
            event_tx: self.event_tx.clone(),
            send_tx: self.send_tx.clone(),
            config: self.config.clone(),
        }
    }

    pub fn spawn<F>(future: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        tokio::spawn(future);
    }

    // Common function for AparteAsync and Aparte, maybe share it in Trait
    pub fn add_command(&mut self, command_parser: CommandParser) {
        let command_parsers = Arc::get_mut(&mut self.command_parsers).unwrap();
        command_parsers.insert(command_parser.name.to_string(), command_parser);
    }

    pub fn send(&mut self, account: &Account, stanza: Element) {
        self.send_tx.try_send((account.clone(), stanza)).unwrap();
    }

    pub fn schedule(&mut self, event: Event) {
        self.event_tx.try_send(event).unwrap();
    }

    pub fn log(&mut self, message: String) {
        let message = Message::log(message);
        self.schedule(Event::Message(None, message));
    }

    pub fn get_mod<'a, T>(&'a self) -> RwLockReadGuard<'a, T>
    where
        T: 'static,
        for<'b> &'b T: From<&'b Mod>,
    {
        match self.mods.get(&TypeId::of::<T>()) {
            Some(r#mod) => RwLockReadGuard::map(r#mod.try_read().unwrap(), |m| m.into()),
            None => unreachable!(),
        }
    }

    #[allow(unused)]
    pub fn get_mod_mut<'a, T>(&'a self) -> RwLockMappedWriteGuard<'a, T>
    where
        T: 'static,
        for<'b> &'b mut T: From<&'b mut Mod>,
    {
        match self.mods.get(&TypeId::of::<T>()) {
            Some(r#mod) => RwLockWriteGuard::map(r#mod.try_write().unwrap(), |m| m.into()),
            None => unreachable!(),
        }
    }

    pub fn current_account(&self) -> Option<Account> {
        self.current_connection.clone()
    }
}

#[derive(Clone)]
pub struct AparteAsync {
    current_connection: Option<Account>,
    mods: Arc<HashMap<TypeId, RwLock<Mod>>>,
    event_tx: mpsc::Sender<Event>,
    send_tx: mpsc::Sender<(Account, Element)>,
    pub config: Config,
}

impl AparteAsync {
    pub async fn send(&mut self, account: &Account, stanza: Element) {
        self.send_tx.send((account.clone(), stanza)).await.unwrap();
    }

    pub fn iq(&mut self, account: &Account, iq: Iq) -> IqFuture {
        return IqFuture::new(self.clone(), account.clone(), iq);
    }

    pub async fn schedule(&mut self, event: Event) {
        self.event_tx.send(event).await.unwrap();
    }

    pub async fn log(&mut self, message: String) {
        let message = Message::log(message);
        self.schedule(Event::Message(None, message)).await;
    }

    pub async fn get_mod<'a, T>(&'a self) -> RwLockReadGuard<'a, T>
    where
        T: 'static,
        for<'b> &'b T: From<&'b Mod>,
    {
        match self.mods.get(&TypeId::of::<T>()) {
            Some(r#mod) => RwLockReadGuard::map(r#mod.read().await, |m| m.into()),
            None => unreachable!(),
        }
    }

    pub async fn get_mod_mut<'a, T>(&'a self) -> RwLockMappedWriteGuard<'a, T>
    where
        T: 'static,
        for<'b> &'b mut T: From<&'b mut Mod>,
    {
        match self.mods.get(&TypeId::of::<T>()) {
            Some(r#mod) => RwLockWriteGuard::map(r#mod.write().await, |m| m.into()),
            None => unreachable!(),
        }
    }

    pub fn current_account(&self) -> Option<Account> {
        self.current_connection.clone()
    }
}
