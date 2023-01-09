/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
#![deny(warnings)]
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
mod i18n;
mod mods;
mod word;

use crate::core::Aparte;

fn help() {
    println!("aparte [BASEDIR]");
    println!("----------------");
    println!("  -h|--help|help - Display this help message");
    println!("  BASEDIR: Directory for config and logs (defaults to ~/.config/aparte and ~/.local/share/aparte)");
}

fn main() {
    let mut args = std::env::args();

    let basedir = if let Some(arg) = args.nth(1) {
        if arg == "help" || arg == "--help" || arg == "-h" {
            help();
            return;
        } else {
            Some(arg)
        }
    } else {
        None
    };

    let (aparte_conf, aparte_data) = if let Some(dir) = basedir {
        // Explicit basedir from arg 1
        let dir = std::path::PathBuf::from(&dir);
        if ! dir.is_dir() {
            panic!("Provided basedir {} is not a folder", dir.display());
        }
        (dir.clone(), dir.clone())
    } else {
        // Default XDG basedirs (~/.config/aparte and ~/.local/share/aparte)
        let (aparte_conf, aparte_data) = (dirs::config_dir().unwrap().join("aparte"), dirs::data_dir().unwrap().join("aparte"));

        if let Err(e) = std::fs::create_dir_all(&aparte_data) {
            panic!("Cannot create aparte data dir: {}", e);
        }

        if let Err(e) = std::fs::create_dir_all(&aparte_conf) {
            panic!("Cannot create aparte data dir: {}", e);
        }

        (aparte_conf, aparte_data)
    };

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

    let config = aparte_conf.join("config.toml");

    info!("Starting aparté");

    let mut aparte = Aparte::new(config);

    aparte.init().unwrap();

    aparte.run();
}
