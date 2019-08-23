use bytes::BytesMut;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::hash;
use std::io::{Error as IoError, ErrorKind};
use std::io::{Write, Stdout};
use std::rc::Rc;
use termion::color;
use termion::cursor::DetectCursorPos;
use termion::raw::{IntoRawMode, RawTerminal};
use termion::screen::AlternateScreen;
use tokio::codec::FramedRead;
use tokio_codec::{Decoder};
use uuid::Uuid;
use xmpp_parsers::{BareJid, Jid};
use chrono::offset::{TimeZone, Local};
use chrono::Utc;

use crate::core::{Plugin, Aparte, Event, Message, XmppMessage, Command, CommandOrMessage, CommandError};

pub type CommandStream = FramedRead<tokio::reactor::PollEvented2<tokio_file_unix::File<std::fs::File>>, KeyCodec>;
type Screen = AlternateScreen<RawTerminal<Stdout>>;

#[derive(Clone)]
enum VerticalPosition {
    Top(u16),
    Bottom(u16),
}

#[derive(Clone)]
enum HorizontalPosition {
    Left(u16),
    #[allow(dead_code)]
    Right(u16),
}

#[derive(Clone)]
enum Dimension {
    Relative(f32, i32),
    #[allow(dead_code)]
    Absolute(u16),
}

#[derive(Clone)]
struct Widget {
    screen: Rc<RefCell<Screen>>,
    vpos: VerticalPosition,
    hpos: HorizontalPosition,
    width: Dimension,
    height: Dimension,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
}

impl Widget {
    fn redraw(&mut self) {
        let (width, height) = termion::terminal_size().unwrap();

        self.x = match self.hpos {
            HorizontalPosition::Left(offset) => 1 + offset,
            HorizontalPosition::Right(offset) => width - offset,
        };

        self.y = match self.vpos {
            VerticalPosition::Top(offset) => 1 + offset,
            VerticalPosition::Bottom(offset) => height - offset,
        };

        self.w = match self.width {
            Dimension::Relative(r, offset) => ((r * width as f32) as i32 + offset) as u16,
            Dimension::Absolute(w) => w,
        };

        self.h = match self.height {
            Dimension::Relative(r, offset) => ((r * height as f32) as i32 + offset) as u16,
            Dimension::Absolute(h) => h,
        };

    }
}

struct Input {
    widget: Widget,
    buf: String,
    tmp_buf: Option<String>,
    password: bool,
    history: Vec<String>,
    history_index: usize,
}

impl Input {
    fn new(screen: Rc<RefCell<Screen>>) -> Self {
        let mut widget = Widget {
            screen: screen,
            vpos: VerticalPosition::Bottom(0),
            hpos: HorizontalPosition::Left(0),
            width: Dimension::Relative(1., 0),
            height: Dimension::Absolute(1),
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        };

        widget.redraw();

        Self {
            widget: widget,
            buf: String::new(),
            tmp_buf: None,
            password: false,
            history: Vec::new(),
            history_index: 0,
        }
    }

    fn key(&mut self, c: char) {
        let mut screen = self.widget.screen.borrow_mut();
        self.buf.push(c);
        if !self.password {
            write!(screen, "{}", c).unwrap();
            screen.flush().unwrap();
        }
    }

    fn delete(&mut self) {
        let mut screen = self.widget.screen.borrow_mut();
        self.buf.pop();
        if !self.password {
            write!(screen, "{} {}", termion::cursor::Left(1), termion::cursor::Left(1)).unwrap();
            screen.flush().unwrap();
        }
    }

    fn clear(&mut self) {
        let mut screen = self.widget.screen.borrow_mut();
        self.buf.clear();
        let _ = self.tmp_buf.take();
        self.password = false;
        write!(screen, "{}", termion::cursor::Goto(self.widget.x, self.widget.y)).unwrap();
        for _ in 0 .. self.widget.w {
            write!(screen, " ").unwrap();
        }
        write!(screen, "{}", termion::cursor::Goto(self.widget.x, self.widget.y)).unwrap();
        screen.flush().unwrap();
    }

