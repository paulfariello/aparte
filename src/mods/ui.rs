/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use backtrace::Backtrace;
use chrono::Local as LocalTz;
use futures::task::{AtomicWaker, Context, Poll};
use futures::Stream;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io::{Error as IoError, ErrorKind as IoErrorKind};
use std::io::{Read, Stdout, Write};
use std::os::fd::AsFd;
use std::panic;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use termion::color;
use termion::event::{parse_event as termion_parse_event, Event as TermionEvent, Key};
use termion::get_tty;
use termion::raw::IntoRawMode;
use termion::screen::IntoAlternateScreen;
use uuid::Uuid;
use xmpp_parsers::{BareJid, Jid};

use crate::color::{id_to_rgb, ColorTuple};
use crate::command::Command;
use crate::config::Config;
use crate::conversation::{Channel, Chat, Conversation};
use crate::core::{Aparte, Event, ModTrait};
use crate::cursor::Cursor;
use crate::i18n;
use crate::message::{Direction, Message, XmppMessageType};
use crate::terminus::{
    self, BufferedScreen, Dimension, DimensionSpec, FrameLayout, Input, Layout, Layouts,
    LinearLayout, ListView, Orientation, Screen, ScrollWin, View,
};
use crate::{contact, conversation};

// Debounce rendering at 350ms pace (based on Doherty Threshold)
const UI_DEBOUNCE_NS: u32 = 35_000_000u32;

enum UIEvent {
    Core(Event),
    Validate(Rc<RefCell<Option<(String, bool)>>>),
    GetInput(Rc<RefCell<Option<(String, Cursor, bool)>>>),
    AddWindow(String, Option<Box<dyn View<UIEvent, Stdout>>>),
}

struct TitleBar {
    name: Option<String>,
    subjects: HashMap<String, HashMap<String, String>>,
    dirty: Cell<bool>,
    pub color: ColorTuple,
}

impl TitleBar {
    fn new(color: &ColorTuple) -> Self {
        Self {
            name: None,
            subjects: HashMap::new(),
            dirty: Cell::new(true),
            color: color.clone(),
        }
    }

    fn set_name(&mut self, name: &str) {
        self.name = Some(name.to_string());
        self.subjects
            .entry(name.to_string())
            .or_insert(HashMap::new());
        self.dirty.set(true);
    }

    fn add_subjects(&mut self, jid: String, subjects: HashMap<String, String>) {
        if Some(&jid) == self.name.as_ref() {
            self.dirty.set(true);
        }
        self.subjects.insert(jid, subjects);
    }
}

