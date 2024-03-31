/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
#[cfg(test)]
use mockall::automock;
use std::cmp;
use std::io::Write;
use std::os::fd::AsFd;
use termion::raw::RawTerminal;
use termion::screen::AlternateScreen;
use unicode_segmentation::UnicodeSegmentation;

pub mod frame_layout;
pub mod input;
pub mod linear_layout;
pub mod list_view;
pub mod scroll_win;

pub type Screen<W> = BufferedScreen<AlternateScreen<RawTerminal<W>>>;

pub struct BufferedScreen<W: Write> {
    inner: W,
    buffer: Vec<u8>,
}

impl<W: Write> BufferedScreen<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            buffer: Vec::with_capacity(100 * 500 * 10),
        }
    }
}

impl<W: Write> Write for BufferedScreen<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.write_all(self.buffer.as_slice())?;
        self.buffer.clear();
        self.inner.flush()
    }
}

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

/// Remove all terminal specific chars sequences
pub fn clean(string: &str) -> String {
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

#[derive(Debug, Clone)]
pub enum LayoutConstraint {
    #[allow(dead_code)]
    Absolute(u16),
    #[allow(dead_code)]
    Relative(f32),
}

#[derive(Debug, Clone)]
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

pub enum MeasureSpec {
    Unspecified,
    Exactly(u16),
    AtMost(u16),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DimensionSpec {
    pub width: Option<u16>,
    pub height: Option<u16>,
}

impl DimensionSpec {
    pub fn layout(&self, x: u16, y: u16) -> Dimension {
        Dimension {
            x,
            y,
            width: self.width.unwrap(),
            height: self.height.unwrap(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Dimension {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
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
pub trait View<E, W>
where
    W: Write + AsFd,
{
    /// Compute the wanted dimension given passed width and height
    ///
    /// dimension_spec is IN/OUT var:
    ///  - IN: constraints
    ///  - OUT: desired dimension
    fn measure(&mut self, dimension_spec: &mut DimensionSpec) {
        let layout = self.get_layout();

        dimension_spec.width = match layout.width {
            LayoutParam::MatchParent => dimension_spec.width,
            LayoutParam::WrapContent => {
                panic!("If view can handle content, then it must define its own measure function")
            }
            LayoutParam::Absolute(width) => match dimension_spec.width {
                Some(width_spec) => Some(cmp::min(width, width_spec)),
                None => Some(width),
            },
        };

        dimension_spec.height = match layout.height {
            LayoutParam::MatchParent => dimension_spec.height,
            LayoutParam::WrapContent => {
                panic!("If view can handle content, then it must define its own measure function")
            }
            LayoutParam::Absolute(height) => match dimension_spec.height {
                Some(height_spec) => Some(cmp::min(height, height_spec)),
                None => Some(height),
            },
        };
    }

    /// Apply the definitive dimension given the top and left position
    ///
    /// dimension are the definitive dimension.
    /// If a dimension is None, then it can be modified, otherwise the dimension must be considered a hard constraint.
    ///
    /// Parent has responsability of storing resulting dimension for all its children.
    fn layout(&mut self, dimension_spec: &DimensionSpec, x: u16, y: u16) -> Dimension {
        dimension_spec.layout(x, y)
    }

    /// Render the view with the given dimensions inside the given screen
    fn render(&self, dimension: &Dimension, screen: &mut Screen<W>);

    /// If the layout of the current view is dirty. Meaning its content requires that its dimension
    /// changes
    fn is_layout_dirty(&self) -> bool {
        match self.get_layout() {
            LayoutParams {
                width: LayoutParam::WrapContent,
                ..
            } => self.is_dirty(),
            LayoutParams {
                height: LayoutParam::WrapContent,
                ..
            } => self.is_dirty(),
            _ => false,
        }
    }

    /// If this view requires to be rendered
    fn is_dirty(&self) -> bool;

    /// Handle an event
    fn event(&mut self, event: &mut E);

    /// Get the desired layout
    fn get_layout(&self) -> LayoutParams;
}

#[macro_export]
macro_rules! vprint {
    ($screen:expr, $fmt:expr) => {
        {
            while let Err(_) = write!($screen, $fmt) { };
        }
    };
    ($screen:expr, $fmt:expr, $($arg:tt)*) => {
        {
            while let Err(_) = write!($screen, $fmt, $($arg)*) { };
        }
    };
}

#[macro_export]
macro_rules! goto {
    ($screen:expr, $x:expr, $y:expr) => {
        crate::terminus::vprint!($screen, "{}", termion::cursor::Goto($x, $y));
    };
}

#[macro_export]
macro_rules! flush {
    ($screen:expr) => {
        while let Err(_) = $screen.flush() {}
    };
}

//#[cfg(not(feature = "no-cursor-save"))]
macro_rules! save_cursor {
    ($screen:expr) => {
        crate::terminus::vprint!($screen, "{}", termion::cursor::Save);
    };
}

//#[cfg(feature = "no-cursor-save")]
//#[macro_export]
//macro_rules! save_cursor {
//    ($screen:expr) => {
//        let (cursor_x, cursor_y) = $screen.cursor_pos().unwrap();
//    };
//}

//#[cfg(not(feature = "no-cursor-save"))]
macro_rules! restore_cursor {
    ($screen:expr) => {
        crate::terminus::vprint!($screen, "{}", termion::cursor::Restore);
    };
}

//#[cfg(feature = "no-cursor-save")]
//#[macro_export]
//macro_rules! restore_cursor {
//    ($screen:expr) => {
//        goto!($screen, cursor_x, cursor_y);
//    };
//}

impl<E, W> dyn View<E, W> where W: Write {}

pub(crate) use {flush, goto, restore_cursor, save_cursor, vprint};

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
        let cleaned = clean(input);

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
