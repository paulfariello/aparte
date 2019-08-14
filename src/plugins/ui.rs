use bytes::{BytesMut, BufMut};
use futures::Sink;
use shell_words::split;
use std::cell::RefCell;
use std::clone::Clone;
use std::collections::HashMap;
use std::fmt;
use std::hash;
use std::io::{Error as IoError, ErrorKind};
use std::io::{Write, Stdout};
use std::rc::Rc;
use termion::cursor::DetectCursorPos;
use termion::raw::{IntoRawMode, RawTerminal};
use termion::screen::AlternateScreen;
use tokio::codec::FramedRead;
use tokio_codec::{Decoder};
use tokio_xmpp;
use uuid::Uuid;
use xmpp_parsers::{BareJid, Jid};

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
    Right,
}

#[derive(Clone)]
enum Width {
    Relative(f32),
    Absolute(u16),
}

#[derive(Clone)]
enum Height {
    Relative(f32),
    Absolute(u16),
}

#[derive(Clone)]
struct Widget {
    screen: Rc<RefCell<Screen>>,
    vpos: VerticalPosition,
    hpos: HorizontalPosition,
    voff: u16,
    hoff: u16,
    width: Width,
    height: Height,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
}

impl Widget {
    fn redraw(&mut self) {
        let (height, width) = termion::terminal_size().unwrap();

        self.x = match self.hpos {
            HorizontalPosition::Left => 1 + self.hoff,
            HorizontalPosition::Right => width - self.hoff,
        };

        self.y = match self.vpos {
            VerticalPosition::Top => 1 + self.voff,
            VerticalPosition::Bottom => height - self.voff,
        };

        self.w = match self.width {
            Width::Relative(r) => (r * width as f32) as u16,
            Width::Absolute(w) => w,
        };

        self.h = match self.height {
            Height::Relative(r) => (r * height as f32) as u16,
            Height::Absolute(w) => w,
        };

    }
}

struct Input {
    widget: Widget,
    buf: String,
    password: bool,
}

impl Input {
    fn new(screen: Rc<RefCell<Screen>>) -> Self {
        let mut widget = Widget {
            screen: screen,
            vpos: VerticalPosition::Bottom,
            hpos: HorizontalPosition::Left,
            voff: 0,
            hoff: 0,
            width: Width::Relative(1.),
            height: Height::Absolute(1),
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        };

        widget.redraw();

        Self {
            widget: widget,
            buf: String::new(),
            password: false,
        }
    }

    fn key(&mut self, c: char) {
        let mut screen = self.widget.screen.borrow_mut();
        self.buf.push(c);
        if !self.password {
            write!(screen, "{}", c);
            screen.flush();
        }
    }

    fn delete(&mut self) {
        let mut screen = self.widget.screen.borrow_mut();
        self.buf.pop();
        if !self.password {
            write!(screen, "{} {}", termion::cursor::Left(1), termion::cursor::Left(1));
            screen.flush();
        }
    }

    fn clear(&mut self) {
        let mut screen = self.widget.screen.borrow_mut();
        self.buf.clear();
        self.password = false;
        write!(screen, "{}", termion::cursor::Goto(self.widget.x, self.widget.y));
        for _i in 1..=self.widget.w {
            write!(screen, " ");
        }
        write!(screen, "{}", termion::cursor::Goto(self.widget.x, self.widget.y));
        screen.flush();
    }

    fn left(&mut self) {
        if !self.password {
            let mut screen = self.widget.screen.borrow_mut();
            write!(screen, "{}", termion::cursor::Left(1));
            screen.flush();
        }
    }

    fn right(&mut self) {
        if !self.password {
            let mut screen = self.widget.screen.borrow_mut();
            let (x, _y) = screen.cursor_pos().unwrap();
            if x as usize <= self.buf.len() {
                write!(screen, "{}", termion::cursor::Right(1));
                screen.flush();
            }
        }
    }

    fn password(&mut self) {
        self.clear();
        self.password = true;
        let mut screen = self.widget.screen.borrow_mut();
        write!(screen, "password: ");
        screen.flush();
    }

    fn redraw(&mut self) {
        self.widget.redraw();
        let mut screen = self.widget.screen.borrow_mut();

        write!(screen, "{}", termion::cursor::Goto(self.widget.x, self.widget.y));
        for _i in 1..=self.widget.w {
            write!(screen, " ");
        }
        write!(screen, "{}", termion::cursor::Goto(self.widget.x, self.widget.y));
        screen.flush();
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

        write!(screen, "{}", termion::cursor::Save);

        self.next_line = 0;
        let mut messages = self.buf.iter();

        for y in self.widget.y ..= self.widget.y + self.widget.h {
            write!(screen, "{}", termion::cursor::Goto(self.widget.x, y));

            for _x in self.widget.x  ..= self.widget.x + self.widget.w {
                write!(screen, " ");
            }

            write!(screen, "{}", termion::cursor::Goto(self.widget.x, y));

            if let Some(message) = messages.next() {
                write!(screen, "{}", message);
                self.next_line += 1;
            }
        }

        write!(screen, "{}", termion::cursor::Restore);

        screen.flush();
    }

    fn message(&mut self, message: T, print: bool) {
        if print {
            if self.next_line > self.widget.h {
                self.scroll();
            }

            let mut screen = self.widget.screen.borrow_mut();

            let x = self.widget.x;
            let y = self.widget.y + self.next_line;

            write!(screen, "{}{}", termion::cursor::Save, termion::cursor::Goto(x, y));

            write!(screen, "{}", message);

            self.next_line += 1;

            write!(screen, "{}", termion::cursor::Restore);

            screen.flush();
        }
        self.buf.push(message);
    }

    fn scroll(&mut self) {
    }

    fn redraw(&mut self) {
        self.widget.redraw();
        let mut screen = self.widget.screen.borrow_mut();
        screen.flush();
    }
}

struct ConsoleWin {
    bufwin: BufferedWin<Message>,
}

struct ChatWin {
    bufwin: BufferedWin<Message>,
    us: BareJid,
    them: BareJid,
}

enum Window {
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
            width: Width::Relative(1.),
            height: Height::Relative(1.),
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
            width: Width::Relative(1.),
            height: Height::Relative(1.),
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
    windows: HashMap<String, Window>,
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
            chat.show();
            return Ok(())
        } else {
            return Err(())
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
        let mut windows = HashMap::new();
        let console = Window::console(screen.clone());

        windows.insert("console".to_string(), console);

        Self {
            screen: screen,
            input: input,
            windows: windows,
            current: "console".to_string(),
            password_command: None,
        }
    }

    fn init(&mut self, _aparte: &Aparte) -> Result<(), ()> {
        const VERSION: &'static str = env!("CARGO_PKG_VERSION");

        {
            let mut screen = self.screen.borrow_mut();
            write!(screen, "{}", termion::clear::All);
            write!(screen, "{}", termion::cursor::Goto(1,1));
            write!(screen, "Welcome to Aparté {}\n", VERSION);
        }

        self.input.redraw();
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
            Message::Log(message) => "console".to_string(),
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
                self.windows.insert(chat_name.clone(), chat);
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
                                    let message = Message::outgoing(id.to_string(), &from, &to, &ui.input.buf);
                                    self.queue.push(Ok(CommandOrMessage::Message(message)));
                                },
                                Window::Console(_) => { },
                            }
                        }
                        ui.input.clear();
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
                                    Some(_) => {}
                                    None => {},
                                }
                            },
                            Some(_) => {},
                            None => {},
                        }
                    },
                    _ => {
                        for c in format!("[{:x}", c as u8).chars() {
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
