use crypto::digest::Digest;
use crypto::sha1::Sha1;
use hsluv::hsluv_to_rgb;
use std::convert::TryInto;

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
