/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use terminus::{
    deserialize_color, serialize_color, BgColor, Color, ColorTuple, FgColor, NamedColor,
    PopupColors,
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

fn default_date_separator_fg() -> FgColor {
    FgColor(Color::Named(NamedColor::Cyan))
}

fn default_date_separator_bg() -> BgColor {
    BgColor(Color::Default)
}

fn default_popup() -> ColorTuple {
    ColorTuple {
        bg: BgColor(Color::Named(NamedColor::Black)),
        fg: FgColor(Color::Named(NamedColor::White)),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub accounts: HashMap<String, ConnectionInfo>,
    #[serde(default = "true_")]
    pub bell: bool,
    pub theme_name: Option<String>,
    pub theme: Theme,
    pub preferred_langs: Vec<String>,
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

    pub fn preferred_langs_strs(&self) -> Vec<&str> {
        self.preferred_langs
            .iter()
            .map(std::string::String::as_str)
            .collect()
    }
}

pub fn builtin_theme(name: &str) -> Option<Theme> {
    match name {
        "profanity" => Some(profanity()),
        "catppuccin-mocha" => Some(catppuccin_mocha()),
        "catppuccin-macchiato" => Some(catppuccin_macchiato()),
        "catppuccin-frappe" => Some(catppuccin_frappe()),
        "catppuccin-latte" => Some(catppuccin_latte()),
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
    #[serde(default = "default_date_separator_fg")]
    #[serde(serialize_with = "serialize_color")]
    #[serde(deserialize_with = "deserialize_color")]
    pub date_separator_fg: FgColor,
    #[serde(default = "default_date_separator_bg")]
    #[serde(serialize_with = "serialize_color")]
    #[serde(deserialize_with = "deserialize_color")]
    pub date_separator_bg: BgColor,
    #[serde(default = "default_popup")]
    pub popup: ColorTuple,
}

impl PopupColors for Theme {
    fn popup_colors(&self) -> ColorTuple {
        self.popup.clone()
    }
}

impl Default for Theme {
    fn default() -> Self {
        profanity()
    }
}

fn profanity() -> Theme {
    Theme {
        title_bar: ColorTuple {
            bg: BgColor(Color::Named(NamedColor::Blue)),
            fg: FgColor(Color::Named(NamedColor::Black)),
        },
        win_bar: ColorTuple {
            bg: BgColor(Color::Named(NamedColor::Blue)),
            fg: FgColor(Color::Named(NamedColor::Black)),
        },
        title_bar_mode: ColorTuple {
            bg: BgColor(Color::Named(NamedColor::Blue)),
            fg: FgColor(Color::Named(NamedColor::Black)),
        },
        roster: ColorTuple {
            bg: BgColor(Color::Named(NamedColor::Blue)),
            fg: FgColor(Color::Named(NamedColor::Black)),
        },
        occupants: ColorTuple {
            bg: BgColor(Color::Named(NamedColor::Blue)),
            fg: FgColor(Color::Named(NamedColor::Black)),
        },
        selected_message: BgColor(Color::Rgb(49, 50, 68)),
        roster_available_fg: FgColor(Color::Named(NamedColor::Green)),
        roster_unavailable_fg: FgColor(Color::Default),
        roster_group_fg: FgColor(Color::Named(NamedColor::Yellow)),
        roster_role_fg: FgColor(Color::Named(NamedColor::Yellow)),
        search_highlight_fg: FgColor(Color::Named(NamedColor::Black)),
        search_highlight_bg: BgColor(Color::Named(NamedColor::Yellow)),
        date_separator_fg: FgColor(Color::Named(NamedColor::Cyan)),
        date_separator_bg: BgColor(Color::Default),
        popup: ColorTuple {
            bg: BgColor(Color::Named(NamedColor::Black)),
            fg: FgColor(Color::Named(NamedColor::White)),
        },
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
        win_bar: ColorTuple {
            bg: BgColor(surface_0),
            fg: FgColor(lavender),
        },
        title_bar: ColorTuple {
            bg: BgColor(surface_0),
            fg: FgColor(lavender),
        },
        title_bar_mode: ColorTuple {
            bg: BgColor(lavender),
            fg: FgColor(base),
        },
        roster: ColorTuple {
            bg: BgColor(mantle),
            fg: FgColor(text),
        },
        occupants: ColorTuple {
            bg: BgColor(mantle),
            fg: FgColor(text),
        },
        selected_message: BgColor(surface_1),
        roster_available_fg: FgColor(green),
        roster_unavailable_fg: FgColor(overlay_0),
        roster_group_fg: FgColor(mauve),
        roster_role_fg: FgColor(mauve),
        search_highlight_fg: FgColor(crust),
        search_highlight_bg: BgColor(yellow),
        date_separator_fg: FgColor(overlay_2),
        date_separator_bg: BgColor(Color::Default),
        popup: ColorTuple {
            fg: FgColor(lavender),
            ..ColorTuple::default()
        },
    }
}

fn catppuccin_latte() -> Theme {
    #[allow(unused_variables)]
    let rosewater = Color::Rgb(220, 138, 120);
    #[allow(unused_variables)]
    let flamingo = Color::Rgb(221, 120, 120);
    #[allow(unused_variables)]
    let pink = Color::Rgb(234, 118, 203);
    #[allow(unused_variables)]
    let mauve = Color::Rgb(136, 57, 239);
    #[allow(unused_variables)]
    let red = Color::Rgb(210, 15, 57);
    #[allow(unused_variables)]
    let maroon = Color::Rgb(230, 69, 83);
    #[allow(unused_variables)]
    let peach = Color::Rgb(254, 100, 11);
    #[allow(unused_variables)]
    let yellow = Color::Rgb(223, 142, 29);
    #[allow(unused_variables)]
    let green = Color::Rgb(64, 160, 43);
    #[allow(unused_variables)]
    let teal = Color::Rgb(23, 146, 153);
    #[allow(unused_variables)]
    let sky = Color::Rgb(4, 165, 229);
    #[allow(unused_variables)]
    let sapphire = Color::Rgb(32, 159, 181);
    #[allow(unused_variables)]
    let blue = Color::Rgb(30, 102, 245);
    #[allow(unused_variables)]
    let lavender = Color::Rgb(114, 135, 253);
    #[allow(unused_variables)]
    let text = Color::Rgb(76, 79, 105);
    #[allow(unused_variables)]
    let subtext_1 = Color::Rgb(92, 95, 119);
    #[allow(unused_variables)]
    let subtext_0 = Color::Rgb(108, 111, 133);
    #[allow(unused_variables)]
    let overlay_2 = Color::Rgb(124, 127, 147);
    #[allow(unused_variables)]
    let overlay_1 = Color::Rgb(140, 143, 161);
    #[allow(unused_variables)]
    let overlay_0 = Color::Rgb(156, 160, 176);
    #[allow(unused_variables)]
    let surface_2 = Color::Rgb(172, 176, 190);
    #[allow(unused_variables)]
    let surface_1 = Color::Rgb(188, 192, 204);
    #[allow(unused_variables)]
    let surface_0 = Color::Rgb(204, 208, 218);
    #[allow(unused_variables)]
    let base = Color::Rgb(239, 241, 245);
    #[allow(unused_variables)]
    let mantle = Color::Rgb(230, 233, 239);
    #[allow(unused_variables)]
    let crust = Color::Rgb(220, 224, 232);

    Theme {
        title_bar: ColorTuple {
            bg: BgColor(base),
            fg: FgColor(text),
        },
        win_bar: ColorTuple {
            bg: BgColor(surface_0),
            fg: FgColor(text),
        },
        title_bar_mode: ColorTuple {
            bg: BgColor(surface_0),
            fg: FgColor(text),
        },
        roster: ColorTuple {
            bg: BgColor(mantle),
            fg: FgColor(text),
        },
        occupants: ColorTuple {
            bg: BgColor(mantle),
            fg: FgColor(text),
        },
        selected_message: BgColor(surface_2),
        roster_available_fg: FgColor(green),
        roster_unavailable_fg: FgColor(overlay_0),
        roster_group_fg: FgColor(mauve),
        roster_role_fg: FgColor(mauve),
        search_highlight_fg: FgColor(text),
        search_highlight_bg: BgColor(yellow),
        date_separator_fg: FgColor(overlay_0),
        date_separator_bg: BgColor(Color::Default),
        popup: ColorTuple {
            bg: BgColor(surface_2),
            fg: FgColor(text),
        },
    }
}

fn catppuccin_frappe() -> Theme {
    #[allow(unused_variables)]
    let rosewater = Color::Rgb(242, 213, 207);
    #[allow(unused_variables)]
    let flamingo = Color::Rgb(238, 190, 190);
    #[allow(unused_variables)]
    let pink = Color::Rgb(244, 184, 228);
    #[allow(unused_variables)]
    let mauve = Color::Rgb(202, 158, 230);
    #[allow(unused_variables)]
    let red = Color::Rgb(231, 130, 132);
    #[allow(unused_variables)]
    let maroon = Color::Rgb(234, 153, 156);
    #[allow(unused_variables)]
    let peach = Color::Rgb(239, 159, 118);
    #[allow(unused_variables)]
    let yellow = Color::Rgb(229, 200, 144);
    #[allow(unused_variables)]
    let green = Color::Rgb(166, 209, 137);
    #[allow(unused_variables)]
    let teal = Color::Rgb(129, 200, 190);
    #[allow(unused_variables)]
    let sky = Color::Rgb(153, 209, 219);
    #[allow(unused_variables)]
    let sapphire = Color::Rgb(133, 193, 220);
    #[allow(unused_variables)]
    let blue = Color::Rgb(140, 170, 238);
    #[allow(unused_variables)]
    let lavender = Color::Rgb(186, 187, 241);
    #[allow(unused_variables)]
    let text = Color::Rgb(198, 208, 245);
    #[allow(unused_variables)]
    let subtext_1 = Color::Rgb(181, 191, 226);
    #[allow(unused_variables)]
    let subtext_0 = Color::Rgb(165, 173, 206);
    #[allow(unused_variables)]
    let overlay_2 = Color::Rgb(148, 156, 187);
    #[allow(unused_variables)]
    let overlay_1 = Color::Rgb(131, 139, 167);
    #[allow(unused_variables)]
    let overlay_0 = Color::Rgb(115, 121, 148);
    #[allow(unused_variables)]
    let surface_2 = Color::Rgb(98, 104, 128);
    #[allow(unused_variables)]
    let surface_1 = Color::Rgb(81, 87, 109);
    #[allow(unused_variables)]
    let surface_0 = Color::Rgb(65, 69, 89);
    #[allow(unused_variables)]
    let base = Color::Rgb(48, 52, 70);
    #[allow(unused_variables)]
    let mantle = Color::Rgb(41, 44, 60);
    #[allow(unused_variables)]
    let crust = Color::Rgb(35, 38, 52);

    Theme {
        title_bar: ColorTuple {
            bg: BgColor(base),
            fg: FgColor(text),
        },
        win_bar: ColorTuple {
            bg: BgColor(surface_0),
            fg: FgColor(text),
        },
        title_bar_mode: ColorTuple {
            bg: BgColor(surface_0),
            fg: FgColor(text),
        },
        roster: ColorTuple {
            bg: BgColor(mantle),
            fg: FgColor(text),
        },
        occupants: ColorTuple {
            bg: BgColor(mantle),
            fg: FgColor(text),
        },
        selected_message: BgColor(surface_1),
        roster_available_fg: FgColor(green),
        roster_unavailable_fg: FgColor(overlay_0),
        roster_group_fg: FgColor(mauve),
        roster_role_fg: FgColor(mauve),
        search_highlight_fg: FgColor(crust),
        search_highlight_bg: BgColor(yellow),
        date_separator_fg: FgColor(overlay_0),
        date_separator_bg: BgColor(surface_0),
        popup: ColorTuple {
            bg: BgColor(surface_1),
            fg: FgColor(text),
        },
    }
}

fn catppuccin_macchiato() -> Theme {
    #[allow(unused_variables)]
    let rosewater = Color::Rgb(244, 219, 214);
    #[allow(unused_variables)]
    let flamingo = Color::Rgb(240, 198, 198);
    #[allow(unused_variables)]
    let pink = Color::Rgb(245, 189, 230);
    #[allow(unused_variables)]
    let mauve = Color::Rgb(198, 160, 246);
    #[allow(unused_variables)]
    let red = Color::Rgb(237, 135, 150);
    #[allow(unused_variables)]
    let maroon = Color::Rgb(238, 153, 160);
    #[allow(unused_variables)]
    let peach = Color::Rgb(245, 169, 127);
    #[allow(unused_variables)]
    let yellow = Color::Rgb(238, 212, 159);
    #[allow(unused_variables)]
    let green = Color::Rgb(166, 218, 149);
    #[allow(unused_variables)]
    let teal = Color::Rgb(139, 213, 202);
    #[allow(unused_variables)]
    let sky = Color::Rgb(145, 215, 227);
    #[allow(unused_variables)]
    let sapphire = Color::Rgb(125, 196, 228);
    #[allow(unused_variables)]
    let blue = Color::Rgb(138, 173, 244);
    #[allow(unused_variables)]
    let lavender = Color::Rgb(183, 189, 248);
    #[allow(unused_variables)]
    let text = Color::Rgb(202, 211, 245);
    #[allow(unused_variables)]
    let subtext_1 = Color::Rgb(184, 192, 224);
    #[allow(unused_variables)]
    let subtext_0 = Color::Rgb(165, 173, 203);
    #[allow(unused_variables)]
    let overlay_2 = Color::Rgb(147, 154, 183);
    #[allow(unused_variables)]
    let overlay_1 = Color::Rgb(128, 135, 162);
    #[allow(unused_variables)]
    let overlay_0 = Color::Rgb(110, 115, 141);
    #[allow(unused_variables)]
    let surface_2 = Color::Rgb(91, 96, 120);
    #[allow(unused_variables)]
    let surface_1 = Color::Rgb(73, 77, 100);
    #[allow(unused_variables)]
    let surface_0 = Color::Rgb(54, 58, 79);
    #[allow(unused_variables)]
    let base = Color::Rgb(36, 39, 58);
    #[allow(unused_variables)]
    let mantle = Color::Rgb(30, 32, 48);
    #[allow(unused_variables)]
    let crust = Color::Rgb(24, 25, 38);

    Theme {
        win_bar: ColorTuple {
            bg: BgColor(surface_0),
            fg: FgColor(lavender),
        },
        title_bar: ColorTuple {
            bg: BgColor(surface_0),
            fg: FgColor(lavender),
        },
        title_bar_mode: ColorTuple {
            bg: BgColor(lavender),
            fg: FgColor(base),
        },
        roster: ColorTuple {
            bg: BgColor(mantle),
            fg: FgColor(text),
        },
        occupants: ColorTuple {
            bg: BgColor(mantle),
            fg: FgColor(text),
        },
        selected_message: BgColor(surface_1),
        roster_available_fg: FgColor(green),
        roster_unavailable_fg: FgColor(overlay_0),
        roster_group_fg: FgColor(mauve),
        roster_role_fg: FgColor(mauve),
        search_highlight_fg: FgColor(crust),
        search_highlight_bg: BgColor(yellow),
        date_separator_fg: FgColor(overlay_2),
        date_separator_bg: BgColor(Color::Default),
        popup: ColorTuple {
            fg: FgColor(lavender),
            ..ColorTuple::default()
        },
    }
}
