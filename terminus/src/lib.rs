use core::fmt;
use std::cell::RefCell;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::rc::Rc;
use std::str::FromStr;
use std::{cmp, iter::Sum};

#[cfg(test)]
use mockall::automock;
use rendering::ScreenFrame;
use termion::color::Color as _;
use termion::raw::RawTerminal;
use termion::screen::AlternateScreen;
use unicode_segmentation::UnicodeSegmentation;

pub mod charxel;
pub mod cursor;
pub mod frame_layout;
pub mod input;
pub mod linear_layout;
pub mod list_view;
pub mod rendering;
pub mod scroll_win;

pub type Screen<W> = AlternateScreen<RawTerminal<W>>;

pub type EventHandler<V, E> = Rc<RefCell<Box<dyn FnMut(&mut V, &mut E)>>>;

pub fn term_string_visible_len(string: &str) -> usize {
    // Count each grapheme on a given struct but ignore invisible chars sequences like '\x1b[…'
    let mut len = 0;
    let mut iter = string.graphemes(true);

    while let Some(grapheme) = iter.next() {
        match grapheme {
            "\x1b" => {
                if let Some(grapheme) = iter.next() {
                    if grapheme == "[" {
                        for grapheme in iter.by_ref() {
                            let chars = grapheme.chars().collect::<Vec<_>>();
                            if chars.len() == 1 {
                                match chars[0] {
                                    '\x30'..='\x3f' => {}     // parameter bytes
                                    '\x20'..='\x2f' => {}     // intermediate bytes
                                    '\x40'..='\x7e' => break, // final byte
                                    _ => break,
                                }
                            } else {
                                len += 1;
                                break;
                            }
                        }
                    }
                }
            }
            _ => {
                len += 1;
            }
        }
    }

    len
}

fn next_word<T: Iterator<Item = char>>(iter: T) -> usize {
    // XXX utf char boundary?
    enum WordParserState {
        Init,
        Space,
        Separator,
        Word,
    }

    use WordParserState::*;

    let mut state = Init;
    let mut count = 0;

    for c in iter {
        state = match state {
            Init => match c {
                ' ' => Space,
                '/' | '\\' | '\'' | '"' | '&' | '(' | ')' | '*' | ',' | ';' | '<' | '=' | '>'
                | '?' | '@' | '[' | ']' | '^' | '{' | '|' | '}' => Separator,
                _ => Word,
            },
            Space => match c {
                ' ' => Space,
                '/' | '\\' | '\'' | '"' | '&' | '(' | ')' | '*' | ',' | ';' | '<' | '=' | '>'
                | '?' | '@' | '[' | ']' | '^' | '{' | '|' | '}' => Separator,
                _ => Word,
            },
            Separator => match c {
                '/' | '\\' | '\'' | '"' | '&' | '(' | ')' | '*' | ',' | ';' | '<' | '=' | '>'
                | '?' | '@' | '[' | ']' | '^' | '{' | '|' | '}' => Separator,
                _ => break,
            },
            Word => match c {
                ' ' | '/' | '\\' | '\'' | '"' | '&' | '(' | ')' | '*' | ',' | ';' | '<' | '='
                | '>' | '?' | '@' | '[' | ']' | '^' | '{' | '|' | '}' => break,
                _ => Word,
            },
        };

        count += 1;
    }

    count
}

pub fn is_clean_str(string: &str) -> bool {
    !string.chars().any(|c| c == '\x1b')
}

/// Remove all terminal specific chars sequences
pub fn clean_str(string: &str) -> String {
    let mut output = String::new();
    let mut iter = string.chars();

    while let Some(c) = iter.next() {
        match c {
            '\x1b' => {
                if let Some(c) = iter.next() {
                    match c {
                        '[' => {
                            for c in iter.by_ref() {
                                match c {
                                    '\x30'..='\x3f' => {}     // parameter bytes
                                    '\x20'..='\x2f' => {}     // intermediate bytes
                                    '\x40'..='\x7e' => break, // final byte
                                    _ => output.push(c),
                                }
                            }
                        }
                        _ => output.push(c),
                    }
                }
            }
            _ => output.push(c),
        }
    }

    output
}