    fn left(&mut self) {
        if !self.password {
            let mut screen = self.widget.screen.borrow_mut();
            write!(screen, "{}", termion::cursor::Left(1)).unwrap();
            screen.flush().unwrap();
        }
    }

    fn right(&mut self) {
        if !self.password {
            let mut screen = self.widget.screen.borrow_mut();
            let (x, _y) = screen.cursor_pos().unwrap();
            if x as usize <= self.buf.len() {
                write!(screen, "{}", termion::cursor::Right(1)).unwrap();
                screen.flush().unwrap();
            }
        }
    }

    fn password(&mut self) {
        self.clear();
        self.password = true;
        let mut screen = self.widget.screen.borrow_mut();
        write!(screen, "password: ").unwrap();
        screen.flush().unwrap();
    }

    fn validate(&mut self) {
        if !self.password {
            self.history.push(self.buf.clone());
            self.history_index = self.history.len();
        }
        self.clear();
    }

    fn previous(&mut self) {
        if self.history_index == 0 {
            return;
        }

        if self.tmp_buf.is_none() {
            self.tmp_buf = Some(self.buf.clone());
        }

        self.history_index -= 1;
        self.buf = self.history[self.history_index].clone();
        self.redraw();
    }

    fn next(&mut self) {
        if self.history_index == self.history.len() {
            return;
        }

        self.history_index += 1;
        if self.history_index == self.history.len() {
            self.buf = self.tmp_buf.take().unwrap();
        } else {
            self.buf = self.history[self.history_index].clone();
        }

        self.redraw();
    }

    fn redraw(&mut self) {
        self.widget.redraw();
        let mut screen = self.widget.screen.borrow_mut();

        write!(screen, "{}", termion::cursor::Goto(self.widget.x, self.widget.y)).unwrap();
        for _ in 0 .. self.widget.w {
            write!(screen, " ").unwrap();
        }
        write!(screen, "{}", termion::cursor::Goto(self.widget.x, self.widget.y)).unwrap();
        write!(screen, "{}", self.buf).unwrap();

        screen.flush().unwrap();
    }
}

struct TitleBar {
    widget: Widget,
    window_name: Option<String>,
}

impl TitleBar {
    fn new(screen: Rc<RefCell<Screen>>) -> Self {
        let mut widget = Widget {
            screen: screen,
            vpos: VerticalPosition::Top(0),
            hpos: HorizontalPosition::Left(0),
            width: Dimension::Relative(1., 0),
            height: Dimension::Absolute(1),
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        };

        widget.redraw();

        Self {
            widget: widget,
            window_name: None,
        }
    }

    fn redraw(&mut self) {
        self.widget.redraw();
        let mut screen = self.widget.screen.borrow_mut();

        write!(screen, "{}", termion::cursor::Save).unwrap();
        write!(screen, "{}", termion::cursor::Goto(self.widget.x, self.widget.y)).unwrap();
        write!(screen, "{}{}", color::Bg(color::Blue), color::Fg(color::White)).unwrap();

        for _ in 0 .. self.widget.w {
            write!(screen, " ").unwrap();
        }
        write!(screen, "{}", termion::cursor::Goto(self.widget.x, self.widget.y)).unwrap();
        if let Some(window_name) = &self.window_name {
            write!(screen, " {}", window_name).unwrap();
        }

        write!(screen, "{}{}", color::Bg(color::Reset), color::Fg(color::Reset)).unwrap();
        write!(screen, "{}", termion::cursor::Restore).unwrap();
        screen.flush().unwrap();
    }

    fn set_name(&mut self, name: &str) {
        self.window_name = Some(name.to_string());
        self.redraw();
    }
}

struct WinBar {
    widget: Widget,
    connection: Option<String>,
    windows: Vec<String>,
    current_window: Option<String>,
    highlighted: Vec<String>,
}

