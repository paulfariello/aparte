use bytes::BytesMut;
use chrono::Utc;
use chrono::offset::{TimeZone, Local};
use std::cell::RefCell;
use std::cmp;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::Hash;
use std::hash;
use std::io::{Error as IoError, ErrorKind};
use std::io::{Write, Stdout};
use std::rc::Rc;
use termion::color;
use termion::cursor::DetectCursorPos;
use termion::event::Key;
use termion::input::TermRead;
use termion::raw::{IntoRawMode, RawTerminal};
use termion::screen::AlternateScreen;
use tokio::codec::FramedRead;
use tokio_codec::{Decoder};
use uuid::Uuid;
use xmpp_parsers::{BareJid, Jid};

use crate::core::{Plugin, Aparte, Event, Message, XmppMessage, Command, CommandOrMessage, CommandError};
use crate::terminus::{View, ViewTrait, Dimension, LinearLayout, FrameLayout, Input, Orientation, BufferedWin, Window};

pub type CommandStream = FramedRead<tokio::reactor::PollEvented2<tokio_file_unix::File<std::fs::File>>, KeyCodec>;
type Screen = AlternateScreen<RawTerminal<Stdout>>;

enum UIEvent<'a> {
    Key(Key),
    Validate(Rc<RefCell<Option<(String, bool)>>>),
    ReadPassword,
    Message(Message),
    AddWindow(String, Option<View<'a, BufferedWin<Message>, UIEvent<'a>>>),
    ChangeWindow(String),
}

struct TitleBar {
    window_name: Option<String>,
}

impl View<'_, TitleBar, UIEvent<'_>> {
    fn new(screen: Rc<RefCell<Screen>>) -> Self {
        Self {
            screen: screen,
            width: Dimension::MatchParent,
            height: Dimension::Absolute(1),
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            content: TitleBar {
                window_name: None,
            },
            event_handler: None,
        }
    }

    fn set_name(&mut self, name: &str) {
        self.content.window_name = Some(name.to_string());
        self.redraw();
    }
}

impl ViewTrait<UIEvent<'_>> for View<'_, TitleBar, UIEvent<'_>> {
    fn redraw(&mut self) {
        let mut screen = self.screen.borrow_mut();

        write!(screen, "{}", termion::cursor::Save).unwrap();
        write!(screen, "{}", termion::cursor::Goto(self.x, self.y)).unwrap();
        write!(screen, "{}{}", color::Bg(color::Blue), color::Fg(color::White)).unwrap();

        for _ in 0 .. self.w {
            write!(screen, " ").unwrap();
        }
        write!(screen, "{}", termion::cursor::Goto(self.x, self.y)).unwrap();
        if let Some(window_name) = &self.content.window_name {
            write!(screen, " {}", window_name).unwrap();
        }

        write!(screen, "{}{}", color::Bg(color::Reset), color::Fg(color::Reset)).unwrap();
        write!(screen, "{}", termion::cursor::Restore).unwrap();
        screen.flush().unwrap();
    }

    fn event(&mut self, event: &mut UIEvent) {
        match event {
            UIEvent::ChangeWindow(name) => {
                self.set_name(name);
            },
            _ => {},
        }
    }
}

struct WinBar {
    connection: Option<String>,
    windows: Vec<String>,
    current_window: Option<String>,
    highlighted: Vec<String>,
}

impl View<'_, WinBar, UIEvent<'_>> {
    fn new(screen: Rc<RefCell<Screen>>) -> Self {
        Self {
            screen: screen,
            width: Dimension::MatchParent,
            height: Dimension::Absolute(1),
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            content: WinBar {
                connection: None,
                windows: Vec::new(),
                current_window: None,
                highlighted: Vec::new(),
            },
            event_handler: None,
        }

    }

    fn add_window(&mut self, window: &str) {
        self.content.windows.push(window.to_string());
        self.redraw();
    }

    fn set_current_window(&mut self, window: &str) {
        self.content.current_window = Some(window.to_string());
        self.content.highlighted.drain_filter(|w| w == &window);
        self.redraw();
    }

    fn highlight_window(&mut self, window: &str) {
        if self.content.highlighted.iter().find(|w| w == &window).is_none() {
            self.content.highlighted.push(window.to_string());
            self.redraw();
        }
    }
}

