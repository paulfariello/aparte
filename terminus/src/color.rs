/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use core::fmt;

use std::str::FromStr;

use crossterm::style::{Color as CColor, SetBackgroundColor, SetForegroundColor};
use serde::{de, ser, Deserialize, Deserializer, Serialize, Serializer};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum NamedColor {
    Black,
    Blue,
    Cyan,
    Green,
    LightBlack,
    LightBlue,
    LightCyan,
    LightGreen,
    LightMagenta,
    LightRed,
    LightWhite,
    LightYellow,
    Magenta,
    Red,
    White,
    Yellow,
}

impl NamedColor {
    fn to_crossterm(self) -> CColor {
        match self {
            NamedColor::Black => CColor::Black,
            NamedColor::Red => CColor::DarkRed,
            NamedColor::Green => CColor::DarkGreen,
            NamedColor::Yellow => CColor::DarkYellow,
            NamedColor::Blue => CColor::DarkBlue,
            NamedColor::Magenta => CColor::DarkMagenta,
            NamedColor::Cyan => CColor::DarkCyan,
            NamedColor::White => CColor::Grey,
            NamedColor::LightBlack => CColor::DarkGrey,
            NamedColor::LightRed => CColor::Red,
            NamedColor::LightGreen => CColor::Green,
            NamedColor::LightYellow => CColor::Yellow,
            NamedColor::LightBlue => CColor::Blue,
            NamedColor::LightMagenta => CColor::Magenta,
            NamedColor::LightCyan => CColor::Cyan,
            NamedColor::LightWhite => CColor::White,
        }
    }

    pub fn write_fg(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", SetForegroundColor(self.to_crossterm()))
    }

    pub fn write_bg(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", SetBackgroundColor(self.to_crossterm()))
    }
}

#[derive(Default, Copy, Clone, Debug, PartialEq)]
pub enum Color {
    Named(NamedColor),
    Rgb(u8, u8, u8),
    #[default]
    Default,
}

fn parse_rgb_str(s: &str) -> Result<Color, String> {
    enum State {
        Initial,
        NumberSign,
        Red(u8),
        Green(u8, Option<u8>),
        Blue(u8, u8, Option<u8>),
        Rgb(u8, u8, u8),
    }

    let mut state = State::Initial;

    for i in s.chars() {
        state =
            match (state, i) {
                (State::Initial, '#') => Ok(State::NumberSign),
                (State::NumberSign, digit @ ('0'..='9' | 'a'..='f')) => {
                    Ok(State::Red(digit.to_digit(16).unwrap() as u8 * 16))
                }
                (State::Red(nibble), digit @ ('0'..='9' | 'a'..='f')) => Ok(State::Green(
                    nibble + digit.to_digit(16).unwrap() as u8,
                    None,
                )),
                (State::Green(red, None), digit @ ('0'..='9' | 'a'..='f')) => Ok(State::Green(
                    red,
                    Some(digit.to_digit(16).unwrap() as u8 * 16),
                )),
                (State::Green(red, Some(nibble)), digit @ ('0'..='9' | 'a'..='f')) => Ok(
                    State::Blue(red, nibble + digit.to_digit(16).unwrap() as u8, None),
                ),
                (State::Blue(red, green, None), digit @ ('0'..='9' | 'a'..='f')) => Ok(
                    State::Blue(red, green, Some(digit.to_digit(16).unwrap() as u8 * 16)),
                ),
                (State::Blue(red, green, Some(nibble)), digit @ ('0'..='9' | 'a'..='f')) => Ok(
                    State::Rgb(red, green, nibble + digit.to_digit(16).unwrap() as u8),
                ),
                _ => Err(format!("Invalid rgb string {}", s)),
            }?;
    }

    match state {
        State::Rgb(r, g, b) => Ok(Color::Rgb(r, g, b)),
        _ => Err(format!("Invalid rgb string {}", s)),
    }
}

