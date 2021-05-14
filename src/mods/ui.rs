/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use chrono::offset::{Local, TimeZone};
use chrono::Local as LocalTz;
use futures::task::{AtomicWaker, Context, Poll};
use futures::Stream;
use linked_hash_set::LinkedHashSet;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io::Error as IoError;
use std::io::{Read, Stdout, Write};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use termion::color;
use termion::event::{parse_event as termion_parse_event, Event as TermionEvent, Key};
use termion::get_tty;
use termion::raw::IntoRawMode;
use termion::screen::AlternateScreen;
use uuid::Uuid;
use xmpp_parsers::{BareJid, Jid};

use crate::color::id_to_rgb;
use crate::command::Command;
use crate::conversation::{Channel, Chat, Conversation};
use crate::core::{Aparte, Event, ModTrait};
use crate::cursor::Cursor;
use crate::i18n;
use crate::message::{Direction, Message, XmppMessageType};
use crate::terminus::{
    self, BufferedWin, Dimension, FrameLayout, Input, Layout, Layouts, LinearLayout, ListView,
    Orientation, Screen, View, Window as _,
};
use crate::{contact, conversation};

enum UIEvent {
    Core(Event),
    Validate(Rc<RefCell<Option<(String, bool)>>>),
    GetInput(Rc<RefCell<Option<(String, Cursor, bool)>>>),
    AddWindow(String, Option<Box<dyn View<UIEvent, Stdout>>>),
}

struct TitleBar {
    name: Option<String>,
    subjects: HashMap<String, HashMap<String, String>>,
    dirty: bool,
}

impl TitleBar {
    fn new() -> Self {
        Self {
            name: None,
            subjects: HashMap::new(),
            dirty: true,
        }
    }

    fn set_name(&mut self, name: &str) {
        self.name = Some(name.to_string());
        self.subjects
            .entry(name.to_string())
            .or_insert(HashMap::new());
        self.dirty = true;
    }

    fn add_subjects(&mut self, jid: String, subjects: HashMap<String, String>) {
        if Some(&jid) == self.name.as_ref() {
            self.dirty = true;
        }
        self.subjects.insert(jid, subjects);
    }
}

