use bytes::BytesMut;
use std::cell::RefCell;
use std::clone::Clone;
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

use crate::core::{Plugin, Aparte, Message, Command, CommandOrMessage, CommandError};

pub type CommandStream = FramedRead<tokio::reactor::PollEvented2<tokio_file_unix::File<std::fs::File>>, KeyCodec>;
type Screen = AlternateScreen<RawTerminal<Stdout>>;

#[derive(Clone)]
enum VerticalPosition {
    Top,
    Bottom,
}

#[derive(Clone)]
enum HorizontalPosition {
    Left,
    #[allow(dead_code)]
    Right,
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
    voff: i32,
    hoff: i32,
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
            HorizontalPosition::Left => (1 + self.hoff) as u16,
            HorizontalPosition::Right => (width as i32 - self.hoff) as u16,
        };

        self.y = match self.vpos {
            VerticalPosition::Top => (1 + self.voff) as u16,
            VerticalPosition::Bottom => (height as i32 - self.voff) as u16,
        };

        self.w = match self.width {
            Dimension::Relative(r, offset) => ((r * width as f32) as i32 - self.hoff + offset) as u16,
            Dimension::Absolute(w) => w,
        };

        self.h = match self.height {
            Dimension::Relative(r, offset) => ((r * height as f32) as i32 - self.voff + offset) as u16,
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
            vpos: VerticalPosition::Bottom,
            hpos: HorizontalPosition::Left,
            voff: 0,
            hoff: 0,
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
            vpos: VerticalPosition::Top,
            hpos: HorizontalPosition::Left,
            voff: 0,
            hoff: 0,
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
        write!(screen, "{}", color::Bg(color::Reset)).unwrap();
        screen.flush().unwrap();

        write!(screen, "{}", termion::cursor::Restore).unwrap();
    }

    fn set_name(&mut self, name: &str) {
        self.window_name = Some(name.to_string());
        self.redraw();
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Message::Log(message) => {
                let timestamp = Local.from_utc_datetime(&message.timestamp.naive_local());
                write!(f, "{} - {}", timestamp.format("%T"), message.body)
            },
            Message::Incoming(message) => {
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
            Message::Outgoing(message) => {
                let timestamp = Local.from_utc_datetime(&message.timestamp.naive_local());
                write!(f, "{} - {}me:{} {}", timestamp.format("%T"), color::Fg(color::Yellow), color::Fg(color::White), message.body)
            }
        }
    }
}

struct BufferedWin<T: fmt::Display + hash::Hash> {
    widget: Widget,
    next_line: u16,
    buf: Vec<T>,
}

impl<T: fmt::Display + hash::Hash> BufferedWin<T> {
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
        self.buf.push(message);
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

pub enum Window {
    Console(ConsoleWin),
    Chat(ChatWin),
}

impl Window {
    fn chat(screen: Rc<RefCell<Screen>>, us: &BareJid, them: &BareJid) -> Self {
        let widget = Widget {
            screen: screen,
            vpos: VerticalPosition::Top,
            hpos: HorizontalPosition::Left,
            voff: 1,
            hoff: 0,
            width: Dimension::Relative(1., 0),
            height: Dimension::Relative(1., -1),
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        };

        let mut bufwin = BufferedWin {
            widget: widget,
            next_line: 0,
            buf: Vec::new(),
        };

        bufwin.redraw();

        Window::Chat(ChatWin {
            bufwin: bufwin,
            us: us.clone(),
            them: them.clone(),
        })
    }

    fn console(screen: Rc<RefCell<Screen>>) -> Self {
        let widget = Widget {
            screen: screen,
            vpos: VerticalPosition::Top,
            hpos: HorizontalPosition::Left,
            voff: 1,
            hoff: 0,
            width: Dimension::Relative(1., 0),
            height: Dimension::Relative(1., -1),
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        };

        let mut bufwin = BufferedWin {
            widget: widget,
            next_line: 0,
            buf: Vec::new(),
        };

        bufwin.redraw();

        Window::Console(ConsoleWin {
            bufwin: bufwin,
        })
    }

    fn redraw(&mut self) {
        match self {
            Window::Chat(chat) => chat.bufwin.redraw(),
            Window::Console(console) => console.bufwin.redraw(),
        }
    }

    fn show(&mut self) {
        match self {
            Window::Chat(chat) => chat.bufwin.show(),
            Window::Console(console) => console.bufwin.show(),
        }
    }

    fn message(&mut self, message: &mut Message, print: bool) {
        match self {
            Window::Chat(chat) => chat.bufwin.message(message.clone(), print),
            Window::Console(console) => console.bufwin.message(message.clone(), print),
        }
    }

    #[allow(dead_code)]
    fn scroll(&mut self) {
        match self {
            Window::Chat(chat) => chat.bufwin.scroll(),
            Window::Console(console) => console.bufwin.scroll(),
        }
    }
}

pub struct UIPlugin {
    screen: Rc<RefCell<Screen>>,
    input: Input,
    title_bar: TitleBar,
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
            chat.show();
            return Ok(())
        } else {
            return Err(())
        }
    }

    fn add_window(&mut self, name: String, window: Window) {
        self.windows.insert(name.clone(), window);
        self.windows_index.push(name.clone());
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

        Self {
            screen: screen,
            input: input,
            title_bar: title_bar,
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
        self.add_window("console".to_string(), console);
        self.title_bar.set_name("console");

        self.input.redraw();
        self.title_bar.redraw();
        self.current_window().redraw();

        Ok(())
    }

    fn on_connect(&mut self, _aparte: Rc<Aparte>) {
    }

    fn on_disconnect(&mut self, _aparte: Rc<Aparte>) {
    }

    fn on_message(&mut self, _aparte: Rc<Aparte>, message: &mut Message) {
        let chat_name = match message {
            Message::Incoming(message) => message.from.to_string(),
            Message::Outgoing(message) => message.to.to_string(),
            Message::Log(_message) => "console".to_string(),
        };

        let chat = match self.windows.get_mut(&chat_name) {
            Some(chat) => chat,
            None => {
                let mut chat = match message {
                    Message::Incoming(message) => Window::chat(self.screen.clone(), &message.to, &message.from),
                    Message::Outgoing(message) => Window::chat(self.screen.clone(), &message.from, &message.to),
                    Message::Log(_) => unreachable!(),
                };
                chat.redraw();
                self.add_window(chat_name.clone(), chat);
                self.windows.get_mut(&chat_name).unwrap()
            },
        };

        if self.current == chat_name {
            chat.message(message, true);
        } else {
            chat.message(message, false);
            self.switch(&chat_name).unwrap();
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
                                    let message = Message::outgoing(id.to_string(), timestamp, &from, &to, &ui.input.buf);
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
