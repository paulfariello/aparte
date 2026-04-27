/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::any::TypeId;
use std::collections::{HashMap, VecDeque};
use std::convert::{TryFrom, TryInto};
use std::fmt::{self, Debug, Display};
use std::fs::OpenOptions;
use std::future::Future;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, Local as LocalTz};
use futures::stream::StreamExt;
use rand::Rng;
use secrecy::ExposeSecret;
use terminus::charxel::IntoCharxels;
use terminus::cursor::Cursor;
use terminus::rendering::OffscreenRenderBuffer;
use termion::event::Key;
use termion::raw::IntoRawMode;
use termion::screen::IntoAlternateScreen;
use tokio::runtime::Runtime as TokioRuntime;
use tokio::signal::unix;
use tokio::sync::{mpsc, Notify, RwLock, RwLockMappedWriteGuard, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;
use uuid::Uuid;

use xmpp_parsers::caps::{self, Caps};
use xmpp_parsers::delay::Delay;
use xmpp_parsers::hashes as xmpp_hashes;
use xmpp_parsers::iq::Iq;
use xmpp_parsers::jid::{BareJid, FullJid, Jid};
use xmpp_parsers::legacy_omemo;
use xmpp_parsers::message::Message as XmppParsersMessage;
use xmpp_parsers::minidom::Element;
use xmpp_parsers::muc::Muc;
use xmpp_parsers::presence::{Presence, Show as PresenceShow, Type as PresenceType};
use xmpp_parsers::pubsub::event as pubsub_event;
use xmpp_parsers::stanza_error::StanzaError;
use xmpp_parsers::{iq, presence};

use tokio_xmpp::{IqFailure, IqResponse, IqResponseToken};

use crate::account::{Account, ConnectionInfo, Password};
use crate::async_iq::IqEnvelope;
use crate::color;
use crate::command::{Command, CommandParser};
use crate::config::Config;
use crate::conversation::{Channel, Conversation};
use crate::crypto::CryptoEngine;
use crate::message::Message;
use crate::mods;
use crate::storage::Storage;
use crate::{
    command_def, generate_arg_autocompletion, generate_command_autocompletions, generate_help,
    parse_command_args, parse_lookup_arg,
};
use crate::{contact, conversation};

// Rendering tick at ~60fps
const UI_TICK_MS: u64 = 16u64;

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
    Connect(ConnectionInfo, Password),
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
    IqResult {
        account: Account,
        uuid: Uuid,
        from: Option<Jid>,
        payload: Option<Element>,
    },
    IqError {
        account: Account,
        uuid: Uuid,
        from: Option<Jid>,
        payload: StanzaError,
    },
    Disco(Account, Vec<String>),
    PubSub {
        account: Account,
        from: Option<Jid>,
        event: pubsub_event::Payload,
    },
    Presence(Account, presence::Presence),
    ReadPassword(Command),
    Win(String),
    Close(String),
    Contact(Account, contact::Contact),
    ContactUpdate(Account, contact::Contact),
    Bookmark(Account, contact::Bookmark),
    BookmarksUpdate(Account, Vec<contact::Bookmark>),
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
    Omemo(mods::omemo::OmemoEvent),
    UIRender(bool),
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

pub trait ModTrait: Display {
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
                r#mod.handle_xmpp_message(aparte, account, message, delay, archive)
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

impl Display for Mod {
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

pub struct Connection {
    pub sink: mpsc::UnboundedSender<Element>,
    pub iq_sink: mpsc::UnboundedSender<IqEnvelope>,
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
        completion: |aparte, _command| {
            aparte.config.accounts.keys().cloned().collect()
        }
    },
    password: Password = {
        lookup: |aparte, _command| {
            aparte.config.accounts.get(&account_name).and_then(|account| account.password.clone())
        }
    },
},
|aparte, _command| {
    let account = {
        if let Some((_, account)) = aparte.config.accounts.iter().find(|(name, _)| *name == &account_name) {
            log::debug!("Use stored config for {account_name}");
            account.clone()
        } else if !account_name.contains('@') {
            anyhow::bail!("Unknown account or invalid jid {account_name}");
        } else if let Ok(jid) = Jid::from_str(&account_name) {
            ConnectionInfo {
                jid: jid.to_string(),
                server: None,
                port: None,
                autoconnect: false,
                password: None,
            }
        } else {
            anyhow::bail!("Unknown account or invalid jid {account_name}");
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
        completion: |aparte, _command| {
            let ui = aparte.get_mod::<mods::ui::UIMod>();
            ui.get_windows()
        }
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
        completion: |aparte, _command| {
            let ui = aparte.get_mod::<mods::ui::UIMod>();
            ui.get_windows()
        }
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
        completion: |aparte, _command| {
            let ui = aparte.get_mod::<mods::ui::UIMod>();
            let conversation_mod = aparte.get_mod::<mods::conversation::ConversationMod>();
            ui.get_windows().iter().map(|window| {
                if let Some(account) = aparte.current_account() {
                    if let Ok(jid) = BareJid::from_str(window) {
                        conversation_mod.get(&account, &jid).cloned()
                    } else {
                        None
                    }
                } else {
                    None
                }
            }).filter_map(|conversation| {
                if let Some(Conversation::Channel(channel)) = conversation {
                    Some(channel.jid.to_string())
                } else {
                    None
                }
            }).collect()
        }
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
        completion: |aparte, _command| {
            let contact = aparte.get_mod::<mods::contact::ContactMod>();
            contact.contacts.values().map(|contact| contact.jid.to_string()).collect()
        }
    },
    message: Option<String>
},
|aparte, _command| {
    let account = aparte.current_account().context("No connection found")?;
    let jid = Jid::from_str(&contact).context("Invalid JID")?;
    aparte.schedule(Event::Chat { account: account.clone(), contact: jid.to_bare() });
    if let Some(body) = message {
        let mut bodies = HashMap::new();
        bodies.insert("".to_string(), body);
        let id = Uuid::new_v4().to_string();
        let from: Jid = account.clone().into();
        let timestamp = LocalTz::now();
        let message = Message::outgoing_chat(id, timestamp.into(), &from, &jid, bodies, None, false);
        aparte.schedule(Event::Message(Some(account.clone()), message.clone()));

        aparte.send(&account, message);
    }
    Ok(())
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
        completion: |aparte, _command| {
            let bookmarks = aparte.get_mod::<mods::bookmarks::BookmarksMod>();
            bookmarks.bookmarks_by_name.keys().cloned().chain(bookmarks.bookmarks_by_jid.keys().map(|a| a.to_string())).collect()
        }
    },
},
|aparte, _command| {
    let account = aparte.current_account().context("No connection found")?;
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
                            Some(nick) => Jid::from(bookmark.jid.with_resource_str(&nick).context("Invalid nick")?),
                            None => Jid::from(bookmark.jid.clone()),
                        }
                    },
                    None => Jid::from_str(&muc)?
                }
            };

            aparte.schedule(Event::Join {
                account,
                channel: jid,
                user_request: true
            });
            Ok(())
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
        completion: |aparte, _command| {
            aparte.command_parsers.iter().map(|c| c.0.to_string()).collect()
        }
    }
},
|aparte, _command| {
    if let Some(cmd) = cmd {
        let help = aparte.command_parsers.get(&cmd).with_context(|| format!("Unknown command {cmd}"))?.help.to_string();

        crate::info!(aparte, "{}", help);
        Ok(())
    } else {
        crate::info!(aparte, "Available commands: {}", aparte.command_parsers.iter().map(|c| c.0.to_string()).collect::<Vec<String>>().join(", "));
        Ok(())
    }
});