impl<W> View<UIEvent, W> for TitleBar
where
    W: Write,
{
    fn render(&mut self, dimension: &Dimension, screen: &mut Screen<W>) {
        save_cursor!(screen);

        vprint!(
            screen,
            "{}",
            termion::cursor::Goto(dimension.x, dimension.y)
        );
        vprint!(
            screen,
            "{}{}",
            color::Bg(color::Blue),
            color::Fg(color::White)
        );

        vprint!(screen, "{}", " ".repeat(dimension.w.unwrap().into()));

        vprint!(
            screen,
            "{}",
            termion::cursor::Goto(dimension.x, dimension.y)
        );

        if let Some(name) = &self.name {
            let clean_name = terminus::term_string_visible_truncate(
                name,
                dimension.w.unwrap().into(),
                Some("…"),
            );
            vprint!(screen, "{}", clean_name);

            let remaining = dimension.w.unwrap()
                - terminus::term_string_visible_len(&clean_name) as u16
                - " – ".len() as u16;
            if remaining > 0 {
                let subjects = self.subjects.get(name).unwrap();
                if !subjects.is_empty() {
                    if let Some((_lang, subject)) = i18n::get_best(subjects, vec![]) {
                        let clean_subject = terminus::term_string_visible_truncate(
                            subject,
                            remaining.into(),
                            Some("…"),
                        );
                        vprint!(screen, " — {}", clean_subject);
                    }
                }
            }
        }

        vprint!(
            screen,
            "{}{}",
            color::Bg(color::Reset),
            color::Fg(color::Reset)
        );

        restore_cursor!(screen);
        flush!(screen);
        self.dirty = false;
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn event(&mut self, event: &mut UIEvent) {
        match event {
            UIEvent::Core(Event::ChangeWindow(name)) => {
                self.set_name(name);
            }
            UIEvent::Core(Event::Subject(_, jid, subjects)) => {
                let window: BareJid = jid.clone().into();
                self.add_subjects(
                    window.to_string(),
                    subjects
                        .iter()
                        .map(|(lang, subject)| (lang.clone(), terminus::clean(subject)))
                        .collect(),
                );
            }
            _ => {}
        }
    }

    fn get_layouts(&self) -> Layouts {
        Layouts {
            width: Layout::MatchParent,
            height: Layout::Absolute(1),
        }
    }
}

struct WinBar {
    connection: Option<String>,
    windows: Vec<String>,
    current_window: Option<String>,
    highlighted: Vec<String>,
    dirty: bool,
}

impl WinBar {
    pub fn new() -> Self {
        Self {
            connection: None,
            windows: Vec::new(),
            current_window: None,
            highlighted: Vec::new(),
            dirty: true,
        }
    }

    pub fn add_window(&mut self, window: String) {
        self.windows.push(window);
        self.dirty = true;
    }

    pub fn set_current_window(&mut self, window: &str) {
        self.current_window = Some(window.to_string());
        // could use self.highlighted.drain_filter(|w| w == &window);
        let mut i = 0;
        while i != self.highlighted.len() {
            if self.highlighted[i] == window {
                self.highlighted.remove(i);
            } else {
                i += 1;
            }
        }
        self.dirty = true;
    }

    pub fn highlight_window(&mut self, window: &str) {
        if self.highlighted.iter().find(|w| w == &window).is_none() {
            self.highlighted.push(window.to_string());
            self.dirty = true;
        }
    }
}

impl<W> View<UIEvent, W> for WinBar
where
    W: Write,
{
    fn render(&mut self, dimension: &Dimension, screen: &mut Screen<W>) {
        save_cursor!(screen);

        let mut written = 0;

        vprint!(
            screen,
            "{}",
            termion::cursor::Goto(dimension.x, dimension.y)
        );
        vprint!(
            screen,
            "{}{}",
            color::Bg(color::Blue),
            color::Fg(color::White)
        );

        for _ in 0..dimension.w.unwrap() {
            vprint!(screen, " ");
        }

        vprint!(
            screen,
            "{}",
            termion::cursor::Goto(dimension.x, dimension.y)
        );
        if let Some(connection) = &self.connection {
            vprint!(screen, " {}", connection);
            written += 1 + connection.len();
        }

        let mut first = true;
        for window in &self.highlighted {
            // Keep space for at least ", …]"
            if window.len() + written + 4 > dimension.w.unwrap() as usize {
                if !first {
                    vprint!(screen, ", …");
                }
                break;
            }

            if first {
                vprint!(screen, " [");
                written += 3; // Also count the closing bracket
                first = false;
            } else {
                vprint!(screen, ", ");
                written += 2;
            }
            vprint!(
                screen,
                "{}{}{}",
                termion::style::Bold,
                window,
                termion::style::NoBold
            );
            written += window.len();
        }

        if !first {
            vprint!(screen, "]");
        }

        vprint!(
            screen,
            "{}{}",
            color::Bg(color::Reset),
            color::Fg(color::Reset)
        );

        restore_cursor!(screen);
        flush!(screen);
        self.dirty = false;
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn event(&mut self, event: &mut UIEvent) {
        match event {
            UIEvent::Core(Event::ChangeWindow(name)) => {
                self.set_current_window(&terminus::clean(name));
            }
            UIEvent::AddWindow(name, _) => {
                self.add_window(terminus::clean(name));
            }
            UIEvent::Core(Event::Connected(account, _)) => {
                self.connection = Some(terminus::clean(&account.to_string()));
                self.dirty = true;
            }
            UIEvent::Core(Event::Message(_, Message::Xmpp(message))) => {
                let mut highlighted = None;
                let messaged_window = terminus::clean(&message.from.to_string());
                for window in &self.windows {
                    if &messaged_window == window && Some(window) != self.current_window.as_ref() {
                        highlighted = Some(window.clone());
                    }
                }
                if highlighted.is_some() {
                    self.highlight_window(&highlighted.unwrap());
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    fn get_layouts(&self) -> Layouts {
        Layouts {
            width: Layout::MatchParent,
            height: Layout::Absolute(1),
        }
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Message::Log(message) => {
                let timestamp = Local.from_utc_datetime(&message.timestamp.naive_local());
                for line in message.body.lines() {
                    write!(
                        f,
                        "{}{} - {}\n",
                        color::Fg(color::White),
                        timestamp.format("%T"),
                        line
                    )?;
                }

                Ok(())
            }
            Message::Xmpp(message) => {
                let author = terminus::clean(
                    &match &message.type_ {
                        XmppMessageType::Channel => match &message.from_full {
                            Jid::Full(from) => from.resource.clone(),
                            Jid::Bare(from) => from.to_string(),
                        },
                        XmppMessageType::Chat => message.from.to_string(),
                    }
                    .to_string(),
                );

                let timestamp =
                    Local.from_utc_datetime(&message.get_original_timestamp().naive_local());
                let body = message.get_last_body();
                let me = body.starts_with("/me");
                let padding_len = match me {
                    true => format!("{} - {}: ", timestamp.format("%T"), author).len(),
                    false => format!("{} - * {}", timestamp.format("%T"), author).len(),
                };
                let padding = " ".repeat(padding_len);

                let (r, g, b) = id_to_rgb(&author);

                let mut attributes = "".to_string();
                if message.has_multiple_version() {
                    attributes.push_str("✎ ");
                }

                match me {
                    true => write!(
                        f,
                        "{}{} - {}* {}{}{}",
                        color::Fg(color::White),
                        timestamp.format("%T"),
                        attributes,
                        color::Fg(color::Rgb(r, g, b)),
                        author,
                        color::Fg(color::White)
                    ),
                    false => write!(
                        f,
                        "{}{} - {}{}{}:{} ",
                        color::Fg(color::White),
                        timestamp.format("%T"),
                        attributes,
                        color::Fg(color::Rgb(r, g, b)),
                        author,
                        color::Fg(color::White)
                    ),
                }?;

                let mut iter = match me {
                    true => body.strip_prefix("/me").unwrap().lines(),
                    false => body.lines(),
                };

                if let Some(line) = iter.next() {
                    write!(f, "{}", terminus::clean(line))?;
                }
                while let Some(line) = iter.next() {
                    write!(f, "\n{}{}", padding, terminus::clean(line))?;
                }

                Ok(())
            }
        }
    }
}

impl fmt::Display for contact::Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}{}",
            color::Fg(color::Yellow),
            terminus::clean(&self.0),
            color::Fg(color::White)
        )
    }
}

impl fmt::Display for contact::Contact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.presence {
            contact::Presence::Available | contact::Presence::Chat => {
                write!(f, "{}", color::Fg(color::Green))?
            }
            contact::Presence::Away
            | contact::Presence::Dnd
            | contact::Presence::Xa
            | contact::Presence::Unavailable => write!(f, "{}", color::Fg(color::White))?,
        };

        match &self.name {
            Some(name) => write!(
                f,
                "{} ({}){}",
                terminus::clean(name),
                terminus::clean(&self.jid.to_string()),
                color::Fg(color::White)
            ),
            None => write!(
                f,
                "{}{}",
                terminus::clean(&self.jid.to_string()),
                color::Fg(color::White)
            ),
        }
    }
}