impl ViewTrait<UIEvent<'_>> for View<'_, WinBar, UIEvent<'_>> {
    fn redraw(&mut self) {
        let mut screen = self.screen.borrow_mut();

        write!(screen, "{}", termion::cursor::Save).unwrap();
        write!(screen, "{}", termion::cursor::Goto(self.x, self.y)).unwrap();
        write!(screen, "{}{}", color::Bg(color::Blue), color::Fg(color::White)).unwrap();

        for _ in 0 .. self.w {
            write!(screen, " ").unwrap();
        }

        write!(screen, "{}", termion::cursor::Goto(self.x, self.y)).unwrap();
        if let Some(connection) = &self.content.connection {
            write!(screen, " {}", connection).unwrap();
        }

        let mut windows = String::new();
        let mut windows_len = 0;

        let mut index = 1;
        for window in &self.content.windows {
            if let Some(current) = &self.content.current_window {
                if window == current {
                    let win = format!("-{}: {}- ", index, window);
                    windows_len += win.len();
                    windows.push_str(&win);
                } else {
                    if self.content.highlighted.iter().find(|w| w == &window).is_some() {
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

        let start = self.x + self.w - windows_len as u16;
        write!(screen, "{}{}", termion::cursor::Goto(start, self.y), windows).unwrap();

        write!(screen, "{}{}", color::Bg(color::Reset), color::Fg(color::Reset)).unwrap();
        write!(screen, "{}", termion::cursor::Restore).unwrap();
        screen.flush().unwrap();
    }

    fn event(&mut self, event: &mut UIEvent) {
        match event {
            UIEvent::ChangeWindow(name) => {
                self.set_current_window(name);
            },
            UIEvent::AddWindow(name, _) => {
                self.add_window(name);
            }
            _ => {},
        }
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

pub struct ChatWin {
    us: BareJid,
    them: BareJid,
}

pub struct GroupchatWin {
    us: BareJid,
    groupchat: BareJid,
}

    //fn chat(screen: Rc<RefCell<Screen>>, us: &BareJid, them: &BareJid) -> Self {
    //    let bufwin = Self::bufwin::<Message>(screen);

    //    Window::Chat(ChatWin {
    //        bufwin: bufwin,
    //        us: us.clone(),
    //        them: them.clone(),
    //    })
    //}

    //fn groupchat(screen: Rc<RefCell<Screen>>, us: &BareJid, groupchat: &BareJid) -> Self {
    //    let bufwin = Self::bufwin::<Message>(screen);

    //    Window::Groupchat(GroupchatWin {
    //        bufwin: bufwin,
    //        us: us.clone(),
    //        groupchat: groupchat.clone(),
    //    })
    //}

pub struct UIPlugin<'a> {
    screen: Rc<RefCell<Screen>>,
    windows: HashSet<String>,
    root: Box<dyn ViewTrait<UIEvent<'a>> + 'a>,
    password_command: Option<Command>,
}

impl<'a> UIPlugin<'a> {
    pub fn command_stream(&self, aparte: Rc<Aparte>) -> CommandStream {
        let file = tokio_file_unix::raw_stdin().unwrap();
        let file = tokio_file_unix::File::new_nb(file).unwrap();
        let file = file.into_io(&tokio::reactor::Handle::default()).unwrap();

        FramedRead::new(file, KeyCodec::new(aparte))
    }

    pub fn event(&mut self, mut event: UIEvent<'a>) {
        self.root.event(&mut event);
    }
}

impl<'a> Plugin for UIPlugin<'a> {
    fn new() -> Self {
        let stdout = std::io::stdout().into_raw_mode().unwrap();
        let screen = Rc::new(RefCell::new(AlternateScreen::from(stdout)));
        let mut layout = View::<LinearLayout::<UIEvent<'a>>, UIEvent<'a>>::new(screen.clone(), Orientation::Vertical, Dimension::MatchParent, Dimension::MatchParent);

        let title_bar = View::<TitleBar, UIEvent>::new(screen.clone());
        let frame = View::<FrameLayout::<String, UIEvent<'a>>, UIEvent<'a>>::new(screen.clone()).with_event(|frame, event| {
            match event {
                UIEvent::ChangeWindow(name) => {
                    frame.current(name.to_string());
                },
                UIEvent::AddWindow(name, view) => {
                    let view = view.take().unwrap();
                    frame.insert(name.to_string(), view);
                },
                event => {
                    for (_, child) in frame.content.children.iter_mut() {
                        child.event(event);
                    }
                },
            }
        });
        let win_bar = View::<WinBar, UIEvent>::new(screen.clone());
        let input = View::<Input, UIEvent<'a>>::new(screen.clone()).with_event(|input, event| {
            match event {
                UIEvent::Key(Key::Char(c)) => input.key(*c),
                UIEvent::Key(Key::Backspace) => input.delete(),
                UIEvent::Key(Key::Up) => input.previous(),
                UIEvent::Key(Key::Down) => input.next(),
                UIEvent::Key(Key::Left) => input.left(),
                UIEvent::Key(Key::Right) => input.right(),
                UIEvent::Validate(result) => {
                    let mut result = result.borrow_mut();
                    result.replace(input.validate());
                },
                UIEvent::ReadPassword => input.password(),
                _ => {}
            }
        });

        layout.push(title_bar);
        layout.push(frame);
        layout.push(win_bar);
        layout.push(input);

        Self {
            screen: screen,
            root: Box::new(layout),
            windows: HashSet::new(),
            password_command: None,
        }
    }

    fn init(&mut self, _aparte: &Aparte) -> Result<(), ()> {
        {
            let mut screen = self.screen.borrow_mut();
            write!(screen, "{}", termion::clear::All).unwrap();
        }

        let (width, height) = termion::terminal_size().unwrap();
        self.root.measure(Some(width), Some(height));
        self.root.layout(1, 1);
        self.root.redraw();

        let console = View::<BufferedWin<Message>, UIEvent<'a>>::new(self.screen.clone()).with_event(|view, event| {
            match event {
                UIEvent::Message(Message::Log(message)) => {
                    view.recv_message(&Message::Log(message.clone()), true);
                },
                UIEvent::Key(Key::PageUp) => view.page_up(),
                UIEvent::Key(Key::PageDown) => view.page_down(),
                _ => {},
            }
        });

        self.windows.insert("console".to_string());
        self.root.event(&mut UIEvent::AddWindow("console".to_string(), Some(console)));
        self.root.event(&mut UIEvent::ChangeWindow("console".to_string()));

        Ok(())
    }

    fn on_event(&mut self, aparte: Rc<Aparte>, event: &Event) {
        match event {
            Event::ReadPassword(command) => {
                self.password_command = Some(command.clone());
                self.root.event(&mut UIEvent::ReadPassword);
            },
            Event::Connected(_jid) => {
                //self.win_bar.borrow_mut().content.connection = match aparte.current_connection() {
                //    Some(jid) => Some(jid.to_string()),
                //    None => None,
                //};
                //self.win_bar.borrow_mut().redraw();
            },
            Event::Message(message) => {
                self.root.event(&mut UIEvent::Message(message.clone()));
                //let chat_name = match message {
                //    Message::Incoming(XmppMessage::Chat(message)) => message.from.to_string(),
                //    Message::Outgoing(XmppMessage::Chat(message)) => message.to.to_string(),
                //    Message::Incoming(XmppMessage::Groupchat(message)) => message.from.to_string(),
                //    Message::Outgoing(XmppMessage::Groupchat(message)) => message.to.to_string(),
                //    Message::Log(_message) => "console".to_string(),
                //};

                //let chat = match self.windows.get_mut(&chat_name) {
                //    Some(chat) => chat,
                //    None => {
                //        let mut chat: Rc<RefCell<dyn Window<Message>>> = match message {
                //            //Message::Incoming(XmppMessage::Chat(message)) => Window::chat(self.screen.clone(), &message.to, &message.from),
                //            //Message::Outgoing(XmppMessage::Chat(message)) => Window::chat(self.screen.clone(), &message.from, &message.to),
                //            //Message::Incoming(XmppMessage::Groupchat(message)) => Window::groupchat(self.screen.clone(), &message.to, &message.from),
                //            //Message::Outgoing(XmppMessage::Groupchat(message)) => Window::groupchat(self.screen.clone(), &message.from, &message.to),
                //            Message::Log(_) => unreachable!(),
                //            _ => unreachable!(),
                //        };
                //        chat.borrow_mut().redraw();
                //        self.add_window(&chat_name, chat);
                //        self.windows.get_mut(&chat_name).unwrap()
                //    },
                //};

                //chat.borrow_mut().recv_message(message, self.current == chat_name);
                //if self.current != chat_name {
                //    self.win_bar.borrow_mut().highlight_window(&chat_name);
                //}
            },
            Event::Chat(jid) => {
                let chat_name = jid.to_string();
                if self.windows.contains(&chat_name) {
                    self.root.event(&mut UIEvent::ChangeWindow(chat_name.clone()));
                } else {
                    // let us = aparte.current_connection().unwrap().clone().into();
                    let chat = View::<BufferedWin<Message>, UIEvent<'a>>::new(self.screen.clone()).with_event(|view, event| {
                        match event {
                            UIEvent::Message(Message::Incoming(XmppMessage::Chat(message))) => {
                                // TODO check to == us
                                view.recv_message(&Message::Incoming(XmppMessage::Chat(message.clone())), true);
                            },
                            UIEvent::Message(Message::Outgoing(XmppMessage::Chat(message))) => {
                                // TODO check from == us
                                view.recv_message(&Message::Outgoing(XmppMessage::Chat(message.clone())), true);
                            },
                            UIEvent::Key(Key::PageUp) => view.page_up(),
                            UIEvent::Key(Key::PageDown) => view.page_down(),
                            _ => {},
                        }
                    });

                    self.windows.insert(chat_name.clone());
                    self.root.event(&mut UIEvent::AddWindow(chat_name.clone(), Some(chat)));
                    self.root.event(&mut UIEvent::ChangeWindow(chat_name.clone()));
                }
            },
            Event::Join(jid) => {
                //let groupchat: BareJid = jid.clone().into();
                //let win_name = groupchat.to_string();
                //if self.switch(&win_name).is_err() {
                //    //let us = aparte.current_connection().unwrap().clone().into();
                //    //let groupchat = jid.clone().into();
                //    //let chat = Window::groupchat(self.screen.clone(), &us, &groupchat);
                //    //self.add_window(&win_name, chat);
                //    //self.switch(&win_name).unwrap();
                //}
            }
            _ => {},
        }
    }
}