mod me {
    use anyhow::{anyhow, Context, Result};
    use chrono::Local as LocalTz;
    use std::collections::HashMap;
    use std::str::FromStr;
    use uuid::Uuid;
    use xmpp_parsers::jid::{BareJid, Jid};

    use crate::account::Account;
    use crate::command::*;
    use crate::conversation::Conversation;
    use crate::core::{Aparte, Event};
    use crate::message::Message;
    use crate::mods;

    fn parse(account: &Option<Account>, context: &str, buf: &str) -> Result<Command> {
        Ok(Command {
            account: account.clone(),
            context: context.to_string(),
            args: vec![buf.to_string()],
            cursor: 0,
        })
    }

    fn exec(aparte: &mut Aparte, command: Command) -> Result<()> {
        let account = command
            .account
            .context("Can't use /me in non XMPP window")?;
        let jid =
            BareJid::from_str(&command.context).context("Can't use /me in non XMPP window")?;
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
                            bodies,
                            None,
                            false,
                        ))
                    }
                    Conversation::Channel(channel) => {
                        let account = &channel.account;
                        let us = account
                            .to_bare()
                            .with_resource_str(&channel.nick)
                            .context("Invalid nick")?;
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
                            bodies,
                            None,
                            false,
                        ))
                    }
                }
            } else {
                Err(anyhow!("Unknown context {}", command.context))
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

#[macro_export]
macro_rules! info(
    ($aparte:ident, $msg:literal, $($args: tt)*) => ({
        ::log::info!($msg, $($args)*);
        $aparte.log(format!($msg, $($args)*))
    });
    ($aparte:ident, $msg:literal) => ({
        ::log::info!($msg);
        $aparte.log(format!($msg))
    });
);

#[macro_export]
macro_rules! error(
    ($aparte:ident, $err:ident, $msg:literal, $($args: tt)*) => ({
        let context = format!($msg, $($args)*);
        ::log::error!("{:?}", $err.context(context.clone()));
        $aparte.log(context)
    });
    ($aparte:ident, $err:ident, $msg:literal) => ({
        let context = format!($msg);
        ::log::error!("{:?}", $err.context(context.clone()));
        $aparte.log(context)
    });
);

pub struct Aparte {
    pub command_parsers: Rc<HashMap<String, CommandParser>>,
    mods: Rc<HashMap<TypeId, RwLock<Mod>>>,
    connections: HashMap<Account, Connection>,
    current_connection: Option<Account>,
    event_tx: mpsc::UnboundedSender<Event>,
    event_rx: Option<mpsc::UnboundedReceiver<Event>>,
    send_tx: mpsc::UnboundedSender<(Account, Element)>,
    send_rx: Option<mpsc::UnboundedReceiver<(Account, Element)>>,
    iq_tx: mpsc::UnboundedSender<(Account, IqEnvelope)>,
    iq_rx: Option<mpsc::UnboundedReceiver<(Account, IqEnvelope)>>,
    crypto_engines: Arc<Mutex<HashMap<(Account, BareJid), CryptoEngine>>>,
    read_password: AtomicBool,
    /// Aparté main configuration
    pub config: Config,
    pub storage: Storage,
    record_path: Option<PathBuf>,
    pending_annotations: Arc<Mutex<Vec<String>>>,
    render_notify: Arc<Notify>,
}

impl Aparte {
    pub fn new(config_path: PathBuf, storage_path: PathBuf, record_path: Option<PathBuf>) -> Result<Self> {
        log::debug!("Loading aparté with {:?}", config_path);
        let mut config_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&config_path)
            .with_context(|| format!("Cannot read config file {:?}", config_path))?;

        let mut config_str = String::new();
        config_file
            .read_to_string(&mut config_str)
            .with_context(|| format!("Cannot read config file {}", config_str))?;

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

        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (send_tx, send_rx) = mpsc::unbounded_channel();
        let (iq_tx, iq_rx) = mpsc::unbounded_channel();

        let mut aparte = Self {
            command_parsers: Rc::new(HashMap::new()),
            mods: Rc::new(HashMap::new()),
            connections: HashMap::new(),
            storage: Storage::new(storage_path)?,
            current_connection: None,
            event_tx,
            event_rx: Some(event_rx),
            send_tx,
            send_rx: Some(send_rx),
            iq_tx,
            iq_rx: Some(iq_rx),
            config: config.clone(),
            crypto_engines: Arc::new(Mutex::new(HashMap::new())),
            read_password: AtomicBool::new(false),
            record_path,
            pending_annotations: Arc::new(Mutex::new(Vec::new())),
            render_notify: Arc::new(Notify::new()),
        };

        aparte.add_mod(Mod::Completion(mods::completion::CompletionMod::default()));
        aparte.add_mod(Mod::Carbons(mods::carbons::CarbonsMod::default()));
        aparte.add_mod(Mod::Contact(mods::contact::ContactMod::default()));
        aparte.add_mod(Mod::Conversation(
            mods::conversation::ConversationMod::default(),
        ));
        aparte.add_mod(Mod::Disco(mods::disco::DiscoMod::new(
            "client", "console", "Aparté", "en",
        )));
        aparte.add_mod(Mod::Bookmarks(mods::bookmarks::BookmarksMod::default()));
        aparte.add_mod(Mod::UI(mods::ui::UIMod::new(&config)));
        aparte.add_mod(Mod::Mam(mods::mam::MamMod::default()));
        aparte.add_mod(Mod::Messages(mods::messages::MessagesMod::default()));
        aparte.add_mod(Mod::Correction(mods::correction::CorrectionMod::default()));
        aparte.add_mod(Mod::Omemo(mods::omemo::OmemoMod::default()));

        Ok(aparte)
    }

    pub fn handle_raw_command(
        &mut self,
        account: &Option<Account>,
        context: &str,
        buf: &str,
    ) -> Result<()> {
        let command_name = Command::parse_name(buf)?;

        let parser = self
            .command_parsers
            .get(command_name)
            .with_context(|| format!("Unknown command {command_name}"))?;

        let command = (parser.parse)(account, context, buf)?;
        (parser.exec)(self, command)
    }

    pub fn handle_command(&mut self, command: Command) -> Result<()> {
        let parser = self
            .command_parsers
            .get(&command.args[0])
            .with_context(|| format!("Unknown command {}", command.args[0]))?;

        (parser.exec)(self, command)
    }

    pub fn add_mod(&mut self, r#mod: Mod) {
        log::info!("Add mod `{}`", r#mod);
        let mods = Rc::get_mut(&mut self.mods).unwrap();
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
                mods.insert(TypeId::of::<mods::ui::UIMod>(), RwLock::new(Mod::UI(r#mod)));
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

    pub fn add_connection(&mut self, account: Account, connection: Connection) {
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
        let rt = TokioRuntime::new().unwrap();

        rt.spawn({
            let tx = self.event_tx.clone();
            async move {
                let mut sigwinch = unix::signal(unix::SignalKind::window_change()).unwrap();
                loop {
                    sigwinch.recv().await;
                    if let Err(err) = tx.send(Event::WindowChange) {
                        log::error!("Cannot send signal to internal channel: {}", err);
                        break;
                    }
                }
            }
        });

        rt.spawn({
            let tx = self.event_tx.clone();
            let mut input_event_stream = {
                let ui = self.get_mod::<mods::ui::UIMod>();
                ui.event_stream()
            };
            async move {
                loop {
                    match input_event_stream.next().await {
                        Some(event) => {
                            log::trace!("Got input event {:?} at {:?}", event, Instant::now());
                            if let Err(err) = tx.send(event) {
                                log::error!("Cannot send event to internal channel: {}", err);
                                break;
                            }
                        }
                        None => {
                            if let Err(err) = tx.send(Event::Quit) {
                                log::error!("Cannot send Quit event to internal channel: {}", err);
                            }
                            break;
                        }
                    }
                }
            }
        });

        rt.spawn({
            let render_buffer = {
                let ui = self.get_mod::<mods::ui::UIMod>();
                Arc::clone(&ui.render_buffer)
            };
            let mut reference_screen = OffscreenRenderBuffer::default();
            let record_path = self.record_path.take();
            let pending_annotations = Arc::clone(&self.pending_annotations);
            let render_notify = Arc::clone(&self.render_notify);

            async move {
                use crate::tee_writer::Screen;

                log::debug!("Start UI thread");
                let base = std::io::stdout()
                    .into_raw_mode()
                    .unwrap()
                    .into_alternate_screen()
                    .unwrap();

                let mut screen: Screen = match record_path {
                    Some(ref path) => {
                        let rec = std::fs::OpenOptions::new()
                            .write(true).create(true).truncate(true)
                            .open(path)
                            .expect("Cannot open recording file");
                        let mut events_path = path.clone();
                        events_path.set_extension("events");
                        let ev = std::fs::OpenOptions::new()
                            .write(true).create(true).truncate(true)
                            .open(events_path)
                            .expect("Cannot open events file");
                        Screen::Recording(crate::tee_writer::TeeWriter::new(base, rec, ev))
                    }
                    None => Screen::Plain(base),
                };

                let _ = write!(&mut screen, "{}", termion::clear::All);
                let mut frame_count: u64 = 0;
                loop {
                    render_notify.notified().await;

                    if let Ok(mut annotations) = pending_annotations.lock() {
                        for tag in annotations.drain(..) {
                            screen.annotate(&tag);
                        }
                    }

                    let render_buffer = std::sync::RwLock::read(&render_buffer).unwrap();
                    render_buffer.render(&mut screen, &mut reference_screen);

                    frame_count += 1;
                    screen.dump_buffer(&reference_screen.dump_text(), frame_count);
                }
            }
        });

        rt.block_on(async move {
            self.schedule(Event::Start);
            let mut event_rx = self.event_rx.take().unwrap();
            let mut send_rx = self.send_rx.take().unwrap();
            let mut iq_rx = self.iq_rx.take().unwrap();

            let mut last_events = VecDeque::new();
            'main: loop {
                let mut events_buf = Vec::new();
                tokio::select! {
                    biased;
                    count = event_rx.recv_many(&mut events_buf, 1000) => match count {
                        0 => {
                            log::error!("Broken event channel");
                            break
                        }
                        events_count => {
                            // Ensure all key events are handled first
                            let (priority_events, filtered_events): (VecDeque<_>, VecDeque<_>) = events_buf.drain(..).partition(|event| matches!(event,
                                                                                                 Event::Key(_)
                                                                                                 | Event::Completed(_, _)
                                                                                                 | Event::ChangeWindow(_)
                                                                                                 | Event::Quit
                                                                                                 | Event::UIRender(_)));

                            log::trace!("Event loop got {} new events ({} priority); have {} last events", events_count, priority_events.len(), last_events.len());

                            last_events.extend(filtered_events);
                            // Handle priority events first
                            let mut priority_start = Instant::now();
                            for event in priority_events {
                                if let Event::ChangeWindow(ref name) = event {
                                    if let Ok(mut q) = self.pending_annotations.lock() {
                                        q.push(format!("WINDOW_CHANGE:{}", name));
                                    }
                                }
                                if self.handle_event(event).is_err() {
                                    break 'main
                                }
                                if priority_start.elapsed() > Duration::from_millis(UI_TICK_MS) {
                                    priority_start = Instant::now();
                                    tokio::task::yield_now().await;
                                }
                            }
                            let mut start = Instant::now();
                            while let Some(event) = last_events.pop_front() {
                                if self.handle_event(event).is_err() {
                                    break 'main;
                                }
                                // Ensure we don't loop here for too long to get UI responsive
                                if start.elapsed() > Duration::from_millis(UI_TICK_MS) {
                                    log::trace!("Event loop, take a breath {} pending events", event_rx.len());
                                    start = Instant::now();
                                    if !event_rx.is_empty() {
                                        break
                                    }
                                }
                            }
                            log::trace!("Event loop, delayed handling of {} events", last_events.len());
                        },
                    },
                    account_and_stanza = send_rx.recv() => match account_and_stanza {
                        Some((account, stanza)) => {
                            self.send_stanza(account, stanza);
                            // Drain remaining ready stanzas in batch
                            while let Ok((account, stanza)) = send_rx.try_recv() {
                                self.send_stanza(account, stanza);
                            }
                        }
                        None => {
                            log::error!("Broken send channel");
                            break;
                        }
                    },
                    Some((account, envelope)) = iq_rx.recv() => {
                        match self.connections.get(&account) {
                            Some(conn) => {
                                if let Err(e) = conn.iq_sink.send(envelope) {
                                    log::warn!("Cannot route IQ to connection: {e}");
                                }
                            }
                            None => log::warn!("No connection for IQ from {account}"),
                        }
                    }
                };

                // Render UI once per event batch (if state changed) and wake render thread.
                if self.get_mod_mut::<mods::ui::UIMod>().render_if_dirty() {
                    self.render_notify.notify_one();
                }
            }
        });
    }

    pub fn start(&mut self) {
        self.log(color::rainbow(WELCOME));
        self.log(format!("Version: {VERSION}"));

        for (name, account) in self.config.accounts.clone() {
            if account.autoconnect {
                self.schedule(Event::RawCommand(
                    None,
                    "console".to_string(),
                    format!("/connect {}", name),
                ));
            }
        }
    }

    fn send_stanza(&mut self, account: Account, stanza: Element) {
        let mut raw = Vec::<u8>::new();
        stanza.write_to(&mut raw).unwrap();
        log::debug!("SEND: {}", String::from_utf8(raw).unwrap());
        match self.connections.get_mut(&account) {
            Some(connection) => {
                if let Err(e) = connection.sink.send(stanza) {
                    log::warn!("Cannot send stanza: {}", e);
                }
            }
            None => {
                log::warn!("No connection found for {}", account);
            }
        }
    }

    pub fn connect(&mut self, connection_info: &ConnectionInfo, password: Password) {
        let account: Account = match Jid::from_str(&connection_info.jid).map(Jid::try_into_full) {
            Ok(Ok(full_jid)) => full_jid,
            Ok(Err(bare_jid)) => {
                let rand_string: String = rand::thread_rng()
                    .sample_iter(&rand::distributions::Alphanumeric)
                    .take(5)
                    .collect();
                bare_jid
                    .with_resource_str(&format!("aparte_{rand_string}"))
                    .unwrap()
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
        let dns_config = match (&connection_info.server, &connection_info.port) {
            (Some(server), Some(port)) => tokio_xmpp::connect::DnsConfig::no_srv(server, *port),
            (Some(server), None) => tokio_xmpp::connect::DnsConfig::no_srv(server, 5222),
            (None, Some(port)) => {
                tokio_xmpp::connect::DnsConfig::no_srv(account.domain().as_str(), *port)
            }
            (None, None) => {
                tokio_xmpp::connect::DnsConfig::srv_default_client(account.domain().as_str())
            }
        };
        log::debug!("Connect with dns_config: {dns_config:?}");
        #[cfg(feature = "insecure-xmpp")]
        let mut client = if std::env::var("APARTE_INSECURE_XMPP").as_deref() == Ok("1") {
            tokio_xmpp::Client::new_plaintext(
                Jid::from(account.clone()),
                password.expose_secret().clone(),
                dns_config,
                tokio_xmpp::xmlstream::Timeouts::default(),
            )
        } else {
            tokio_xmpp::Client::new_starttls(
                Jid::from(account.clone()),
                password.expose_secret().clone(),
                dns_config,
                tokio_xmpp::xmlstream::Timeouts::default(),
            )
        };
        #[cfg(not(feature = "insecure-xmpp"))]
        let mut client = tokio_xmpp::Client::new_starttls(
            Jid::from(account.clone()),
            password.expose_secret().clone(),
            dns_config,
            tokio_xmpp::xmlstream::Timeouts::default(),
        );

        let (connection_channel, mut rx) = mpsc::unbounded_channel();
        let (iq_channel, mut iq_rx) = mpsc::unbounded_channel::<IqEnvelope>();

        self.add_connection(
            account.clone(),
            Connection {
                sink: connection_channel,
                iq_sink: iq_channel,
            },
        );

        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(envelope) = iq_rx.recv() => {
                        let token = client.send_iq(envelope.to, envelope.request).await;
                        let _ = envelope.token_tx.send(token);
                    }
                    maybe_element = rx.recv() => {
                        let Some(element) = maybe_element else { break };
                        let stanza = match element.name() {
                            "iq" => match xmpp_parsers::iq::Iq::try_from(element) {
                                Ok(iq) => tokio_xmpp::Stanza::Iq(iq),
                                Err(err) => {
                                    log::error!("Cannot parse outgoing iq: {}", err);
                                    continue;
                                }
                            },
                            "message" => match XmppParsersMessage::try_from(element) {
                                Ok(message) => tokio_xmpp::Stanza::Message(message),
                                Err(err) => {
                                    log::error!("Cannot parse outgoing message: {}", err);
                                    continue;
                                }
                            },
                            "presence" => match Presence::try_from(element) {
                                Ok(presence) => tokio_xmpp::Stanza::Presence(presence),
                                Err(err) => {
                                    log::error!("Cannot parse outgoing presence: {}", err);
                                    continue;
                                }
                            },
                            other => {
                                log::error!("Cannot send unknown stanza '{}'", other);
                                continue;
                            }
                        };
                        if let Err(err) = client.send_stanza(stanza).await {
                            log::error!("cannot send Stanza to internal channel: {}", err);
                            break;
                        }
                    }
                    maybe_event = client.next() => {
                        let Some(event) = maybe_event else { break };
                        log::debug!("XMPP Event: {:?}", event);
                        match event {
                            tokio_xmpp::Event::Disconnected(tokio_xmpp::Error::Auth(e)) => {
                                if let Err(err) =
                                    event_tx.send(Event::AuthError(account.clone(), format!("{e}")))
                                {
                                    log::error!("Cannot send event to internal channel: {}", err);
                                };
                                break;
                            }
                            tokio_xmpp::Event::Disconnected(e) => {
                                if let Err(err) =
                                    event_tx.send(Event::Disconnected(account.clone(), format!("{e}")))
                                {
                                    log::error!("Cannot send event to internal channel: {}", err);
                                };
                            }
                            tokio_xmpp::Event::Online {
                                bound_jid: jid,
                                resumed: true,
                            } => {
                                log::debug!("Reconnected to {}", jid);
                            }
                            tokio_xmpp::Event::Online {
                                bound_jid: jid,
                                resumed: false,
                            } => {
                                if let Err(err) = event_tx.send(Event::Connected(account.clone(), jid)) {
                                    log::error!("Cannot send event to internal channel: {}", err);
                                    break;
                                }
                            }
                            tokio_xmpp::Event::Stanza(stanza) => {
                                let element: Element = match stanza {
                                    tokio_xmpp::Stanza::Iq(iq) => Element::from(iq),
                                    tokio_xmpp::Stanza::Message(message) => Element::from(message),
                                    tokio_xmpp::Stanza::Presence(presence) => Element::from(presence),
                                };
                                log::debug!("RECV: {}", String::from(&element));
                                if let Err(err) = event_tx.send(Event::Stanza(account.clone(), element)) {
                                    log::error!("Cannot send stanza to internal channel: {}", err);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    pub fn handle_event(&mut self, event: Event) -> Result<(), ()> {
        if self.read_password.load(Relaxed) && matches!(event, Event::Key(..)) {
            log::trace!("Handle event: {:?}", Event::Key(Key::Char('*')),);
        } else {
            log::trace!("Handle event: {:?}", event);
        }
        let before = Instant::now();

        {
            let mods = self.mods.clone();
            for (_, r#mod) in mods.iter() {
                let before = Instant::now();
                r#mod.try_write().unwrap().on_event(self, &event);
                log::trace!("{:?} handled event in {:.2?}", r#mod, before.elapsed());
            }
        }

        match event {
            Event::Start => {
                self.start();
            }
            Event::Command(command) => {
                self.read_password.swap(false, Relaxed);
                if let Err(err) = self.handle_command(command) {
                    self.log(err);
                }
            }
            Event::RawCommand(account, context, buf) => {
                if let Err(err) = self.handle_raw_command(&account, &context, &buf) {
                    self.log(err);
                }
            }
            Event::SendMessage(account, message) => {
                let will_encrypt = message.encryption_recipient().is_some_and(|recipient| {
                    self.crypto_engines
                        .lock()
                        .unwrap()
                        .contains_key(&(account.clone(), recipient))
                });
                let mut display_message = message.clone();
                display_message.set_encrypted(will_encrypt);
                self.schedule(Event::Message(Some(account.clone()), display_message));

                // Encrypt if required
                let encryption = message.encryption_recipient().and_then(|recipient| {
                    let mut crypto_engines = self.crypto_engines.lock().unwrap();
                    crypto_engines
                        .get_mut(&(account.clone(), recipient))
                        .map(|crypto_engine| crypto_engine.encrypt(self, &account, &message))
                });

                match encryption {
                    Some(Ok(encrypted_message)) => self.send(&account, encrypted_message),
                    Some(Err(e)) => {
                        log::error!("Cannot encrypt message (TODO print error in UI): {e}")
                    }
                    None => self.send(&account, message),
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
                let verification_string =
                    caps::hash_caps(&disco, xmpp_hashes::Algo::Blake2b_512).unwrap();
                let caps = Caps::new("aparté", verification_string);
                presence.add_payload(caps);

                self.send(&account, presence);
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
            Event::RawMessage {
                account,
                message,
                delay,
                archive,
            } => {
                self.handle_xmpp_message(account, message, delay, archive);
            }
            Event::Join {
                account,
                channel,
                user_request,
            } => {
                let to = match channel.try_as_full() {
                    Ok(full_jid) => full_jid.clone(),
                    Err(bare_jid) => bare_jid
                        .with_resource_str(account.node().as_ref().unwrap())
                        .unwrap(),
                };
                let from: Jid = account.clone().into();

                let mut presence = Presence::new(PresenceType::None);
                presence = presence.with_to(Jid::from(to.clone()));
                presence = presence.with_from(from);
                presence.add_payload(Muc::new());
                self.send(&account, presence);

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
                self.send(&channel.account, presence);
            }
            Event::ReadPassword(_) => {
                self.read_password.swap(true, Relaxed);
            }
            Event::Quit => {
                return Err(());
            }
            _ => {}
        }

        log::trace!("Fully handled event in {:.2?}", before.elapsed());
        Ok(())
    }

    fn handle_stanza(&mut self, account: Account, stanza: Element) {
        match stanza.name() {
            "iq" => match Iq::try_from(stanza) {
                Ok(iq) => self.handle_iq(account, iq),
                Err(err) => log::error!("Cannot parse IQ stanza: {}", err),
            },
            "presence" => match Presence::try_from(stanza) {
                Ok(presence) => self.schedule(Event::Presence(account, presence)),
                Err(err) => log::error!("{}", err),
            },
            "message" => match XmppParsersMessage::try_from(stanza) {
                Ok(message) => self.handle_xmpp_message(account, message, None, false),
                Err(err) => log::error!("{}", err),
            },
            _ => log::error!("unknown stanza: {}", stanza.name()),
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
        let mut message = message;

        let encryption_ns = message
            .payloads
            .iter()
            .find_map(|p| {
                xmpp_parsers::eme::ExplicitMessageEncryption::try_from((*p).clone())
                    .ok()
                    .map(|eme| eme.namespace)
            })
            .or(message.payloads.iter().find_map(|p| {
                legacy_omemo::Encrypted::try_from((*p).clone())
                    .ok()
                    .map(|_| xmpp_parsers::ns::LEGACY_OMEMO.to_string())
            }));

        // Decrypt if required
        // TODO EME can't be required
        if let (Some(encryption_ns), Some(from)) = (encryption_ns, message.from.clone()) {
            let mut crypto_engines = self.crypto_engines.lock().unwrap();
            if let Some(crypto_engine) = crypto_engines.get_mut(&(account.clone(), from.to_bare()))
            {
                if encryption_ns == crypto_engine.ns() {
                    message = match crypto_engine.decrypt(self, &account, &message) {
                        Ok(message) => message,
                        Err(err) => {
                            log::error!(
                                "Cannot decrypt message with {}: {}",
                                crypto_engine.ns(),
                                err
                            );
                            message
                        }
                    };
                } else {
                    log::warn!(
                        "Incompatible crypto engine found for {:?} (found {} expecting {})",
                        message.from,
                        crypto_engine.ns(),
                        encryption_ns
                    );
                }
            } else {
                log::warn!(
                    "No crypto engine found for {:?} (encrypted with {})",
                    message.from,
                    encryption_ns
                );
            }
        }

        let mods = self.mods.clone();
        for (_, r#mod) in mods.iter() {
            let message_match = r#mod
                .try_write()
                .unwrap()
                .can_handle_xmpp_message(self, &account, &message, &delay);
            if message_match > best_match {
                matched_mod = Some(r#mod);
                best_match = message_match;
            }
        }

        if let Some(r#mod) = matched_mod {
            log::debug!("Handling xmpp message by {:?}", r#mod);
            r#mod
                .try_write()
                .unwrap()
                .handle_xmpp_message(self, &account, &message, &delay, archive);
        } else {
            log::info!("Don't know how to handle message: {:?}", message);
        }
    }

    fn handle_iq(&mut self, account: Account, iq: Iq) {
        match iq {
            Iq::Error { error, .. } => {
                if let Some(text) = error.texts.get("en") {
                    let message = Message::log(text.clone());
                    self.schedule(Event::Message(Some(account.clone()), message));
                }
            }
            Iq::Result { payload, .. } => {
                log::info!("Received unexpected Iq result {:?}", payload);
            }
            other => {
                self.schedule(Event::Iq(account, other));
            }
        }
    }

    // TODO maybe use From<>
    pub fn proxy(&self) -> AparteAsync {
        AparteAsync {
            current_connection: self.current_connection.clone(),
            event_tx: self.event_tx.clone(),
            send_tx: self.send_tx.clone(),
            iq_tx: self.iq_tx.clone(),
            config: self.config.clone(),
            storage: self.storage.clone(),
            crypto_engines: self.crypto_engines.clone(),
        }
    }

    pub fn spawn<F>(future: F)
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        tokio::spawn(future);
    }

    pub fn add_command(&mut self, command_parser: CommandParser) {
        let command_parsers = Rc::get_mut(&mut self.command_parsers).unwrap();
        command_parsers.insert(command_parser.name.to_string(), command_parser);
    }

    // Common function for AparteAsync and Aparte, maybe share it in Trait
    pub fn add_crypto_engine(
        &mut self,
        account: &Account,
        recipient: &BareJid,
        crypto_engine: CryptoEngine,
    ) {
        let mut crypto_engines = self.crypto_engines.lock().unwrap();
        crypto_engines.insert((account.clone(), recipient.clone()), crypto_engine);
    }

    pub fn send<T>(&mut self, account: &Account, element: T)
    where
        T: TryInto<Element> + Debug,
    {
        match element.try_into() {
            Ok(stanza) => self.send_tx.send((account.clone(), stanza)).unwrap(),
            Err(_e) => {
                log::error!("Cannot convert to element");
            }
        };
    }

    pub fn schedule(&mut self, event: Event) {
        log::trace!("Schedule event {:?}", event);
        self.event_tx.send(event).unwrap();
    }

    pub fn log<T: IntoCharxels>(&mut self, message: T) {
        let message = Message::log(message.into_charxels());
        self.schedule(Event::Message(None, message));
    }

    pub fn error<T: Display>(&mut self, message: T, err: anyhow::Error) {
        let message = Message::log(format!("{}: {:#}", message, err));
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
    event_tx: mpsc::UnboundedSender<Event>,
    send_tx: mpsc::UnboundedSender<(Account, Element)>,
    crypto_engines: Arc<Mutex<HashMap<(Account, BareJid), CryptoEngine>>>,
    iq_tx: mpsc::UnboundedSender<(Account, IqEnvelope)>,
    pub config: Config,
    pub storage: Storage,
}

impl AparteAsync {
    pub fn send(&mut self, account: &Account, stanza: Element) {
        self.send_tx.send((account.clone(), stanza)).unwrap();
    }

    pub async fn iq(&mut self, account: &Account, iq: Iq) -> Result<IqResponse, IqFailure> {
        use tokio_xmpp::IqRequest;
        let (to, request) = match iq {
            Iq::Get { to, payload, .. } => (to, IqRequest::Get(payload)),
            Iq::Set { to, payload, .. } => (to, IqRequest::Set(payload)),
            _ => panic!("iq() called with non-request Iq variant"),
        };
        // IqResponseTracker matches responses by (from, id) but stores by (to, id).
        // Without an explicit `to`, the stored key is (None, id). The server replies
        // with from=bare_jid, giving lookup key (Some(bare_jid), id) — no match.
        // RFC 6121: self-addressed IQs (roster, etc.) should use the user's bare JID
        // as `to`; the server echoes that back as `from`, so both sides use the same key.
        let to = to.or_else(|| Some(Jid::from(account.to_bare())));
        let (token_tx, token_rx) = tokio::sync::oneshot::channel::<IqResponseToken>();
        let envelope = IqEnvelope {
            to,
            request,
            token_tx,
        };
        self.iq_tx.send((account.clone(), envelope)).unwrap();
        let token = token_rx
            .await
            .expect("connection task dropped before returning IqResponseToken");
        token.await
    }

    pub fn schedule(&mut self, event: Event) {
        self.event_tx.send(event).unwrap();
    }

    pub fn log<T: ToString>(&mut self, message: T) {
        let message = Message::log(message.to_string());
        self.schedule(Event::Message(None, message));
    }

    pub fn error<T: Display>(&mut self, message: T, err: anyhow::Error) {
        let message = Message::log(format!("{}: {:#}", message, err));
        self.schedule(Event::Message(None, message));
    }

    pub fn current_account(&self) -> Option<Account> {
        self.current_connection.clone()
    }

    pub fn add_crypto_engine(
        &mut self,
        account: &Account,
        recipient: &BareJid,
        crypto_engine: CryptoEngine,
    ) {
        let mut crypto_engines = self.crypto_engines.lock().unwrap();
        crypto_engines.insert((account.clone(), recipient.clone()), crypto_engine);
    }
}
