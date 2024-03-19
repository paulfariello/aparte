/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use crate::cursor::Cursor;
use linked_hash_map::{Entry, LinkedHashMap};
use std::cell::RefCell;
use std::cmp;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::{self};
use std::hash::Hash;
use std::io::Write;
use std::os::fd::AsFd;
use std::rc::Rc;
use termion::raw::RawTerminal;
use termion::screen::AlternateScreen;
use unicode_segmentation::UnicodeSegmentation;

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
        remaining -= append.graphemes(true).count();
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
pub struct LayoutConstraints {
    max: Option<LayoutConstraint>,
    min: Option<LayoutConstraint>,
}

#[derive(Debug, Clone)]
pub enum LayoutBehavior {
    MatchParent,
    WrapContent(LayoutConstraints),
    Absolute(u16),
}

#[derive(Debug, Clone)]
pub struct Layout {
    behavior: LayoutBehavior,
}

impl Layout {
    pub fn match_parent() -> Self {
        Self {
            behavior: LayoutBehavior::MatchParent,
        }
    }

    pub fn wrap_content() -> Self {
        Self {
            behavior: LayoutBehavior::WrapContent(LayoutConstraints {
                max: None,
                min: None,
            }),
        }
    }

    pub fn absolute(value: u16) -> Self {
        Self {
            behavior: LayoutBehavior::Absolute(value),
        }
    }

    #[allow(dead_code)]
    pub fn with_absolute_max(mut self, value: u16) -> Self {
        if let LayoutBehavior::WrapContent(ref mut constraint) = self.behavior {
            constraint.max = Some(LayoutConstraint::Absolute(value));
        } else {
            unreachable!();
        }

        self
    }

    #[allow(dead_code)]
    pub fn with_absolute_min(mut self, value: u16) -> Self {
        if let LayoutBehavior::WrapContent(ref mut constraint) = self.behavior {
            constraint.min = Some(LayoutConstraint::Absolute(value));
        } else {
            unreachable!();
        }

        self
    }

    #[allow(dead_code)]
    pub fn with_relative_max(mut self, value: f32) -> Self {
        if let LayoutBehavior::WrapContent(ref mut constraint) = self.behavior {
            constraint.max = Some(LayoutConstraint::Relative(value));
        } else {
            unreachable!();
        }

        self
    }

    #[allow(dead_code)]
    pub fn with_relative_min(mut self, value: f32) -> Self {
        if let LayoutBehavior::WrapContent(ref mut constraint) = self.behavior {
            constraint.min = Some(LayoutConstraint::Relative(value));
        } else {
            unreachable!();
        }

        self
    }
}

pub fn apply_constraints(value: u16, parent: Option<u16>, constraints: &LayoutConstraints) -> u16 {
    let mut value = value;
    if let Some(min) = constraints.min.clone() {
        let min = match min {
            LayoutConstraint::Absolute(min) => min,
            LayoutConstraint::Relative(min) => match parent {
                Some(parent) => (parent as f32 * min).floor() as u16,
                None => value,
            },
        };
        value = cmp::max(min, value);
    }

    let max = {
        if let Some(max) = constraints.max.clone() {
            match max {
                LayoutConstraint::Absolute(max) => match parent {
                    Some(parent) => cmp::min(max, parent),
                    None => max,
                },
                LayoutConstraint::Relative(max) => match parent {
                    Some(parent) => (parent as f32 * max).floor() as u16,
                    None => value,
                },
            }
        } else {
            match parent {
                Some(parent) => parent,
                None => value,
            }
        }
    };
    cmp::min(max, value)
}

#[derive(Debug, Clone)]
pub struct Layouts {
    pub width: Layout,
    pub height: Layout,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Orientation {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone)]
pub struct Dimension {
    pub x: u16,
    pub y: u16,
    pub w: Option<u16>,
    pub h: Option<u16>,
}

impl Dimension {
    pub fn new() -> Self {
        Self {
            x: 1,
            y: 1,
            w: None,
            h: None,
        }
    }
}

pub trait View<E, W>
where
    W: Write + AsFd,
{
    /// Compute the wanted dimension given passed width and height
    fn measure(
        &mut self,
        dimension: &mut Dimension,
        width_spec: Option<u16>,
        height_spec: Option<u16>,
    ) {
        let layout = self.get_layouts();
        dimension.w = match layout.width.behavior {
            LayoutBehavior::MatchParent => width_spec,
            LayoutBehavior::WrapContent(_) => unreachable!(),
            LayoutBehavior::Absolute(width) => match width_spec {
                Some(width_spec) => Some(cmp::min(width, width_spec)),
                None => Some(width),
            },
        };

        dimension.h = match layout.height.behavior {
            LayoutBehavior::MatchParent => height_spec,
            LayoutBehavior::WrapContent(_) => unreachable!(),
            LayoutBehavior::Absolute(height) => match height_spec {
                Some(height_spec) => Some(cmp::min(height, height_spec)),
                None => Some(height),
            },
        };
    }

    /// Apply the definitive dimension given the top and left position
    fn layout(&mut self, dimension: &mut Dimension, top: u16, left: u16) {
        dimension.x = left;
        dimension.y = top;
    }

    /// If the layout of the current view is dirty. Meaning its content requires that its dimension
    /// changes
    fn is_layout_dirty(&self) -> bool {
        match self.get_layouts() {
            Layouts {
                width:
                    Layout {
                        behavior: LayoutBehavior::WrapContent(_),
                        ..
                    },
                ..
            } => self.is_dirty(),
            Layouts {
                height:
                    Layout {
                        behavior: LayoutBehavior::WrapContent(_),
                        ..
                    },
                ..
            } => self.is_dirty(),
            _ => false,
        }
    }

    /// If this view requires to be rendered
    fn is_dirty(&self) -> bool;

    /// Render the view with the given dimensions inside the given screen
    fn render(&mut self, dimension: &Dimension, screen: &mut Screen<W>);

    /// Handle an event
    fn event(&mut self, event: &mut E);

    /// Get the desired layout
    fn get_layouts(&self) -> Layouts;
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
        vprint!($screen, "{}", termion::cursor::Goto($x, $y));
    };
}