impl fmt::Display for contact::Bookmark {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.name {
            Some(name) => write!(f, "{}{}", terminus::clean(name), color::Fg(color::White)),
            None => write!(
                f,
                "{}{}",
                terminus::clean(&self.jid.to_string()),
                color::Fg(color::White)
            ),
        }
    }
}

#[derive(Clone, Debug, Ord, PartialOrd)]
pub enum RosterItem {
    Contact(contact::Contact),
    Bookmark(contact::Bookmark),
    Window(String),
}

impl Hash for RosterItem {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Contact(contact) => contact.jid.hash(state),
            Self::Bookmark(bookmark) => bookmark.jid.hash(state),
            Self::Window(window) => window.hash(state),
        };
    }
}

impl PartialEq for RosterItem {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Contact(a), Self::Contact(b)) => a.eq(b),
            (Self::Bookmark(a), Self::Bookmark(b)) => a.eq(b),
            (Self::Window(a), Self::Window(b)) => a.eq(b),
            _ => false,
        }
    }
}

impl Eq for RosterItem {}

impl fmt::Display for RosterItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self {
            Self::Contact(contact) => contact.fmt(f),
            Self::Bookmark(bookmark) => bookmark.fmt(f),
            Self::Window(window) => write!(f, "{}", terminus::clean(window)),
        }
    }
}

impl fmt::Display for conversation::Occupant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (r, g, b) = id_to_rgb(&self.nick);
        let mut nick = self.nick.clone();
        if nick.len() > 40 {
            nick.truncate(39);
            nick.push('…');
        };

        write!(
            f,
            "{}{}{}",
            color::Fg(color::Rgb(r, g, b)),
            terminus::clean(&nick),
            color::Fg(color::White)
        )
    }
}

impl fmt::Display for conversation::Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            conversation::Role::Moderator => write!(
                f,
                "{}Moderators{}",
                color::Fg(color::Yellow),
                color::Fg(color::Yellow)
            ),
            conversation::Role::Participant => write!(
                f,
                "{}Participants{}",
                color::Fg(color::Yellow),
                color::Fg(color::Yellow)
            ),
            conversation::Role::Visitor => write!(
                f,
                "{}Visitors{}",
                color::Fg(color::Yellow),
                color::Fg(color::Yellow)
            ),
            conversation::Role::None => write!(
                f,
                "{}Others{}",
                color::Fg(color::Yellow),
                color::Fg(color::Yellow)
            ),
        }
    }
}

pub struct Scheduler {
    queue: Rc<RefCell<Vec<Event>>>,
}

impl Scheduler {
    pub fn schedule(&self, event: Event) {
        let mut queue = self.queue.borrow_mut();
        queue.push(event);
    }
}

pub struct UIMod {
    screen: Screen<Stdout>,
    windows: Vec<String>,
    current_window: Option<String>,
    unread_windows: LinkedHashSet<String>,
    conversations: HashMap<String, Conversation>,
    root: LinearLayout<UIEvent, Stdout>,
    dimension: Option<Dimension>,
    password_command: Option<Command>,
    outgoing_event_queue: Rc<RefCell<Vec<Event>>>,
}

