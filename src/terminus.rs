use std::cell::RefCell;
use std::cmp;
use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::io::{Write, Stdout};
use std::rc::Rc;
use termion::cursor::DetectCursorPos;
use termion::raw::{IntoRawMode, RawTerminal};
use termion::screen::AlternateScreen;

type Screen = AlternateScreen<RawTerminal<Stdout>>;

#[derive(Clone)]
pub enum Dimension {
    MatchParent,
    WrapContent,
    Absolute(u16),
}

pub trait ViewTrait<E> {
    fn measure(&mut self, width_spec: Option<u16>, height_spec: Option<u16>);
    fn layout(&mut self, top: u16, left: u16);
    fn get_measured_width(&self) -> Option<u16>;
    fn get_measured_height(&self) -> Option<u16>;
    fn redraw(&mut self);
    fn event(&mut self, event: &mut E);
}

pub struct View<'a, T, E> {
    pub screen: Rc<RefCell<Screen>>,
    pub width: Dimension,
    pub height: Dimension,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub content: T,
    pub event_handler: Option<Rc<RefCell<Box<dyn FnMut(&mut Self, &mut E) + 'a>>>>,
}

default impl<'a, T, E> ViewTrait<E> for View<'a, T, E> {
    fn measure(&mut self, width_spec: Option<u16>, height_spec: Option<u16>) {
        self.w = match self.width {
            Dimension::MatchParent => {
                match width_spec {
                    Some(width_spec) => width_spec,
                    None => 0,
                }
            },
            Dimension::WrapContent => unreachable!(),
            Dimension::Absolute(width) => {
                match width_spec {
                    Some(width_spec) => cmp::min(width, width_spec),
                    None => width,
                }
            }
        };

        self.h = match self.height {
            Dimension::MatchParent => {
                match height_spec {
                    Some(height_spec) => height_spec,
                    None => 0,
                }
            },
            Dimension::WrapContent => unreachable!(),
            Dimension::Absolute(height) => {
                match height_spec {
                    Some(height_spec) => cmp::min(height, height_spec),
                    None => height,
                }
            },
        };
    }

    fn layout(&mut self, top: u16, left: u16) {
        self.x = left;
        self.y = top;
    }

    fn get_measured_width(&self) -> Option<u16> {
        if self.w > 0 {
            Some(self.w)
        } else {
            None
        }
    }

    fn get_measured_height(&self) -> Option<u16> {
        if self.h > 0 {
            Some(self.h)
        } else {
            None
        }
    }

    fn event(&mut self, event: &mut E) {
        if let Some(handler) = &self.event_handler {
            let handler = Rc::clone(handler);
            let handler = &mut *handler.borrow_mut();
            handler(self, event);
        }
    }
}

pub struct FrameLayout<'a, K, E>
    where K: Hash + Eq
{
    pub children: HashMap<K, Box<dyn ViewTrait<E> + 'a>>,
    pub current: Option<K>,
}

impl<'a, K, E> View<'a, FrameLayout<'a, K, E>, E>
    where K: Hash + Eq
{
    pub fn new(screen: Rc<RefCell<Screen>>) -> Self {
        Self {
            screen: screen,
            width: Dimension::MatchParent,
            height: Dimension::MatchParent,
            x: 1,
            y: 1,
            w: 0,
            h: 0,
            content: FrameLayout {
                children: HashMap::new(),
                current: None,
            },
            event_handler: None,
        }
    }

    pub fn with_event<F>(mut self, event_handler: F) -> Self
        where F: FnMut(&mut Self, &mut E), F: 'a
    {
        self.event_handler = Some(Rc::new(RefCell::new(Box::new(event_handler))));
        self
    }

    pub fn current(&mut self, key: K) {
        self.content.current = Some(key);
        self.redraw();
    }

    pub fn insert<T>(&mut self, key: K, mut widget: T)
        where T: ViewTrait<E>, T: 'a
    {
        widget.measure(Some(self.w), Some(self.h));
        widget.layout(self.y, self.x);
        self.content.children.insert(key, Box::new(widget));
    }
}

