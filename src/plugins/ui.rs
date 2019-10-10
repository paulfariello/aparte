use bytes::BytesMut;
use chrono::Utc;
use chrono::offset::{TimeZone, Local};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::io::{Error as IoError, ErrorKind};
use std::io::{Write, Stdout};
use std::rc::Rc;
use termion::color;
use termion::event::Key;
use termion::input::TermRead;
use termion::raw::{IntoRawMode, RawTerminal};
use termion::screen::AlternateScreen;
use tokio::codec::FramedRead;
use tokio_codec::{Decoder};
use uuid::Uuid;
use xmpp_parsers::{BareJid, Jid};
use std::str::FromStr;

use crate::core::{Plugin, Aparte, Event, Message, XmppMessage, Command, CommandOrMessage, CommandError, contact};
use crate::terminus::{View, ViewTrait, Dimension, LinearLayout, FrameLayout, Input, Orientation, BufferedWin, Window, ListView};

pub type CommandStream = FramedRead<tokio::reactor::PollEvented2<tokio_file_unix::File<std::fs::File>>, KeyCodec>;
type Screen = AlternateScreen<RawTerminal<Stdout>>;

enum UIEvent<'a> {
    Key(Key),
    Validate(Rc<RefCell<Option<(String, bool)>>>),
    ReadPassword,
    Connected(String),
    Message(Message),
    AddWindow(String, Option<Box<dyn ViewTrait<UIEvent<'a>> + 'a>>),
    ChangeWindow(String),
    ContactGroup(contact::Group),
    Contact(contact::Contact),
    ContactUpdate(contact::Contact),
}

#[derive(Debug, Clone)]
enum ConversationKind {
    Chat,
    Group,
}

#[derive(Debug, Clone)]
struct Conversation {
    jid: BareJid,
    kind: ConversationKind,
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
            #[cfg(feature = "no-cursor-save")]
            cursor_x: None,
            #[cfg(feature = "no-cursor-save")]
            cursor_y: None,
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
        self.save_cursor();

        {
            let mut screen = self.screen.borrow_mut();

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
        }

        self.restore_cursor();
        self.screen.borrow_mut().flush().unwrap();
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
            #[cfg(feature = "no-cursor-save")]
            cursor_x: None,
            #[cfg(feature = "no-cursor-save")]
            cursor_y: None,
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
        self.save_cursor();

        {
            let mut screen = self.screen.borrow_mut();

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
        }

        self.restore_cursor();
        self.screen.borrow_mut().flush().unwrap();
    }

    fn event(&mut self, event: &mut UIEvent) {
        match event {
            UIEvent::ChangeWindow(name) => {
                self.set_current_window(name);
            },
            UIEvent::AddWindow(name, _) => {
                self.add_window(name);
            }
            UIEvent::Connected(jid) => {
                self.content.connection = Some(jid.clone());
                self.redraw();
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

impl fmt::Display for contact::Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}{}", color::Fg(color::Yellow), self.0, color::Fg(color::White))
    }
}

impl fmt::Display for contact::Contact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.presence {
            contact::Presence::Available | contact::Presence::Chat => write!(f, "{}", color::Fg(color::Green)),
            contact::Presence::Away | contact::Presence::Dnd | contact::Presence::Xa | contact::Presence::Unavailable => write!(f, "{}", color::Fg(color::White)),
        };

        match &self.name {
            Some(name) => write!(f, "{} ({}){}", name, self.jid, color::Fg(color::White)),
            None => write!(f, "{}{}", self.jid, color::Fg(color::White)),
        }
    }
}