#[macro_export]
macro_rules! flush {
    ($screen:expr) => {
        while let Err(_) = $screen.flush() {}
    };
}

#[cfg(not(feature = "no-cursor-save"))]
#[macro_export]
macro_rules! save_cursor {
    ($screen:expr) => {
        vprint!($screen, "{}", termion::cursor::Save);
    };
}

#[cfg(feature = "no-cursor-save")]
#[macro_export]
macro_rules! save_cursor {
    ($screen:expr) => {
        let (cursor_x, cursor_y) = $screen.cursor_pos().unwrap();
    };
}

#[cfg(not(feature = "no-cursor-save"))]
#[macro_export]
macro_rules! restore_cursor {
    ($screen:expr) => {
        vprint!($screen, "{}", termion::cursor::Restore);
    };
}

#[cfg(feature = "no-cursor-save")]
#[macro_export]
macro_rules! restore_cursor {
    ($screen:expr) => {
        goto!($screen, cursor_x, cursor_y);
    };
}

impl<E, W> dyn View<E, W> where W: Write {}

pub struct FrameLayout<E, W, K>
where
    K: Hash + Eq + Clone,
    W: Write + AsFd,
{
    children: HashMap<K, (Dimension, Box<dyn View<E, W>>)>,
    current: Option<K>,
    event_handler: Option<Rc<RefCell<Box<dyn FnMut(&mut Self, &mut E)>>>>,
    dirty: bool,
    layouts: Layouts,
}

impl<E, W, K> FrameLayout<E, W, K>
where
    K: Hash + Eq + Clone,
    W: Write + AsFd,
{
    pub fn new() -> Self {
        Self {
            children: HashMap::new(),
            current: None,
            event_handler: None,
            dirty: true,
            layouts: Layouts {
                width: Layout::match_parent(),
                height: Layout::match_parent(),
            },
        }
    }

    pub fn with_event<F>(mut self, event_handler: F) -> Self
    where
        F: FnMut(&mut Self, &mut E) + 'static,
    {
        self.event_handler = Some(Rc::new(RefCell::new(Box::new(event_handler))));
        self
    }

    #[allow(unused)]
    pub fn with_layouts(mut self, layouts: Layouts) -> Self {
        self.layouts = layouts;
        self
    }

    pub fn set_current(&mut self, key: K) {
        self.current = Some(key);
        self.dirty = true;
    }

    pub fn get_current_mut<'a>(&'a mut self) -> Option<&'a mut Box<dyn View<E, W>>> {
        if let Some(current) = &self.current {
            if let Some((_, view)) = self.children.get_mut(current) {
                Some(view)
            } else {
                unreachable!();
            }
        } else {
            None
        }
    }

    #[allow(unused)]
    pub fn get_current<'a>(&'a self) -> Option<&'a Box<dyn View<E, W>>> {
        if let Some(current) = &self.current {
            if let Some((_, view)) = self.children.get(current) {
                Some(view)
            } else {
                unreachable!();
            }
        } else {
            None
        }
    }

    #[allow(unused)]
    pub fn get_current_key<'a>(&'a self) -> Option<&'a K> {
        self.current.as_ref()
    }

    #[allow(unused)]
    pub fn insert<T>(&mut self, key: K, view: T)
    where
        T: View<E, W> + 'static,
    {
        let child_dimension = Dimension::new();
        self.children.insert(key, (child_dimension, Box::new(view)));
    }

    pub fn insert_boxed(&mut self, key: K, view: Box<dyn View<E, W> + 'static>) {
        let child_dimension = Dimension::new();
        self.children.insert(key, (child_dimension, view));
    }

    pub fn remove(&mut self, key: &K) {
        self.children.remove(key);
        if Some(key) == self.current.as_ref() {
            self.current = self.children.keys().next().cloned();
        }
    }

    pub fn iter_children_mut<'a>(
        &'a mut self,
    ) -> impl Iterator<Item = &'a mut Box<dyn View<E, W>>> {
        self.children
            .iter_mut()
            .map(|(_, (_, child_view))| child_view)
    }

    #[allow(unused)]
    pub fn iter_children<'a>(&'a self) -> impl Iterator<Item = &'a Box<dyn View<E, W>>> {
        self.children.iter().map(|(_, (_, child_view))| child_view)
    }
}

