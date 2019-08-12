use bytes::{BytesMut, BufMut};
use futures::Sink;
use std::cell::RefCell;
use std::clone::Clone;
use std::collections::HashMap;
use std::fmt;
use std::io::{Error as IoError, ErrorKind};
use std::io::{Write, Stdout};
use std::rc::Rc;
use termion::cursor::DetectCursorPos;
use termion::raw::{IntoRawMode, RawTerminal};
use termion::screen::AlternateScreen;
use tokio::codec::FramedRead;
use tokio_codec::{Decoder};
use tokio_xmpp;
use xmpp_parsers::Jid;

use crate::core::Message;
use crate::core::{Command, CommandError};

pub type CommandStream = FramedRead<tokio::reactor::PollEvented2<tokio_file_unix::File<std::fs::File>>, KeyCodec>;
type Screen = AlternateScreen<RawTerminal<Stdout>>;

trait Widget {
    fn redraw(&mut self);
}

enum VerticalPosition {
    Top,
    Bottom,
}

enum HorizontalPosition {
    Left,
    Right,
}

struct Position {
    v: VerticalPosition,
    h: HorizontalPosition,
    voff: u16,
    hoff: u16,
}

impl Position {
    fn TopLeft(voff: u16, hoff: u16) -> Self {
        Self {
            v: VerticalPosition::Top,
            h: HorizontalPosition::Left,
            voff: voff,
            hoff: hoff,
        }
    }

    fn TopRight(voff: u16, hoff: u16) -> Self {
        Self {
            v: VerticalPosition::Top,
            h: HorizontalPosition::Right,
            voff: voff,
            hoff: hoff,
        }
    }

    fn BottomLeft(voff: u16, hoff: u16) -> Self {
        Self {
            v: VerticalPosition::Bottom,
            h: HorizontalPosition::Left,
            voff: voff,
            hoff: hoff,
        }
    }

    fn BottomRight(voff: u16, hoff: u16) -> Self {
        Self {
            v: VerticalPosition::Bottom,
            h: HorizontalPosition::Right,
            voff: voff,
            hoff: hoff,
        }
    }
}

enum Width {
    Relative(f32),
    Absolute(u16),
}

enum Height {
    Relative(f32),
    Absolute(u16),
}

struct Input {
    position: Position,
    width: Width,
    x: u16,
    y: u16,
    w: u16,
    buf: String,
    screen: Rc<RefCell<Screen>>,
}

impl Input {
    fn new(screen: Rc<RefCell<Screen>>, position: Position, width: Width) -> Self {
        Self {
            position: position,
            width: width,
            x: 0,
            y: 0,
            w: 0,
            screen: screen,
            buf: String::new(),
        }
    }

    fn key(&mut self, c: char) {
        let mut screen = self.screen.borrow_mut();
        self.buf.push(c);
        write!(screen, "{}", c);
        screen.flush();
    }

    fn delete(&mut self) {
        let mut screen = self.screen.borrow_mut();
        write!(screen, "{} {}", termion::cursor::Left(1), termion::cursor::Left(1));
        self.buf.pop();
        screen.flush();
    }

    fn clear(&mut self) {
        let mut screen = self.screen.borrow_mut();
        self.buf.clear();
        write!(screen, "{}", termion::cursor::Goto(self.x, self.y));
        for _i in 1..=self.w {
            write!(screen, " ");
        }
        write!(screen, "{}", termion::cursor::Goto(self.x, self.y));
        screen.flush();
    }

    fn left(&mut self) {
        let mut screen = self.screen.borrow_mut();
        write!(screen, "{}", termion::cursor::Left(1));
        screen.flush();
    }

    fn right(&mut self) {
        let mut screen = self.screen.borrow_mut();
        let (x, _y) = screen.cursor_pos().unwrap();
        if x as usize <= self.buf.len() {
            write!(screen, "{}", termion::cursor::Right(1));
            screen.flush();
        }
    }
}

