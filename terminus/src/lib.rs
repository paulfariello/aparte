use std::cell::RefCell;
/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::io::Write;
use std::os::fd::AsFd;
use std::rc::Rc;
use std::{cmp, iter::Sum};

#[cfg(test)]
use mockall::automock;
use termion::raw::RawTerminal;
use termion::screen::AlternateScreen;
use unicode_segmentation::UnicodeSegmentation;

pub mod cursor;
pub mod frame_layout;
pub mod input;
pub mod linear_layout;
pub mod list_view;
pub mod scroll_win;

pub type Screen<W> = BufferedScreen<AlternateScreen<RawTerminal<W>>>;

pub type EventHandler<V, E> = Rc<RefCell<Box<dyn FnMut(&mut V, &mut E)>>>;

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

pub fn clear_screen<W>(dimensions: &Dimensions, screen: &mut Screen<W>)
where
    W: AsFd + Write,
{
    log::debug!("Clear screen: {:?}", dimensions);
    if dimensions.left == 1 {
        for top in dimensions.top..dimensions.top + dimensions.height {
            // Use fast erase if possible
            goto!(screen, 1 + dimensions.width, top);
            vprint!(screen, "{}", "\x1B[1K");
        }
    } else {
        for top in dimensions.top..dimensions.top + dimensions.height {
            goto!(screen, dimensions.left, top);
            vprint!(screen, "{: <1$}", "", dimensions.width as usize);
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum LayoutParam {
    MatchParent,
    WrapContent,
    #[allow(unused)]
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
    fn measure(&self, measure_specs: &MeasureSpecs) -> RequestedDimensions;

    /// Apply the definitive dimension given the top and left position
    ///
    /// dimension are the definitive dimension.
    ///
    /// View has responsability of storing dimension for later rendering if needed.
    fn layout(&mut self, dimensions: &Dimensions);

    /// Render the view with the given dimensions inside the given screen
    /// A view must not decide wether rendering should be avoided inside this function.
    /// Such decision is made with parent and with the help of is_dirty()
    fn render(&self, screen: &mut Screen<W>);

    /// Force dirty state on view
    fn set_dirty(&mut self);

    /// If this view requires to be rendered
    // we could avoid this function if view could decide wether they can avoid rendering on their
    // own:
    //  - layout has changed
    //  - content has changed
    // but a parent could have changed without impacting layout (FrameLayout is such an example)
    // then we must have a way to force rendering.
    //
    // Other option is to keep is_dirty to tell about layout change and/or content change
    // and let render be an equivalent of force rendering.
    //
    // But then what happens with ScrollWin:
    //  - 1 child is dirty (content only)
    //
    //  - is_dirty() is called on ScrollWin -> return true
    //  - render() is called on ScrollWin -> render all children
    //
    // If 1 child is dirty with content change only, then the layout phase from parent will not set
    // dirty any other children
    fn is_dirty(&self) -> bool;

    /// Handle an event
    fn event(&mut self, event: &mut E);
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
        $crate::vprint!($screen, "{}", ::termion::cursor::Goto($x, $y));
    };
}

#[macro_export]
macro_rules! flush {
    ($screen:expr) => {
        while let Err(_) = $screen.flush() {}
    };
}

//#[cfg(not(feature = "no-cursor-save"))]
#[macro_export]
macro_rules! save_cursor {
    ($screen:expr) => {
        $crate::vprint!($screen, "{}", ::termion::cursor::Save);
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
#[macro_export]
macro_rules! restore_cursor {
    ($screen:expr) => {
        $crate::vprint!($screen, "{}", ::termion::cursor::Restore);
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
}