impl<E, W, K> View<E, W> for FrameLayout<E, W, K>
where
    K: Hash + Eq + Clone,
    W: Write + AsFd,
{
    fn measure(
        &mut self,
        dimension: &mut Dimension,
        width_spec: Option<u16>,
        height_spec: Option<u16>,
    ) {
        dimension.w = width_spec;
        dimension.h = height_spec;

        for (_, (child_dimension, child_view)) in self.children.iter_mut() {
            child_view.measure(child_dimension, dimension.w, dimension.h);
        }
    }

    fn layout(&mut self, dimension: &mut Dimension, top: u16, left: u16) {
        dimension.x = left;
        dimension.y = top;

        for (_, (child_dimension, child_view)) in self.children.iter_mut() {
            child_view.layout(child_dimension, top, left);
        }

        // Set dirty to ensure all children are rendered on next render
        self.dirty = true;
    }

    fn render(&mut self, _dimension: &Dimension, screen: &mut Screen<W>) {
        if let Some(current) = &self.current {
            let (child_dimension, child_view) = self.children.get_mut(current).unwrap();
            if self.dirty || child_view.is_dirty() {
                child_view.render(child_dimension, screen);
            }
        }
        self.dirty = false;
    }

    fn is_layout_dirty(&self) -> bool {
        match self.dirty {
            true => true,
            _ => {
                if let Some(current) = &self.current {
                    let (_, child_view) = self.children.get(current).unwrap();
                    child_view.is_layout_dirty()
                } else {
                    false
                }
            }
        }
    }

    fn is_dirty(&self) -> bool {
        match self.dirty {
            true => true,
            _ => {
                if let Some(current) = &self.current {
                    let (_, child_view) = self.children.get(current).unwrap();
                    child_view.is_dirty()
                } else {
                    false
                }
            }
        }
    }

    fn event(&mut self, event: &mut E) {
        if let Some(handler) = &self.event_handler {
            let handler = Rc::clone(handler);
            let handler = &mut *handler.borrow_mut();
            handler(self, event);
        } else {
            for child in self.iter_children_mut() {
                child.event(event);
            }
        }
    }

    fn get_layouts(&self) -> Layouts {
        self.layouts.clone()
    }
}

pub struct LinearLayout<E, W> {
    pub orientation: Orientation,
    pub children: Vec<(Dimension, Box<dyn View<E, W>>)>,
    pub event_handler: Option<Rc<RefCell<Box<dyn FnMut(&mut Self, &mut E)>>>>,
    pub dirty: bool,
    layouts: Layouts,
}

impl<E, W> LinearLayout<E, W>
where
    W: Write + AsFd,
{
    pub fn new(orientation: Orientation) -> Self {
        Self {
            orientation,
            children: Vec::new(),
            event_handler: None,
            dirty: true,
            layouts: Layouts {
                width: Layout::match_parent(),
                height: Layout::match_parent(),
            },
        }
    }

    pub fn push<T>(&mut self, view: T)
    where
        T: View<E, W> + 'static,
    {
        let dimension = Dimension::new();
        self.children.push((dimension, Box::new(view)));
    }

    pub fn with_event<F>(mut self, event_handler: F) -> Self
    where
        F: FnMut(&mut Self, &mut E) + 'static,
    {
        self.event_handler = Some(Rc::new(RefCell::new(Box::new(event_handler))));
        self
    }

    #[allow(unused)]
    pub fn with_layouts(mut self, layouts: Layouts) -> Self {
        self.layouts = layouts;
        self
    }

    pub fn iter_children_mut<'a>(
        &'a mut self,
    ) -> impl Iterator<Item = &'a mut Box<dyn View<E, W>>> {
        self.children.iter_mut().map(|(_, child_view)| child_view)
    }

    #[allow(unused)]
    pub fn iter_children<'a>(&'a self) -> impl Iterator<Item = &'a Box<dyn View<E, W>>> {
        self.children.iter().map(|(_, child_view)| child_view)
    }
}