impl FromStr for Color {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" | "None" | "default" | "Default" => Ok(Color::Default),
            "Black" | "black" => Ok(Color::Named(NamedColor::Black)),
            "Blue" | "blue" => Ok(Color::Named(NamedColor::Blue)),
            "Cyan" | "cyan" => Ok(Color::Named(NamedColor::Cyan)),
            "Green" | "green" => Ok(Color::Named(NamedColor::Green)),
            "LightBlack" | "lightblack" => Ok(Color::Named(NamedColor::LightBlack)),
            "LightBlue" | "lightblue" => Ok(Color::Named(NamedColor::LightBlue)),
            "LightCyan" | "lightcyan" => Ok(Color::Named(NamedColor::LightCyan)),
            "LightGreen" | "lightgreen" => Ok(Color::Named(NamedColor::LightGreen)),
            "LightMagenta" | "lightmagenta" => Ok(Color::Named(NamedColor::LightMagenta)),
            "LightRed" | "lightred" => Ok(Color::Named(NamedColor::LightRed)),
            "LightWhite" | "lightwhite" => Ok(Color::Named(NamedColor::LightWhite)),
            "LightYellow" | "lightyellow" => Ok(Color::Named(NamedColor::LightYellow)),
            "Magenta" | "magenta" => Ok(Color::Named(NamedColor::Magenta)),
            "Red" | "red" => Ok(Color::Named(NamedColor::Red)),
            "White" | "white" => Ok(Color::Named(NamedColor::White)),
            "Yellow" | "yellow" => Ok(Color::Named(NamedColor::Yellow)),
            _ if s.starts_with('#') => parse_rgb_str(s),
            _ => Err(format!("Invalid color {}", s)),
        }
    }
}

#[allow(clippy::to_string_trait_impl)]
impl ToString for Color {
    fn to_string(&self) -> String {
        match self {
            Color::Default => "none".to_string(),
            Color::Named(NamedColor::Black) => "black".to_string(),
            Color::Named(NamedColor::Blue) => "blue".to_string(),
            Color::Named(NamedColor::Cyan) => "cyan".to_string(),
            Color::Named(NamedColor::Green) => "green".to_string(),
            Color::Named(NamedColor::LightBlack) => "lightblack".to_string(),
            Color::Named(NamedColor::LightBlue) => "lightblue".to_string(),
            Color::Named(NamedColor::LightCyan) => "lightcyan".to_string(),
            Color::Named(NamedColor::LightGreen) => "lightgreen".to_string(),
            Color::Named(NamedColor::LightMagenta) => "lightmagenta".to_string(),
            Color::Named(NamedColor::LightRed) => "lightred".to_string(),
            Color::Named(NamedColor::LightWhite) => "lightwhite".to_string(),
            Color::Named(NamedColor::LightYellow) => "lightyellow".to_string(),
            Color::Named(NamedColor::Magenta) => "magenta".to_string(),
            Color::Named(NamedColor::Red) => "red".to_string(),
            Color::Named(NamedColor::White) => "white".to_string(),
            Color::Named(NamedColor::Yellow) => "yellow".to_string(),
            Color::Rgb(r, g, b) => format!("#{:02x}{:02x}{:02x}", r, g, b),
        }
    }
}

/// ConfigColor is just a Fg or Bg color that isn't strongly typed in the config
pub trait ConfigColor
where
    Self: Sized,
{
    type Err;

    fn to_string(&self) -> String;
    fn from_str(string: &str) -> Result<Self, Self::Err>;
}

#[derive(Default, Debug, Copy, Clone, PartialEq)]
pub struct FgColor(pub Color);
#[derive(Default, Debug, Copy, Clone, PartialEq)]
pub struct BgColor(pub Color);

pub fn deserialize_color<'de, C, D>(deserializer: D) -> Result<C, D::Error>
where
    D: Deserializer<'de>,
    C: ConfigColor,
    <C as ConfigColor>::Err: std::fmt::Display,
{
    let s: &str = de::Deserialize::deserialize(deserializer)?;
    C::from_str(s).map_err(de::Error::custom)
}