impl WinBar {
    fn new(screen: Rc<RefCell<Screen>>) -> Self {
        let mut widget = Widget {
            screen: screen,
            vpos: VerticalPosition::Bottom(1),
            hpos: HorizontalPosition::Left(0),
            width: Dimension::Relative(1., 0),
            height: Dimension::Absolute(1),
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        };

        widget.redraw();

        Self {
            widget: widget,
            connection: None,
            windows: Vec::new(),
            current_window: None,
            highlighted: Vec::new(),
        }
    }

    fn add_window(&mut self, window: &str) {
        self.windows.push(window.to_string());
        self.redraw();
    }

    fn set_current_window(&mut self, window: &str) {
        self.current_window = Some(window.to_string());
        self.highlighted.drain_filter(|w| w == &window);
        self.redraw();
    }

    fn highlight_window(&mut self, window: &str) {
        if self.highlighted.iter().find(|w| w == &window).is_none() {
            self.highlighted.push(window.to_string());
            self.redraw();
        }
    }

    fn redraw(&mut self) {
        self.widget.redraw();
        let mut screen = self.widget.screen.borrow_mut();

        write!(screen, "{}", termion::cursor::Save).unwrap();
        write!(screen, "{}", termion::cursor::Goto(self.widget.x, self.widget.y)).unwrap();
        write!(screen, "{}{}", color::Bg(color::Blue), color::Fg(color::White)).unwrap();

        for _ in 0 .. self.widget.w {
            write!(screen, " ").unwrap();
        }

        write!(screen, "{}", termion::cursor::Goto(self.widget.x, self.widget.y)).unwrap();
        if let Some(connection) = &self.connection {
            write!(screen, " {}", connection).unwrap();
        }

        let mut windows = String::new();
        let mut windows_len = 0;

        let mut index = 1;
        for window in &self.windows {
            if let Some(current) = &self.current_window {
                if window == current {
                    let win = format!("-{}: {}- ", index, window);
                    windows_len += win.len();
                    windows.push_str(&win);
                } else {
                    if self.highlighted.iter().find(|w| w == &window).is_some() {
                        windows.push_str(&format!("{}", termion::style::Bold));
                    }
                    let win = format!("[{}: {}] ", index, window);
                    windows_len += win.len();
                    windows.push_str(&win);
                    windows.push_str(&format!("{}", termion::style::NoBold));
                }
            }
            index += 1;
        }

        let start = self.widget.x + self.widget.w - windows_len as u16;
        write!(screen, "{}{}", termion::cursor::Goto(start, self.widget.y), windows).unwrap();

        write!(screen, "{}{}", color::Bg(color::Reset), color::Fg(color::Reset)).unwrap();
        write!(screen, "{}", termion::cursor::Restore).unwrap();
        screen.flush().unwrap();
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Message::Log(message) => {
                let timestamp = Local.from_utc_datetime(&message.timestamp.naive_local());
                write!(f, "{} - {}", timestamp.format("%T"), message.body)
            },
            Message::Incoming(XmppMessage::Chat(message)) => {
                let timestamp = Local.from_utc_datetime(&message.timestamp.naive_local());
                let padding_len = format!("{} - {}: ", timestamp.format("%T"), message.from).len();
                let padding = " ".repeat(padding_len);

                write!(f, "{} - {}{}:{} ", timestamp.format("%T"), color::Fg(color::Green), message.from, color::Fg(color::White))?;

                let mut iter = message.body.lines();
                if let Some(line) = iter.next() {
                    write!(f, "{}", line)?;
                }
                while let Some(line) = iter.next() {
                    write!(f, "\n{}{}", padding, line)?;
                }

                Ok(())
            },
            Message::Outgoing(XmppMessage::Chat(message)) => {
                let timestamp = Local.from_utc_datetime(&message.timestamp.naive_local());
                write!(f, "{} - {}me:{} {}", timestamp.format("%T"), color::Fg(color::Yellow), color::Fg(color::White), message.body)
            }
            Message::Incoming(XmppMessage::Groupchat(message)) => {
                if let Jid::Full(from) = &message.from_full {
                    let timestamp = Local.from_utc_datetime(&message.timestamp.naive_local());
                    let padding_len = format!("{} - {}: ", timestamp.format("%T"), from.resource).len();
                    let padding = " ".repeat(padding_len);

                    write!(f, "{} - {}{}:{} ", timestamp.format("%T"), color::Fg(color::Green), from.resource, color::Fg(color::White))?;

                    let mut iter = message.body.lines();
                    if let Some(line) = iter.next() {
                        write!(f, "{}", line)?;
                    }
                    while let Some(line) = iter.next() {
                        write!(f, "\n{}{}", padding, line)?;
                    }
                }
                Ok(())
            },
            Message::Outgoing(XmppMessage::Groupchat(message)) => {
                let timestamp = Local.from_utc_datetime(&message.timestamp.naive_local());
                write!(f, "{} - {}me:{} {}", timestamp.format("%T"), color::Fg(color::Yellow), color::Fg(color::White), message.body)
            }
        }
    }
}