/// Truncate the string to max visible chars. Optionnaly appending the (already clean) 'append' string.
pub fn term_string_visible_truncate(string: &str, max: usize, append: Option<&str>) -> String {
    let mut iter = string.graphemes(true);
    let mut remaining = max;
    if let Some(append) = append {
        let count = append.graphemes(true).count();
        remaining -= count;
    }
    let mut output = String::new();

    while let Some(grapheme) = iter.next() {
        output.push_str(grapheme);
        match grapheme {
            "\x1b" => {
                if let Some(grapheme) = iter.next() {
                    output.push_str(grapheme);
                    if grapheme == "[" {
                        for grapheme in iter.by_ref() {
                            output.push_str(grapheme);
                            let chars = grapheme.chars().collect::<Vec<_>>();
                            if chars.len() == 1 {
                                match chars[0] {
                                    '\x30'..='\x3f' => {}     // parameter bytes
                                    '\x20'..='\x2f' => {}     // intermediate bytes
                                    '\x40'..='\x7e' => break, // final byte
                                    _ => break,
                                }
                            } else {
                                remaining -= 1;
                                break;
                            }
                        }
                    }
                }
            }
            _ => {
                remaining -= 1;
            }
        }

        if remaining == 0 {
            if let Some(append) = append {
                output.push_str(append);
            }
            break;
        }
    }

    output
}

#[derive(Default, Copy, Clone, Debug, Eq, PartialEq)]
pub struct CursorPos {
    top: u16,
    left: u16,
}