impl Widget for Input {
    fn redraw(&mut self) {
        let mut screen = self.screen.borrow_mut();
        let (height, width) = termion::terminal_size().unwrap();

        self.x = match self.position.h {
            HorizontalPosition::Left => 1 + self.position.voff,
            HorizontalPosition::Right => width - self.position.voff,
        };

        self.y = match self.position.v {
            VerticalPosition::Top => 1 + self.position.voff,
            VerticalPosition::Bottom => height - self.position.voff,
        };

        self.w = match self.width {
            Width::Relative(r) => (r * width as f32) as u16,
            Width::Absolute(w) => w,
        };

        write!(screen, "{}", termion::cursor::Goto(self.x, self.y));
        for _i in 1..=self.w {
            write!(screen, " ");
        }
        write!(screen, "{}", termion::cursor::Goto(self.x, self.y));
        screen.flush();
    }
}

struct Chat {
    name: String,
    position: Position,
    width: Width,
    height: Height,
    x: u16,
    y: u16,
    w: u16,
    h: u16,
    next_line: u16,
    buf: Vec<Message>,
    screen: Rc<RefCell<Screen>>,
}

impl Chat {
    fn new(screen: Rc<RefCell<Screen>>, name: String, position: Position, width: Width, height: Height) -> Self {
        Self {
            name: name,
            position: position,
            width: width,
            height: height,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            next_line: 0,
            screen: screen,
            buf: Vec::new(),
        }
    }

    fn show(&mut self) {
        let mut screen = self.screen.borrow_mut();

        write!(screen, "{}", termion::cursor::Save);

        let mut messages = self.buf.iter();

        for y in self.y ..= self.y + self.h {
            write!(screen, "{}", termion::cursor::Goto(self.x, y));

            for x in self.x  ..= self.x + self.w {
                write!(screen, " ");
            }

            write!(screen, "{}", termion::cursor::Goto(self.x, y));

            if let Some(message) = messages.next() {
                let _result = match & message.from {
                    Some(Jid::Bare(from)) => write!(screen, "{}: {}", from, message.body),
                    Some(Jid::Full(from)) => write!(screen, "{}: {}", from, message.body),
                    None => write!(screen, "{}", message.body),
                };
            }
            screen.flush();
        }

        write!(screen, "{}", termion::cursor::Restore);

        screen.flush();
    }

    fn print_message(&mut self, message: &Message) {
        if self.next_line > self.h {
            self.scroll();
        }

        let mut screen = self.screen.borrow_mut();

        let x = self.x;
        let y = self.y + self.next_line;

        write!(screen, "{}{}", termion::cursor::Save, termion::cursor::Goto(x, y));

        let _result = match & message.from {
            Some(Jid::Bare(from)) => write!(screen, "{}: {}", from, message.body),
            Some(Jid::Full(from)) => write!(screen, "{}: {}", from, message.body),
            None => write!(screen, "{}", message.body),
        };

        self.next_line += 1;

        write!(screen, "{}", termion::cursor::Restore);

        screen.flush();
    }

    fn message(&mut self, message: &mut Message, print: bool) {
        self.buf.push(message.clone());
        if print {
            self.print_message(message);
        }
    }

    fn scroll(&mut self) {
    }
}

impl Widget for Chat {
    fn redraw(&mut self) {
        let mut screen = self.screen.borrow_mut();
        let (height, width) = termion::terminal_size().unwrap();

        self.x = match self.position.h {
            HorizontalPosition::Left => 1 + self.position.hoff,
            HorizontalPosition::Right => width - self.position.hoff,
        };

        self.y = match self.position.v {
            VerticalPosition::Top => 1 + self.position.voff,
            VerticalPosition::Bottom => height - self.position.voff,
        };

        self.w = match self.width {
            Width::Relative(r) => (r * width as f32) as u16,
            Width::Absolute(w) => w,
        };

        self.h = match self.height {
            Height::Relative(r) => (r * height as f32) as u16,
            Height::Absolute(w) => w,
        };

        screen.flush();
    }
}

pub struct UIPlugin {
    screen: Rc<RefCell<Screen>>,
    input: Input,
    chats: HashMap<String, Chat>,
    current: String,
}

