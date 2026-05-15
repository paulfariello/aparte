use core::fmt;
use std::cell::RefCell;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};
/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::rc::Rc;
use std::{cmp, iter::Sum};

use crossterm::style::{Attribute, SetAttribute};
#[cfg(test)]
use mockall::automock;
use rendering::ScreenFrame;
use unicode_segmentation::UnicodeSegmentation;

pub mod charxel;
pub mod color;
pub mod cursor;
pub mod frame_layout;
pub mod input;
pub mod label;
pub mod linear_layout;
pub mod list_view;
pub mod popup;
pub mod rendering;
pub mod scroll_win;
pub mod stories;

pub type EventHandler<V, E> = Rc<RefCell<Box<dyn FnMut(&mut V, &mut E)>>>;

pub use charxel::ContinuationCell;
pub use color::{
    deserialize_color, serialize_color, BgColor, Color, ColorTuple, FgColor, NamedColor,
};
pub use popup::PopupColors;
pub use scroll_win::Searchable;

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

#[derive(Default, Debug, Copy, Clone, Eq, PartialEq)]
pub enum CursorStyle {
    #[default]
    SteadyBar,
    SteadyBlock,
}

impl From<CursorStyle> for crossterm::cursor::SetCursorStyle {
    fn from(style: CursorStyle) -> Self {
        match style {
            CursorStyle::SteadyBar => crossterm::cursor::SetCursorStyle::SteadyBar,
            CursorStyle::SteadyBlock => crossterm::cursor::SetCursorStyle::SteadyBlock,
        }
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

impl fmt::Display for Style {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let attr = match self {
            Style::Bold => Attribute::Bold,
            Style::Faint => Attribute::Dim,
            Style::Italic => Attribute::Italic,
            Style::Underline => Attribute::Underlined,
            Style::Blink => Attribute::SlowBlink,
            Style::Invert => Attribute::Reverse,
            Style::CrossedOut => Attribute::CrossedOut,
            Style::Framed => Attribute::Framed,
        };
        write!(f, "{}", SetAttribute(attr))
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
pub trait View<E, C = ()> {
    /// Compute the wanted dimension given passed width and height
    fn measure(&self, measure_specs: &MeasureSpecs) -> RequestedDimensions;

    /// Apply the definitive dimension given the top and left position
    ///
    /// dimension are the definitive dimension.
    ///
    /// View has responsibility of storing dimension for later rendering if needed.
    fn layout(&mut self, dimensions: &Dimensions);

    /// Render the view with the given dimensions inside the given screen
    fn render<'a>(&self, frame: ScreenFrame<'a>, config: &C);

    /// Handle an event
    fn event(&mut self, event: &mut E);
}

impl<E, C> dyn View<E, C> {}

#[cfg(test)]
mod tests {
    use super::*;

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
}