impl<E, W> View<E, W> for LinearLayout<E, W>
where
    W: Write + AsFd,
{
    fn measure(
        &mut self,
        dimension: &mut Dimension,
        width_spec: Option<u16>,
        height_spec: Option<u16>,
    ) {
        /* Measure dimension of this layout with the following stpes:
         *
         *  - Compute max dimension from parent
         *  - Compute min dimension from children
         *  - Split remaining space for each child that don't have strong size requirement
         *    (answered 0 to first measure pass)
         *  - Set dimension for each children
         */
        let layouts = self.get_layouts();
        let max_width = match layouts.width.behavior {
            LayoutBehavior::MatchParent => width_spec,
            LayoutBehavior::WrapContent(_) => width_spec,
            LayoutBehavior::Absolute(width) => match width_spec {
                Some(width_spec) => Some(cmp::min(width, width_spec)),
                None => Some(width),
            },
        };

        let max_height = match layouts.height.behavior {
            LayoutBehavior::MatchParent => height_spec,
            LayoutBehavior::WrapContent(_) => height_spec,
            LayoutBehavior::Absolute(height) => match height_spec {
                Some(height_spec) => Some(cmp::min(height, height_spec)),
                None => Some(height),
            },
        };

        let mut min_width = 0;
        let mut min_height = 0;
        for (child_dimension, child_view) in self.children.iter_mut() {
            child_view.measure(child_dimension, None, None);
            let child_layouts = child_view.get_layouts();
            let requested_width = match &child_layouts {
                Layouts {
                    width:
                        Layout {
                            behavior: LayoutBehavior::WrapContent(constraints),
                            ..
                        },
                    ..
                } => apply_constraints(child_dimension.w.unwrap_or(0), max_width, constraints),
                _ => child_dimension.w.unwrap_or(0),
            };

            let requested_height = match &child_layouts {
                Layouts {
                    height:
                        Layout {
                            behavior: LayoutBehavior::WrapContent(constraints),
                            ..
                        },
                    ..
                } => apply_constraints(child_dimension.h.unwrap_or(0), max_height, constraints),
                _ => child_dimension.h.unwrap_or(0),
            };

            match self.orientation {
                Orientation::Vertical => {
                    min_width = cmp::max(min_width, requested_width);
                    min_height += requested_height;
                }
                Orientation::Horizontal => {
                    min_width += requested_width;
                    min_height = cmp::max(min_height, requested_height);
                }
            }
        }

        let remaining_width = match max_width {
            Some(max_width) => max_width - min_width,
            None => 0,
        };

        let remaining_height = match max_height {
            Some(max_height) => max_height - min_height,
            None => 0,
        };

        // Split remaining space to children that don't know their size
        let splitted_width = match self.orientation {
            Orientation::Vertical => max_width,
            Orientation::Horizontal => {
                let unsized_children = self
                    .children
                    .iter()
                    .filter(|(dimension, _)| dimension.w.is_none());
                Some(match unsized_children.count() {
                    0 => 0,
                    count => remaining_width / count as u16,
                })
            }
        };
        let splitted_height = match self.orientation {
            Orientation::Vertical => {
                let unsized_children = self
                    .children
                    .iter()
                    .filter(|(dimension, _)| dimension.h.is_none());
                Some(match unsized_children.count() {
                    0 => 0,
                    count => remaining_height / count as u16,
                })
            }
            Orientation::Horizontal => max_height,
        };

        dimension.w = Some(0);
        dimension.h = Some(0);

        for (child_dimension, child_view) in self.children.iter_mut() {
            let mut width_spec = match child_dimension.w {
                Some(w) => Some(w),
                None => splitted_width,
            };

            let mut height_spec = match child_dimension.h {
                Some(h) => Some(h),
                None => splitted_height,
            };

            if self.orientation == Orientation::Horizontal && max_width.is_some() {
                width_spec = Some(cmp::min(
                    width_spec.unwrap(),
                    max_width.unwrap() - dimension.w.unwrap(),
                ));
            }

            if self.orientation == Orientation::Vertical && max_height.is_some() {
                height_spec = Some(cmp::min(
                    height_spec.unwrap(),
                    max_height.unwrap() - dimension.h.unwrap(),
                ));
            }

            child_view.measure(child_dimension, width_spec, height_spec);

            match self.orientation {
                Orientation::Vertical => {
                    dimension.w = Some(cmp::max(
                        dimension.w.unwrap(),
                        child_dimension.w.unwrap_or(0),
                    ));
                    dimension.h = Some(dimension.h.unwrap() + child_dimension.h.unwrap_or(0));
                }
                Orientation::Horizontal => {
                    dimension.w = Some(dimension.w.unwrap() + child_dimension.w.unwrap_or(0));
                    dimension.h = Some(cmp::max(
                        dimension.h.unwrap(),
                        child_dimension.h.unwrap_or(0),
                    ));
                }
            }
        }
    }

    fn layout(&mut self, dimension: &mut Dimension, top: u16, left: u16) {
        dimension.x = left;
        dimension.y = top;

        let mut x = dimension.x;
        let mut y = dimension.y;

        for (child_dimension, child_view) in self.children.iter_mut() {
            child_view.layout(child_dimension, y, x);
            match self.orientation {
                Orientation::Vertical => y += child_dimension.h.unwrap(),
                Orientation::Horizontal => x += child_dimension.w.unwrap(),
            }
        }

        // Set dirty to ensure all children are rendered on next render
        self.dirty = true;
    }

    fn render(&mut self, _dimension: &Dimension, screen: &mut Screen<W>) {
        for (child_dimension, child_view) in self.children.iter_mut() {
            if self.dirty || child_view.is_dirty() {
                child_view.render(child_dimension, screen);
            }
        }
        self.dirty = false;
    }

    fn is_layout_dirty(&self) -> bool {
        match self.dirty {
            true => true,
            _ => {
                let mut dirty = false;
                for (_, child_view) in self.children.iter() {
                    dirty |= child_view.is_layout_dirty()
                }
                dirty
            }
        }
    }

    fn is_dirty(&self) -> bool {
        match self.dirty {
            true => true,
            _ => {
                let mut dirty = false;
                for (_, child_view) in self.children.iter() {
                    dirty |= child_view.is_dirty()
                }
                dirty
            }
        }
    }

    fn event(&mut self, event: &mut E) {
        if let Some(handler) = &self.event_handler {
            let handler = Rc::clone(handler);
            let handler = &mut *handler.borrow_mut();
            handler(self, event);
        } else {
            for child in self.iter_children_mut() {
                child.event(event);
            }
        }
    }

    fn get_layouts(&self) -> Layouts {
        self.layouts.clone()
    }
}

pub struct Input<E> {
    pub buf: String,
    pub tmp_buf: Option<String>,
    pub password: bool,
    pub history: Vec<String>,
    pub history_index: usize,
    // Used to index code points in buf (don't use it to directly index buf)
    pub cursor: Cursor,
    // start index (in code points) of the view inside the buffer
    // |-----------------------|
    // | buffer text           |
    // |-----------------------|
    //     |-----------|
    //     | view      |
    //     |-----------|
    pub view: Cursor,
    pub event_handler: Option<Rc<RefCell<Box<dyn FnMut(&mut Self, &mut E)>>>>,
    pub dirty: bool,
    width: usize,
}