impl<'a> fmt::Display for UIPlugin<'a> {
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

        let mut keys = buf.keys();
        while let Some(key) = keys.next() {
            match key {
                Ok(Key::Backspace) => {
                    ui.event(UIEvent::Key(Key::Backspace));
                },
                Ok(Key::Left) => {
                    ui.event(UIEvent::Key(Key::Left));
                },
                Ok(Key::Right) => {
                    ui.event(UIEvent::Key(Key::Right));
                },
                Ok(Key::Up) => {
                    ui.event(UIEvent::Key(Key::Up));
                },
                Ok(Key::Down) => {
                    ui.event(UIEvent::Key(Key::Down));
                },
                Ok(Key::PageUp) => {
                    ui.event(UIEvent::Key(Key::PageUp));
                },
                Ok(Key::PageDown) => {
                    ui.event(UIEvent::Key(Key::PageDown));
                },
                Ok(Key::Char('\n')) => {
                    let result = Rc::new(RefCell::new(None));
                    let event = UIEvent::Validate(Rc::clone(&result));

                    ui.event(event);

                    let result = result.borrow_mut();
                    let (raw_buf, password) = result.as_ref().unwrap();
                    let raw_buf = raw_buf.clone();
                    if *password {
                        let mut command = ui.password_command.take().unwrap();
                        command.args.push(raw_buf.clone());
                        self.queue.push(Ok(CommandOrMessage::Command(command)));
                    } else if raw_buf.starts_with("/") {
                        let splitted = shell_words::split(&raw_buf);
                        match splitted {
                            Ok(splitted) => {
                                let command = Command::new(splitted[0][1..].to_string(), splitted[1..].to_vec());
                                self.queue.push(Ok(CommandOrMessage::Command(command)));
                            },
                            Err(err) => self.queue.push(Err(CommandError::Parse(err))),
                        }
                    } else if raw_buf.len() > 0 {
                        //match ui.current_window() {
                        //    Window::Chat(chat) => {
                        //        let from: Jid = chat.us.clone().into();
                        //        let to: Jid = chat.them.clone().into();
                        //        let id = Uuid::new_v4();
                        //        let timestamp = Utc::now();
                        //        let message = Message::outgoing_chat(id.to_string(), timestamp, &from, &to, &raw_buf);
                        //        self.queue.push(Ok(CommandOrMessage::Message(message)));
                        //    },
                        //    Window::Groupchat(groupchat) => {
                        //        let from: Jid = groupchat.us.clone().into();
                        //        let to: Jid = groupchat.groupchat.clone().into();
                        //        let id = Uuid::new_v4();
                        //        let timestamp = Utc::now();
                        //        let message = Message::outgoing_groupchat(id.to_string(), timestamp, &from, &to, &raw_buf);
                        //        self.queue.push(Ok(CommandOrMessage::Message(message)));
                        //    },
                        //}
                    }
                },
                Ok(Key::Alt('\x1b')) => {
                    match keys.next() {
                        Some(Ok(Key::Char('['))) => {
                            match keys.next() {
                                Some(Ok(Key::Char('C'))) => {
                                    //let _ = ui.next_window();
                                },
                                Some(Ok(Key::Char('D'))) => {
                                    //let _ = ui.prev_window();
                                },
                                Some(Ok(_)) => {},
                                Some(Err(_)) => {},
                                None => {},
                            };
                        },
                        Some(Ok(_)) => {},
                        Some(Err(_)) => {},
                        None => {},
                    };
                },
                Ok(Key::Char(c)) => {
                    ui.event(UIEvent::Key(Key::Char(c)));
                },
                Ok(Key::Ctrl('c')) => {
                    self.queue.push(Err(CommandError::Io(IoError::new(ErrorKind::BrokenPipe, "ctrl+c"))));
                },
                Ok(_) => {},
                Err(_) => {},
            };
        }

        buf.clear();

        match self.queue.pop() {
            Some(Ok(command)) => Ok(Some(command)),
            Some(Err(err)) => Err(err),
            None => Ok(None),
        }
    }
}
