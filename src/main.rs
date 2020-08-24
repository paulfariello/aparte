/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
#![feature(drain_filter)]
#![feature(trait_alias)]
#![feature(specialization)]
#[macro_use]
extern crate log;
extern crate simple_logging;
extern crate tokio;
extern crate tokio_xmpp;
extern crate xmpp_parsers;
extern crate rpassword;
extern crate futures;
extern crate derive_error;
extern crate tokio_file_unix;
extern crate dirs;
extern crate signal_hook;

use chrono::Utc;
use futures::{future, Future, Sink, Stream};
use log::LevelFilter;
use signal_hook::iterator::Signals;
use std::convert::TryFrom;
use std::rc::Rc;
use std::str::FromStr;
use tokio::runtime::current_thread::Runtime;
use tokio_xmpp::{Client, Error as XmppError};
use uuid::Uuid;
use xmpp_parsers::iq::{Iq, IqType};
use xmpp_parsers::message::{Message as XmppParsersMessage, MessageType as XmppParsersMessageType};
use xmpp_parsers::muc::Muc;
use xmpp_parsers::presence::{Presence, Show as PresenceShow, Type as PresenceType};
use xmpp_parsers::{Element, Jid};

mod core;
mod config;
mod account;
mod contact;
mod conversation;
mod message;
mod command;
mod terminus;
mod plugins;

use crate::core::{Aparte, Plugin, Event};
use crate::message::{Message};
use crate::command::{CommandParser, Command};

fn handle_stanza(aparte: Rc<Aparte>, stanza: Element) {
    if let Some(message) = XmppParsersMessage::try_from(stanza.clone()).ok() {
        handle_message(aparte, message);
    } else if let Some(iq) = Iq::try_from(stanza.clone()).ok() {
        if let IqType::Error(stanza) = iq.payload.clone() {
            if let Some(text) = stanza.texts.get("en") {
              let message = Message::log(text.clone());
              Rc::clone(&aparte).event(Event::Message(message));
            }
        }
        Rc::clone(&aparte).event(Event::Iq(iq));
    } else if let Some(presence) = Presence::try_from(stanza.clone()).ok() {
        Rc::clone(&aparte).event(Event::Presence(presence));
    }
}

fn handle_message(aparte: Rc<Aparte>, message: XmppParsersMessage) {
    if let (Some(from), Some(to)) = (message.from, message.to) {
        if let Some(ref body) = message.bodies.get("") {
            match message.type_ {
                XmppParsersMessageType::Error => {},
                XmppParsersMessageType::Chat => {
                    let id = message.id.unwrap_or_else(|| Uuid::new_v4().to_string());
                    let timestamp = Utc::now();
                    let message = Message::incoming_chat(id, timestamp, &from, &to, &body.0);
                    Rc::clone(&aparte).event(Event::Message(message));
                },
                XmppParsersMessageType::Groupchat => {
                    let id = message.id.unwrap_or_else(|| Uuid::new_v4().to_string());
                    let timestamp = Utc::now();
                    let message = Message::incoming_groupchat(id, timestamp, &from, &to, &body.0);
                    Rc::clone(&aparte).event(Event::Message(message));
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
                                Rc::clone(&aparte).event(Event::Message(message));
                            }
                        }
                    }
                }
            }
        }
    }
}

command_def!{
    connect,
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
    account: {
        completion: |aparte, _command| {
            aparte.config.accounts.iter().map(|(name, account)| name.clone()).collect()
        }
    },
    (password) password,
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


        Rc::clone(&aparte).log(format!("Connecting to {}", account));
        let client = Client::new(&full_jid.to_string(), &password).unwrap();

        let (sink, stream) = client.split();
        let (tx, rx) = futures::unsync::mpsc::unbounded();

        Rc::clone(&aparte).add_connection(full_jid.clone(), tx);

        tokio::runtime::current_thread::spawn(
            rx.forward(
                sink.sink_map_err(|_| panic!("Pipe"))
                ).map(|(rx, mut sink)| {
                drop(rx);
                let _ = sink.close();
            }).map_err(|e| {
                    panic!("Send error: {:?}", e);
                })
            );

        let event_aparte = Rc::clone(&aparte);
        let client = stream.for_each(move |event| {
            if event.is_online() {
                Rc::clone(&event_aparte).log(format!("Connected as {}", account));

                Rc::clone(&event_aparte).event(Event::Connected(full_jid.clone()));

                let mut presence = Presence::new(PresenceType::None);
                presence.show = Some(PresenceShow::Chat);

                event_aparte.send(presence.into());
            } else if let Some(stanza) = event.into_stanza() {
                debug!("RECV: {}", String::from(&stanza));

                handle_stanza(Rc::clone(&event_aparte), stanza);
            }

            future::ok(())
        });

        let error_aparte = Rc::clone(&aparte);
        let client = client.map_err(move |error| {
            match error {
                XmppError::Auth(auth) => {
                    Rc::clone(&error_aparte).log(format!("Authentication failed {}", auth));
                },
                error => {
                    Rc::clone(&error_aparte).log(format!("Connection error {:?}", error));
                },
            }
        });

        tokio::runtime::current_thread::spawn(client);

        Ok(())
    }
}

command_def!{
    win,
    r#"Usage: /win <window>

  window        Name of the window to switch to

Description:
  Switch to a given window.

Examples:
  /win console
  /win contact@server.tld"#,
    window: {
        completion: |aparte, _command| {
            let ui = aparte.get_plugin::<plugins::ui::UIPlugin>().unwrap();
            ui.get_windows()
        }
    },
    |aparte, _command| {
        aparte.event(Event::Win(window.clone()));
        Ok(())
    }
}

