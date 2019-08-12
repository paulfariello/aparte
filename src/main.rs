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
use std::env;
use std::path::Path;
use std::rc::Rc;
use tokio::runtime::current_thread::Runtime;
use tokio_xmpp::{Client, Packet};
use xmpp_parsers::Element;
use xmpp_parsers::carbons;
use xmpp_parsers::message::{Message, MessageType};
use xmpp_parsers::presence::{Presence, Show as PresenceShow, Type as PresenceType};

mod core;
mod plugins;

use plugins::{Plugin, PluginManager};
use plugins::ui::CommandStream;

fn main_loop(mgr: Rc<PluginManager>) {
    let mut rt = Runtime::new().unwrap();
    let command_stream = {
        let ui = mgr.get::<plugins::ui::UIPlugin>().unwrap();
        ui.command_stream(Rc::clone(&mgr))
    };
    let mgr = Rc::clone(&mgr);

    let commands = command_stream.for_each(move |command| {
        let mgr = Rc::clone(&mgr);
        if ! command.command.starts_with("/") {
            // TODO send message
            let mut message = core::Message::outgoing(command.command);
            mgr.on_message(&mut message);
        } else {
            match command.command.as_ref() {
                "/connect" => {
                    let account = "needle@trashserver.net";
                    info!("Connecting to {}", account);
                    let client = Client::new(account, "pass").unwrap();

                    let (mut sink, stream) = client.split();

                    let client = stream.for_each(move |event| {
                        let mgr = Rc::clone(&mgr);
                        if event.is_online() {
                            info!("Connected as {}", account);

                            mgr.on_connect(&mut sink);

                            let mut presence = Presence::new(PresenceType::None);
                            presence.show = Some(PresenceShow::Chat);

                            sink.start_send(Packet::Stanza(presence.into())).unwrap();
                        } else if let Some(stanza) = event.into_stanza() {
                            trace!("RECV: {}", String::from(&stanza));

                            handle_stanza(mgr, stanza);
                        }

                        future::ok(())
                    }).map_err(|e| {
                        error!("Err: {:?}", e);
                    });

                    tokio::runtime::current_thread::spawn(client);

                },
                _ => {
                    let mut message = core::Message::log(format!("Unknown command {}", command.command));
                    mgr.on_message(&mut message);
                }
            }
        }

        Ok(())
    });

    let res = rt.block_on(commands);
    info!("! {:?}", res);
}

fn handle_stanza(mgr: Rc<PluginManager>, stanza: Element) {
    if let Some(message) = Message::try_from(stanza).ok() {
        handle_message(mgr, message);
    }
}

fn handle_message(mgr: Rc<PluginManager>, message: Message) {
    let from = match message.from {
        Some(from) => from,
        None => return,
    };

    if let Some(ref body) = message.bodies.get("") {
        if message.type_ != MessageType::Error {
            let mut message = core::Message::incoming(from.clone(), body.0.clone());
            mgr.on_message(&mut message);
        }
    }

    for payload in message.payloads {
        if let Some(received) = carbons::Received::try_from(payload).ok() {
            if let Some(ref original) = received.forwarded.stanza {
                if message.type_ != MessageType::Error {
                    if let Some(body) = original.bodies.get("") {
                        let mut message = core::Message::incoming(from.clone(), body.0.clone());
                        mgr.on_message(&mut message);
                    }
                }
            }
        }
    }
}

fn main() {
    let data_dir = dirs::data_dir().unwrap();

    let aparte_data = data_dir.join("aparté");

    if let Err(e) = std::fs::create_dir_all(&aparte_data) {
        panic!("Cannot create aparté data dir: {}", e);
    }

    let aparte_log = aparte_data.join("aparte.log");
    simple_logging::log_to_file(aparte_log, LevelFilter::Info);

    info!("Starting aparté");

    let mut plugin_manager = PluginManager::new();
    plugin_manager.add::<plugins::disco::Disco>(Box::new(plugins::disco::Disco::new())).unwrap();
    plugin_manager.add::<plugins::carbons::CarbonsPlugin>(Box::new(plugins::carbons::CarbonsPlugin::new())).unwrap();
    plugin_manager.add::<plugins::ui::UIPlugin>(Box::new(plugins::ui::UIPlugin::new())).unwrap();

    plugin_manager.init().unwrap();

    main_loop(Rc::new(plugin_manager))
}