pub fn serialize_color<C, S>(color: &C, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    C: ConfigColor,
{
    let color = color.to_string();
    ser::Serialize::serialize(&color, serializer)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorTuple {
    #[serde(serialize_with = "serialize_color")]
    #[serde(deserialize_with = "deserialize_color")]
    pub bg: BgColor,
    #[serde(serialize_with = "serialize_color")]
    #[serde(deserialize_with = "deserialize_color")]
    pub fg: FgColor,
}

impl ColorTuple {
    pub fn new(bg: Color, fg: Color) -> Self {
        Self {
            bg: BgColor(bg),
            fg: FgColor(fg),
        }
    }
}

impl ConfigColor for FgColor {
    type Err = String;

    fn to_string(&self) -> String {
        self.0.to_string()
    }

    fn from_str(string: &str) -> Result<Self, Self::Err> {
        Ok(Self(Color::from_str(string)?))
    }
}

impl ConfigColor for BgColor {
    type Err = String;

    fn to_string(&self) -> String {
        self.0.to_string()
    }

    fn from_str(string: &str) -> Result<Self, Self::Err> {
        Ok(Self(Color::from_str(string)?))
    }
}

impl fmt::Display for FgColor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.0 {
            Color::Named(color) => color.write_fg(f),
            Color::Rgb(r, g, b) => write!(f, "{}", SetForegroundColor(CColor::Rgb { r, g, b })),
            Color::Default => write!(f, "{}", SetForegroundColor(CColor::Reset)),
        }
    }
}

impl fmt::Display for BgColor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.0 {
            Color::Named(color) => color.write_bg(f),
            Color::Rgb(r, g, b) => write!(f, "{}", SetBackgroundColor(CColor::Rgb { r, g, b })),
            Color::Default => write!(f, "{}", SetBackgroundColor(CColor::Reset)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgb_from_str() {
        // Given
        let input = "#0122a3";

        // When
        let rgb = parse_rgb_str(input);

        // Then
        assert_eq!(rgb, Ok(Color::Rgb(0x01, 0x22, 0xa3)));
    }

    #[test]
    fn test_truncated_rgb_from_str() {
        // Given
        let input = "#1122";

        // When
        let rgb = parse_rgb_str(input);

        // Then
        assert_eq!(rgb, Err("Invalid rgb string #1122".to_string()));
    }

    #[test]
    fn test_invalid_rgb_from_str() {
        // Given
        let input = "#1122zz";

        // When
        let rgb = parse_rgb_str(input);

        // Then
        assert_eq!(rgb, Err("Invalid rgb string #1122zz".to_string()));
    }

    #[test]
    fn test_rgb_color_from_str() {
        // Given
        let input = "#0122a3";

        // When
        let rgb = Color::from_str(input);

        // Then
        assert_eq!(rgb, Ok(Color::Rgb(0x01, 0x22, 0xa3)));
    }

    #[test]
    fn test_named_color_from_str() {
        // Given
        let input = "Cyan";

        // When
        let rgb = Color::from_str(input);

        // Then
        assert_eq!(rgb, Ok(Color::Named(NamedColor::Cyan)));
    }

    #[test]
    fn test_invalid_color_from_str() {
        // Given
        let input = "teal";

        // When
        let rgb = Color::from_str(input);

        // Then
        assert_eq!(rgb, Err("Invalid color teal".to_string()));
    }

    #[test]
    fn test_none_color_from_str() {
        assert_eq!(Color::from_str("none"), Ok(Color::Default));
        assert_eq!(Color::from_str("None"), Ok(Color::Default));
        assert_eq!(Color::from_str("default"), Ok(Color::Default));
        assert_eq!(Color::from_str("Default"), Ok(Color::Default));
    }

    #[test]
    fn test_color_to_string_roundtrip() {
        let colors = [
            Color::Default,
            Color::Named(NamedColor::Cyan),
            Color::Named(NamedColor::LightBlue),
            Color::Rgb(0x01, 0x22, 0xa3),
        ];
        for color in colors {
            let s = color.to_string();
            assert_eq!(Color::from_str(&s), Ok(color));
        }
    }

    #[test]
    fn test_rgb_to_string() {
        assert_eq!(Color::Rgb(0x01, 0x22, 0xa3).to_string(), "#0122a3");
    }

    #[test]
    fn test_none_to_string() {
        assert_eq!(Color::Default.to_string(), "none");
    }
}
