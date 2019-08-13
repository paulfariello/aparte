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

use plugins::ui::CommandStream;
use crate::core::{ Plugin, PluginManager, Command, CommandManager, CommandParser, CommandOrMessage };

fn main_loop(commands: CommandManager, plugins: Rc<PluginManager>) {
    let mut rt = Runtime::new().unwrap();
    let command_stream = {
        let ui = plugins.get::<plugins::ui::UIPlugin>().unwrap();
        ui.command_stream(Rc::clone(&plugins))
    };
    let plugins = Rc::clone(&plugins);

    let commands_fut = command_stream.for_each(move |command_or_message| {
        match command_or_message {
            CommandOrMessage::Message(mut message) => {
                let plugins = Rc::clone(&plugins);
                plugins.on_message(&mut message);
            }
            CommandOrMessage::Command(command) => {
                let result = {
                    let plugins = Rc::clone(&plugins);
                    commands.parse(plugins, &command)
                };

                if result.is_err() {
                    let plugins = Rc::clone(&plugins);
                    plugins.log(format!("Unknown command {}", command.name));
                }
            }
        };

        Ok(())
    });

    let res = rt.block_on(commands_fut);
    info!("! {:?}", res);
}

fn handle_stanza(plugins: Rc<PluginManager>, stanza: Element) {
    if let Some(message) = Message::try_from(stanza).ok() {
        handle_message(plugins, message);
    }
}

fn handle_message(plugins: Rc<PluginManager>, message: Message) {
    if let (Some(from), Some(to)) = (message.from, message.to) {
        if let Some(ref body) = message.bodies.get("") {
            if message.type_ != MessageType::Error {
                let mut message = core::Message::incoming(&from, &to, &body.0);
                plugins.on_message(&mut message);
            }
        }

        for payload in message.payloads {
            if let Some(received) = carbons::Received::try_from(payload).ok() {
                if let Some(ref original) = received.forwarded.stanza {
                    if message.type_ != MessageType::Error {
                        if let Some(body) = original.bodies.get("") {
                            let mut message = core::Message::incoming(&from, &to, &body.0);
                            plugins.on_message(&mut message);
                        }
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

    let connect = |plugins: Rc<PluginManager>, command: &Command| {
        match command.args.len() {
            1 => {
                let mut ui = plugins.get_mut::<plugins::ui::UIPlugin>().unwrap();
                ui.read_password(command.clone());
                Ok(())
            },
            2 => {
                let account = command.args[0].clone();
                let password = command.args[1].clone();
                plugins.log(format!("Connecting to {}", account));
                let client = Client::new(&account, "pass").unwrap();

                let (mut sink, stream) = client.split();

                let client = stream.for_each(move |event| {
                    let plugins = Rc::clone(&plugins);
                    if event.is_online() {
                        plugins.log(format!("Connected as {}", account));

                        plugins.on_connect(&mut sink);

                        let mut presence = Presence::new(PresenceType::None);
                        presence.show = Some(PresenceShow::Chat);

                        sink.start_send(Packet::Stanza(presence.into())).unwrap();
                    } else if let Some(stanza) = event.into_stanza() {
                        trace!("RECV: {}", String::from(&stanza));

                        handle_stanza(plugins, stanza);
                    }

                    future::ok(())
                }).map_err(|e| {
                    error!("Err: {:?}", e);
                });

                tokio::runtime::current_thread::spawn(client);
                Ok(())
            }
            _ => {
                plugins.log(format!("Missing jid"));
                Err(())
            }
        }
    };

    let win = |plugins: Rc<PluginManager>, command: &Command| {
        if command.args.len() != 1 {
            plugins.log(format!("Missing windows name"));
            Err(())
        } else {
            let result = {
                let mut ui = plugins.get_mut::<plugins::ui::UIPlugin>().unwrap();
                ui.switch(&command.args[0])
            };

            if result.is_err() {
                plugins.log(format!("Unknown window {}", &command.args[0]));
            };
            Ok(())
        }
    };

    let mut command_manager = CommandManager::new();
    command_manager.add_command("connect", &connect);
    command_manager.add_command("win", &win);

    main_loop(command_manager, Rc::new(plugin_manager))
}
