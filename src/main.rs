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
extern crate minidom;
#[macro_use]
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
use xmpp_parsers::iq::Iq;
use xmpp_parsers::message::{Message as XmppParsersMessage, MessageType as XmppParsersMessageType};
use xmpp_parsers::muc::Muc;
use xmpp_parsers::presence::{Presence, Show as PresenceShow, Type as PresenceType};
use xmpp_parsers::{Element, Jid};

mod core;
mod terminus;
mod plugins;

use crate::core::{Aparte, Plugin, Event, Command, CommandOrMessage, Message};

fn handle_stanza(aparte: Rc<Aparte>, stanza: Element) {
    if let Some(message) = XmppParsersMessage::try_from(stanza.clone()).ok() {
        handle_message(aparte, message);
    } else if let Some(iq) = Iq::try_from(stanza.clone()).ok() {
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

fn connect(aparte: Rc<Aparte>, command: Command) -> Result<(), ()> {
    match command.args.len() {
        1 => {
            Rc::clone(&aparte).event(Event::ReadPassword(command.clone()));
            Ok(())
        },
        2 => {
            let account = command.args[0].clone();
            let password = command.args[1].clone();

            if let Ok(jid) = Jid::from_str(&command.args[0]) {
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
            } else {
                Rc::clone(&aparte).log(format!("Invalid JID {}", command.args[0]));
                Err(())
            }
        }
        _ => {
            Rc::clone(&aparte).log(format!("Missing jid"));
            Err(())
        }
    }
}

fn win(aparte: Rc<Aparte>, command: Command) -> Result<(), ()> {
    if command.args.len() != 1 {
        Rc::clone(&aparte).log(format!("Missing windows name"));
        Err(())
    } else {
        aparte.event(Event::Win(command.args[0].clone()));
        Ok(())
    }
}

fn msg(aparte: Rc<Aparte>, command: Command) -> Result<(), ()> {
    match command.args.len() {
        0 => {
            Rc::clone(&aparte).log(format!("Missing contact and message"));
            Err(())
        },
        1 => {
            match aparte.current_connection() {
                Some(_connection) => {
                    match Jid::from_str(&command.args[0]) {
                        Ok(jid) => {
                            let to = match jid {
                                Jid::Bare(jid) => jid,
                                Jid::Full(jid) => jid.into(),
                            };
                            Rc::clone(&aparte).event(Event::Chat(to));
                            Ok(())
                        },
                        Err(err) => {
                            Rc::clone(&aparte).log(format!("Invalid JID {}: {}", command.args[0], err));
                            Err(())
                        }
                    }
                },
                None => {
                    Rc::clone(&aparte).log(format!("No connection found"));
                    Ok(())
                }
            }
        },
        2 => {
            match aparte.current_connection() {
                Some(connection) => {
                    match Jid::from_str(&command.args[0]) {
                        Ok(to) => {
                            let id = Uuid::new_v4().to_string();
                            let from: Jid = connection.into();
                            let timestamp = Utc::now();
                            let message = Message::outgoing_chat(id, timestamp, &from, &to, &command.args[1]);
                            Rc::clone(&aparte).event(Event::Message(message.clone()));

                            aparte.send(Element::try_from(message).unwrap());

                            let to = match to {
                                Jid::Bare(jid) => jid,
                                Jid::Full(jid) => jid.into(),
                            };
                            Rc::clone(&aparte).event(Event::Chat(to));

                            Ok(())
                        },
                        Err(err) => {
                            Rc::clone(&aparte).log(format!("Invalid JID {}: {}", command.args[0], err));
                            Err(())
                        }
                    }
                },
                None => {
                    Rc::clone(&aparte).log(format!("No connection found"));
                    Ok(())
                }
            }
        },
        _ => {
            Rc::clone(&aparte).log(format!("Too many arguments"));
            Err(())
        }
    }
}

fn join(aparte: Rc<Aparte>, command: Command) -> Result<(), ()> {
    match command.args.len() {
        0 => {
            Rc::clone(&aparte).log(format!("Missing Muc"));
            Err(())
        },
        1 => {
            match aparte.current_connection() {
                Some(connection) => {
                    match Jid::from_str(&command.args[0]) {
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
                            presence = presence.with_to(Some(Jid::Full(to.clone())));
                            presence = presence.with_from(Some(from));
                            presence.add_payload(Muc::new());
                            aparte.send(presence.into());
                            aparte.event(Event::Join(to.clone()));

                            Ok(())
                        },
                        Err(err) => {
                            Rc::clone(&aparte).log(format!("Invalid JID {}: {}", command.args[0], err));
                            Err(())
                        }
                    }
                },
                None => {
                    Rc::clone(&aparte).log(format!("No connection found"));
                    Ok(())
                }
            }
        },
        _ => {
            Rc::clone(&aparte).log(format!("Too many arguments"));
            Err(())
        }
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
    if let Err(e) = simple_logging::log_to_file(aparte_log, LevelFilter::Debug) {
        panic!("Cannot setup log to file: {}", e);
    }

    info!("Starting aparté");

    let mut aparte = Aparte::new();
    aparte.add_plugin(plugins::disco::Disco::new());
    aparte.add_plugin(plugins::carbons::CarbonsPlugin::new());
    aparte.add_plugin(plugins::contact::ContactPlugin::new());
    aparte.add_plugin(plugins::conversation::ConversationPlugin::new());
    aparte.add_plugin(plugins::ui::UIPlugin::new());

    aparte.add_command("connect", connect);
    aparte.add_command("win", win);
    aparte.add_command("msg", msg);
    aparte.add_command("join", join);

    aparte.init().unwrap();

    let aparte = Rc::new(aparte);

    Rc::clone(&aparte).log(format!("Welcome to Aparté {}", VERSION));

    let mut rt = Runtime::new().unwrap();
    let command_stream = {
        let ui = aparte.get_plugin::<plugins::ui::UIPlugin>().unwrap();
        ui.command_stream(Rc::clone(&aparte))
    };

    let sig_aparte = Rc::clone(&aparte); // TODO use ARC ?
    let signals = Signals::new(&[signal_hook::SIGWINCH]).unwrap().into_async().unwrap().for_each(move |sig| {
        Rc::clone(&sig_aparte).event(Event::Signal(sig));
        Ok(())
    }).map_err(|e| panic!("{}", e));

    rt.spawn(signals);

    rt.block_on(command_stream.for_each(move |command_or_message| {
        match command_or_message {
            CommandOrMessage::Message(message) => {
                Rc::clone(&aparte).event(Event::Message(message.clone()));
                if let Ok(xmpp_message) = Element::try_from(message) {
                    aparte.send(xmpp_message);
                }
            }
            CommandOrMessage::Command(command) => {
                let result = {
                    Rc::clone(&aparte).parse_command(command.clone())
                };

                if result.is_err() {
                    Rc::clone(&aparte).log(format!("Unknown command {}", command.name));
                }
            }
        };

        Ok(())
    }));
}
