/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
#![cfg_attr(feature = "strict", deny(warnings))]
#![allow(incomplete_features)]
#[macro_use]
extern crate log;
extern crate derive_error;
extern crate dirs;
extern crate flexi_logger;
extern crate futures;
extern crate rpassword;
extern crate tokio;
extern crate tokio_xmpp;
extern crate xmpp_parsers;

#[macro_use]
mod terminus;
mod account;
mod config;
mod contact;
mod conversation;
mod core;
mod message;
#[macro_use]
mod command;
mod color;
mod cursor;
mod mods;
mod word;

use crate::core::{Aparte, Mod};

fn main() {
    let data_dir = dirs::data_dir().unwrap();
    let aparte_data = data_dir.join("aparte");

    if let Err(e) = std::fs::create_dir_all(&aparte_data) {
        panic!("Cannot create aparte data dir: {}", e);
    }

    let file_writer = flexi_logger::writers::FileLogWriter::builder()
        .directory(aparte_data)
        .suppress_timestamp()
        .try_build()
        .unwrap();
    let log_target = flexi_logger::LogTarget::Writer(Box::new(file_writer));
    let logger = flexi_logger::Logger::with_env_or_str("info").log_target(log_target);
    if let Err(e) = logger.start() {
        panic!("Cannot start logger: {}", e);
    }

    let conf_dir = dirs::config_dir().unwrap();
    let aparte_conf = conf_dir.join("aparte");

    if let Err(e) = std::fs::create_dir_all(&aparte_conf) {
        panic!("Cannot create aparte data dir: {}", e);
    }

    let config = aparte_conf.join("config.toml");

    info!("Starting aparté");

    let mut aparte = Aparte::new(config);
    aparte.add_mod(Mod::Completion(mods::completion::CompletionMod::new()));
    aparte.add_mod(Mod::Carbons(mods::carbons::CarbonsMod::new()));
    aparte.add_mod(Mod::Contact(mods::contact::ContactMod::new()));
    aparte.add_mod(Mod::Conversation(mods::conversation::ConversationMod::new()));
    aparte.add_mod(Mod::Disco(mods::disco::DiscoMod::new()));
    aparte.add_mod(Mod::Bookmarks(mods::bookmarks::BookmarksMod::new()));
    aparte.add_mod(Mod::UI(mods::ui::UIMod::new()));
    aparte.add_mod(Mod::Mam(mods::mam::MamMod::new()));
    aparte.add_mod(Mod::Messages(mods::messages::MessagesMod::new()));

    aparte.init().unwrap();

    aparte.run();
}