impl<E> Input<E> {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            tmp_buf: None,
            password: false,
            history: Vec::new(),
            history_index: 0,
            cursor: Cursor::new(0),
            view: Cursor::new(0),
            event_handler: None,
            dirty: true,
            width: 0,
        }
    }

    pub fn with_event<F>(mut self, event_handler: F) -> Self
    where
        F: FnMut(&mut Self, &mut E) + 'static,
    {
        self.event_handler = Some(Rc::new(RefCell::new(Box::new(event_handler))));
        self
    }

    pub fn key(&mut self, c: char) {
        let byte_index = self.cursor.index(&self.buf);
        self.buf.insert(byte_index, c);
        self.cursor += 1;

        if !self.password {
            self.dirty = true;
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor > Cursor::new(0) {
            self.cursor -= 1;
            let mut byte_index = self.cursor.index(&self.buf);
            if byte_index == self.buf.len() {
                byte_index -= 1;
            }
            self.buf.remove(byte_index);
            // TODO work on grapheme
            while !self.buf.is_char_boundary(byte_index) {
                self.buf.remove(byte_index);
            }
        }
        if !self.password {
            self.dirty = true;
        }
    }

    pub fn backward_delete_word(&mut self) {
        let iter = self.buf[..self.cursor.index(&self.buf)].chars().rev();
        let mut word_start = self.cursor.clone();
        word_start -= next_word(iter);
        self.buf.replace_range(
            word_start.index(&self.buf)..self.cursor.index(&self.buf),
            "",
        );
        self.cursor = word_start;
        if !self.password {
            self.dirty = true;
        }
    }

    pub fn delete_from_cursor_to_start(&mut self) {
        self.buf.replace_range(0..self.cursor.index(&self.buf), "");
        self.cursor = Cursor::new(0);
        self.view = Cursor::new(0);
        if !self.password {
            self.dirty = true;
        }
    }

    pub fn delete_from_cursor_to_end(&mut self) {
        self.buf.replace_range(self.cursor.index(&self.buf).., "");
        if !self.password {
            self.dirty = true;
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.buf.graphemes(true).count() {
            let byte_index = self.cursor.index(&self.buf);

            self.buf.remove(byte_index);
            while !self.buf.is_char_boundary(byte_index) {
                self.buf.remove(byte_index);
            }
        }
        if !self.password {
            self.dirty = true;
        }
    }

    pub fn home(&mut self) {
        self.cursor = Cursor::new(0);
        self.view = Cursor::new(0);
        if !self.password {
            self.dirty = true;
        }
    }

    pub fn end(&mut self) {
        self.cursor = Cursor::new(self.buf.graphemes(true).count());
        if self.cursor > self.width - 1 {
            self.view = &self.cursor - (self.width - 1);
        } else {
            self.view = Cursor::new(0);
        }
        if !self.password {
            self.dirty = true;
        }
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.cursor = Cursor::new(0);
        self.view = Cursor::new(0);
        let _ = self.tmp_buf.take();
        self.password = false;
        self.dirty = true;
    }

    pub fn left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
        if !self.password {
            self.dirty = true;
        }
    }

    pub fn right(&mut self) {
        if self.cursor < term_string_visible_len(&self.buf) {
            self.cursor += 1;
        }
        if !self.password {
            self.dirty = true;
        }
    }

    pub fn word_left(&mut self) {
        log::debug!("Word left");
        let iter = self.buf[..self.cursor.index(&self.buf)].chars().rev();
        self.cursor -= next_word(iter);

        if !self.password {
            self.dirty = true;
        }
    }

    pub fn word_right(&mut self) {
        log::debug!("Word right");
        let iter = self.buf[self.cursor.index(&self.buf)..].chars();
        self.cursor += next_word(iter);

        if !self.password {
            self.dirty = true;
        }
    }

    pub fn password(&mut self) {
        self.password = true;
        self.dirty = true;
    }

    pub fn validate(&mut self) -> (String, bool) {
        if !self.password {
            self.history.push(self.buf.clone());
            self.history_index = self.history.len();
        }
        let buf = self.buf.clone();
        let password = self.password;
        self.clear();
        (buf, password)
    }

    pub fn previous(&mut self) {
        if self.history_index == 0 {
            return;
        }

        if self.tmp_buf.is_none() {
            self.tmp_buf = Some(self.buf.clone());
        }

        self.history_index -= 1;
        self.buf = self.history[self.history_index].clone();
        self.end();
        self.dirty = true;
    }

    pub fn next(&mut self) {
        if self.history_index == self.history.len() {
            return;
        }

        self.history_index += 1;
        if self.history_index == self.history.len() {
            self.buf = self.tmp_buf.take().unwrap();
        } else {
            self.buf = self.history[self.history_index].clone();
        }
        self.end();
        self.dirty = true;
    }
}

