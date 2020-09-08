/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
#![feature(stmt_expr_attributes)]
#![feature(drain_filter)]
#![feature(trait_alias)]
#![allow(incomplete_features)]
#![feature(specialization)]
#[macro_use]
extern crate log;
extern crate flexi_logger;
extern crate tokio;
extern crate tokio_xmpp;
extern crate xmpp_parsers;
extern crate rpassword;
extern crate futures;
extern crate derive_error;
extern crate dirs;

mod core;
mod config;
mod account;
mod contact;
mod conversation;
mod message;
#[macro_use]
mod command;
mod terminus;
mod plugins;

use crate::core::{Aparte, Plugin};

fn main() {
    let data_dir = dirs::data_dir().unwrap();
    let aparte_data = data_dir.join("aparté");

    if let Err(e) = std::fs::create_dir_all(&aparte_data) {
        panic!("Cannot create aparté data dir: {}", e);
    }

    let file_writer = flexi_logger::writers::FileLogWriter::builder().directory(aparte_data).suppress_timestamp().try_build().unwrap();
    let log_target = flexi_logger::LogTarget::Writer(Box::new(file_writer));
    let logger = flexi_logger::Logger::with_env_or_str("info").log_target(log_target);
    if let Err(e) = logger.start() {
      panic!("Cannot start logger: {}", e);
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
    aparte.add_plugin(plugins::bookmarks::BookmarksPlugin::new());
    aparte.add_plugin(plugins::ui::UIPlugin::new());
    aparte.add_plugin(plugins::mam::MamPlugin::new());

    aparte.init().unwrap();

    aparte.run();

}