trait BufferedMessage = fmt::Display + hash::Hash + std::cmp::Eq + std::clone::Clone;

struct BufferedWin<T: BufferedMessage> {
    widget: Widget,
    next_line: u16,
    buf: Vec<T>,
    history: HashMap<T, usize>,
}

impl<T: BufferedMessage> BufferedWin<T> {
    fn show(&mut self) {
        let mut screen = self.widget.screen.borrow_mut();

        write!(screen, "{}", termion::cursor::Save).unwrap();

        self.next_line = 0;
        let mut buffers = self.buf.iter().flat_map(|m| format!("{}", m).lines().map(str::to_owned).collect::<Vec<_>>());

        for y in self.widget.y .. self.widget.y + self.widget.h {
            write!(screen, "{}", termion::cursor::Goto(self.widget.x, y)).unwrap();

            for _ in self.widget.x  .. self.widget.x + self.widget.w {
                write!(screen, " ").unwrap();
            }

            write!(screen, "{}", termion::cursor::Goto(self.widget.x, y)).unwrap();

            if let Some(buf) = buffers.next() {
                write!(screen, "{}", buf).unwrap();
                self.next_line += 1;
            }
            screen.flush().unwrap();
        }

        write!(screen, "{}", termion::cursor::Restore).unwrap();

        screen.flush().unwrap();
    }

    fn message(&mut self, message: T, print: bool) {
        if self.history.contains_key(&message) {
            return;
        }

        self.history.insert(message.clone(), self.buf.len());
        self.buf.push(message.clone());

        if print {
            {
                let mut screen = self.widget.screen.borrow_mut();
                write!(screen, "{}", termion::cursor::Save).unwrap();
            }

            let buf = format!("{}", message);
            for line in buf.lines() {
                if self.next_line > self.widget.h {
                    self.scroll();
                }

                let mut screen = self.widget.screen.borrow_mut();

                let x = self.widget.x;
                let y = self.widget.y + self.next_line;

                write!(screen, "{}", termion::cursor::Goto(x, y)).unwrap();

                write!(screen, "{}", line).unwrap();

                self.next_line += 1;
            }

            let mut screen = self.widget.screen.borrow_mut();
            write!(screen, "{}", termion::cursor::Restore).unwrap();

            screen.flush().unwrap();
        }
    }

    fn scroll(&mut self) {
    }

    fn redraw(&mut self) {
        self.widget.redraw();
        let mut screen = self.widget.screen.borrow_mut();
        screen.flush().unwrap();
    }
}

pub struct ConsoleWin {
    bufwin: BufferedWin<Message>,
}

pub struct ChatWin {
    bufwin: BufferedWin<Message>,
    us: BareJid,
    them: BareJid,
}

pub struct GroupchatWin {
    bufwin: BufferedWin<Message>,
    us: BareJid,
    groupchat: BareJid,
}

pub enum Window {
    Console(ConsoleWin),
    Chat(ChatWin),
    Groupchat(GroupchatWin),
}