impl<W> View<UIEvent, W> for TitleBar
where
    W: Write + AsFd,
{
    fn render(&self, dimension: &Dimension, screen: &mut Screen<W>) {
        save_cursor!(screen);

        vprint!(
            screen,
            "{}",
            termion::cursor::Goto(dimension.x, dimension.y)
        );
        vprint!(
            screen,
            "{}{}{}",
            self.color.bg,
            self.color.fg,
            termion::style::Bold,
        );

        vprint!(screen, "{}", " ".repeat(dimension.width.into()));

        vprint!(
            screen,
            "{}",
            termion::cursor::Goto(dimension.x, dimension.y)
        );

        if let Some(name) = &self.name {
            let clean_name =
                terminus::term_string_visible_truncate(name, dimension.width.into(), Some("…"));
            vprint!(screen, "{}", clean_name);

            let remaining = dimension.width
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
            "{}{}{}",
            color::Bg(color::Reset),
            color::Fg(color::Reset),
            termion::style::NoBold
        );

        restore_cursor!(screen);
        self.dirty.set(false);
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn event(&mut self, event: &mut UIEvent) {
        match event {
            UIEvent::Core(Event::ChangeWindow(name)) => {
                self.set_name(name);
            }
            UIEvent::Core(Event::Subject(_, jid, subjects)) => {
                let window: BareJid = jid.to_bare();
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
            width: Layout::match_parent(),
            height: Layout::absolute(1),
        }
    }
}

struct WinBar {
    connection: Option<String>,
    windows: Vec<String>,
    current_window: Option<String>,
    highlighted: HashMap<String, (u64, u64)>,
    dirty: Cell<bool>,
    pub color: ColorTuple,
}

impl WinBar {
    pub fn new(color: &ColorTuple) -> Self {
        Self {
            connection: None,
            windows: Vec::new(),
            current_window: None,
            highlighted: HashMap::new(),
            dirty: Cell::new(true),
            color: color.clone(),
        }
    }

    pub fn add_window(&mut self, window: String) {
        self.windows.push(window);
        self.dirty.set(true);
    }

    pub fn del_window(&mut self, window: &str) {
        self.windows.retain(|win| win != window);
        self.highlighted.remove(window);
        self.dirty.set(true);
    }

    pub fn set_current_window(&mut self, window: &str) {
        self.current_window = Some(window.to_string());
        self.dirty.set(self.highlighted.remove(window).is_some());
    }

    pub fn highlight_window(&mut self, window: &str, important: bool) {
        if self.current_window.as_deref() != Some(window) {
            let state = self.highlighted.entry(window.to_string()).or_insert((0, 0));
            state.0 += 1;
            if important {
                state.1 += 1;
            }
            self.dirty.set(true);
        }
    }
}

impl<W> View<UIEvent, W> for WinBar
where
    W: Write + AsFd,
{
    fn render(&self, dimension: &Dimension, screen: &mut Screen<W>) {
        save_cursor!(screen);

        let mut written = 0;

        vprint!(
            screen,
            "{}",
            termion::cursor::Goto(dimension.x, dimension.y)
        );
        vprint!(screen, "{}{}", self.color.bg, self.color.fg,);

        for _ in 0..dimension.width {
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
        let mut remaining = self.highlighted.len();

        let mut sorted = self.highlighted.iter().collect::<Vec<_>>();
        sorted.sort_by(|(_, (_, a)), (_, (_, b))| b.partial_cmp(a).unwrap());

        for (window, state) in sorted {
            // Keep space for at least ", +X]"
            let remaining_len = if remaining > 1 {
                format!("{remaining}").len() + 4
            } else {
                0
            };

            if window.len() + written + remaining_len > dimension.width as usize {
                if !first {
                    vprint!(screen, ", +{}", remaining);
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

            if state.1 > 0 {
                vprint!(
                    screen,
                    "{}{}{} ({}{}{}, {})",
                    termion::style::Bold,
                    window,
                    termion::style::NoBold,
                    termion::style::Bold,
                    state.1,
                    termion::style::NoBold,
                    state.0,
                );
                written += window.len();
                written += 5; // " (" + ", " + ")"
                written += state.0.to_string().len();
                written += state.1.to_string().len();
            } else {
                vprint!(screen, "{} ({})", window, state.0);
                written += window.len();
                written += 3; // " (" + ")"
                written += state.0.to_string().len();
            }
            remaining -= 1;
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
        self.dirty.set(false);
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn event(&mut self, event: &mut UIEvent) {
        match event {
            UIEvent::Core(Event::ChangeWindow(name)) => {
                self.set_current_window(&terminus::clean(name));
            }
            UIEvent::AddWindow(name, _) => {
                self.add_window(terminus::clean(name));
            }
            UIEvent::Core(Event::Close(window)) => {
                self.del_window(window);
            }
            UIEvent::Core(Event::Connected(account, _)) => {
                self.connection = Some(terminus::clean(&account.to_string()));
                self.dirty.set(true);
            }
            UIEvent::Core(Event::Notification {
                conversation,
                important,
            }) => {
                self.highlight_window(&conversation.get_jid().to_string(), *important);
            }
            _ => {}
        }
    }

    fn get_layouts(&self) -> Layouts {
        Layouts {
            width: Layout::match_parent(),
            height: Layout::absolute(1),
        }
    }
}

impl fmt::Display for contact::Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}{}{}{}",
            color::Bg(color::Reset),
            color::Fg(color::Yellow),
            terminus::clean(&self.0),
            color::Bg(color::Reset),
            color::Fg(color::Reset)
        )
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
            Self::Contact(contact) => {
                match contact.presence {
                    contact::Presence::Available | contact::Presence::Chat => {
                        write!(f, "{}", color::Fg(color::Green))?
                    }
                    contact::Presence::Away
                    | contact::Presence::Dnd
                    | contact::Presence::Xa
                    | contact::Presence::Unavailable => write!(f, "{}", color::Fg(color::Reset))?,
                };

                let disp = match &contact.name {
                    Some(name) => format!(
                        "{} ({})",
                        terminus::clean(name),
                        terminus::clean(&contact.jid.to_string()),
                    ),
                    None => terminus::clean(&contact.jid.to_string()),
                };

                write!(f, "{}{}", disp, color::Fg(color::Reset))
            }

            Self::Bookmark(bookmark) => {
                let disp = match &bookmark.name {
                    Some(name) => terminus::clean(name),
                    None => terminus::clean(&bookmark.jid.to_string()),
                };

                write!(f, "{}{}", disp, color::Fg(color::Reset))
            }
            Self::Window(window) => {
                let disp = terminus::clean(window);

                write!(f, "{disp}")
            }
        }
    }
}

impl fmt::Display for conversation::Occupant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (r, g, b) = id_to_rgb(&self.nick);
        let nick = self.nick.clone();

        write!(
            f,
            "{}{}{}",
            color::Fg(color::Rgb(r, g, b)),
            terminus::clean(&nick),
            color::Fg(color::Reset)
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
                color::Fg(color::Reset)
            ),
            conversation::Role::Participant => write!(
                f,
                "{}Participants{}",
                color::Fg(color::Yellow),
                color::Fg(color::Reset)
            ),
            conversation::Role::Visitor => write!(
                f,
                "{}Visitors{}",
                color::Fg(color::Yellow),
                color::Fg(color::Reset)
            ),
            conversation::Role::None => write!(
                f,
                "{}Others{}",
                color::Fg(color::Yellow),
                color::Fg(color::Reset)
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

struct PanicHandler {
    panic: Arc<Mutex<Option<String>>>,
    backtrace: Arc<Mutex<Option<Backtrace>>>,
}

impl PanicHandler {
    pub fn new() -> Self {
        let panic = Arc::new(Mutex::new(None));
        let backtrace = Arc::new(Mutex::new(None));

        let panic_for_hook = panic.clone();
        let backtrace_for_hook = backtrace.clone();
        panic::set_hook(Box::new(move |info| {
            let panic = format!("{info}");
            panic_for_hook
                .lock()
                .expect("cannot lock panic")
                .replace(panic);

            let backtrace = Backtrace::new_unresolved();
            backtrace_for_hook
                .lock()
                .expect("cannot lock backtrace")
                .replace(backtrace);
        }));

        Self { panic, backtrace }
    }
}

impl Drop for PanicHandler {
    fn drop(&mut self) {
        if let Some(panic) = self.panic.lock().expect("cannot lock panic").as_ref() {
            println!("Oops Aparté {panic}");
            log::error!("Oops Aparté {}", panic);
            println!("This isn’t normal behavior. Please report issue.");
            log::error!("This isn’t normal behavior. Please report issue.");
            if let Some(backtrace) = self
                .backtrace
                .lock()
                .expect("cannot lock backtrace")
                .as_mut()
            {
                println!("Aparté is gathering more info in logfile…");
                backtrace.resolve();
                log::error!("{:?}", backtrace);
                println!("All done.");
                let data_dir = dirs::data_dir().unwrap();
                let aparte_data = data_dir.join("aparte").join("aparte.log");
                println!("Please check {}", aparte_data.to_str().unwrap());
            }
        }
    }
}

pub struct UIMod {
    screen: Screen<Stdout>,
    windows: Vec<String>,
    current_window: Option<String>,
    unread_windows: HashMap<String, u64>,
    conversations: HashMap<String, Conversation>,
    root: LinearLayout<UIEvent, Stdout>,
    last_render: Instant,
    debounced: u32,
    dimension: Option<Dimension>,
    password_command: Option<Command>,
    outgoing_event_queue: Rc<RefCell<Vec<Event>>>,
    #[allow(dead_code)]
    panic_handler: PanicHandler, // Defining panic_handler last guarantee that it will be dropped last (after terminal restoration)
}

impl UIMod {
    pub fn new(config: &Config) -> Self {
        let stdout = std::io::stdout()
            .into_raw_mode()
            .unwrap()
            .into_alternate_screen()
            .unwrap();
        let screen = BufferedScreen::new(stdout);

        let panic_handler = PanicHandler::new();

        let mut layout = LinearLayout::<UIEvent, Stdout>::new(Orientation::Vertical).with_event(
            |layout, event| {
                for child in layout.iter_children_mut() {
                    child.event(event);
                }
            },
        );

        let title_bar = TitleBar::new(&config.theme.title_bar);
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
                UIEvent::Core(Event::Close(window)) => {
                    frame.remove(window);

                    // propagate Close with name only to each subview
                    // required at least for console view
                    for child in frame.iter_children_mut() {
                        child.event(&mut UIEvent::Core(Event::Close(window.clone())));
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
        let win_bar = WinBar::new(&config.theme.win_bar);
        let input = Input::new().with_event(|input, event| {
            if let UIEvent::Core(Event::Key(event)) = event {
                log::debug!("Input event: {:?}", event);
            }
            match event {
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
                UIEvent::Core(Event::Key(Key::CtrlLeft)) => input.word_left(),
                UIEvent::Core(Event::Key(Key::CtrlRight)) => input.word_right(),
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
                    input.dirty.set(true);
                }
                UIEvent::Core(Event::ReadPassword(_)) => input.password(),
                _ => {}
            }
        });

        layout.push(title_bar);
        layout.push(frame);
        layout.push(win_bar);
        layout.push(input);

        Self {
            screen,
            root: layout,
            dimension: None,
            windows: Vec::new(),
            unread_windows: HashMap::new(),
            current_window: None,
            conversations: HashMap::new(),
            password_command: None,
            outgoing_event_queue: Rc::new(RefCell::new(Vec::new())),
            panic_handler,
            last_render: Instant::now(),
            debounced: 0,
        }
    }

    pub fn event_stream(&self) -> EventStream {
        EventStream::default()
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
                let chatwin =
                    ScrollWin::<UIEvent, Stdout, Message>::new().with_event(move |view, event| {
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
                    });

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
                let chanwin =
                    ScrollWin::<UIEvent, Stdout, Message>::new().with_event(move |view, event| {
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
                    });
                layout.push(chanwin);

                let roster_jid = channel.jid.clone();
                let roster =
                    ListView::<UIEvent, Stdout, conversation::Role, conversation::Occupant>::new()
                        .with_layouts(Layouts {
                            width: Layout::wrap_content(),
                            height: Layout::match_parent(),
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
        } else if !self.windows.is_empty() {
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
        } else if !self.windows.is_empty() {
            self.change_window(&self.windows[0].clone());
        }
    }

    pub fn get_windows(&self) -> Vec<String> {
        self.windows.clone()
    }

    pub fn current_window(&self) -> Option<&String> {
        self.current_window.as_ref()
    }
}

impl ModTrait for UIMod {
    fn init(&mut self, _aparte: &mut Aparte) -> Result<(), ()> {
        vprint!(&mut self.screen, "{}", termion::clear::All);

        let (width, height) = termion::terminal_size().unwrap();
        let mut dimension_spec = terminus::DimensionSpec {
            width: Some(width),
            height: Some(height),
        };
        self.root.measure(&mut dimension_spec);
        let dimension = self.root.layout(&mut dimension_spec, 1, 1);
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
            ScrollWin::<UIEvent, Stdout, Message>::new()
                .with_layouts(Layouts {
                    width: Layout::wrap_content().with_relative_max(0.7),
                    height: Layout::match_parent(),
                })
                .with_event(|view, event| match event {
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
                width: Layout::wrap_content().with_relative_max(0.3),
                height: Layout::match_parent(),
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
                    if !contact.groups.is_empty() {
                        for group in &contact.groups {
                            view.insert(RosterItem::Contact(contact.clone()), Some(group.clone()));
                        }
                    } else {
                        let group = contact::Group(String::from("Contacts"));
                        view.insert(RosterItem::Contact(contact.clone()), Some(group));
                    }
                }
                UIEvent::Core(Event::Bookmark(_, bookmark)) => {
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
                    let _ = view.remove(RosterItem::Bookmark(bookmark), Some(group));
                }
                UIEvent::AddWindow(name, _) => {
                    let group = contact::Group(String::from("Windows"));
                    view.insert(RosterItem::Window(name.clone()), Some(group));
                }
                UIEvent::Core(Event::Close(window)) => {
                    let group = contact::Group(String::from("Windows"));
                    let _ = view.remove(RosterItem::Window(window.clone()), Some(group));
                }
                _ => {}
            });
        console.push(roster);

        self.add_window("console".to_string(), Box::new(console));
        self.change_window("console");

        Ok(())
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        let mut force_render = false;

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
                                        nick: account.as_ref().unwrap().resource().to_string(),
                                        name: None,
                                        occupants: HashMap::new(),
                                    }),
                                    Direction::Outgoing => Conversation::Channel(Channel {
                                        account: account.clone().unwrap(),
                                        jid: message.to.clone(),
                                        nick: account.as_ref().unwrap().resource().to_string(),
                                        name: None,
                                        occupants: HashMap::new(),
                                    }),
                                },
                            };

                            self.add_conversation(aparte, conversation);
                        }

                        if message.direction == Direction::Incoming {
                            let mut window = None;
                            for existing in &self.windows {
                                if &message.from.to_string() == existing
                                    && Some(existing) != self.current_window.as_ref()
                                {
                                    window = Some(existing.clone());
                                }
                            }

                            if window != self.current_window {
                                if let Some(window) = window {
                                    let important = self.unread_windows.entry(window).or_insert(0);
                                    *important += 1;
                                }
                            }
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
                if !self.windows.contains(&win_name) {
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
                let bare: BareJid = channel.to_bare();
                let win_name = bare.to_string();
                if !self.windows.contains(&win_name) {
                    self.add_conversation(
                        aparte,
                        Conversation::Channel(Channel {
                            account: account.clone(),
                            jid: channel.to_bare(),
                            nick: channel.resource().to_string(),
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
                    self.change_window(window);
                } else {
                    crate::info!(aparte, "Unknown window {window}");
                }
            }
            Event::WindowChange => {
                let (width, height) = termion::terminal_size().unwrap();
                let mut dimension_spec = DimensionSpec {
                    width: Some(width),
                    height: Some(height),
                };
                self.root.measure(&mut dimension_spec);
                let dimension = self.root.layout(&mut dimension_spec, 1, 1);
                self.root.render(&dimension, &mut self.screen);
                self.dimension = Some(dimension);
            }
            Event::Close(window) => {
                if window != "console" {
                    self.windows.retain(|win| win != window);
                    self.unread_windows.remove(window);
                    if Some(window) == self.current_window.as_ref() {
                        let current = self.windows.first().cloned();
                        if let Some(current) = current {
                            self.change_window(&current);
                        }
                    }
                    self.root
                        .event(&mut UIEvent::Core(Event::Close(window.clone())))
                }
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
                                cursor,
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
                            command.args.push(raw_buf);
                            aparte.schedule(Event::Command(command));
                        } else if raw_buf.starts_with('/') {
                            let window = self.current_window.clone().unwrap();
                            let account = match self.conversations.get(&window) {
                                Some(Conversation::Chat(chat)) => Some(chat.account.clone()),
                                Some(Conversation::Channel(channel)) => {
                                    Some(channel.account.clone())
                                }
                                _ => None,
                            };
                            aparte.schedule(Event::RawCommand(account, window, raw_buf));
                        } else if !raw_buf.is_empty() {
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
                                            bodies.insert("".to_string(), raw_buf);
                                            let message = Message::outgoing_chat(
                                                id.to_string(),
                                                timestamp,
                                                &from,
                                                &to,
                                                &bodies,
                                                false,
                                            );
                                            aparte.schedule(Event::SendMessage(
                                                account.clone(),
                                                message,
                                            ));
                                        }
                                        Conversation::Channel(channel) => {
                                            let account = &channel.account;
                                            let us = account
                                                .to_bare()
                                                .with_resource_str(&channel.nick)
                                                .unwrap(); // TODO avoid unwrap
                                            let from: Jid = us.into();
                                            let to: Jid = channel.jid.clone().into();
                                            let id = Uuid::new_v4();
                                            let timestamp = LocalTz::now().into();
                                            let mut bodies = HashMap::new();
                                            bodies.insert("".to_string(), raw_buf);
                                            let message = Message::outgoing_channel(
                                                id.to_string(),
                                                timestamp,
                                                &from,
                                                &to,
                                                &bodies,
                                                false,
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
                        if !self.unread_windows.is_empty() {
                            let next = {
                                let mut sorted = self.unread_windows.iter().collect::<Vec<_>>();
                                sorted.sort_by(|(_, a), (_, b)| b.partial_cmp(a).unwrap());
                                sorted[0].0.clone()
                            };

                            self.unread_windows.remove(&next);
                            self.change_window(&next);
                        }
                    }
                    _ => {
                        aparte.schedule(Event::ResetCompletion);
                        self.root.event(&mut UIEvent::Core(Event::Key(*key)));
                    }
                }
            }
            Event::Completed(raw_buf, cursor) => {
                self.root.event(&mut UIEvent::Core(Event::Completed(
                    raw_buf.clone(),
                    cursor.clone(),
                )));
            }
            Event::Notification {
                conversation,
                important,
            } => {
                if *important && aparte.config.bell {
                    vprint!(self.screen, "\x07");
                }
                self.root.event(&mut UIEvent::Core(Event::Notification {
                    conversation: conversation.clone(),
                    important: *important,
                }));
            }
            Event::UIRender => {
                log::debug!("Force render");
                force_render = true;
            }
            // Forward all unknown events
            event => self.root.event(&mut UIEvent::Core(event.clone())),
        }

        // Debounce rendering
        if force_render || self.last_render.elapsed() > Duration::new(0, UI_DEBOUNCE_NS) {
            // Update rendering
            if self.root.is_layout_dirty() {
                log::debug!("Render (saved {} rendering)", self.debounced);
                self.last_render = Instant::now();
                self.debounced = 0;

                let (width, height) = termion::terminal_size().unwrap();
                let mut dimension_spec = DimensionSpec {
                    width: Some(width),
                    height: Some(height),
                };
                self.root.measure(&mut dimension_spec);
                let dimension = self.root.layout(&mut dimension_spec, 1, 1);
                self.root.render(&dimension, &mut self.screen);
                flush!(self.screen);
                self.dimension = Some(dimension);
            } else if self.root.is_dirty() {
                log::debug!("Render (saved {} rendering)", self.debounced);
                self.last_render = Instant::now();
                self.debounced = 0;

                let dimension: &Dimension = self.dimension.as_ref().unwrap();
                self.root.render(dimension, &mut self.screen);
                flush!(self.screen);
            }
        } else {
            log::debug!("Debounce rendering");
            if self.debounced == 0 {
                // Ensure we will render this debounced event right in time
                Aparte::spawn({
                    let mut aparte = aparte.proxy();
                    async move {
                        thread::sleep(Duration::new(0, UI_DEBOUNCE_NS));
                        aparte.schedule(Event::UIRender)
                    }
                })
            }
            self.debounced += 1;
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
            let mut input = get_tty().expect("cannot get tty for stdin reading");
            let mut buf = [0u8; 256];
            loop {
                match input.read(&mut buf[..]) {
                    Ok(n) => {
                        for byte in buf[..n].iter() {
                            if send.send(Ok(*byte)).is_err() {
                                // channel has been closed, get out
                                return;
                            }
                        }
                        waker_for_tty.wake();
                    }
                    Err(err) => match err.kind() {
                        IoErrorKind::Interrupted => continue,
                        _ => {
                            log::error!("Cannot read input pipe: {}", err);
                            break;
                        }
                    },
                }
            }
        });

        Self {
            channel: recv,
            waker,
        }
    }
}