impl<E, W> View<E, W> for Input<E>
where
    W: Write + AsFd,
{
    fn render(&mut self, dimension: &Dimension, screen: &mut Screen<W>) {
        self.width = dimension.w.unwrap() as usize;
        match self.password {
            true => {
                goto!(screen, dimension.x, dimension.y);
                vprint!(screen, "password: ");
                flush!(screen);

                self.dirty = false;
            }
            false => {
                // Max displayable size is view width less 1 for cursor
                let max_size = (dimension.w.unwrap() - 1) as usize;

                // cursor must always be inside the view
                if self.cursor < self.view {
                    if self.cursor < max_size {
                        self.view = Cursor::new(0);
                    } else {
                        self.view = &self.cursor - (dimension.w.unwrap() as usize - 1);
                    }
                } else if self.cursor > &self.view + (dimension.w.unwrap() as usize - 1) {
                    self.view = &self.cursor - (dimension.w.unwrap() as usize - 1);
                }
                assert!(self.cursor >= self.view);
                assert!(self.cursor <= &self.view + (max_size + 1));

                let start_index = self.view.index(&self.buf);
                let end_index = (&self.view + max_size).index(&self.buf);
                let buf = &self.buf[start_index..end_index];
                let cursor = &self.cursor - &self.view;

                goto!(screen, dimension.x, dimension.y);
                for _ in 0..max_size {
                    vprint!(screen, " ");
                }

                goto!(screen, dimension.x, dimension.y);
                vprint!(screen, "{}", buf);
                goto!(screen, dimension.x + cursor.get() as u16, dimension.y);

                flush!(screen);

                self.dirty = false;
            }
        }
    }

    fn is_layout_dirty(&self) -> bool {
        // Content can never change Input dimension
        false
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn event(&mut self, event: &mut E) {
        if let Some(handler) = &self.event_handler {
            let handler = Rc::clone(handler);
            let handler = &mut *handler.borrow_mut();
            handler(self, event);
        }
    }

    fn get_layouts(&self) -> Layouts {
        Layouts {
            width: Layout::match_parent(),
            height: Layout::absolute(1),
        }
    }
}

pub trait Window<E, W, T>: View<E, W>
where
    W: Write + AsFd,
    T: fmt::Display + Hash + Eq + Ord,
{
    fn insert(&mut self, item: T);
    /// PageUp the window, return true if top is reached
    fn page_up(&mut self) -> bool;
    /// PageDown the window, return true if bottom is reached
    fn page_down(&mut self) -> bool;
}

pub struct BufferedWin<E, W, I>
where
    I: fmt::Display + Hash + Eq + Ord,
{
    pub next_line: u16,
    pub history: BTreeSet<I>,
    pub view: usize,
    pub event_handler: Option<Rc<RefCell<Box<dyn FnMut(&mut Self, &mut E)>>>>,
    pub dirty: bool,
    width: usize,
    height: usize,
    layouts: Layouts,
}

impl<E, W, I> BufferedWin<E, W, I>
where
    I: fmt::Display + Hash + Eq + Ord,
{
    pub fn new() -> Self {
        Self {
            next_line: 0,
            history: BTreeSet::new(),
            view: 0,
            event_handler: None,
            dirty: true,
            width: 0,
            height: 0,
            layouts: Layouts {
                width: Layout::match_parent(),
                height: Layout::match_parent(),
            },
        }
    }

    pub fn with_event<F>(mut self, event_handler: F) -> Self
    where
        F: FnMut(&mut Self, &mut E) + 'static,
    {
        self.event_handler = Some(Rc::new(RefCell::new(Box::new(event_handler))));
        self
    }

    #[allow(unused)]
    pub fn with_layouts(mut self, layouts: Layouts) -> Self {
        self.layouts = layouts;
        self
    }

    fn get_rendered_items(&self) -> Vec<String> {
        let max_len = self.width;
        let mut buffers: Vec<String> = Vec::new();

        for buf in &self.history {
            let formatted = format!("{buf}");
            for line in formatted.lines() {
                let mut words = line.split_word_bounds();

                let mut line_len = 0;
                let mut chunk = String::new();
                while let Some(word) = words.next() {
                    let visible_word;
                    let mut remaining = String::new();

                    // We can safely unwrap here because split_word_bounds produce non empty words
                    let first_char = word.chars().next().unwrap();

                    if first_char == '\x1b' {
                        // Handle Escape sequence: see https://www.ecma-international.org/publications/files/ECMA-ST/Ecma-048.pdf
                        // First char is a word boundary
                        //
                        // We must ignore them for the visible length count but include them in the
                        // final chunk that will be written to the terminal

                        if let Some(word) = words.next() {
                            match word {
                                "[" => {
                                    // Control Sequence Introducer are accepted and can safely be
                                    // written to terminal
                                    let mut escape = String::from("\x1b[");
                                    let mut end = false;

                                    for word in words.by_ref() {
                                        for c in word.chars() {
                                            // Push all char belonging to escape sequence
                                            // but keep remaining for wrap computation
                                            if !end {
                                                escape.push(c);
                                                match c {
                                                    '\x30'..='\x3f' => {} // parameter bytes
                                                    '\x20'..='\x2f' => {} // intermediate bytes
                                                    '\x40'..='\x7e' => {
                                                        // final byte
                                                        chunk.push_str(&escape);
                                                        end = true;
                                                    }
                                                    _ => {
                                                        // Invalid escape sequence, just ignore it
                                                        end = true;
                                                    }
                                                }
                                            } else {
                                                remaining.push(c);
                                            }
                                        }

                                        if end {
                                            break;
                                        }
                                    }
                                }
                                _ => {
                                    // Other sequence are not handled and just ignored
                                }
                            }
                        } else {
                            // Nothing is following the escape char
                            // We can simply ignore it
                        }
                        visible_word = remaining.as_str();
                    } else {
                        visible_word = word;
                    }

                    if visible_word.is_empty() {
                        continue;
                    }

                    let grapheme_count = visible_word.graphemes(true).count();

                    if line_len + grapheme_count > max_len {
                        // Wrap line
                        buffers.push(chunk);
                        chunk = String::new();
                        line_len = 0;
                    }

                    chunk.push_str(visible_word);
                    line_len += grapheme_count;
                }

                buffers.push(chunk);
            }
        }

        buffers
    }

    #[allow(dead_code)]
    pub fn first<'a>(&'a self) -> Option<&'a I> {
        self.history.iter().next()
    }
}