impl<K, E> ViewTrait<E> for View<'_, FrameLayout<'_, K, E>, E>
    where K: Hash + Eq
{
    fn measure(&mut self, width_spec: Option<u16>, height_spec: Option<u16>) {
        self.w = width_spec.unwrap_or(0);
        self.h = height_spec.unwrap_or(0);

        for (_, child) in self.content.children.iter_mut() {
            child.measure(Some(self.w), Some(self.h));
        }
    }

    fn layout(&mut self, top: u16, left: u16) {
        self.x = left;
        self.y = top;

        for (_, child) in self.content.children.iter_mut() {
            child.layout(top, left);
        }
    }

    fn redraw(&mut self) {
        if let Some(current) = &self.content.current {
            let child = self.content.children.get_mut(current).unwrap();
            child.redraw();
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum Orientation {
    Vertical,
    Horizontal,
}

pub struct LinearLayout<'a, E> {
    pub orientation: Orientation,
    pub children: Vec<Box<dyn ViewTrait<E> + 'a>>,
}

impl<'a, E> View<'a, LinearLayout<'a, E>, E> {
    pub fn new(screen: Rc<RefCell<Screen>>, orientation: Orientation, width: Dimension, height: Dimension) -> Self {
        Self {
            screen: screen,
            width: width,
            height: height,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            content: LinearLayout {
                orientation: orientation,
                children: Vec::new(),
            },
            event_handler: None,
        }
    }

    pub fn push<T>(&mut self, widget: T)
        where T: ViewTrait<E>, T: 'a
    {
        self.content.children.push(Box::new(widget));
    }
}

impl<E> ViewTrait<E> for View<'_, LinearLayout<'_, E>, E> {
    fn measure(&mut self, width_spec: Option<u16>, height_spec: Option<u16>) {
        let max_width = match self.width {
            Dimension::MatchParent => width_spec,
            Dimension::WrapContent => width_spec,
            Dimension::Absolute(width) => {
                match width_spec {
                    Some(width_spec) => Some(cmp::min(width, width_spec)),
                    None => Some(width),
                }
            },
        };

        let max_height = match self.height {
            Dimension::MatchParent => height_spec,
            Dimension::WrapContent => height_spec,
            Dimension::Absolute(height) => {
                match height_spec {
                    Some(height_spec) => Some(cmp::min(height, height_spec)),
                    None => Some(height),
                }
            },
        };

        let mut min_width = 0;
        let mut min_height = 0;
        for child in self.content.children.iter_mut() {
            child.measure(None, None);
            match self.content.orientation {
                Orientation::Vertical => {
                    min_width = cmp::max(min_width, child.get_measured_width().unwrap_or(0));
                    min_height += child.get_measured_height().unwrap_or(0);
                },
                Orientation::Horizontal => {
                    min_width += child.get_measured_height().unwrap_or(0);
                    min_height = cmp::max(min_height, child.get_measured_height().unwrap_or(0));
                },
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
        let splitted_width = match self.content.orientation {
            Orientation::Vertical => max_width,
            Orientation::Horizontal => {
                let unsized_children = self.content.children.iter().filter(|child| child.get_measured_width().is_none());
                Some(match unsized_children.collect::<Vec<_>>().len() {
                    0 => 0,
                    count => remaining_width / count as u16,
                })
            },
        };
        let splitted_height = match self.content.orientation {
            Orientation::Vertical => {
                let unsized_children = self.content.children.iter().filter(|child| child.get_measured_height().is_none());
                Some(match unsized_children.collect::<Vec<_>>().len() {
                    0 => 0,
                    count => remaining_height / count as u16,
                })
            },
            Orientation::Horizontal => max_height,
        };

        self.w = 0;
        self.h = 0;

        for child in self.content.children.iter_mut() {
            let mut width_spec = match child.get_measured_width() {
                Some(w) => Some(w),
                None => splitted_width,
            };

            let mut height_spec = match child.get_measured_height() {
                Some(h) => Some(h),
                None => splitted_height,
            };

            if self.content.orientation == Orientation::Horizontal && max_width.is_some() {
               width_spec = Some(cmp::min(width_spec.unwrap(), max_width.unwrap() - self.w));
            }

            if self.content.orientation == Orientation::Vertical && max_height.is_some() {
                height_spec = Some(cmp::min(height_spec.unwrap(), max_height.unwrap() - self.h));
            }

            child.measure(width_spec, height_spec);

            match self.content.orientation {
                Orientation::Vertical => {
                    self.w = cmp::max(self.w, child.get_measured_width().unwrap());
                    self.h += child.get_measured_height().unwrap();
                },
                Orientation::Horizontal => {
                    self.w += child.get_measured_width().unwrap();
                    self.h = cmp::max(self.w, child.get_measured_height().unwrap());
                },
            }
        }
    }

    fn layout(&mut self, top: u16, left: u16) {
        self.x = left;
        self.y = top;

        let mut x = self.x;
        let mut y = self.y;

        for child in self.content.children.iter_mut() {
            child.layout(y, x);
            match self.content.orientation {
                Orientation::Vertical => y += child.get_measured_height().unwrap(),
                Orientation::Horizontal => x += child.get_measured_width().unwrap(),
            }
        }
    }

    fn redraw(&mut self) {
        for child in self.content.children.iter_mut() {
            child.redraw();
        }
    }

    fn event(&mut self, event: &mut E) {
        for child in self.content.children.iter_mut() {
            child.event(event);
        }
    }
}

pub struct Input {
    pub buf: String,
    pub tmp_buf: Option<String>,
    pub password: bool,
    pub history: Vec<String>,
    pub history_index: usize,
}

impl<'a, E> View<'a, Input, E> {
    pub fn new(screen: Rc<RefCell<Screen>>) -> Self {
        Self {
            screen: screen,
            width: Dimension::MatchParent,
            height: Dimension::Absolute(1),
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            content: Input {
                buf: String::new(),
                tmp_buf: None,
                password: false,
                history: Vec::new(),
                history_index: 0,
            },
            event_handler: None,
        }
    }

    pub fn with_event<F>(mut self, event_handler: F) -> Self
        where F: FnMut(&mut Self, &mut E), F: 'a
    {
        self.event_handler = Some(Rc::new(RefCell::new(Box::new(event_handler))));
        self
    }

    pub fn key(&mut self, c: char) {
        let mut screen = self.screen.borrow_mut();
        self.content.buf.push(c);
        if !self.content.password {
            write!(screen, "{}", c).unwrap();
            screen.flush().unwrap();
        }
    }

    pub fn delete(&mut self) {
        let mut screen = self.screen.borrow_mut();
        self.content.buf.pop();
        if !self.content.password {
            write!(screen, "{} {}", termion::cursor::Left(1), termion::cursor::Left(1)).unwrap();
            screen.flush().unwrap();
        }
    }

    pub fn clear(&mut self) {
        let mut screen = self.screen.borrow_mut();
        self.content.buf.clear();
        let _ = self.content.tmp_buf.take();
        self.content.password = false;
        write!(screen, "{}", termion::cursor::Goto(self.x, self.y)).unwrap();
        for _ in 0 .. self.w {
            write!(screen, " ").unwrap();
        }
        write!(screen, "{}", termion::cursor::Goto(self.x, self.y)).unwrap();
        screen.flush().unwrap();
    }

    pub fn left(&mut self) {
        if !self.content.password {
            let mut screen = self.screen.borrow_mut();
            write!(screen, "{}", termion::cursor::Left(1)).unwrap();
            screen.flush().unwrap();
        }
    }

    pub fn right(&mut self) {
        if !self.content.password {
            let mut screen = self.screen.borrow_mut();
            let (x, _y) = screen.cursor_pos().unwrap();
            if x as usize <= self.content.buf.len() {
                write!(screen, "{}", termion::cursor::Right(1)).unwrap();
                screen.flush().unwrap();
            }
        }
    }

    pub fn password(&mut self) {
        self.clear();
        self.content.password = true;
        let mut screen = self.screen.borrow_mut();
        write!(screen, "password: ").unwrap();
        screen.flush().unwrap();
    }

    pub fn validate(&mut self) {
        if !self.content.password {
            self.content.history.push(self.content.buf.clone());
            self.content.history_index = self.content.history.len();
        }
        self.clear();
    }

    pub fn previous(&mut self) {
        if self.content.history_index == 0 {
            return;
        }

        if self.content.tmp_buf.is_none() {
            self.content.tmp_buf = Some(self.content.buf.clone());
        }

        self.content.history_index -= 1;
        self.content.buf = self.content.history[self.content.history_index].clone();
        self.redraw();
    }

    pub fn next(&mut self) {
        if self.content.history_index == self.content.history.len() {
            return;
        }

        self.content.history_index += 1;
        if self.content.history_index == self.content.history.len() {
            self.content.buf = self.content.tmp_buf.take().unwrap();
        } else {
            self.content.buf = self.content.history[self.content.history_index].clone();
        }

        self.redraw();
    }
}

impl<E> ViewTrait<E> for View<'_, Input, E> {
    fn redraw(&mut self) {
        let mut screen = self.screen.borrow_mut();

        write!(screen, "{}", termion::cursor::Goto(self.x, self.y)).unwrap();
        for _ in 0 .. self.w {
            write!(screen, " ").unwrap();
        }
        write!(screen, "{}", termion::cursor::Goto(self.x, self.y)).unwrap();
        write!(screen, "{}", self.content.buf).unwrap();

        screen.flush().unwrap();
    }
}

pub trait BufferedMessage = fmt::Display + Hash + std::cmp::Eq + std::clone::Clone;

pub trait Window<T: BufferedMessage, E>: ViewTrait<E> {
    fn recv_message(&mut self, message: &T, print: bool);
    fn send_message(&self);
    fn page_up(&mut self);
    fn page_down(&mut self);
}

pub struct BufferedWin<T: BufferedMessage> {
    pub next_line: u16,
    pub buf: Vec<T>,
    pub history: HashMap<T, usize>,
    pub view: usize,
}

impl<'a, T: BufferedMessage, E> View<'a, BufferedWin<T>, E> {
    pub fn new(screen: Rc<RefCell<Screen>>) -> Self {
        Self {
            screen: screen,
            width: Dimension::MatchParent,
            height: Dimension::MatchParent,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            content: BufferedWin {
                next_line: 0,
                buf: Vec::new(),
                history: HashMap::new(),
                view: 0,
            },
            event_handler: None,
        }
    }

    pub fn with_event<F>(mut self, event_handler: F) -> Self
        where F: FnMut(&mut Self, &mut E), F: 'a
    {
        self.event_handler = Some(Rc::new(RefCell::new(Box::new(event_handler))));
        self
    }
}

impl<T: BufferedMessage, E> Window<T, E> for View<'_, BufferedWin<T>, E> {
    fn recv_message(&mut self, message: &T, print: bool) {
        if self.content.history.contains_key(message) {
            return;
        }

        self.content.history.insert(message.clone(), self.content.buf.len());
        self.content.buf.push(message.clone());

        if print {
            self.redraw();
        }
    }

    fn page_up(&mut self) {
        let buffers = self.content.buf.iter().flat_map(|m| format!("{}", m).lines().map(str::to_owned).collect::<Vec<_>>());
        let count = buffers.collect::<Vec<_>>().len();

        if count < self.h as usize {
            return;
        }

        let max = count - self.h as usize;

        if self.content.view + (self.h as usize) < max {
            self.content.view += self.h as usize;
        } else {
            self.content.view = max;
        }

        self.redraw();
    }

    fn page_down(&mut self) {
        if self.content.view > self.h as usize {
            self.content.view -= self.h as usize;
        } else {
            self.content.view = 0;
        }
        self.redraw();
    }

    fn send_message(&self) {
    }
}

impl<T: BufferedMessage, E> ViewTrait<E> for View<'_, BufferedWin<T>, E> {
    fn redraw(&mut self) {
        let mut screen = self.screen.borrow_mut();

        write!(screen, "{}", termion::cursor::Save).unwrap();

        self.content.next_line = 0;
        let buffers = self.content.buf.iter().flat_map(|m| format!("{}", m).lines().map(str::to_owned).collect::<Vec<_>>());
        let count = buffers.collect::<Vec<_>>().len();

        let mut buffers = self.content.buf.iter().flat_map(|m| format!("{}", m).lines().map(str::to_owned).collect::<Vec<_>>());

        if count > self.h as usize {
            for _ in 0 .. count - self.h as usize - self.content.view {
                if buffers.next().is_none() {
                    break;
                }
            }
        }

        for y in self.y .. self.y + self.h {
            write!(screen, "{}", termion::cursor::Goto(self.x, y)).unwrap();

            for _ in self.x  .. self.x + self.w {
                write!(screen, " ").unwrap();
            }

            write!(screen, "{}", termion::cursor::Goto(self.x, y)).unwrap();

            if let Some(buf) = buffers.next() {
                write!(screen, "{}", buf).unwrap();
                self.content.next_line += 1;
            }
            screen.flush().unwrap();
        }

        write!(screen, "{}", termion::cursor::Restore).unwrap();

        screen.flush().unwrap();
    }
}
