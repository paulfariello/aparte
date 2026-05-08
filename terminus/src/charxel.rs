use std::{collections::HashSet, slice::Split};

use itertools::Itertools;
use unicode_display_width;
use unicode_segmentation::UnicodeSegmentation as _;

#[cfg(feature = "image")]
use sixel_image::SixelImage;

use crate::{is_clean_str, BgColor, ColorTuple, FgColor, Style};

#[derive(Debug, Clone, PartialEq)]
pub struct Grapheme(String);

impl std::fmt::Display for Grapheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Default for Grapheme {
    fn default() -> Self {
        Grapheme(String::from(" "))
    }
}

impl From<&str> for Grapheme {
    fn from(value: &str) -> Self {
        Grapheme(String::from(value))
    }
}

impl Grapheme {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Charxel {
    pub grapheme: Grapheme,
    pub foreground: FgColor,
    pub background: BgColor,
    pub styles: HashSet<Style>,
}

impl Charxel {
    pub fn new(grapheme: Grapheme) -> Self {
        Self {
            grapheme,
            ..Default::default()
        }
    }

    pub fn set_color(&mut self, color: ColorTuple) {
        self.background = color.bg;
        self.foreground = color.fg;
    }

    pub fn set_background(&mut self, color: BgColor) {
        self.background = color;
    }

    pub fn set_foreground(&mut self, color: FgColor) {
        self.foreground = color;
    }

    pub fn set_styles(&mut self, styles: &[Style]) {
        self.styles = HashSet::from_iter(styles.iter().cloned());
    }

    pub fn add_style(&mut self, style: Style) {
        self.styles.insert(style);
    }

    pub fn set_grapheme(&mut self, grapheme: String) {
        self.grapheme = Grapheme(grapheme);
    }

    pub fn display_width(&self) -> u16 {
        unicode_display_width::width(&self.grapheme.0) as u16
    }
}

#[derive(Default, Debug, Clone)]
pub struct Charxels(Vec<Charxel>);

impl Charxels {
    pub fn lines(&self) -> Lines<'_, impl FnMut(&Charxel) -> bool> {
        Lines(self.0.split(|charxel: &Charxel| charxel.grapheme.0 == "\n"))
    }

    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        self.0
            .iter()
            .map(|charxel| charxel.grapheme.0.clone())
            .join("")
    }

    pub fn split_word_bounds(&self) -> WordBounds<'_, impl Iterator<Item = &'_ Charxel>> {
        let whole_string = self.to_string();

        WordBounds {
            charxels_iter: self.0.iter(),
            words_sizes: whole_string
                .split_word_bounds()
                .map(|word: &str| word.graphemes(true).count())
                .collect(),
        }
    }

    pub fn display_width(&self) -> u16 {
        self.0.iter().map(|c| c.display_width()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn push(&mut self, charxel: Charxel) {
        self.0.push(charxel)
    }

    pub fn append(&mut self, other: impl IntoCharxels) {
        self.0.append(&mut other.into_charxels().0)
    }

    pub fn truncate(&mut self, len: u16, end: impl IntoCharxels) {
        let end = end.into_charxels();
        assert!(len >= end.display_width());
        if self.display_width() > len {
            let target = len - end.display_width();
            let mut acc = 0u16;
            let mut elem_count = 0;
            for charxel in &self.0 {
                let w = charxel.display_width();
                if acc + w > target {
                    break;
                }
                acc += w;
                elem_count += 1;
            }
            self.0.truncate(elem_count);
            self.append(end);
            assert!(self.display_width() <= len);
        }
    }
}

impl FromIterator<Charxel> for Charxels {
    fn from_iter<T: IntoIterator<Item = Charxel>>(iter: T) -> Self {
        Self(iter.into_iter().collect::<_>())
    }
}

pub struct WordBounds<'a, C>
where
    C: Iterator<Item = &'a Charxel>,
{
    charxels_iter: C,
    words_sizes: Vec<usize>,
}

impl<'a, C> Iterator for WordBounds<'a, C>
where
    C: Iterator<Item = &'a Charxel>,
{
    type Item = Vec<&'a Charxel>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.words_sizes.is_empty() {
            return None;
        }

        let size = self.words_sizes.remove(0);
        Some(self.charxels_iter.by_ref().take(size).collect())
    }
}

pub struct Lines<'a, P>(Split<'a, Charxel, P>)
where
    P: FnMut(&Charxel) -> bool;

impl<P> Iterator for Lines<'_, P>
where
    P: FnMut(&Charxel) -> bool,
{
    type Item = Charxels;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|charxels| charxels.into_charxels())
    }
}

impl IntoIterator for Charxels {
    type Item = Charxel;

    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

pub trait CharxelDisplay<C = ()> {
    fn colored_fmt(&self, config: &C) -> Charxels;
}

pub trait IntoCharxels: Sized {
    fn into_charxels(self) -> Charxels;

    fn with_color(self, color: &ColorTuple) -> Charxels {
        let mut charxels = self.into_charxels();
        for charxel in charxels.0.iter_mut() {
            charxel.set_background(color.bg);
            charxel.set_foreground(color.fg);
        }
        charxels
    }

    fn with_background(self, background: BgColor) -> Charxels {
        let mut charxels = self.into_charxels();
        for charxel in charxels.0.iter_mut() {
            charxel.set_background(background);
        }
        charxels
    }

    fn with_foreground(self, foreground: FgColor) -> Charxels {
        let mut charxels = self.into_charxels();
        for charxel in charxels.0.iter_mut() {
            charxel.set_foreground(foreground);
        }
        charxels
    }

    fn with_styles(self, styles: &[Style]) -> Charxels {
        let mut charxels = self.into_charxels();
        for charxel in charxels.0.iter_mut() {
            charxel.set_styles(styles);
        }
        charxels
    }

    fn with_style(self, style: Style) -> Charxels {
        let mut charxels = self.into_charxels();
        for charxel in charxels.0.iter_mut() {
            charxel.add_style(style);
        }
        charxels
    }
}

impl IntoCharxels for String {
    fn into_charxels(self) -> Charxels {
        assert!(is_clean_str(&self));
        Charxels(
            self.graphemes(true)
                .map(|grapheme| Charxel::new(Grapheme::from(grapheme)))
                .collect(),
        )
    }
}

impl IntoCharxels for &String {
    fn into_charxels(self) -> Charxels {
        assert!(is_clean_str(self));
        Charxels(
            self.graphemes(true)
                .map(|grapheme| Charxel::new(Grapheme::from(grapheme)))
                .collect(),
        )
    }
}

impl IntoCharxels for &str {
    fn into_charxels(self) -> Charxels {
        assert!(is_clean_str(self));
        Charxels(
            self.graphemes(true)
                .map(|grapheme| Charxel::new(Grapheme::from(grapheme)))
                .collect(),
        )
    }
}

impl IntoCharxels for Charxels {
    fn into_charxels(self) -> Charxels {
        self
    }
}

impl IntoCharxels for &Charxels {
    fn into_charxels(self) -> Charxels {
        self.clone()
    }
}

impl IntoCharxels for anyhow::Error {
    fn into_charxels(self) -> Charxels {
        self.to_string().into_charxels()
    }
}

impl IntoCharxels for &[Charxel] {
    fn into_charxels(self) -> Charxels {
        Charxels(self.to_vec())
    }
}

#[cfg(feature = "image")]
impl IntoCharxels for &SixelImage {
    fn into_charxels(self) -> Charxels {
        self.serialize();
        todo!()
    }
}