impl<E, W, I> Window<E, W, I> for BufferedWin<E, W, I>
where
    W: Write + AsFd,
    I: fmt::Display + Hash + Eq + Ord,
{
    fn insert(&mut self, item: I) {
        // We don't care about rendered buffer, we avoid computation here at cost of false positive
        // (set dirty while in fact it shouldn't)
        let len = self.history.len();
        let position = len
            - self
                .history
                .iter()
                .position(|iter| iter > &item)
                .unwrap_or(self.history.len());
        self.history.replace(item);
        self.dirty |= position >= self.view && position <= self.view + self.height;
    }

    fn page_up(&mut self) -> bool {
        let buffers = self.get_rendered_items();
        let count = buffers.len();

        if count < self.height {
            return true;
        }

        self.dirty = true;

        let max = count - self.height;

        if self.view + self.height < max {
            self.view += self.height;
            false
        } else {
            self.view = max;
            true
        }
    }

    fn page_down(&mut self) -> bool {
        self.dirty = true;
        if self.view > self.height {
            self.view -= self.height;
            false
        } else {
            self.view = 0;
            true
        }
    }
}

impl<E, W, I> View<E, W> for BufferedWin<E, W, I>
where
    W: Write + AsFd,
    I: fmt::Display + Hash + Eq + Ord,
{
    fn render(&mut self, dimension: &Dimension, screen: &mut Screen<W>) {
        save_cursor!(screen);
        self.width = dimension.w.unwrap() as usize;
        self.height = dimension.h.unwrap() as usize;

        self.next_line = 0;

        let buffers = self.get_rendered_items();
        let count = buffers.len();
        let mut iter = buffers.iter();

        if count > dimension.h.unwrap() as usize {
            for _ in 0..count - dimension.h.unwrap() as usize - self.view {
                if iter.next().is_none() {
                    break;
                }
            }
        }

        for y in dimension.y..dimension.y + dimension.h.unwrap() {
            goto!(screen, dimension.x, y);
            for _ in dimension.x..dimension.x + dimension.w.unwrap() {
                vprint!(screen, " ");
            }

            goto!(screen, dimension.x, y);
            if let Some(buf) = iter.next() {
                vprint!(screen, "{}", buf);
                self.next_line += 1;
            }
        }

        restore_cursor!(screen);

        self.dirty = false;
    }

    fn event(&mut self, event: &mut E) {
        if let Some(handler) = &self.event_handler {
            let handler = Rc::clone(handler);
            let handler = &mut *handler.borrow_mut();
            handler(self, event);
        }
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn get_layouts(&self) -> Layouts {
        self.layouts.clone()
    }
}

pub struct ListView<E, W, G, V>
where
    G: fmt::Display + Hash + Eq,
    V: fmt::Display + Hash + Eq,
{
    items: LinkedHashMap<Option<G>, HashSet<V>>,
    unique: bool,
    sort_item: Option<Box<dyn FnMut(&V, &V) -> cmp::Ordering>>,
    #[allow(dead_code)]
    sort_group: Option<Box<dyn FnMut(&G, &G) -> cmp::Ordering>>,
    event_handler: Option<Rc<RefCell<Box<dyn FnMut(&mut Self, &mut E)>>>>,
    dirty: bool,
    layouts: Layouts,
}

impl<E, W, G, V> ListView<E, W, G, V>
where
    G: fmt::Display + Hash + Eq,
    V: fmt::Display + Hash + Eq,
{
    pub fn new() -> Self {
        Self {
            items: LinkedHashMap::new(),
            unique: false,
            sort_item: None,
            sort_group: None,
            event_handler: None,
            dirty: true,
            layouts: Layouts {
                width: Layout::match_parent(),
                height: Layout::match_parent(),
            },
        }
    }

    pub fn with_event<F>(mut self, event_handler: F) -> Self
    where
        F: FnMut(&mut Self, &mut E) + 'static,
    {
        self.event_handler = Some(Rc::new(RefCell::new(Box::new(event_handler))));
        self
    }

    pub fn with_layouts(mut self, layouts: Layouts) -> Self {
        self.layouts = layouts;
        self
    }

    pub fn with_none_group(mut self) -> Self {
        if let Entry::Vacant(vacant) = self.items.entry(None) {
            vacant.insert(HashSet::new());
        }
        self
    }

    pub fn with_unique_item(mut self) -> Self {
        self.unique = true;
        self
    }

    pub fn with_sort_item(mut self) -> Self
    where
        V: Ord,
    {
        self.sort_item = Some(Box::new(|a, b| a.cmp(b)));
        self
    }

    #[allow(unused)] // XXX Should be removed once terminus is in its own crate
    pub fn with_sort_item_by<F>(mut self, compare: F) -> Self
    where
        F: FnMut(&V, &V) -> cmp::Ordering + 'static,
    {
        self.sort_item = Some(Box::new(compare));
        self
    }

    #[allow(unused)] // XXX Should be removed once terminus is in its own crate
    pub fn with_sort_group(mut self) -> Self
    where
        G: Ord,
    {
        self.sort_group = Some(Box::new(|a, b| a.cmp(b)));
        self
    }

    #[allow(unused)] // XXX Should be removed once terminus is in its own crate
    pub fn with_sort_group_by<F>(mut self, compare: F) -> Self
    where
        F: FnMut(&G, &G) -> cmp::Ordering + 'static,
    {
        self.sort_group = Some(Box::new(compare));
        self
    }

    #[allow(unused)] // XXX Should be removed once terminus is in its own crate
    pub fn add_group(&mut self, group: G) {
        if let Entry::Vacant(vacant) = self.items.entry(Some(group)) {
            vacant.insert(HashSet::new());
        }

        self.dirty = true;
    }

    pub fn insert(&mut self, item: V, group: Option<G>) {
        if self.unique {
            for (_, items) in self.items.iter_mut() {
                items.remove(&item);
            }
        }
        match self.items.entry(group) {
            Entry::Vacant(vacant) => {
                let mut items = HashSet::new();
                items.insert(item);
                vacant.insert(items);
            }
            Entry::Occupied(mut occupied) => {
                occupied.get_mut().replace(item);
            }
        }

        self.dirty = true;
    }

    pub fn remove(&mut self, item: V, group: Option<G>) -> Result<(), ()> {
        match self.items.entry(group) {
            Entry::Vacant(_) => Err(()),
            Entry::Occupied(mut occupied) => {
                self.dirty |= occupied.get_mut().remove(&item);
                Ok(())
            }
        }
    }
}

impl<E, W, G, V> View<E, W> for ListView<E, W, G, V>
where
    W: Write + AsFd,
    G: fmt::Display + Hash + Eq,
    V: fmt::Display + Hash + Eq,
{
    fn measure(
        &mut self,
        dimension: &mut Dimension,
        width_spec: Option<u16>,
        height_spec: Option<u16>,
    ) {
        let layouts = self.get_layouts();
        dimension.w = match layouts.width.behavior {
            LayoutBehavior::MatchParent => width_spec,
            LayoutBehavior::WrapContent(_) => {
                let mut width: u16 = 0;
                for (group, items) in &self.items {
                    if let Some(group) = group {
                        width =
                            cmp::max(width, term_string_visible_len(&format!("{group}")) as u16);
                    }

                    let indent = match group {
                        Some(_) => "  ",
                        None => "",
                    };

                    for item in items {
                        width = cmp::max(
                            width,
                            term_string_visible_len(&format!("{indent}{item}")) as u16,
                        );
                    }
                }

                match width_spec {
                    Some(width_spec) => Some(cmp::min(width_spec, width)),
                    None => Some(width),
                }
            }
            LayoutBehavior::Absolute(width) => match width_spec {
                Some(width_spec) => Some(cmp::min(width, width_spec)),
                None => Some(width),
            },
        };

        dimension.h = match layouts.height.behavior {
            LayoutBehavior::MatchParent => height_spec,
            LayoutBehavior::WrapContent(_) => {
                let mut height: u16 = 0;
                for (group, items) in &self.items {
                    if group.is_some() {
                        height += 1;
                    }

                    height += items.len() as u16;
                }

                match height_spec {
                    Some(height_spec) => Some(cmp::min(height_spec, height)),
                    None => Some(height),
                }
            }
            LayoutBehavior::Absolute(height) => match height_spec {
                Some(height_spec) => Some(cmp::min(height, height_spec)),
                None => Some(height),
            },
        };
    }

    fn render(&mut self, dimension: &Dimension, screen: &mut Screen<W>) {
        save_cursor!(screen);

        let mut y = dimension.y;
        let width: usize = dimension.w.unwrap().into();

        for y in dimension.y..dimension.y + dimension.h.unwrap() {
            goto!(screen, dimension.x, y);
            for _ in dimension.x..dimension.x + dimension.w.unwrap() {
                vprint!(screen, " ");
            }

            goto!(screen, dimension.x, y);
        }

        for (group, items) in &self.items {
            if y > dimension.y + dimension.h.unwrap() {
                break;
            }

            goto!(screen, dimension.x, y);

            if group.is_some() {
                let mut disp = format!("{}", group.as_ref().unwrap());
                if term_string_visible_len(&disp) > width {
                    disp = term_string_visible_truncate(&disp, width, Some("…"));
                }
                vprint!(screen, "{}", disp);
                y += 1;
            }

            let mut items = items.iter().collect::<Vec<&V>>();
            if let Some(sort) = &mut self.sort_item {
                items.sort_by(|a, b| sort(*a, *b));
            }

            for item in items {
                if y > dimension.y + dimension.h.unwrap() {
                    break;
                }

                goto!(screen, dimension.x, y);

                let mut disp = match group {
                    Some(_) => format!("  {item}"),
                    None => format!("{item}"),
                };
                if term_string_visible_len(&disp) > width {
                    disp = term_string_visible_truncate(&disp, width, Some("…"));
                }
                vprint!(screen, "{}", disp);

                y += 1;
            }
        }

        restore_cursor!(screen);

        self.dirty = false;
    }

    fn event(&mut self, event: &mut E) {
        if let Some(handler) = &self.event_handler {
            let handler = Rc::clone(handler);
            let handler = &mut *handler.borrow_mut();
            handler(self, event);
        }
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn get_layouts(&self) -> Layouts {
        self.layouts.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::predicate::*;
    use mockall::*;

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

    mock! {
        Writer {
        }

        impl Write for Writer {
            fn write(&mut self, buf: &[u8]) -> std::result::Result<usize, std::io::Error> {
            }
            fn flush(&mut self) -> std::result::Result<(), std::io::Error> {
            }
        }
    }

    #[test]
    fn test_input_backspace() {
        // Given
        let mut input = Input::<()>::new();

        // When
        input.key('a');
        input.key('b');
        input.key('c');
        input.backspace();

        // Then
        assert_eq!(input.buf, "ab".to_string());
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
