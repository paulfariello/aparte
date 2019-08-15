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

use futures::{future, Future, Sink, Stream};
use log::LevelFilter;
use std::convert::TryFrom;
use std::rc::Rc;
use tokio::runtime::current_thread::Runtime;
use tokio_xmpp::Client;
use uuid::Uuid;
use xmpp_parsers::{Element, Jid, FullJid};
use xmpp_parsers::presence::{Presence, Show as PresenceShow, Type as PresenceType};
use std::str::FromStr;

mod core;
mod plugins;

use crate::core::{Aparte, Plugin, Command, CommandOrMessage, Message};

fn handle_stanza(aparte: Rc<Aparte>, stanza: Element) {
    if let Some(message) = xmpp_parsers::message::Message::try_from(stanza).ok() {
        handle_message(aparte, message);
    }
}

fn handle_message(aparte: Rc<Aparte>, message: xmpp_parsers::message::Message) {
    if let (Some(from), Some(to)) = (message.from, message.to) {
        if let Some(ref body) = message.bodies.get("") {
            if message.type_ != xmpp_parsers::message::MessageType::Error {
                let id = message.id.unwrap_or_else(|| Uuid::new_v4().to_string());
                let mut message = Message::incoming(id, &from, &to, &body.0);
                Rc::clone(&aparte).on_message(&mut message);
            }
        }

        for payload in message.payloads {
            if let Some(received) = xmpp_parsers::carbons::Received::try_from(payload).ok() {
                if let Some(ref original) = received.forwarded.stanza {
                    if message.type_ != xmpp_parsers::message::MessageType::Error {
                        if let Some(body) = original.bodies.get("") {
                            let id = original.id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
                            let mut message = Message::incoming(id, &from, &to, &body.0);
                            Rc::clone(&aparte).on_message(&mut message);
                        }
                    }
                }
            }
        }
    }
}

fn connect(aparte: Rc<Aparte>, command: &Command) -> Result<(), ()> {
    match command.args.len() {
        1 => {
            let mut ui = aparte.get_plugin_mut::<plugins::ui::UIPlugin>().unwrap();
            ui.read_password(command.clone());
            Ok(())
        },
        2 => {
            let account = command.args[0].clone();
            let password = command.args[1].clone();

            if let Ok(jid) = FullJid::from_str(&command.args[0]) {
                Rc::clone(&aparte).log(format!("Connecting to {}", account));
                let client = Client::new(&account, &password).unwrap();

                let (sink, stream) = client.split();
                let (tx, rx) = futures::unsync::mpsc::unbounded();

                Rc::clone(&aparte).add_connection(jid, tx);

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

                let client = stream.for_each(move |event| {
                    if event.is_online() {
                        Rc::clone(&aparte).log(format!("Connected as {}", account));

                        Rc::clone(&aparte).on_connect();

                        let mut presence = Presence::new(PresenceType::None);
                        presence.show = Some(PresenceShow::Chat);

                        aparte.send(presence.into());
                    } else if let Some(stanza) = event.into_stanza() {
                        trace!("RECV: {}", String::from(&stanza));

                        handle_stanza(Rc::clone(&aparte), stanza);
                    }

                    future::ok(())
                }).map_err(|e| {
                    error!("Err: {:?}", e);
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

fn win(aparte: Rc<Aparte>, command: &Command) -> Result<(), ()> {
    if command.args.len() != 1 {
        Rc::clone(&aparte).log(format!("Missing windows name"));
        Err(())
    } else {
        let result = {
            let mut ui = aparte.get_plugin_mut::<plugins::ui::UIPlugin>().unwrap();
            ui.switch(&command.args[0])
        };

        if result.is_err() {
            Rc::clone(&aparte).log(format!("Unknown window {}", &command.args[0]));
        };
        Ok(())
    }
}

fn main() {
    let data_dir = dirs::data_dir().unwrap();

    let aparte_data = data_dir.join("aparté");

    if let Err(e) = std::fs::create_dir_all(&aparte_data) {
        panic!("Cannot create aparté data dir: {}", e);
    }

    let aparte_log = aparte_data.join("aparte.log");
    if let Err(e) = simple_logging::log_to_file(aparte_log, LevelFilter::Trace) {
        panic!("Cannot setup log to file: {}", e);
    }

    info!("Starting aparté");

    let mut aparte = Aparte::new();
    aparte.add_plugin::<plugins::disco::Disco>(Box::new(plugins::disco::Disco::new())).unwrap();
    aparte.add_plugin::<plugins::carbons::CarbonsPlugin>(Box::new(plugins::carbons::CarbonsPlugin::new())).unwrap();
    aparte.add_plugin::<plugins::ui::UIPlugin>(Box::new(plugins::ui::UIPlugin::new())).unwrap();

    aparte.add_command("connect", connect);
    aparte.add_command("win", win);

    aparte.init().unwrap();

    let aparte = Rc::new(aparte);

    let mut rt = Runtime::new().unwrap();
    let command_stream = {
        let ui = aparte.get_plugin::<plugins::ui::UIPlugin>().unwrap();
        ui.command_stream(Rc::clone(&aparte))
    };

    let res = rt.block_on(command_stream.for_each(move |command_or_message| {
        match command_or_message {
            CommandOrMessage::Message(mut message) => {
                Rc::clone(&aparte).on_message(&mut message);
                match message {
                    Message::Incoming(_message) => {},
                    Message::Outgoing(message) => {
                        let mut xmpp_message = xmpp_parsers::message::Message::new(Some(Jid::Bare(message.to)));
                        xmpp_message.bodies.insert(String::new(), xmpp_parsers::message::Body(message.body));
                        aparte.send(xmpp_message.into());
                    },
                    Message::Log(_log) => {},
                }
            }
            CommandOrMessage::Command(command) => {
                let result = {
                    Rc::clone(&aparte).parse_command(&command)
                };

                if result.is_err() {
                    Rc::clone(&aparte).log(format!("Unknown command {}", command.name));
                }
            }
        };

        Ok(())
    }));

    info!("! {:?}", res);
}