impl UIMod {
    pub fn new() -> Self {
        let stdout = std::io::stdout().into_raw_mode().unwrap();
        let mut layout = LinearLayout::<UIEvent, Stdout>::new(Orientation::Vertical).with_event(
            |layout, event| {
                for child in layout.iter_children_mut() {
                    child.event(event);
                }
            },
        );

        let title_bar = TitleBar::new();
        let frame =
            FrameLayout::<UIEvent, Stdout, String>::new().with_event(|frame, event| match event {
                UIEvent::Core(Event::ChangeWindow(name)) => {
                    frame.set_current(name.to_string());
                }
                UIEvent::AddWindow(name, view) => {
                    let view = view.take().unwrap();
                    frame.insert_boxed(name.to_string(), view);

                    // propagate AddWindow with name only to each subview
                    // required at least for console view
                    for child in frame.iter_children_mut() {
                        child.event(&mut UIEvent::AddWindow(name.to_string(), None));
                    }
                }
                UIEvent::Core(Event::Key(Key::PageUp))
                | UIEvent::Core(Event::Key(Key::PageDown)) => {
                    if let Some(current) = frame.get_current_mut() {
                        current.event(event);
                    }
                }
                _ => {
                    for child in frame.iter_children_mut() {
                        child.event(event);
                    }
                }
            });
        let win_bar = WinBar::new();
        let input = Input::new().with_event(|input, event| match event {
            UIEvent::Core(Event::Key(Key::Char(c))) => input.key(*c),
            UIEvent::Core(Event::Key(Key::Backspace)) => input.backspace(),
            UIEvent::Core(Event::Key(Key::Delete)) => input.delete(),
            UIEvent::Core(Event::Key(Key::Home)) => input.home(),
            UIEvent::Core(Event::Key(Key::End)) => input.end(),
            UIEvent::Core(Event::Key(Key::Up)) => input.previous(),
            UIEvent::Core(Event::Key(Key::Down)) => input.next(),
            UIEvent::Core(Event::Key(Key::Left)) => input.left(),
            UIEvent::Core(Event::Key(Key::Right)) => input.right(),
            UIEvent::Core(Event::Key(Key::Ctrl('a'))) => input.home(),
            UIEvent::Core(Event::Key(Key::Ctrl('b'))) => input.left(),
            UIEvent::Core(Event::Key(Key::Ctrl('e'))) => input.end(),
            UIEvent::Core(Event::Key(Key::Ctrl('f'))) => input.right(),
            UIEvent::Core(Event::Key(Key::Ctrl('h'))) => input.backspace(),
            UIEvent::Core(Event::Key(Key::Ctrl('w'))) => input.backward_delete_word(),
            UIEvent::Core(Event::Key(Key::Ctrl('u'))) => input.delete_from_cursor_to_start(),
            UIEvent::Core(Event::Key(Key::Ctrl('k'))) => input.delete_from_cursor_to_end(),
            UIEvent::Validate(result) => {
                let mut result = result.borrow_mut();
                result.replace(input.validate());
            }
            UIEvent::GetInput(result) => {
                let mut result = result.borrow_mut();
                result.replace((input.buf.clone(), input.cursor.clone(), input.password));
            }
            UIEvent::Core(Event::Completed(raw_buf, cursor)) => {
                input.buf = raw_buf.clone();
                input.cursor = cursor.clone();
                input.dirty = true;
            }
            UIEvent::Core(Event::ReadPassword(_)) => input.password(),
            _ => {}
        });

        layout.push(title_bar);
        layout.push(frame);
        layout.push(win_bar);
        layout.push(input);

        Self {
            screen: AlternateScreen::from(stdout),
            root: layout,
            dimension: None,
            windows: Vec::new(),
            unread_windows: LinkedHashSet::new(),
            current_window: None,
            conversations: HashMap::new(),
            password_command: None,
            outgoing_event_queue: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn event_stream(&self) -> EventStream {
        EventStream::new()
    }

    fn get_scheduler(&self) -> Scheduler {
        Scheduler {
            queue: self.outgoing_event_queue.clone(),
        }
    }

    fn add_conversation(&mut self, _aparte: &mut Aparte, conversation: Conversation) {
        let scheduler = self.get_scheduler();
        match &conversation {
            Conversation::Chat(chat) => {
                let chat_for_event = chat.clone();
                let chatwin = BufferedWin::<UIEvent, Stdout, Message>::new().with_event(
                    move |view, event| {
                        match event {
                            UIEvent::Core(Event::Message(_, Message::Xmpp(message))) => {
                                match message.direction {
                                    // TODO check to == us
                                    Direction::Incoming => {
                                        if message.from == chat_for_event.contact {
                                            view.insert(Message::Xmpp(message.clone()));
                                        }
                                    }
                                    Direction::Outgoing => {
                                        // TODO check from == us
                                        if message.to == chat_for_event.contact {
                                            view.insert(Message::Xmpp(message.clone()));
                                        }
                                    }
                                }
                            }
                            UIEvent::Core(Event::Key(Key::PageUp)) => {
                                if view.page_up() {
                                    let from = view.first().map(|message| message.timestamp());
                                    scheduler.schedule(Event::LoadChatHistory {
                                        account: chat_for_event.account.clone(),
                                        contact: chat_for_event.contact.clone(),
                                        from: from.cloned(),
                                    });
                                }
                            }
                            UIEvent::Core(Event::Key(Key::PageDown)) => {
                                view.page_down();
                            }
                            _ => {}
                        }
                    },
                );

                self.add_window(chat.contact.to_string(), Box::new(chatwin));
                self.conversations
                    .insert(chat.contact.to_string(), conversation.clone());
            }
            Conversation::Channel(channel) => {
                let mut layout = LinearLayout::<UIEvent, Stdout>::new(Orientation::Horizontal)
                    .with_event(|layout, event| {
                        for child in layout.iter_children_mut() {
                            child.event(event);
                        }
                    });

                let channel_for_event = channel.clone();
                let chanwin = BufferedWin::<UIEvent, Stdout, Message>::new().with_event(
                    move |view, event| {
                        match event {
                            UIEvent::Core(Event::Message(_, Message::Xmpp(message))) => {
                                match message.direction {
                                    // TODO check to == us
                                    Direction::Incoming => {
                                        if message.from == channel_for_event.jid {
                                            view.insert(Message::Xmpp(message.clone()));
                                        }
                                    }
                                    Direction::Outgoing => {
                                        // TODO check from == us
                                        if message.to == channel_for_event.jid {
                                            view.insert(Message::Xmpp(message.clone()));
                                        }
                                    }
                                }
                            }
                            UIEvent::Core(Event::Key(Key::PageUp)) => {
                                if view.page_up() {
                                    let from = view.first().map(|message| message.timestamp());
                                    scheduler.schedule(Event::LoadChannelHistory {
                                        account: channel_for_event.account.clone(),
                                        jid: channel_for_event.jid.clone(),
                                        from: from.cloned(),
                                    });
                                }
                            }
                            UIEvent::Core(Event::Key(Key::PageDown)) => {
                                view.page_down();
                            }
                            _ => {}
                        }
                    },
                );
                layout.push(chanwin);

                let roster_jid = channel.jid.clone();
                let roster =
                    ListView::<UIEvent, Stdout, conversation::Role, conversation::Occupant>::new()
                        .with_layouts(Layouts {
                            width: Layout::WrapContent,
                            height: Layout::MatchParent,
                        })
                        .with_none_group()
                        .with_unique_item()
                        .with_sort_item()
                        .with_event(move |view, event| match event {
                            UIEvent::Core(Event::Occupant {
                                conversation,
                                occupant,
                                ..
                            }) => {
                                if roster_jid == *conversation {
                                    view.insert(occupant.clone(), Some(occupant.role));
                                }
                            }
                            _ => {}
                        });
                layout.push(roster);

                self.add_window(channel.get_name(), Box::new(layout));
                self.conversations
                    .insert(channel.get_name(), conversation.clone());
            }
        }
    }

    fn add_window(&mut self, name: String, window: Box<dyn View<UIEvent, Stdout>>) {
        self.windows.push(name.clone());
        self.root.event(&mut UIEvent::AddWindow(name, Some(window)));
    }

    pub fn change_window(&mut self, window: &str) {
        self.root
            .event(&mut UIEvent::Core(Event::ChangeWindow(window.to_string())));
        self.current_window = Some(window.to_string());
    }

    #[allow(unused)] // XXX Should be used when alt+arrow is fixed see https://gitlab.redox-os.org/redox-os/termion/-/issues/183
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

    #[allow(unused)] // XXX Should be used when alt+arrow is fixed see https://gitlab.redox-os.org/redox-os/termion/-/issues/183
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

    pub fn get_windows(&self) -> Vec<String> {
        self.windows.clone()
    }
}

impl ModTrait for UIMod {
    fn init(&mut self, _aparte: &mut Aparte) -> Result<(), ()> {
        vprint!(&mut self.screen, "{}", termion::clear::All);

        let (width, height) = termion::terminal_size().unwrap();
        let mut dimension = Dimension::new();
        self.root.measure(&mut dimension, Some(width), Some(height));
        self.root.layout(&mut dimension, 1, 1);
        self.root.render(&dimension, &mut self.screen);
        self.dimension = Some(dimension);

        let mut console = LinearLayout::<UIEvent, Stdout>::new(Orientation::Horizontal).with_event(
            |layout, event| {
                for (_, child_view) in layout.children.iter_mut() {
                    child_view.event(event);
                }
            },
        );
        console.push(
            BufferedWin::<UIEvent, Stdout, Message>::new().with_event(|view, event| match event {
                UIEvent::Core(Event::Message(_, Message::Log(message))) => {
                    view.insert(Message::Log(message.clone()));
                }
                UIEvent::Core(Event::Key(Key::PageUp)) => {
                    view.page_up();
                }
                UIEvent::Core(Event::Key(Key::PageDown)) => {
                    view.page_down();
                }
                _ => {}
            }),
        );
        let roster = ListView::<UIEvent, Stdout, contact::Group, RosterItem>::new()
            .with_layouts(Layouts {
                width: Layout::WrapContent,
                height: Layout::MatchParent,
            })
            .with_none_group()
            .with_sort_item()
            .with_event(|view, event| match event {
                UIEvent::Core(Event::Connected(_, _)) => {
                    view.add_group(contact::Group(String::from("Windows")));
                    view.add_group(contact::Group(String::from("Contacts")));
                    view.add_group(contact::Group(String::from("Bookmarks")));
                }
                UIEvent::Core(Event::Contact(_, contact))
                | UIEvent::Core(Event::ContactUpdate(_, contact)) => {
                    if contact.groups.len() > 0 {
                        for group in &contact.groups {
                            view.insert(RosterItem::Contact(contact.clone()), Some(group.clone()));
                        }
                    } else {
                        let group = contact::Group(String::from("Contacts"));
                        view.insert(RosterItem::Contact(contact.clone()), Some(group));
                    }
                }
                UIEvent::Core(Event::Bookmark(bookmark)) => {
                    let group = contact::Group(String::from("Bookmarks"));
                    view.insert(RosterItem::Bookmark(bookmark.clone()), Some(group));
                }
                UIEvent::Core(Event::DeletedBookmark(jid)) => {
                    let group = contact::Group(String::from("Bookmarks"));
                    let bookmark = contact::Bookmark {
                        jid: jid.clone(),
                        name: None,
                        nick: None,
                        password: None,
                        autojoin: false,
                        extensions: None,
                    };
                    let _ = view.remove(RosterItem::Bookmark(bookmark.clone()), Some(group));
                }
                UIEvent::AddWindow(name, _) => {
                    let group = contact::Group(String::from("Windows"));
                    view.insert(RosterItem::Window(name.clone()), Some(group));
                }
                _ => {}
            });
        console.push(roster);

        self.add_window("console".to_string(), Box::new(console));
        self.change_window("console");

        Ok(())
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::ReadPassword(command) => {
                self.password_command = Some(command.clone());
                self.root
                    .event(&mut UIEvent::Core(Event::ReadPassword(command.clone())));
            }
            Event::Connected(account, jid) => {
                self.root.event(&mut UIEvent::Core(Event::Connected(
                    account.clone(),
                    jid.clone(),
                )));
            }
            Event::Message(account, message) => {
                match message {
                    Message::Xmpp(message) => {
                        let window_name = match message.direction {
                            Direction::Incoming => message.from.to_string(),
                            Direction::Outgoing => message.to.to_string(),
                        };

                        if !self.conversations.contains_key(&window_name) {
                            let conversation = match message.type_ {
                                XmppMessageType::Chat => match message.direction {
                                    Direction::Incoming => Conversation::Chat(Chat {
                                        account: account.clone().unwrap(),
                                        contact: message.from.clone(),
                                    }),
                                    Direction::Outgoing => Conversation::Chat(Chat {
                                        account: account.clone().unwrap(),
                                        contact: message.to.clone(),
                                    }),
                                },
                                XmppMessageType::Channel => match message.direction {
                                    Direction::Incoming => Conversation::Channel(Channel {
                                        account: account.clone().unwrap(),
                                        jid: message.from.clone(),
                                        nick: account.as_ref().unwrap().resource.clone(),
                                        name: None,
                                        occupants: HashMap::new(),
                                    }),
                                    Direction::Outgoing => Conversation::Channel(Channel {
                                        account: account.clone().unwrap(),
                                        jid: message.to.clone(),
                                        nick: account.as_ref().unwrap().resource.clone(),
                                        name: None,
                                        occupants: HashMap::new(),
                                    }),
                                },
                            };

                            self.add_conversation(aparte, conversation);
                        }

                        if message.direction == Direction::Incoming {
                            let mut unread = None;
                            for window in &self.windows {
                                if &message.from.to_string() == window
                                    && Some(window) != self.current_window.as_ref()
                                {
                                    unread = Some(window.clone());
                                }
                            }
                            if unread.is_some() {
                                self.unread_windows.insert(unread.unwrap());
                            }
                            aparte.schedule(Event::Notification(String::from("")));
                        }
                    }
                    Message::Log(_message) => {}
                };

                self.root.event(&mut UIEvent::Core(Event::Message(
                    account.clone(),
                    message.clone(),
                )));
            }
            Event::Chat { account, contact } => {
                // Should we store account association?
                let win_name = contact.to_string();
                if !self.conversations.contains_key(&win_name) {
                    self.add_conversation(
                        aparte,
                        Conversation::Chat(Chat {
                            account: account.clone(),
                            contact: contact.clone(),
                        }),
                    );
                }
                self.change_window(&win_name);
            }
            Event::Joined {
                account,
                channel,
                user_request,
            } => {
                let bare: BareJid = channel.clone().into();
                let win_name = bare.to_string();
                if !self.conversations.contains_key(&win_name) {
                    self.add_conversation(
                        aparte,
                        Conversation::Channel(Channel {
                            account: account.clone(),
                            jid: channel.clone().into(),
                            nick: channel.resource.clone(),
                            name: None, // TODO use name from bookmark
                            occupants: HashMap::new(),
                        }),
                    );
                }
                if *user_request {
                    self.change_window(&win_name);
                }
            }
            Event::Win(window) => {
                if self.windows.contains(window) {
                    self.change_window(&window);
                } else {
                    aparte.log(format!("Unknown window {}", window));
                }
            }
            Event::WindowChange => {
                let (width, height) = termion::terminal_size().unwrap();
                let mut dimension = Dimension::new();
                self.root.measure(&mut dimension, Some(width), Some(height));
                self.root.layout(&mut dimension, 1, 1);
                self.root.render(&dimension, &mut self.screen);
                self.dimension = Some(dimension);
            }
            Event::Key(key) => {
                match key {
                    Key::Char('\t') => {
                        let result = Rc::new(RefCell::new(None));

                        let (raw_buf, cursor, password) = {
                            self.root.event(&mut UIEvent::GetInput(Rc::clone(&result)));

                            let result = result.borrow_mut();
                            result.as_ref().unwrap().clone()
                        };

                        if password {
                            aparte.schedule(Event::Key(Key::Char('\t')));
                        } else {
                            let window = self.current_window.clone().unwrap();
                            let account = match self.conversations.get(&window) {
                                Some(Conversation::Chat(chat)) => Some(chat.account.clone()),
                                Some(Conversation::Channel(channel)) => {
                                    Some(channel.account.clone())
                                }
                                _ => None,
                            };
                            aparte.schedule(Event::AutoComplete {
                                account,
                                context: window,
                                raw_buf,
                                cursor: cursor,
                            });
                        }
                    }
                    Key::Char('\n') => {
                        let result = Rc::new(RefCell::new(None));
                        // TODO avoid direct send to root, should go back to main event loop
                        self.root.event(&mut UIEvent::Validate(Rc::clone(&result)));

                        let result = result.borrow_mut();
                        let (raw_buf, password) = result.as_ref().unwrap();
                        let raw_buf = raw_buf.clone();
                        if *password {
                            let mut command = self.password_command.take().unwrap();
                            command.args.push(raw_buf.clone());
                            aparte.schedule(Event::Command(command));
                        } else if raw_buf.starts_with("/") {
                            let window = self.current_window.clone().unwrap();
                            let account = match self.conversations.get(&window) {
                                Some(Conversation::Chat(chat)) => Some(chat.account.clone()),
                                Some(Conversation::Channel(channel)) => {
                                    Some(channel.account.clone())
                                }
                                _ => None,
                            };
                            aparte.schedule(Event::RawCommand(account, window, raw_buf.clone()));
                        } else if raw_buf.len() > 0 {
                            if let Some(current_window) = self.current_window.clone() {
                                if let Some(conversation) = self.conversations.get(&current_window)
                                {
                                    match conversation {
                                        Conversation::Chat(chat) => {
                                            let account = &chat.account;
                                            let us = account.clone().into();
                                            let from: Jid = us;
                                            let to: Jid = chat.contact.clone().into();
                                            let id = Uuid::new_v4();
                                            let timestamp = LocalTz::now().into();
                                            let mut bodies = HashMap::new();
                                            bodies.insert("".to_string(), raw_buf.clone());
                                            let message = Message::outgoing_chat(
                                                id.to_string(),
                                                timestamp,
                                                &from,
                                                &to,
                                                &bodies,
                                            );
                                            aparte.schedule(Event::SendMessage(
                                                account.clone(),
                                                message,
                                            ));
                                        }
                                        Conversation::Channel(channel) => {
                                            let account = &channel.account;
                                            let mut us = account.clone();
                                            us.resource = channel.nick.clone();
                                            let from: Jid = us.into();
                                            let to: Jid = channel.jid.clone().into();
                                            let id = Uuid::new_v4();
                                            let timestamp = LocalTz::now().into();
                                            let mut bodies = HashMap::new();
                                            bodies.insert("".to_string(), raw_buf.clone());
                                            let message = Message::outgoing_channel(
                                                id.to_string(),
                                                timestamp,
                                                &from,
                                                &to,
                                                &bodies,
                                            );
                                            aparte.schedule(Event::SendMessage(
                                                account.clone(),
                                                message,
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Key::Alt('a') => {
                        if let Some(window) = self.unread_windows.pop_front() {
                            self.change_window(&window);
                        }
                    }
                    _ => {
                        aparte.schedule(Event::ResetCompletion);
                        self.root.event(&mut UIEvent::Core(Event::Key(key.clone())));
                    }
                }
            }
            Event::Completed(raw_buf, cursor) => {
                self.root.event(&mut UIEvent::Core(Event::Completed(
                    raw_buf.clone(),
                    cursor.clone(),
                )));
            }
            Event::Notification(_) => {
                vprint!(self.screen, "\x07");
                flush!(self.screen);
            }
            // Forward all unknown events
            event => self.root.event(&mut UIEvent::Core(event.clone())),
        }

        // Update rendering
        if self.root.is_layout_dirty() {
            let (width, height) = termion::terminal_size().unwrap();
            let mut dimension = Dimension::new();
            self.root.measure(&mut dimension, Some(width), Some(height));
            self.root.layout(&mut dimension, 1, 1);
            self.root.render(&dimension, &mut self.screen);
            self.dimension = Some(dimension);
        } else if self.root.is_dirty() {
            let dimension: &Dimension = self.dimension.as_ref().unwrap();
            self.root.render(dimension, &mut self.screen);
        }

        // Handle queued outgoing event
        for event in self.outgoing_event_queue.borrow_mut().drain(..) {
            aparte.schedule(event);
        }
    }
}

impl fmt::Display for UIMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Aparté UI")
    }
}

struct TermionEventStream {
    channel: mpsc::Receiver<Result<u8, IoError>>,
    waker: Arc<AtomicWaker>,
}

impl TermionEventStream {
    pub fn new() -> Self {
        let (send, recv) = mpsc::channel();
        let waker = Arc::new(AtomicWaker::new());

        let waker_for_tty = waker.clone();
        thread::spawn(move || {
            for i in get_tty().unwrap().bytes() {
                if send.send(i).is_err() {
                    return;
                }
                waker_for_tty.wake();
            }
        });

        Self {
            channel: recv,
            waker: waker,
        }
    }
}

struct IterWrapper<'a, T> {
    inner: &'a mut mpsc::Receiver<T>,
}

impl<'a, T> IterWrapper<'a, T> {
    fn new(inner: &'a mut mpsc::Receiver<T>) -> Self {
        Self { inner: inner }
    }
}

impl<'a, T> Iterator for IterWrapper<'a, T> {
    type Item = T;
    fn next(&mut self) -> Option<Self::Item> {
        match self.inner.try_recv() {
            Ok(e) => Some(e),
            Err(_) => None,
        }
    }
}

impl Stream for TermionEventStream {
    type Item = TermionEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let byte = match self.channel.try_recv() {
            Ok(Ok(byte)) => byte,
            Ok(Err(_)) => return Poll::Ready(None),
            Err(mpsc::TryRecvError::Empty) => {
                self.waker.register(cx.waker());
                return Poll::Pending;
            }
            Err(mpsc::TryRecvError::Disconnected) => return Poll::Ready(None),
        };

        let mut iter = IterWrapper::new(&mut self.channel);
        if let Ok(event) = termion_parse_event(byte, &mut iter) {
            Poll::Ready(Some(event))
        } else {
            self.waker.register(cx.waker());
            Poll::Pending
        }
    }
}

pub struct EventStream {
    inner: TermionEventStream,
}

impl EventStream {
    pub fn new() -> Self {
        Self {
            inner: TermionEventStream::new(),
        }
    }
}

impl Stream for EventStream {
    type Item = Event;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(TermionEvent::Key(key))) => {
                match key {
                    //Key::Alt('\x1b') => {
                    //    match Pin::new(&mut self.inner).poll_next(cx) {
                    //        Poll::Ready(Some(TermionEvent::Key(Key::Char('[')))) => {
                    //            match Pin::new(&mut self.inner).poll_next(cx) {
                    //                Poll::Ready(Some(TermionEvent::Key(Key::Char('C')))) => Poll::Pending,
                    //                Poll::Ready(Some(TermionEvent::Key(Key::Char('D')))) => Poll::Pending,
                    //                Poll::Ready(Some(TermionEvent::Key(_))) => Poll::Pending,
                    //                Poll::Ready(Some(TermionEvent::Key(_))) => Poll::Pending,
                    //                Poll::Ready(None) => Poll::Pending,
                    //                Poll::Pending => Poll::Pending,
                    //            }
                    //        },
                    //        _ => Poll::Pending,
                    //    };
                    //},
                    Key::Char(c) => Poll::Ready(Some(Event::Key(Key::Char(c)))),
                    Key::Backspace => Poll::Ready(Some(Event::Key(Key::Backspace))),
                    Key::Delete => Poll::Ready(Some(Event::Key(Key::Delete))),
                    Key::Home => Poll::Ready(Some(Event::Key(Key::Home))),
                    Key::End => Poll::Ready(Some(Event::Key(Key::End))),
                    Key::Up => Poll::Ready(Some(Event::Key(Key::Up))),
                    Key::Down => Poll::Ready(Some(Event::Key(Key::Down))),
                    Key::Left => Poll::Ready(Some(Event::Key(Key::Left))),
                    Key::Right => Poll::Ready(Some(Event::Key(Key::Right))),
                    Key::Ctrl(c) => Poll::Ready(Some(Event::Key(Key::Ctrl(c)))),
                    Key::Alt(c) => Poll::Ready(Some(Event::Key(Key::Alt(c)))),
                    Key::PageUp => Poll::Ready(Some(Event::Key(Key::PageUp))),
                    Key::PageDown => Poll::Ready(Some(Event::Key(Key::PageDown))),
                    _ => {
                        self.inner.waker.register(cx.waker());
                        Poll::Pending
                    }
                }
            }
            Poll::Ready(Some(TermionEvent::Mouse(_))) => {
                self.inner.waker.register(cx.waker());
                Poll::Pending
            }
            Poll::Ready(Some(TermionEvent::Unsupported(_))) => {
                self.inner.waker.register(cx.waker());
                Poll::Pending
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => {
                self.inner.waker.register(cx.waker());
                Poll::Pending
            }
        }
    }
}
