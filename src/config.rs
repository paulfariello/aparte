/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use terminus::{BgColor, Color, FgColor, NamedColor};

use crate::account::ConnectionInfo;
use crate::color::ColorTuple;

fn true_() -> bool {
    true
}

fn default_selected_message() -> BgColor {
    BgColor(Color::Rgb(49, 50, 68))
}

fn default_roster_available_fg() -> FgColor {
    FgColor(Color::Named(NamedColor::Green))
}

fn default_roster_unavailable_fg() -> FgColor {
    FgColor(Color::Default)
}

fn default_roster_group_fg() -> FgColor {
    FgColor(Color::Named(NamedColor::Yellow))
}

fn default_roster_role_fg() -> FgColor {
    FgColor(Color::Named(NamedColor::Yellow))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub accounts: HashMap<String, ConnectionInfo>,
    #[serde(default = "true_")]
    pub bell: bool,
    pub theme_name: Option<String>,
    pub theme: Theme,
}

impl Config {
    pub fn get_theme(&self) -> Theme {
        if let Some(name) = &self.theme_name {
            if let Some(theme) = builtin_theme(name) {
                return theme;
            }
        }
        self.theme.clone()
    }
}

pub fn builtin_theme(name: &str) -> Option<Theme> {
    match name {
        "profanity" => Some(profanity()),
        "catppuccin-mocha" => Some(catppuccin_mocha()),
        "catppuccin-latte" => Some(catppuccin_latte()),
        "catppuccin-frappe" => Some(catppuccin_frappe()),
        _ => None,
    }
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
    #[serde(default = "default_roster_available_fg")]
    #[serde(serialize_with = "crate::color::serialize_color")]
    #[serde(deserialize_with = "crate::color::deserialize_color")]
    pub roster_available_fg: FgColor,
    #[serde(default = "default_roster_unavailable_fg")]
    #[serde(serialize_with = "crate::color::serialize_color")]
    #[serde(deserialize_with = "crate::color::deserialize_color")]
    pub roster_unavailable_fg: FgColor,
    #[serde(default = "default_roster_group_fg")]
    #[serde(serialize_with = "crate::color::serialize_color")]
    #[serde(deserialize_with = "crate::color::deserialize_color")]
    pub roster_group_fg: FgColor,
    #[serde(default = "default_roster_role_fg")]
    #[serde(serialize_with = "crate::color::serialize_color")]
    #[serde(deserialize_with = "crate::color::deserialize_color")]
    pub roster_role_fg: FgColor,
}

impl Default for Theme {
    fn default() -> Self {
        profanity()
    }
}

fn profanity() -> Theme {
    Theme {
        title_bar: ColorTuple::new(
            Color::Named(NamedColor::Blue),
            Color::Named(NamedColor::Black),
        ),
        win_bar: ColorTuple::new(
            Color::Named(NamedColor::Blue),
            Color::Named(NamedColor::Black),
        ),
        roster: ColorTuple::new(
            Color::Named(NamedColor::Blue),
            Color::Named(NamedColor::Black),
        ),
        occupants: ColorTuple::new(
            Color::Named(NamedColor::Blue),
            Color::Named(NamedColor::Black),
        ),
        selected_message: BgColor(Color::Rgb(49, 50, 68)),
        roster_available_fg: FgColor(Color::Named(NamedColor::Green)),
        roster_unavailable_fg: FgColor(Color::Default),
        roster_group_fg: FgColor(Color::Named(NamedColor::Yellow)),
        roster_role_fg: FgColor(Color::Named(NamedColor::Yellow)),
    }
}

fn catppuccin_mocha() -> Theme {
    Theme {
        title_bar: ColorTuple::new(
            Color::Rgb(30, 30, 46),    // Base
            Color::Rgb(205, 214, 244), // Text
        ),
        win_bar: ColorTuple::new(
            Color::Rgb(49, 50, 68),    // Surface0
            Color::Rgb(205, 214, 244), // Text
        ),
        roster: ColorTuple::new(
            Color::Rgb(24, 24, 37),    // Mantle
            Color::Rgb(205, 214, 244), // Text
        ),
        occupants: ColorTuple::new(
            Color::Rgb(24, 24, 37),    // Mantle
            Color::Rgb(205, 214, 244), // Text
        ),
        selected_message: BgColor(Color::Rgb(69, 71, 90)), // Surface1
        roster_available_fg: FgColor(Color::Rgb(166, 227, 161)), // Green
        roster_unavailable_fg: FgColor(Color::Rgb(108, 112, 134)), // Overlay0
        roster_group_fg: FgColor(Color::Rgb(203, 166, 247)), // Mauve
        roster_role_fg: FgColor(Color::Rgb(203, 166, 247)), // Mauve
    }
}

fn catppuccin_latte() -> Theme {
    Theme {
        title_bar: ColorTuple::new(
            Color::Rgb(239, 241, 245), // Base
            Color::Rgb(76, 79, 105),   // Text
        ),
        win_bar: ColorTuple::new(
            Color::Rgb(204, 208, 218), // Surface0
            Color::Rgb(76, 79, 105),   // Text
        ),
        roster: ColorTuple::new(
            Color::Rgb(230, 233, 239), // Mantle
            Color::Rgb(76, 79, 105),   // Text
        ),
        occupants: ColorTuple::new(
            Color::Rgb(230, 233, 239), // Mantle
            Color::Rgb(76, 79, 105),   // Text
        ),
        selected_message: BgColor(Color::Rgb(172, 176, 190)), // Surface2
        roster_available_fg: FgColor(Color::Rgb(64, 160, 43)), // Green
        roster_unavailable_fg: FgColor(Color::Rgb(156, 160, 176)), // Overlay0
        roster_group_fg: FgColor(Color::Rgb(136, 57, 239)),   // Mauve
        roster_role_fg: FgColor(Color::Rgb(136, 57, 239)),    // Mauve
    }
}

fn catppuccin_frappe() -> Theme {
    Theme {
        title_bar: ColorTuple::new(
            Color::Rgb(48, 52, 70),    // Base
            Color::Rgb(198, 208, 245), // Text
        ),
        win_bar: ColorTuple::new(
            Color::Rgb(65, 69, 89),    // Surface0
            Color::Rgb(198, 208, 245), // Text
        ),
        roster: ColorTuple::new(
            Color::Rgb(41, 44, 60),    // Mantle
            Color::Rgb(198, 208, 245), // Text
        ),
        occupants: ColorTuple::new(
            Color::Rgb(41, 44, 60),    // Mantle
            Color::Rgb(198, 208, 245), // Text
        ),
        selected_message: BgColor(Color::Rgb(81, 87, 109)), // Surface1
        roster_available_fg: FgColor(Color::Rgb(166, 209, 137)), // Green
        roster_unavailable_fg: FgColor(Color::Rgb(115, 121, 148)), // Overlay0
        roster_group_fg: FgColor(Color::Rgb(202, 158, 230)), // Mauve
        roster_role_fg: FgColor(Color::Rgb(202, 158, 230)), // Mauve
    }
}
