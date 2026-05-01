/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use terminus::{BgColor, Color};

use crate::account::ConnectionInfo;
use crate::color::ColorTuple;

fn true_() -> bool {
    true
}

fn default_selected_message() -> BgColor {
    BgColor(Color::Rgb(49, 50, 68))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub accounts: HashMap<String, ConnectionInfo>,
    #[serde(default = "true_")]
    pub bell: bool,
    pub theme: Theme,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub title_bar: ColorTuple,
    pub win_bar: ColorTuple,
    pub roster: ColorTuple,
    pub occupants: ColorTuple,
    #[serde(default = "default_selected_message")]
    #[serde(serialize_with = "crate::color::serialize_color")]
    #[serde(deserialize_with = "crate::color::deserialize_color")]
    pub selected_message: BgColor,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            title_bar: ColorTuple::new(
                Color::Named(terminus::NamedColor::Blue),
                Color::Named(terminus::NamedColor::Black),
            ),
            win_bar: ColorTuple::new(
                Color::Named(terminus::NamedColor::Blue),
                Color::Named(terminus::NamedColor::Black),
            ),
            roster: ColorTuple::new(
                Color::Named(terminus::NamedColor::Blue),
                Color::Named(terminus::NamedColor::Black),
            ),
            occupants: ColorTuple::new(
                Color::Named(terminus::NamedColor::Blue),
                Color::Named(terminus::NamedColor::Black),
            ),
            selected_message: BgColor(Color::Rgb(49, 50, 68)),
        }
    }
}