impl Window {
    fn bufwin<T: BufferedMessage>(screen: Rc<RefCell<Screen>>) -> BufferedWin<T> {
        let widget = Widget {
            screen: screen,
            vpos: VerticalPosition::Top(1),
            hpos: HorizontalPosition::Left(0),
            width: Dimension::Relative(1., 0),
            height: Dimension::Relative(1., -3),
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        };

        let mut bufwin = BufferedWin {
            widget: widget,
            next_line: 0,
            buf: Vec::new(),
            history: HashMap::new(),
        };

        bufwin.redraw();

        bufwin
    }
    fn chat(screen: Rc<RefCell<Screen>>, us: &BareJid, them: &BareJid) -> Self {
        let bufwin = Self::bufwin::<Message>(screen);

        Window::Chat(ChatWin {
            bufwin: bufwin,
            us: us.clone(),
            them: them.clone(),
        })
    }

    fn console(screen: Rc<RefCell<Screen>>) -> Self {
        let bufwin = Self::bufwin::<Message>(screen);

        Window::Console(ConsoleWin {
            bufwin: bufwin,
        })
    }

    fn groupchat(screen: Rc<RefCell<Screen>>, us: &BareJid, groupchat: &BareJid) -> Self {
        let bufwin = Self::bufwin::<Message>(screen);

        Window::Groupchat(GroupchatWin {
            bufwin: bufwin,
            us: us.clone(),
            groupchat: groupchat.clone(),
        })
    }

    fn redraw(&mut self) {
        match self {
            Window::Chat(chat) => chat.bufwin.redraw(),
            Window::Console(console) => console.bufwin.redraw(),
            Window::Groupchat(groupchat) => groupchat.bufwin.redraw(),
        }
    }

    fn show(&mut self) {
        match self {
            Window::Chat(chat) => chat.bufwin.show(),
            Window::Console(console) => console.bufwin.show(),
            Window::Groupchat(groupchat) => groupchat.bufwin.show(),
        }
    }

    fn message(&mut self, message: &Message, print: bool) {
        match self {
            Window::Chat(chat) => chat.bufwin.message(message.clone(), print),
            Window::Console(console) => console.bufwin.message(message.clone(), print),
            Window::Groupchat(groupchat) => groupchat.bufwin.message(message.clone(), print),
        }
    }

    #[allow(dead_code)]
    fn scroll(&mut self) {
        match self {
            Window::Chat(chat) => chat.bufwin.scroll(),
            Window::Console(console) => console.bufwin.scroll(),
            Window::Groupchat(groupchat) => groupchat.bufwin.scroll(),
        }
    }
}

pub struct UIPlugin {
    screen: Rc<RefCell<Screen>>,
    input: Input,
    title_bar: TitleBar,
    win_bar: WinBar,
    windows: HashMap<String, Window>,
    windows_index: Vec<String>,
    current: String,
    password_command: Option<Command>,
}

impl UIPlugin {
    pub fn command_stream(&self, aparte: Rc<Aparte>) -> CommandStream {
        let file = tokio_file_unix::raw_stdin().unwrap();
        let file = tokio_file_unix::File::new_nb(file).unwrap();
        let file = file.into_io(&tokio::reactor::Handle::default()).unwrap();

        FramedRead::new(file, KeyCodec::new(aparte))
    }

    pub fn current_window(&mut self) -> &mut Window {
        self.windows.get_mut(&self.current).unwrap()
    }

    pub fn switch(&mut self, chat: &str) -> Result<(), ()> {
        self.current = chat.to_string();
        if let Some(chat) = self.windows.get_mut(chat) {
            self.title_bar.set_name(&self.current);
            self.win_bar.set_current_window(&self.current);
            chat.show();
            return Ok(())
        } else {
            return Err(())
        }
    }

    fn add_window(&mut self, name: &str, window: Window) {
        self.windows.insert(name.to_string(), window);
        self.windows_index.push(name.to_string());
        self.win_bar.add_window(name);
    }

    pub fn next_window(&mut self) -> Result<(), ()> {
        let index = self.windows_index.iter().position(|name| name == &self.current).unwrap();
        if index + 1 < self.windows_index.len() {
            let name = self.windows_index[index + 1].clone();
            self.switch(&name)
        } else {
            Err(())
        }
    }