command_def!{
    msg,
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
    contact: {
        completion: |aparte, _command| {
            let contact = aparte.get_plugin::<plugins::contact::ContactPlugin>().unwrap();
            contact.contacts.iter().map(|c| c.0.to_string()).collect()
        }
    },
    (optional) message,
    |aparte, _command| {
        match aparte.current_connection() {
            Some(connection) => {
                match Jid::from_str(&contact.clone()) {
                    Ok(jid) => {
                        let to = match jid.clone() {
                            Jid::Bare(jid) => jid,
                            Jid::Full(jid) => jid.into(),
                        };
                        Rc::clone(&aparte).event(Event::Chat(to));
                        if message.is_some() {
                            let id = Uuid::new_v4().to_string();
                            let from: Jid = connection.into();
                            let timestamp = Utc::now();
                            let message = Message::outgoing_chat(id, timestamp, &from, &jid, &message.unwrap());
                            Rc::clone(&aparte).event(Event::Message(message.clone()));

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
    }
}

command_def!{
    join,
    r#"/join <channel>

  channel       Channel JID to join
Description:
  Open a window and join a given channel.

Example:
  /join channel@conference.server.tld"#,
    muc,
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
                        aparte.event(Event::Join(to.clone()));

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
    }
}

command_def!{
    quit,
    r#"/quit

Description:
  Quit Aparté.

Example:
  /quit"#,
    |aparte, _command| {
        aparte.event(Event::Quit);

        Ok(())
    }
}

command_def!{
    help,
    r#"/help <command>

  command       Name of command

Description:
  Print help of a given command.

Examples:
  /help help
  /help win"#,
    cmd: {
        completion: |aparte, _command| {
            aparte.commands.iter().map(|c| c.0.to_string()).collect()
        }
    },
    |aparte, _command| {
        let command = aparte.commands.get(&cmd);
        match command {
            Some(command) => Rc::clone(&aparte).log(command.help.to_string()),
            None => Rc::clone(&aparte).log(format!("Unknown command {}", cmd)),
        }

        Ok(())
    }
}

fn main() {
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    let data_dir = dirs::data_dir().unwrap();
    let aparte_data = data_dir.join("aparté");

    if let Err(e) = std::fs::create_dir_all(&aparte_data) {
        panic!("Cannot create aparté data dir: {}", e);
    }

    let aparte_log = aparte_data.join("aparte.log");
    if let Err(e) = simple_logging::log_to_file(aparte_log, LevelFilter::Info) {
        panic!("Cannot setup log to file: {}", e);
    }

    let conf_dir = dirs::config_dir().unwrap();
    let aparte_conf = conf_dir.join("aparté");

    if let Err(e) = std::fs::create_dir_all(&aparte_conf) {
        panic!("Cannot create aparté data dir: {}", e);
    }

    let config = aparte_conf.join("config.toml");

    info!("Starting aparté");

    let mut aparte = Aparte::new(config);
    aparte.add_plugin(plugins::completion::CompletionPlugin::new());
    aparte.add_plugin(plugins::carbons::CarbonsPlugin::new());
    aparte.add_plugin(plugins::contact::ContactPlugin::new());
    aparte.add_plugin(plugins::conversation::ConversationPlugin::new());
    aparte.add_plugin(plugins::disco::Disco::new());
    aparte.add_plugin(plugins::ui::UIPlugin::new());
    aparte.add_plugin(plugins::mam::MamPlugin::new());

    aparte.add_command(help());
    aparte.add_command(connect());
    aparte.add_command(win());
    aparte.add_command(msg());
    aparte.add_command(join());
    aparte.add_command(quit());

    aparte.init().unwrap();

    let aparte = Rc::new(aparte);

    Rc::clone(&aparte).log(r#"
▌ ▌   ▜               ▐      ▞▀▖         ▐   ▞
▌▖▌▞▀▖▐ ▞▀▖▞▀▖▛▚▀▖▞▀▖ ▜▀ ▞▀▖ ▙▄▌▛▀▖▝▀▖▙▀▖▜▀ ▞▀▖
▙▚▌▛▀ ▐ ▌ ▖▌ ▌▌▐ ▌▛▀  ▐ ▖▌ ▌ ▌ ▌▙▄▘▞▀▌▌  ▐ ▖▛▀
▘ ▘▝▀▘ ▘▝▀ ▝▀ ▘▝ ▘▝▀▘  ▀ ▝▀  ▘ ▘▌  ▝▀▘▘   ▀ ▝▀▘
"#.to_string());
    Rc::clone(&aparte).log(format!("Version: {}", VERSION));

    let mut rt = Runtime::new().unwrap();
    let event_stream = {
        let ui = aparte.get_plugin::<plugins::ui::UIPlugin>().unwrap();
        ui.event_stream(Rc::clone(&aparte))
    };

    let sig_aparte = Rc::clone(&aparte); // TODO use ARC ?
    let signals = Signals::new(&[signal_hook::SIGWINCH]).unwrap().into_async().unwrap().for_each(move |sig| {
        Rc::clone(&sig_aparte).event(Event::Signal(sig));
        Ok(())
    }).map_err(|e| panic!("{}", e));

    rt.spawn(signals);

    if let Err(e) = rt.block_on(event_stream.for_each(move |event| {
        Rc::clone(&aparte).event(event);

        Ok(())
    })) {
      info!("Error in event stream: {}", e);
    }

}
