/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
#![cfg_attr(feature = "strict", deny(warnings))]
#![allow(incomplete_features)]

use anyhow::Result;
use clap::Parser;

#[macro_use]
mod terminus;
mod account;
mod async_iq;
mod config;
mod contact;
mod conversation;
mod core;
mod message;
#[macro_use]
mod command;
mod color;
mod crypto;
mod cursor;
mod i18n;
mod image;
mod mods;
mod storage;
mod word;

use crate::core::Aparte;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the config file
    #[arg(short, long)]
    config: Option<std::path::PathBuf>,
    /// Path to the shared dir
    #[arg(short, long)]
    shared: Option<std::path::PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let aparte_data = if let Some(shared) = args.shared {
        shared
    } else {
        let data_dir = dirs::data_dir().unwrap();
        let aparte_data = data_dir.join("aparte");

        if let Err(e) = std::fs::create_dir_all(&aparte_data) {
            panic!("Cannot create aparte data dir: {}", e);
        }

        aparte_data
    };

    let logger = flexi_logger::Logger::try_with_env_or_str("info")?.log_to_file(
        flexi_logger::FileSpec::default()
            .directory(&aparte_data)
            .suppress_timestamp(),
    );
    if let Err(e) = logger.start() {
        panic!("Cannot start logger: {}", e);
    }

    let config = if let Some(config) = args.config {
        config
    } else {
        let conf_dir = dirs::config_dir().unwrap();
        let aparte_conf = conf_dir.join("aparte");

        if let Err(e) = std::fs::create_dir_all(&aparte_conf) {
            panic!("Cannot create aparte data dir: {}", e);
        }

        aparte_conf.join("config.toml")
    };

    // TODO
    let storage = aparte_data.join("storage.sqlite");

    log::info!("Starting aparté");

    let mut aparte = Aparte::new(config, storage)?;

    aparte.init().unwrap();

    aparte.run();

    Ok(())
}
