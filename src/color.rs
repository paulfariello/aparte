/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use hsluv::hsluv_to_rgb;
use serde::{de, ser, Deserialize, Deserializer, Serialize, Serializer};
use sha1::{Digest, Sha1};
use std::convert::TryInto;
use terminus::{
    charxel::{Charxel, Charxels},
    BgColor, Color, ConfigColor, FgColor,
};
use unicode_segmentation::UnicodeSegmentation;

fn deserialize_color<'de, C, D>(deserializer: D) -> Result<C, D::Error>
where
    D: Deserializer<'de>,
    C: ConfigColor,
    <C as ConfigColor>::Err: std::fmt::Display,
{
    let s: &str = de::Deserialize::deserialize(deserializer)?;
    C::from_str(s).map_err(de::Error::custom)
}

fn serialize_color<C, S>(color: &C, serializer: S) -> Result<S::Ok, S::Error>
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

pub fn id_to_rgb(identifier: &str) -> (u8, u8, u8) {
    // Follow xep 0392 for color generation
    let mut hasher = Sha1::new();
    hasher.update(identifier);
    let hash = hasher.finalize();

    let a = u16::from_le_bytes(hash[..2].try_into().unwrap());
    let hue_angle = f64::from(a) / 65536f64 * 360f64;
    let hue = (hue_angle, 100.0, 75.0);
    let (r, g, b) = hsluv_to_rgb(hue);
    let (r, g, b) = (r * 255.0, g * 255.0, b * 255.0);
    (r as u8, g as u8, b as u8)
}

struct Rainbow {
    line: f64,
    shift: f64,
    spread: f64,
    frequency: f64,
}

impl Rainbow {
    pub fn new(origin: f64) -> Self {
        Self {
            line: origin,
            shift: origin,
            spread: 3f64,
            frequency: 0.1f64,
        }
    }

    pub fn get_color(&mut self) -> (u8, u8, u8) {
        let i = self.frequency * self.shift / self.spread;
        let red = i.sin() * 127.00 + 128.00;
        let green = (i + (std::f64::consts::PI * 2.00 / 3.00)).sin() * 127.00 + 128.00;
        let blue = (i + (std::f64::consts::PI * 4.00 / 3.00)).sin() * 127.00 + 128.00;

        self.shift += 1.0;

        (red as u8, green as u8, blue as u8)
    }

    pub fn new_line(&mut self) {
        self.line += 1f64;
        self.shift = self.line;
    }
}

pub fn rainbow(input: &str) -> Charxels {
    let mut output = Charxels::default();
    let mut rainbow = Rainbow::new(rand::random::<f64>() * 10e9);

    for c in input.graphemes(true) {
        match c {
            "\n" => {
                rainbow.new_line();
                output.push(Charxel::new(c.into()));
            }
            c => {
                let (r, g, b) = rainbow.get_color();
                let mut charxel = Charxel::new(c.into());
                charxel.set_foreground(FgColor(Color::Rgb(r, g, b)));
                output.push(charxel);
            }
        }
    }

    output
}