pub struct UIPlugin<'a> {
    screen: Rc<RefCell<Screen>>,
    windows: Vec<String>,
    current_window: Option<String>,
    conversations: HashMap<String, Conversation>,
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

    fn event(&mut self, mut event: UIEvent<'a>) {
        self.root.event(&mut event);
    }

    fn add_conversation(&mut self, conversation: Conversation) {
        match conversation.kind {
            ConversationKind::Chat => {
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

                self.windows.push(conversation.jid.to_string());
                self.root.event(&mut UIEvent::AddWindow(conversation.jid.to_string(), Some(Box::new(chat))));
                self.conversations.insert(conversation.jid.to_string(), conversation);
            },
            ConversationKind::Group => {
                let chat = View::<BufferedWin<Message>, UIEvent<'a>>::new(self.screen.clone()).with_event(|view, event| {
                    match event {
                        UIEvent::Message(Message::Incoming(XmppMessage::Groupchat(message))) => {
                            // TODO check to == us
                            view.recv_message(&Message::Incoming(XmppMessage::Groupchat(message.clone())), true);
                        },
                        UIEvent::Message(Message::Outgoing(XmppMessage::Groupchat(message))) => {
                            // TODO check from == us
                            view.recv_message(&Message::Outgoing(XmppMessage::Groupchat(message.clone())), true);
                        },
                        UIEvent::Key(Key::PageUp) => view.page_up(),
                        UIEvent::Key(Key::PageDown) => view.page_down(),
                        _ => {},
                    }
                });

                self.windows.push(conversation.jid.to_string());
                self.root.event(&mut UIEvent::AddWindow(conversation.jid.to_string(), Some(Box::new(chat))));
                self.conversations.insert(conversation.jid.to_string(), conversation);
            }
        }
    }

    pub fn change_window(&mut self, window: &str) {
        self.root.event(&mut UIEvent::ChangeWindow(window.to_string()));
        self.current_window = Some(window.to_string());
    }

    pub fn next_window(&mut self) {
        if let Some(current) = &self.current_window {
            let index = self.windows.iter().position(|e| e == current).unwrap();
            if index < self.windows.len() - 1 {
                self.change_window(&self.windows[index + 1].clone());
            }
        } else if self.windows.len() > 0 {
            self.change_window(&self.windows[0].clone());
        }
    }

    pub fn prev_window(&mut self) {
        if let Some(current) = &self.current_window {
            let index = self.windows.iter().position(|e| e == current).unwrap();
            if index > 0 {
                self.change_window(&self.windows[index - 1].clone());
            }
        } else if self.windows.len() > 0 {
            self.change_window(&self.windows[0].clone());
        }
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
            windows: Vec::new(),
            current_window: None,
            conversations: HashMap::new(),
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

        let mut console = View::<LinearLayout::<UIEvent<'a>>, UIEvent<'a>>::new(self.screen.clone(), Orientation::Horizontal, Dimension::MatchParent, Dimension::MatchParent);
        console.push(View::<BufferedWin<Message>, UIEvent<'a>>::new(self.screen.clone()).with_event(|view, event| {
            match event {
                UIEvent::Message(Message::Log(message)) => {
                    view.recv_message(&Message::Log(message.clone()), true);
                },
                UIEvent::Key(Key::PageUp) => view.page_up(),
                UIEvent::Key(Key::PageDown) => view.page_down(),
                _ => {},
            }
        }));
        let mut roster = View::<ListView<contact::Group, contact::Contact>, UIEvent<'a>>::new(self.screen.clone()).with_none_group().with_event(|view, event| {
            match event {
                UIEvent::Contact(contact) => {
                    view.insert(contact.clone(), None);
                },
                UIEvent::ContactGroup(group) => {
                    view.add_group(group.clone());
                },
                UIEvent::ContactUpdate(contact) => {
                    view.insert(contact.clone(), None);
                }
                _ => {},
            }
        });
        console.push(roster);

        self.windows.push("console".to_string());
        self.root.event(&mut UIEvent::AddWindow("console".to_string(), Some(Box::new(console))));
        self.change_window("console");

        Ok(())
    }

    fn on_event(&mut self, aparte: Rc<Aparte>, event: &Event) {
        match event {
            Event::ReadPassword(command) => {
                self.password_command = Some(command.clone());
                self.root.event(&mut UIEvent::ReadPassword);
            },
            Event::Connected(jid) => {
                self.root.event(&mut UIEvent::Connected(jid.to_string()));
            },
            Event::Message(message) => {
                match message {
                    Message::Incoming(XmppMessage::Chat(message)) => {
                        let window_name = message.from.to_string();
                        if !self.conversations.contains_key(&window_name) {
                            self.add_conversation(Conversation {
                                jid: BareJid::from_str(&window_name).unwrap(),
                                kind: ConversationKind::Chat,
                            });
                        }
                    },
                    Message::Outgoing(XmppMessage::Chat(message)) => {
                        let window_name = message.to.to_string();
                        if !self.conversations.contains_key(&window_name) {
                            self.add_conversation(Conversation {
                                jid: BareJid::from_str(&window_name).unwrap(),
                                kind: ConversationKind::Chat,
                            });
                        }
                    },
                    Message::Incoming(XmppMessage::Groupchat(message)) => {
                        let window_name = message.from.to_string();
                        if !self.conversations.contains_key(&window_name) {
                            self.add_conversation(Conversation {
                                jid: BareJid::from_str(&window_name).unwrap(),
                                kind: ConversationKind::Group,
                            });
                        }
                    },
                    Message::Outgoing(XmppMessage::Groupchat(message)) => {
                        let window_name = message.to.to_string();
                        if !self.conversations.contains_key(&window_name) {
                            self.add_conversation(Conversation {
                                jid: BareJid::from_str(&window_name).unwrap(),
                                kind: ConversationKind::Group,
                            });
                        }
                    }
                    Message::Log(_message) => {}
                };

                self.root.event(&mut UIEvent::Message(message.clone()));
            },
            Event::Chat(jid) => {
                let win_name = jid.to_string();
                if !self.conversations.contains_key(&win_name) {
                    self.add_conversation(Conversation {
                        jid: BareJid::from_str(&win_name).unwrap(),
                        kind: ConversationKind::Chat,
                    });
                }
                self.change_window(&win_name);
            },
            Event::Join(jid) => {
                let bare: BareJid = jid.clone().into();
                let win_name = bare.to_string();
                if !self.conversations.contains_key(&win_name) {
                    self.add_conversation(Conversation {
                        jid: BareJid::from_str(&win_name).unwrap(),
                        kind: ConversationKind::Group,
                    });
                }
                self.change_window(&win_name);
            },
            Event::Win(window) => {
                if self.windows.contains(window) {
                    self.change_window(&window);
                } else {
                    aparte.log(format!("Unknown window {}", window));
                }
            },
            Event::Contact(contact) => {
                self.root.event(&mut UIEvent::Contact(contact.clone()));
            },
            Event::ContactUpdate(contact) => {
                self.root.event(&mut UIEvent::ContactUpdate(contact.clone()));
            },
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
                Ok(Key::Char('\t')) => {},
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
                        if let Some(current_window) = ui.current_window.clone() {
                            if let Some(conversation) = ui.conversations.get(&current_window) {
                                let us = self.aparte.current_connection().unwrap().clone().into();
                                match conversation.kind {
                                    ConversationKind::Chat => {
                                        let from: Jid = us;
                                        let to: Jid = conversation.jid.clone().into();
                                        let id = Uuid::new_v4();
                                        let timestamp = Utc::now();
                                        let message = Message::outgoing_chat(id.to_string(), timestamp, &from, &to, &raw_buf);
                                        self.queue.push(Ok(CommandOrMessage::Message(message)));
                                    },
                                    ConversationKind::Group => {
                                        let from: Jid = us;
                                        let to: Jid = conversation.jid.clone().into();
                                        let id = Uuid::new_v4();
                                        let timestamp = Utc::now();
                                        let message = Message::outgoing_groupchat(id.to_string(), timestamp, &from, &to, &raw_buf);
                                        self.queue.push(Ok(CommandOrMessage::Message(message)));
                                    },
                                }
                            }
                        }
                    }
                },
                Ok(Key::Alt('\x1b')) => {
                    match keys.next() {
                        Some(Ok(Key::Char('['))) => {
                            match keys.next() {
                                Some(Ok(Key::Char('C'))) => {
                                    ui.next_window();
                                },
                                Some(Ok(Key::Char('D'))) => {
                                    ui.prev_window();
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
