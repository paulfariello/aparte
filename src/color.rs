use crypto::digest::Digest;
use crypto::sha1::Sha1;
use hsluv::hsluv_to_rgb;
use serde::Deserialize;
use std::convert::TryInto;
use termion::color;

#[derive(Debug, Clone, Deserialize)]
pub struct ColorTuple {
    pub bg: String,
    pub fg: String,
}

impl ColorTuple {
    pub fn new<B: color::Color, F: color::Color>(bg: B, fg: F) -> Self {
        Self {
            bg: color::Bg(bg).to_string(),
            fg: color::Fg(fg).to_string(),
        }
    }
}

pub fn id_to_rgb(identifier: &str) -> (u8, u8, u8) {
    // Follow xep 0392 for color generation
    let mut hasher = Sha1::new();
    hasher.input_str(identifier);
    let mut hash = [0; 20];
    hasher.result(&mut hash);

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

pub fn rainbow(input: &str) -> String {
    let mut output = String::new();
    let mut rainbow = Rainbow::new(rand::random::<f64>() * 10e9);

    for c in input.chars() {
        match c {
            '\n' => rainbow.new_line(),
            _ => {
                let (r, g, b) = rainbow.get_color();
                output.push_str(&format!("{}", color::Fg(color::Rgb(r, g, b))));
            }
        }
        output.push(c);
    }

    output
}