impl UIPlugin {
    pub fn command_stream(&self, mgr: Rc<super::PluginManager>) -> CommandStream {
        let file = tokio_file_unix::raw_stdin().unwrap();
        let file = tokio_file_unix::File::new_nb(file).unwrap();
        let file = file.into_io(&tokio::reactor::Handle::default()).unwrap();

        FramedRead::new(file, KeyCodec::new(mgr))
    }

    pub fn current_chat(&mut self) -> &mut Chat {
        self.chats.get_mut(&self.current).unwrap()
    }

    pub fn switch(&mut self, chat: &str) {
        self.current = chat.to_string();
        let chat = self.chats.get_mut(chat).unwrap();
        chat.show();
    }
}

impl super::Plugin for UIPlugin {
    fn new() -> Self {
        let stdout = std::io::stdout().into_raw_mode().unwrap();
        let screen = Rc::new(RefCell::new(AlternateScreen::from(stdout)));
        let input = Input::new(screen.clone(), Position::BottomLeft(0, 0), Width::Relative(1.));
        let mut chats = HashMap::new();
        let console = Chat::new(screen.clone(), "console".to_string(), Position::TopLeft(1, 0), Width::Relative(1.), Height::Relative(1.));

        chats.insert("console".to_string(), console);

        Self {
            screen: screen,
            input: input,
            chats: chats,
            current: "console".to_string(),
        }
    }

    fn init(&mut self, _mgr: &super::PluginManager) -> Result<(), ()> {
        const VERSION: &'static str = env!("CARGO_PKG_VERSION");

        {
            let mut screen = self.screen.borrow_mut();
            write!(screen, "{}", termion::clear::All);
            write!(screen, "{}", termion::cursor::Goto(1,1));
            write!(screen, "Welcome to Aparté {}\n", VERSION);
        }

        self.input.redraw();
        self.current_chat().redraw();

        Ok(())
    }

    fn on_connect(&mut self, _sink: &mut dyn Sink<SinkItem=tokio_xmpp::Packet, SinkError=tokio_xmpp::Error>) {
    }

    fn on_disconnect(&mut self) {
    }

    fn on_message(&mut self, message: &mut Message) {
        let from = match & message.from {
            Some(Jid::Bare(from)) => from.to_string(),
            Some(Jid::Full(from)) => from.to_string(),
            None => "console".to_string(),
        };

        let chat = match self.chats.get_mut(&from) {
            Some(chat) => chat,
            None => {
                let mut chat = Chat::new(self.screen.clone(), from.clone(), Position::TopLeft(1, 0), Width::Relative(1.), Height::Relative(1.));
                chat.redraw();
                self.chats.insert(from.clone(), chat);
                self.chats.get_mut(&from).unwrap()
            }
        };

        if self.current == chat.name {
            chat.message(message, true);
        } else {
            chat.message(message, false);
            self.switch(&from);
        }
    }
}

impl fmt::Display for UIPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Aparté UI")
    }
}

pub struct KeyCodec {
    queue: Vec<Command>,
    mgr: Rc<super::PluginManager>,
}

impl KeyCodec {
    pub fn new(mgr: Rc<super::PluginManager>) -> Self {
        Self {
            queue: Vec::new(),
            mgr: mgr,
        }
    }
}

impl Decoder for KeyCodec {
    type Item = Command;
    type Error = CommandError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let mut ui = self.mgr.get_mut::<UIPlugin>().unwrap();
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
                        self.queue.push(Command::new(ui.input.buf.clone()));
                        ui.input.clear();
                    },
                    '\x7f' => {
                        ui.input.delete();
                    },
                    '\x03' => return Err(CommandError::Io(IoError::new(ErrorKind::BrokenPipe, "ctrl+c"))),
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
                        let mut screen = ui.screen.borrow_mut();
                        write!(screen, "^{:x}", c as u8);
                        screen.flush();
                    },
                }
            }
        }

        match self.queue.pop() {
            Some(command) => Ok(Some(command)),
            None => Ok(None),
        }
    }
}
