/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use bytes::BytesMut;
use chrono::Utc;
use chrono::offset::{TimeZone, Local};
use std::cell::RefCell;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use std::io::{Error as IoError, ErrorKind};
use std::io::{Write, Stdout};
use std::rc::Rc;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use termion::color;
use termion::event::Key;
use termion::input::TermRead;
use termion::raw::{IntoRawMode, RawTerminal};
use termion::screen::AlternateScreen;
use tokio::codec::FramedRead;
use tokio_codec::{Decoder};
use uuid::Uuid;
use xmpp_parsers::{BareJid, Jid};

use crate::core::{Plugin, Aparte, Event, CommandOrMessage};
use crate::{contact, conversation};
use crate::message::{Message, XmppMessage};
use crate::command::{Command, CommandError};
use crate::terminus::{View, ViewTrait, Dimension, LinearLayout, FrameLayout, Input, Orientation, BufferedWin, Window, ListView};

pub type CommandStream = FramedRead<tokio::reactor::PollEvented2<tokio_file_unix::File<std::fs::File>>, KeyCodec>;
type Screen = AlternateScreen<RawTerminal<Stdout>>;

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

impl View<'_, TitleBar, Event> {
    fn new(screen: Rc<RefCell<Screen>>) -> Self {
        Self {
            screen: screen,
            width: Dimension::MatchParent,
            height: Dimension::Absolute(1),
            x: 0,
            y: 0,
            w: None,
            h: None,
            dirty: true,
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

impl ViewTrait<Event> for View<'_, TitleBar, Event> {
    fn redraw(&mut self) {
        self.save_cursor();

        {
            let mut screen = self.screen.borrow_mut();

            write!(screen, "{}", termion::cursor::Goto(self.x, self.y)).unwrap();
            write!(screen, "{}{}", color::Bg(color::Blue), color::Fg(color::White)).unwrap();

            for _ in 0 .. self.w.unwrap() {
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

    fn event(&mut self, event: &mut Event) {
        match event {
            Event::ChangeWindow(name) => {
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

impl View<'_, WinBar, Event> {
    fn new(screen: Rc<RefCell<Screen>>) -> Self {
        Self {
            screen: screen,
            width: Dimension::MatchParent,
            height: Dimension::Absolute(1),
            x: 0,
            y: 0,
            w: None,
            h: None,
            dirty: true,
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

impl ViewTrait<Event> for View<'_, WinBar, Event> {
    fn redraw(&mut self) {
        self.save_cursor();

        {
            let mut screen = self.screen.borrow_mut();

            write!(screen, "{}", termion::cursor::Goto(self.x, self.y)).unwrap();
            write!(screen, "{}{}", color::Bg(color::Blue), color::Fg(color::White)).unwrap();

            for _ in 0 .. self.w.unwrap() {
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

            let start = self.x + self.w.unwrap() - windows_len as u16;
            write!(screen, "{}{}", termion::cursor::Goto(start, self.y), windows).unwrap();

            write!(screen, "{}{}", color::Bg(color::Reset), color::Fg(color::Reset)).unwrap();
        }

        self.restore_cursor();
        self.screen.borrow_mut().flush().unwrap();
    }

    fn event(&mut self, event: &mut Event) {
        match event {
            Event::ChangeWindow(name) => {
                self.set_current_window(name);
            },
            Event::AddWindow(name, _) => {
                self.add_window(name);
            }
            Event::Connected(jid) => {
                self.content.connection = Some(jid.to_string());
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
                for line in message.body.lines() {
                    write!(f, "{} - {}\n", timestamp.format("%T"), line);
                }

                Ok(())
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
            contact::Presence::Available | contact::Presence::Chat => write!(f, "{}", color::Fg(color::Green))?,
            contact::Presence::Away | contact::Presence::Dnd | contact::Presence::Xa | contact::Presence::Unavailable => write!(f, "{}", color::Fg(color::White))?,
        };

        match &self.name {
            Some(name) => write!(f, "{} ({}){}", name, self.jid, color::Fg(color::White)),
            None => write!(f, "{}{}", self.jid, color::Fg(color::White)),
        }
    }
}

impl fmt::Display for conversation::Occupant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}{}", color::Fg(color::Green), self.nick, color::Fg(color::White))
    }
}

impl fmt::Display for conversation::Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            conversation::Role::Moderator => write!(f, "{}Moderators{}", color::Fg(color::Yellow), color::Fg(color::Yellow)),
            conversation::Role::Participant => write!(f, "{}Participants{}", color::Fg(color::Yellow), color::Fg(color::Yellow)),
            conversation::Role::Visitor => write!(f, "{}Visitors{}", color::Fg(color::Yellow), color::Fg(color::Yellow)),
        }
    }
}

pub struct UIPlugin {
    screen: Rc<RefCell<Screen>>,
    windows: Vec<String>,
    current_window: Option<String>,
    conversations: HashMap<String, Conversation>,
    root: Box<dyn ViewTrait<Event>>,
    password_command: Option<Command>,
    completion: Option<Vec<String>>,
    current_completion: usize,
    running: Rc<AtomicBool>,
}

impl UIPlugin {
    pub fn command_stream(&self, aparte: Rc<Aparte>) -> CommandStream {
        let file = tokio_file_unix::raw_stdin().unwrap();
        let file = tokio_file_unix::File::new_nb(file).unwrap();
        let file = file.into_io(&tokio::reactor::Handle::default()).unwrap();

        FramedRead::new(file, KeyCodec::new(aparte, Rc::clone(&self.running)))
    }

    fn add_conversation(&mut self, aparte: Rc<Aparte>, conversation: Conversation) {
        let jid = conversation.jid.clone();
        match conversation.kind {
            ConversationKind::Chat => {
                let chat = View::<BufferedWin<Message>, Event>::new(self.screen.clone()).with_event(move |view, event| {
                    match event {
                        Event::Message(Message::Incoming(XmppMessage::Chat(message))) => {
                            // TODO check to == us
                            view.recv_message(&Message::Incoming(XmppMessage::Chat(message.clone())), true);
                        },
                        Event::Message(Message::Outgoing(XmppMessage::Chat(message))) => {
                            // TODO check from == us
                            view.recv_message(&Message::Outgoing(XmppMessage::Chat(message.clone())), true);
                        },
                        Event::Key(Key::PageUp) => {
                            Rc::clone(&aparte).event(Event::LoadHistory(jid.clone()));
                            view.page_up();
                        },
                        Event::Key(Key::PageDown) => view.page_down(),
                        _ => {},
                    }
                });

                self.windows.push(conversation.jid.to_string());
                self.root.event(&mut Event::AddWindow(conversation.jid.to_string(), Some(Box::new(chat))));
                self.conversations.insert(conversation.jid.to_string(), conversation);
            },
            ConversationKind::Group => {
                let mut layout = View::<LinearLayout::<Event>, Event>::new(self.screen.clone(), Orientation::Horizontal, Dimension::MatchParent, Dimension::MatchParent).with_event(|layout, event| {
                    for child in layout.content.children.iter_mut() {
                        child.event(event);
                    }
                });
                let chat = View::<BufferedWin<Message>, Event>::new(self.screen.clone()).with_event(|view, event| {
                    match event {
                        Event::Message(Message::Incoming(XmppMessage::Groupchat(message))) => {
                            // TODO check to == us
                            view.recv_message(&Message::Incoming(XmppMessage::Groupchat(message.clone())), true);
                        },
                        Event::Message(Message::Outgoing(XmppMessage::Groupchat(message))) => {
                            // TODO check from == us
                            view.recv_message(&Message::Outgoing(XmppMessage::Groupchat(message.clone())), true);
                        },
                        Event::Key(Key::PageUp) => view.page_up(),
                        Event::Key(Key::PageDown) => view.page_down(),
                        _ => {},
                    }
                });
                layout.push(chat);

                let roster = View::<ListView<conversation::Role, conversation::Occupant>, Event>::new(self.screen.clone()).with_none_group().with_event(|view, event| {
                    match event {
                        Event::Occupant(occupant) => {
                            view.insert(occupant.clone(), Some(occupant.role));
                        },
                        _ => {},
                    }
                });
                layout.push(roster);

                self.windows.push(conversation.jid.to_string());
                self.root.event(&mut Event::AddWindow(conversation.jid.to_string(), Some(Box::new(layout))));
                self.conversations.insert(conversation.jid.to_string(), conversation);
            }
        }
    }

    pub fn change_window(&mut self, window: &str) {
        self.root.event(&mut Event::ChangeWindow(window.to_string()));
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

    pub fn autocomplete(&mut self, command: &mut Command) {
        let completion = match &self.completion {
            None => {
                return;
            }
            Some(completion) => {
                completion
            }
        };

        if completion.len() == 0 {
            return;
        }

        if command.cursor < command.args.len() {
            command.args[command.cursor] = completion[self.current_completion].clone();
        } else {
            command.args.push(completion[self.current_completion].clone());
        }

        self.current_completion += 1;
        self.current_completion %= completion.len();
    }

    pub fn reset_completion(&mut self) {
        self.completion = None;
        self.current_completion = 0;
    }

    pub fn get_windows(&self) -> Vec<String> {
        self.windows.clone()
    }
}

impl Plugin for UIPlugin {
    fn new() -> Self {
        let stdout = std::io::stdout().into_raw_mode().unwrap();
        let screen = Rc::new(RefCell::new(AlternateScreen::from(stdout)));
        let mut layout = View::<LinearLayout::<Event>, Event>::new(screen.clone(), Orientation::Vertical, Dimension::MatchParent, Dimension::MatchParent).with_event(|layout, event| {
            for child in layout.content.children.iter_mut() {
                child.event(event);
            }

            if layout.is_dirty() {
                layout.measure(layout.w, layout.h);
                layout.layout(layout.x, layout.y);
                layout.redraw();
            }
        });


        let title_bar = View::<TitleBar, Event>::new(screen.clone());
        let frame = View::<FrameLayout::<String, Event>, Event>::new(screen.clone()).with_event(|frame, event| {
            match event {
                Event::ChangeWindow(name) => {
                    frame.current(name.to_string());
                },
                Event::AddWindow(name, view) => {
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
        let win_bar = View::<WinBar, Event>::new(screen.clone());
        let input = View::<Input, Event>::new(screen.clone()).with_event(|input, event| {
            match event {
                Event::Key(Key::Char(c)) => input.key(*c),
                Event::Key(Key::Backspace) => input.backspace(),
                Event::Key(Key::Delete) => input.delete(),
                Event::Key(Key::Home) => input.home(),
                Event::Key(Key::End) => input.end(),
                Event::Key(Key::Up) => input.previous(),
                Event::Key(Key::Down) => input.next(),
                Event::Key(Key::Left) => input.left(),
                Event::Key(Key::Right) => input.right(),
                Event::Key(Key::Ctrl('w')) => input.backward_delete_word(),
                Event::Validate(result) => {
                    let mut result = result.borrow_mut();
                    result.replace(input.validate());
                },
                Event::Complete(result) => {
                    let mut result = result.borrow_mut();
                    result.replace((input.content.buf.clone(), input.content.cursor, input.content.password));
                },
                Event::Completed(completion) => {
                    input.content.buf = completion.clone();
                    input.content.cursor = input.content.buf.len();
                    input.redraw();
                },
                Event::ReadPassword(_) => input.password(),
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
            completion: None,
            current_completion: 0,
            running: Rc::new(AtomicBool::new(true)),
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

        let mut console = View::<LinearLayout::<Event>, Event>::new(self.screen.clone(), Orientation::Horizontal, Dimension::MatchParent, Dimension::MatchParent).with_event(|layout, event| {
            for child in layout.content.children.iter_mut() {
                child.event(event);
            }
        });
        console.push(View::<BufferedWin<Message>, Event>::new(self.screen.clone()).with_event(|view, event| {
            match event {
                Event::Message(Message::Log(message)) => {
                    view.recv_message(&Message::Log(message.clone()), true);
                },
                Event::Key(Key::PageUp) => view.page_up(),
                Event::Key(Key::PageDown) => view.page_down(),
                _ => {},
            }
        }));
        let roster = View::<ListView<contact::Group, contact::Contact>, Event>::new(self.screen.clone()).with_none_group().with_event(|view, event| {
            match event {
                Event::Contact(contact) | Event::ContactUpdate(contact) => {
                    if contact.groups.len() > 0 {
                        for group in &contact.groups {
                            view.insert(contact.clone(), Some(group.clone()));
                        }
                    } else {
                            view.insert(contact.clone(), None);
                    }
                }
                _ => {},
            }
        });
        console.push(roster);

        self.windows.push("console".to_string());
        self.root.event(&mut Event::AddWindow("console".to_string(), Some(Box::new(console))));
        self.change_window("console");

        Ok(())
    }

    fn on_event(&mut self, aparte: Rc<Aparte>, event: &Event) {
        match event {
            Event::ReadPassword(command) => {
                self.password_command = Some(command.clone());
                self.root.event(&mut Event::ReadPassword(command.clone()));
            },
            Event::Connected(jid) => {
                self.root.event(&mut Event::Connected(jid.clone()));
            },
            Event::Message(message) => {
                match message {
                    Message::Incoming(XmppMessage::Chat(message)) => {
                        let window_name = message.from.to_string();
                        if !self.conversations.contains_key(&window_name) {
                            self.add_conversation(aparte, Conversation {
                                jid: BareJid::from_str(&window_name).unwrap(),
                                kind: ConversationKind::Chat,
                            });
                        }
                    },
                    Message::Outgoing(XmppMessage::Chat(message)) => {
                        let window_name = message.to.to_string();
                        if !self.conversations.contains_key(&window_name) {
                            self.add_conversation(aparte, Conversation {
                                jid: BareJid::from_str(&window_name).unwrap(),
                                kind: ConversationKind::Chat,
                            });
                        }
                    },
                    Message::Incoming(XmppMessage::Groupchat(message)) => {
                        let window_name = message.from.to_string();
                        if !self.conversations.contains_key(&window_name) {
                            self.add_conversation(aparte, Conversation {
                                jid: BareJid::from_str(&window_name).unwrap(),
                                kind: ConversationKind::Group,
                            });
                        }
                    },
                    Message::Outgoing(XmppMessage::Groupchat(message)) => {
                        let window_name = message.to.to_string();
                        if !self.conversations.contains_key(&window_name) {
                            self.add_conversation(aparte, Conversation {
                                jid: BareJid::from_str(&window_name).unwrap(),
                                kind: ConversationKind::Group,
                            });
                        }
                    }
                    Message::Log(_message) => {}
                };

                self.root.event(&mut Event::Message(message.clone()));
            },
            Event::Chat(jid) => {
                let win_name = jid.to_string();
                if !self.conversations.contains_key(&win_name) {
                    self.add_conversation(aparte, Conversation {
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
                    self.add_conversation(aparte, Conversation {
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
                self.root.event(&mut Event::Contact(contact.clone()));
            },
            Event::ContactUpdate(contact) => {
                self.root.event(&mut Event::ContactUpdate(contact.clone()));
            },
            Event::Occupant(occupant) => {
                self.root.event(&mut Event::Occupant(occupant.clone()));
            },
            Event::Signal(signal_hook::SIGWINCH) => {
                let (width, height) = termion::terminal_size().unwrap();
                self.root.measure(Some(width), Some(height));
                self.root.layout(1, 1);
                self.root.redraw();
            },
            Event::Key(key) => {
                self.reset_completion();
                self.root.event(&mut Event::Key(key.clone()));
            },
            Event::Quit => {
                self.running.swap(false, Ordering::Relaxed);
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
    running: Rc<AtomicBool>,
}

impl KeyCodec {
    pub fn new(aparte: Rc<Aparte>, running: Rc<AtomicBool>) -> Self {
        Self {
            queue: Vec::new(),
            aparte: aparte,
            running: running,
        }
    }
}

impl Decoder for KeyCodec {
    type Item = CommandOrMessage;
    type Error = CommandError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if self.running.load(Ordering::Relaxed) {
            let mut keys = buf.keys();
            while let Some(key) = keys.next() {
                match key {
                    Ok(Key::Backspace) => {
                        Rc::clone(&self.aparte).event(Event::Key(Key::Backspace));
                    },
                    Ok(Key::Delete) => {
                        Rc::clone(&self.aparte).event(Event::Key(Key::Delete));
                    },
                    Ok(Key::Home) => {
                        Rc::clone(&self.aparte).event(Event::Key(Key::Home));
                    },
                    Ok(Key::End) => {
                        Rc::clone(&self.aparte).event(Event::Key(Key::End));
                    },
                    Ok(Key::Left) => {
                        Rc::clone(&self.aparte).event(Event::Key(Key::Left));
                    },
                    Ok(Key::Right) => {
                        Rc::clone(&self.aparte).event(Event::Key(Key::Right));
                    },
                    Ok(Key::Up) => {
                        Rc::clone(&self.aparte).event(Event::Key(Key::Up));
                    },
                    Ok(Key::Down) => {
                        Rc::clone(&self.aparte).event(Event::Key(Key::Down));
                    },
                    Ok(Key::PageUp) => {
                        Rc::clone(&self.aparte).event(Event::Key(Key::PageUp));
                    },
                    Ok(Key::PageDown) => {
                        Rc::clone(&self.aparte).event(Event::Key(Key::PageDown));
                    },
                    Ok(Key::Char('\t')) => {
                        let result = Rc::new(RefCell::new(None));
                        let event = Event::Complete(Rc::clone(&result));

                        let (raw_buf, cursor, password) = {
                            Rc::clone(&self.aparte).event(event);

                            let result = result.borrow_mut();
                            result.as_ref().unwrap().clone()
                        };

                        if password {
                            Rc::clone(&self.aparte).event(Event::Key(Key::Char('\t')));
                        } else {
                            let raw_buf = raw_buf.clone();
                            if raw_buf.starts_with("/") {
                                if let Ok(mut command) = Command::parse_with_cursor(&raw_buf, cursor) {
                                    {
                                        let call_completion = {
                                            let ui = self.aparte.get_plugin::<UIPlugin>().unwrap();
                                            ui.completion.is_none()
                                        };

                                        if call_completion {
                                            let mut completion = self.aparte.autocomplete(command.clone());
                                            if command.cursor < command.args.len() {
                                                completion = completion.iter().filter_map(|c| {
                                                    if c.starts_with(&command.args[command.cursor]) {
                                                        Some(c.to_string())
                                                    } else {
                                                        None
                                                    }
                                                }).collect();
                                            }
                                            let mut ui = self.aparte.get_plugin_mut::<UIPlugin>().unwrap();
                                            ui.completion = Some(completion);
                                            ui.current_completion = 0;
                                        }
                                    }

                                    let mut ui = self.aparte.get_plugin_mut::<UIPlugin>().unwrap();
                                    ui.autocomplete(&mut command);
                                    Rc::clone(&self.aparte).event(Event::Completed(command.assemble()));
                                }
                            }
                        }
                    },
                    Ok(Key::Char('\n')) => {
                        let mut ui = self.aparte.get_plugin_mut::<UIPlugin>().unwrap();
                        let result = Rc::new(RefCell::new(None));
                        let event = Event::Validate(Rc::clone(&result));

                        Rc::clone(&self.aparte).event(event);

                        let result = result.borrow_mut();
                        let (raw_buf, password) = result.as_ref().unwrap();
                        let raw_buf = raw_buf.clone();
                        if *password {
                            let mut command = ui.password_command.take().unwrap();
                            command.args.push(raw_buf.clone());
                            self.queue.push(Ok(CommandOrMessage::Command(command)));
                        } else if raw_buf.starts_with("/") {
                            match Command::try_from(&*raw_buf) {
                                Ok(command) => {
                                    self.queue.push(Ok(CommandOrMessage::Command(command)));
                                },
                                Err(_) => self.queue.push(Err(CommandError::Parse)),
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
                        let mut ui = self.aparte.get_plugin_mut::<UIPlugin>().unwrap();
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
                        Rc::clone(&self.aparte).event(Event::Key(Key::Char(c)));
                    },
                    Ok(Key::Ctrl('w')) => {
                        Rc::clone(&self.aparte).event(Event::Key(Key::Ctrl('w')));
                    },
                    Ok(_) => {},
                    Err(_) => {},
                };
            }

            buf.clear();
        } else {
            self.queue.push(Err(CommandError::Io(IoError::new(ErrorKind::BrokenPipe, "quit"))));
        }

        match self.queue.pop() {
            Some(Ok(command)) => Ok(Some(command)),
            Some(Err(err)) => Err(err),
            None => Ok(None),
        }
    }
}