    pub fn prev_window(&mut self) -> Result<(), ()> {
        let index = self.windows_index.iter().position(|name| name == &self.current).unwrap();
        if index > 0 {
            let name = self.windows_index[index - 1].clone();
            self.switch(&name)
        } else {
            Err(())
        }
    }

    pub fn read_password(&mut self, command: Command) {
        self.password_command = Some(command);
        self.input.password();
    }
}

impl Plugin for UIPlugin {
    fn new() -> Self {
        let stdout = std::io::stdout().into_raw_mode().unwrap();
        let screen = Rc::new(RefCell::new(AlternateScreen::from(stdout)));
        let input = Input::new(screen.clone());
        let title_bar = TitleBar::new(screen.clone());
        let win_bar = WinBar::new(screen.clone());

        Self {
            screen: screen,
            input: input,
            title_bar: title_bar,
            win_bar: win_bar,
            windows: HashMap::new(),
            windows_index: Vec::new(),
            current: "console".to_string(),
            password_command: None,
        }
    }

    fn init(&mut self, _aparte: &Aparte) -> Result<(), ()> {
        {
            let mut screen = self.screen.borrow_mut();
            write!(screen, "{}", termion::clear::All).unwrap();
        }

        let console = Window::console(self.screen.clone());
        self.add_window("console", console);
        self.title_bar.set_name("console");

        self.input.redraw();
        self.title_bar.redraw();
        self.win_bar.redraw();
        self.switch("console").unwrap();

        Ok(())
    }

    fn on_event(&mut self, aparte: Rc<Aparte>, event: &Event) {
        match event {
            Event::Connected(_jid) => {
                self.win_bar.connection = match aparte.current_connection() {
                    Some(jid) => Some(jid.to_string()),
                    None => None,
                };
                self.win_bar.redraw();
            },
            Event::Message(message) => {
                let chat_name = match message {
                    Message::Incoming(XmppMessage::Chat(message)) => message.from.to_string(),
                    Message::Outgoing(XmppMessage::Chat(message)) => message.to.to_string(),
                    Message::Incoming(XmppMessage::Groupchat(message)) => message.from.to_string(),
                    Message::Outgoing(XmppMessage::Groupchat(message)) => message.to.to_string(),
                    Message::Log(_message) => "console".to_string(),
                };

                let chat = match self.windows.get_mut(&chat_name) {
                    Some(chat) => chat,
                    None => {
                        let mut chat = match message {
                            Message::Incoming(XmppMessage::Chat(message)) => Window::chat(self.screen.clone(), &message.to, &message.from),
                            Message::Outgoing(XmppMessage::Chat(message)) => Window::chat(self.screen.clone(), &message.from, &message.to),
                            Message::Incoming(XmppMessage::Groupchat(message)) => Window::groupchat(self.screen.clone(), &message.to, &message.from),
                            Message::Outgoing(XmppMessage::Groupchat(message)) => Window::groupchat(self.screen.clone(), &message.from, &message.to),
                            Message::Log(_) => unreachable!(),
                        };
                        chat.redraw();
                        self.add_window(&chat_name, chat);
                        self.windows.get_mut(&chat_name).unwrap()
                    },
                };

                chat.message(message, self.current == chat_name);
                if self.current != chat_name {
                    self.win_bar.highlight_window(&chat_name);
                }
            },
            Event::Join(jid) => {
                let groupchat: BareJid = jid.clone().into();
                let win_name = groupchat.to_string();
                if self.switch(&win_name).is_err() {
                    let us = aparte.current_connection().unwrap().clone().into();
                    let groupchat = jid.clone().into();
                    let chat = Window::groupchat(self.screen.clone(), &us, &groupchat);
                    self.add_window(&win_name, chat);
                    self.switch(&win_name).unwrap();
                }
            }
            _ => {},
        }
    }
}

impl fmt::Display for UIPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Aparté UI")
    }
}

pub struct KeyCodec {
    queue: Vec<Result<CommandOrMessage, CommandError>>,
    aparte: Rc<Aparte>,
}

