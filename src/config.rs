/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use terminus::{
    deserialize_color, serialize_color, BgColor, Color, ColorTuple, FgColor, NamedColor,
};

use crate::account::ConnectionInfo;

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

fn default_search_highlight_fg() -> FgColor {
    FgColor(Color::Named(NamedColor::Black))
}

fn default_search_highlight_bg() -> BgColor {
    BgColor(Color::Named(NamedColor::Yellow))
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
    pub title_bar_mode: ColorTuple,
    pub roster: ColorTuple,
    pub occupants: ColorTuple,
    #[serde(default = "default_selected_message")]
    #[serde(serialize_with = "serialize_color")]
    #[serde(deserialize_with = "deserialize_color")]
    pub selected_message: BgColor,
    #[serde(default = "default_roster_available_fg")]
    #[serde(serialize_with = "serialize_color")]
    #[serde(deserialize_with = "deserialize_color")]
    pub roster_available_fg: FgColor,
    #[serde(default = "default_roster_unavailable_fg")]
    #[serde(serialize_with = "serialize_color")]
    #[serde(deserialize_with = "deserialize_color")]
    pub roster_unavailable_fg: FgColor,
    #[serde(default = "default_roster_group_fg")]
    #[serde(serialize_with = "serialize_color")]
    #[serde(deserialize_with = "deserialize_color")]
    pub roster_group_fg: FgColor,
    #[serde(default = "default_roster_role_fg")]
    #[serde(serialize_with = "serialize_color")]
    #[serde(deserialize_with = "deserialize_color")]
    pub roster_role_fg: FgColor,
    #[serde(default = "default_search_highlight_fg")]
    #[serde(serialize_with = "serialize_color")]
    #[serde(deserialize_with = "deserialize_color")]
    pub search_highlight_fg: FgColor,
    #[serde(default = "default_search_highlight_bg")]
    #[serde(serialize_with = "serialize_color")]
    #[serde(deserialize_with = "deserialize_color")]
    pub search_highlight_bg: BgColor,
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
        title_bar_mode: ColorTuple::new(
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
        search_highlight_fg: FgColor(Color::Named(NamedColor::Black)),
        search_highlight_bg: BgColor(Color::Named(NamedColor::Yellow)),
    }
}

fn catppuccin_mocha() -> Theme {
    #[allow(unused_variables)]
    let rosewater = Color::Rgb(245, 224, 220);
    #[allow(unused_variables)]
    let flamingo = Color::Rgb(242, 205, 205);
    #[allow(unused_variables)]
    let pink = Color::Rgb(245, 194, 231);
    #[allow(unused_variables)]
    let mauve = Color::Rgb(203, 166, 247);
    #[allow(unused_variables)]
    let red = Color::Rgb(243, 139, 168);
    #[allow(unused_variables)]
    let maroon = Color::Rgb(235, 160, 172);
    #[allow(unused_variables)]
    let peach = Color::Rgb(250, 179, 135);
    #[allow(unused_variables)]
    let yellow = Color::Rgb(249, 226, 175);
    #[allow(unused_variables)]
    let green = Color::Rgb(166, 227, 161);
    #[allow(unused_variables)]
    let teal = Color::Rgb(148, 226, 213);
    #[allow(unused_variables)]
    let sky = Color::Rgb(137, 220, 235);
    #[allow(unused_variables)]
    let sapphire = Color::Rgb(116, 199, 236);
    #[allow(unused_variables)]
    let blue = Color::Rgb(137, 180, 250);
    #[allow(unused_variables)]
    let lavender = Color::Rgb(180, 190, 254);
    #[allow(unused_variables)]
    let text = Color::Rgb(205, 214, 244);
    #[allow(unused_variables)]
    let subtext_1 = Color::Rgb(186, 194, 222);
    #[allow(unused_variables)]
    let subtext_0 = Color::Rgb(166, 173, 200);
    #[allow(unused_variables)]
    let overlay_2 = Color::Rgb(147, 153, 178);
    #[allow(unused_variables)]
    let overlay_1 = Color::Rgb(127, 132, 156);
    #[allow(unused_variables)]
    let overlay_0 = Color::Rgb(108, 112, 134);
    #[allow(unused_variables)]
    let surface_2 = Color::Rgb(88, 91, 112);
    #[allow(unused_variables)]
    let surface_1 = Color::Rgb(69, 71, 90);
    #[allow(unused_variables)]
    let surface_0 = Color::Rgb(49, 50, 68);
    #[allow(unused_variables)]
    let base = Color::Rgb(30, 30, 46);
    #[allow(unused_variables)]
    let mantle = Color::Rgb(24, 24, 37);
    #[allow(unused_variables)]
    let crust = Color::Rgb(17, 17, 27);

    Theme {
        win_bar: ColorTuple::new(surface_0, lavender),
        title_bar: ColorTuple::new(surface_0, lavender),
        title_bar_mode: ColorTuple::new(lavender, base),
        roster: ColorTuple::new(mantle, text),
        occupants: ColorTuple::new(mantle, text),
        selected_message: BgColor(surface_1),
        roster_available_fg: FgColor(green),
        roster_unavailable_fg: FgColor(overlay_0),
        roster_group_fg: FgColor(mauve),
        roster_role_fg: FgColor(mauve),
        search_highlight_fg: FgColor(crust),
        search_highlight_bg: BgColor(yellow),
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
        title_bar_mode: ColorTuple::new(
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
        search_highlight_fg: FgColor(Color::Rgb(76, 79, 105)), // Text dark
        search_highlight_bg: BgColor(Color::Rgb(223, 142, 29)), // Yellow
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
        title_bar_mode: ColorTuple::new(
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
        search_highlight_fg: FgColor(Color::Rgb(17, 17, 27)), // Crust
        search_highlight_bg: BgColor(Color::Rgb(229, 200, 144)), // Yellow
    }
}