struct IterWrapper<'a, T> {
    inner: &'a mut mpsc::Receiver<T>,
}

impl<'a, T> IterWrapper<'a, T> {
    fn new(inner: &'a mut mpsc::Receiver<T>) -> Self {
        Self { inner }
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

impl Default for EventStream {
    fn default() -> Self {
        Self::new()
    }
}

impl Stream for EventStream {
    type Item = Event;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(TermionEvent::Key(key))) => match key {
                Key::Char(c) => Poll::Ready(Some(Event::Key(Key::Char(c)))),
                Key::Backspace => Poll::Ready(Some(Event::Key(Key::Backspace))),
                Key::Delete => Poll::Ready(Some(Event::Key(Key::Delete))),
                Key::Home => Poll::Ready(Some(Event::Key(Key::Home))),
                Key::End => Poll::Ready(Some(Event::Key(Key::End))),
                Key::Up => Poll::Ready(Some(Event::Key(Key::Up))),
                Key::Down => Poll::Ready(Some(Event::Key(Key::Down))),
                Key::Left => Poll::Ready(Some(Event::Key(Key::Left))),
                Key::Right => Poll::Ready(Some(Event::Key(Key::Right))),
                Key::CtrlLeft => Poll::Ready(Some(Event::Key(Key::CtrlLeft))),
                Key::CtrlRight => Poll::Ready(Some(Event::Key(Key::CtrlRight))),
                Key::Ctrl(c) => Poll::Ready(Some(Event::Key(Key::Ctrl(c)))),
                Key::Alt(c) => Poll::Ready(Some(Event::Key(Key::Alt(c)))),
                Key::PageUp => Poll::Ready(Some(Event::Key(Key::PageUp))),
                Key::PageDown => Poll::Ready(Some(Event::Key(Key::PageDown))),
                _ => {
                    self.inner.waker.register(cx.waker());
                    Poll::Pending
                }
            },
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