impl KeyCodec {
    pub fn new(aparte: Rc<Aparte>) -> Self {
        Self {
            queue: Vec::new(),
            aparte: aparte,
        }
    }
}

impl Decoder for KeyCodec {
    type Item = CommandOrMessage;
    type Error = CommandError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let mut ui = self.aparte.get_plugin_mut::<UIPlugin>().unwrap();
        let copy = buf.clone();
        trace!("< {:?}", copy);
        let string = match std::str::from_utf8(&copy) {
            Ok(string) => {
                buf.clear();
                string
            },
            Err(err) => {
                let index = err.valid_up_to();
                buf.advance(index);
                std::str::from_utf8(&copy[..index]).unwrap()
            }
        };

        let mut chars = string.chars();
        while let Some(c) = chars.next() {
            if !c.is_control() {
                ui.input.key(c);
            } else {
                match c {
                    '\r' => {
                        if ui.input.password {
                            let mut command = ui.password_command.take().unwrap();
                            command.args.push(ui.input.buf.clone());
                            self.queue.push(Ok(CommandOrMessage::Command(command)));
                        } else if ui.input.buf.starts_with("/") {
                            let splitted = shell_words::split(&ui.input.buf);
                            match splitted {
                                Ok(splitted) => {
                                    let command = Command::new(splitted[0][1..].to_string(), splitted[1..].to_vec());
                                    self.queue.push(Ok(CommandOrMessage::Command(command)));
                                },
                                Err(err) => self.queue.push(Err(CommandError::Parse(err))),
                            }
                        } else {
                            match ui.current_window() {
                                Window::Chat(chat) => {
                                    let from: Jid = chat.us.clone().into();
                                    let to: Jid = chat.them.clone().into();
                                    let id = Uuid::new_v4();
                                    let timestamp = Utc::now();
                                    let message = Message::outgoing_chat(id.to_string(), timestamp, &from, &to, &ui.input.buf);
                                    self.queue.push(Ok(CommandOrMessage::Message(message)));
                                },
                                Window::Groupchat(groupchat) => {
                                    let from: Jid = groupchat.us.clone().into();
                                    let to: Jid = groupchat.groupchat.clone().into();
                                    let id = Uuid::new_v4();
                                    let timestamp = Utc::now();
                                    let message = Message::outgoing_groupchat(id.to_string(), timestamp, &from, &to, &ui.input.buf);
                                    self.queue.push(Ok(CommandOrMessage::Message(message)));
                                },
                                Window::Console(_) => { },
                            }
                        }
                        ui.input.validate();
                    },
                    '\x7f' => {
                        ui.input.delete();
                    },
                    '\x03' => self.queue.push(Err(CommandError::Io(IoError::new(ErrorKind::BrokenPipe, "ctrl+c")))),
                    '\x1b' => {
                        match chars.next() {
                            Some('[') => {
                                match chars.next() {
                                    Some('C') => {
                                        ui.input.right();
                                    },
                                    Some('D') => {
                                        ui.input.left();
                                    },
                                    Some('A') => {
                                        ui.input.previous();
                                    },
                                    Some('B') => {
                                        ui.input.next();
                                    },
                                    Some(_) => {}
                                    None => {},
                                }
                            },
                            Some('\x1b') => {
                                match chars.next() {
                                    Some('[') => {
                                        match chars.next() {
                                            Some('C') => {
                                                let _ = ui.next_window();
                                            }
                                            Some('D') => {
                                                let _ = ui.prev_window();
                                            }
                                            Some(_) => {}
                                            None => {},
                                        }
                                    },
                                    Some(_) => {}
                                    None => {},
                                }
                            }
                            Some(_) => {},
                            None => {},
                        }
                    },
                    _ => {
                        info!("Unknown control code {}", c);
                        for c in format!("^[{:x}", c as u8).chars() {
                            ui.input.key(c);
                        }
                    },
                }
            }
        }

        match self.queue.pop() {
            Some(Ok(command)) => Ok(Some(command)),
            Some(Err(err)) => Err(err),
            None => Ok(None),
        }
    }
}