impl From<(u16, u16)> for CursorPos {
    fn from((x, y): (u16, u16)) -> Self {
        CursorPos { top: y, left: x }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum LayoutParam {
    MatchParent,
    WrapContent,
    Absolute(u16),
}

#[derive(Debug, Clone)]
pub struct LayoutParams {
    pub width: LayoutParam,
    pub height: LayoutParam,
}

#[derive(Debug, Clone, Default, PartialEq, Copy)]
pub enum MeasureSpec {
    #[default]
    Unspecified,
    AtMost(u16),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeasureSpecs {
    pub width: MeasureSpec,
    pub height: MeasureSpec,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RequestedDimension {
    ExpandMax,
    Absolute(u16),
}

impl Sum for RequestedDimension {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(
            RequestedDimension::Absolute(0),
            |sum, requested_dimension| match (sum, requested_dimension) {
                (RequestedDimension::Absolute(a), RequestedDimension::Absolute(b)) => {
                    RequestedDimension::Absolute(a + b)
                }
                _ => RequestedDimension::ExpandMax,
            },
        )
    }
}

impl Eq for RequestedDimension {}

impl Ord for RequestedDimension {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        match (self, other) {
            (RequestedDimension::ExpandMax, RequestedDimension::ExpandMax) => cmp::Ordering::Equal,
            (RequestedDimension::ExpandMax, RequestedDimension::Absolute(_)) => {
                cmp::Ordering::Greater
            }
            (RequestedDimension::Absolute(_), RequestedDimension::ExpandMax) => cmp::Ordering::Less,
            (RequestedDimension::Absolute(a), RequestedDimension::Absolute(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for RequestedDimension {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq<u16> for RequestedDimension {
    fn eq(&self, other: &u16) -> bool {
        match self {
            RequestedDimension::ExpandMax => false,
            RequestedDimension::Absolute(dimension) => dimension.eq(other),
        }
    }
}

impl PartialOrd<u16> for RequestedDimension {
    fn partial_cmp(&self, other: &u16) -> Option<cmp::Ordering> {
        match self {
            RequestedDimension::ExpandMax => Some(cmp::Ordering::Greater),
            RequestedDimension::Absolute(dimension) => dimension.partial_cmp(other),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RequestedDimensions {
    pub height: RequestedDimension,
    pub width: RequestedDimension,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dimensions {
    pub top: u16,
    pub left: u16,
    pub height: u16,
    pub width: u16,
}

impl Dimensions {
    fn reconcile_dimension(
        measure_spec: &MeasureSpec,
        requested_dimension: &RequestedDimension,
    ) -> u16 {
        match (measure_spec, requested_dimension) {
            (MeasureSpec::Unspecified, RequestedDimension::ExpandMax) => panic!(
                "Cannot resolve unspecified measure spec with expand max requested dimension"
            ),
            (MeasureSpec::Unspecified, RequestedDimension::Absolute(dimension)) => *dimension,
            (MeasureSpec::AtMost(dimension), RequestedDimension::ExpandMax) => *dimension,
            (MeasureSpec::AtMost(at_most), RequestedDimension::Absolute(requested)) => {
                cmp::min(*at_most, *requested)
            }
        }
    }
    pub fn reconcile(
        measure_specs: &MeasureSpecs,
        requested_dimensions: &RequestedDimensions,
        top: u16,
        left: u16,
    ) -> Self {
        Self {
            top,
            left,
            width: Self::reconcile_dimension(&measure_specs.width, &requested_dimensions.width),
            height: Self::reconcile_dimension(&measure_specs.height, &requested_dimensions.height),
        }
    }
}

impl From<&Dimensions> for MeasureSpecs {
    fn from(dimensions: &Dimensions) -> Self {
        MeasureSpecs {
            width: MeasureSpec::AtMost(dimensions.width),
            height: MeasureSpec::AtMost(dimensions.height),
        }
    }
}

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
    pub fn write_fg(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            NamedColor::Black => termion::color::Black.write_fg(f),
            NamedColor::Blue => termion::color::Blue.write_fg(f),
            NamedColor::Cyan => termion::color::Cyan.write_fg(f),
            NamedColor::Green => termion::color::Green.write_fg(f),
            NamedColor::LightBlack => termion::color::LightBlack.write_fg(f),
            NamedColor::LightBlue => termion::color::LightBlue.write_fg(f),
            NamedColor::LightCyan => termion::color::LightCyan.write_fg(f),
            NamedColor::LightGreen => termion::color::LightGreen.write_fg(f),
            NamedColor::LightMagenta => termion::color::LightMagenta.write_fg(f),
            NamedColor::LightRed => termion::color::LightRed.write_fg(f),
            NamedColor::LightWhite => termion::color::LightWhite.write_fg(f),
            NamedColor::LightYellow => termion::color::LightYellow.write_fg(f),
            NamedColor::Magenta => termion::color::Magenta.write_fg(f),
            NamedColor::Red => termion::color::Red.write_fg(f),
            NamedColor::White => termion::color::White.write_fg(f),
            NamedColor::Yellow => termion::color::Yellow.write_fg(f),
        }
    }

    pub fn write_bg(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            NamedColor::Black => termion::color::Black.write_bg(f),
            NamedColor::Blue => termion::color::Blue.write_bg(f),
            NamedColor::Cyan => termion::color::Cyan.write_bg(f),
            NamedColor::Green => termion::color::Green.write_bg(f),
            NamedColor::LightBlack => termion::color::LightBlack.write_bg(f),
            NamedColor::LightBlue => termion::color::LightBlue.write_bg(f),
            NamedColor::LightCyan => termion::color::LightCyan.write_bg(f),
            NamedColor::LightGreen => termion::color::LightGreen.write_bg(f),
            NamedColor::LightMagenta => termion::color::LightMagenta.write_bg(f),
            NamedColor::LightRed => termion::color::LightRed.write_bg(f),
            NamedColor::LightWhite => termion::color::LightWhite.write_bg(f),
            NamedColor::LightYellow => termion::color::LightYellow.write_bg(f),
            NamedColor::Magenta => termion::color::Magenta.write_bg(f),
            NamedColor::Red => termion::color::Red.write_bg(f),
            NamedColor::White => termion::color::White.write_bg(f),
            NamedColor::Yellow => termion::color::Yellow.write_bg(f),
        }
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
        todo!()
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
            Color::Rgb(r, g, b) => termion::color::Rgb(r, g, b).write_fg(f),
            Color::Default => termion::color::Reset.write_fg(f),
        }
    }
}

impl fmt::Display for BgColor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.0 {
            Color::Named(color) => color.write_bg(f),
            Color::Rgb(r, g, b) => termion::color::Rgb(r, g, b).write_bg(f),
            Color::Default => termion::color::Reset.write_bg(f),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Style {
    Bold,
    Faint,
    Italic,
    Underline,
    Blink,
    Invert,
    CrossedOut,
    Framed,
}

impl Debug for Style {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Style::Bold => f.write_str("Bold"),
            Style::Faint => f.write_str("Faint"),
            Style::Italic => f.write_str("Italic"),
            Style::Underline => f.write_str("Underline"),
            Style::Blink => f.write_str("Blink"),
            Style::Invert => f.write_str("Invert"),
            Style::CrossedOut => f.write_str("CrossedOut"),
            Style::Framed => f.write_str("Framed"),
        }
    }
}

impl Eq for Style {}

impl PartialEq for Style {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self).eq(&std::mem::discriminant(other))
    }
}

impl Hash for Style {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state)
    }
}

/// Represent any component that can be displayed.
///
/// Rendering is done in 3 steps:
///
///  - measure
///  - layout
///  - render
///
/// Measure compute the wanted dimension.
/// Layout apply the definitive dimension, setting the final x and y position.
/// Render draw the component on the given screen.
#[cfg_attr(test, automock)]
pub trait View<E> {
    /// Compute the wanted dimension given passed width and height
    fn measure(&self, measure_specs: &MeasureSpecs) -> RequestedDimensions;

    /// Apply the definitive dimension given the top and left position
    ///
    /// dimension are the definitive dimension.
    ///
    /// View has responsibility of storing dimension for later rendering if needed.
    fn layout(&mut self, dimensions: &Dimensions);

    /// Render the view with the given dimensions inside the given screen
    #[allow(clippy::needless_lifetimes)]
    fn render<'a>(&self, frame: ScreenFrame<'a>);

    /// Handle an event
    fn event(&mut self, event: &mut E);
}

impl<E> dyn View<E> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_term_string_visible_len_is_correct() {
        assert_eq!(
            term_string_visible_len(&format!(
                "{}ab{}",
                termion::color::Bg(termion::color::Red),
                termion::cursor::Goto(1, 123)
            )),
            2
        );
        assert_eq!(
            term_string_visible_len(&format!(
                "{}ab{}",
                termion::cursor::Goto(1, 123),
                termion::color::Bg(termion::color::Red)
            )),
            2
        );
        assert_eq!(
            term_string_visible_len(&format!(
                "{}🍻{}",
                termion::cursor::Goto(1, 123),
                termion::color::Bg(termion::color::Red)
            )),
            1
        );
        assert_eq!(
            term_string_visible_len(&format!(
                "{}12:34:56 - {}me:{}",
                termion::color::Fg(termion::color::White),
                termion::color::Fg(termion::color::Yellow),
                termion::color::Fg(termion::color::White)
            )),
            14
        )
    }

    #[test]
    fn test_term_string_clean() {
        // Given
        let input = "test \x1b[5mBlink";

        // When
        let cleaned = clean_str(input);

        // Then
        assert_eq!(cleaned, "test Blink");
    }

    #[test]
    fn test_term_string_visible_truncate() {
        // Given
        let input = "test \x1b[5mBlink";

        // When
        let truncated = term_string_visible_truncate(input, 6, None);

        // Then
        assert_eq!(truncated, "test \x1b[5mB");
    }

    #[test]
    fn test_term_string_visible_truncate_and_append() {
        // Given
        let input = "test \x1b[5mBlink";

        // When
        let truncated = term_string_visible_truncate(input, 6, Some("…"));

        // Then
        assert_eq!(truncated, "test …");
    }

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
}
