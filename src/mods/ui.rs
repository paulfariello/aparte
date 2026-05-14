/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use backtrace::Backtrace;
use chrono::{DateTime, FixedOffset, Local as LocalTz};
use crossterm::event::{
    Event as CrosstermEvent, EventStream as CrosstermEventStream, KeyCode, KeyEvent, KeyModifiers,
};
use crossterm::{execute, terminal};
use futures::task::{Context, Poll};
use futures::Stream;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::panic;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::RwLock;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use terminus::charxel::{CharxelDisplay, Charxels, IntoCharxels};
use terminus::linear_layout::LayoutChild;
use terminus::rendering::{OffscreenRenderBuffer, ScreenFrame};
use terminus::Style;
use terminus::{
    self,
    cursor::Cursor,
    frame_layout::FrameLayout,
    input::Input,
    linear_layout::{LinearLayout, Orientation},
    list_view::ListView,
    scroll_win::ScrollWin,
    CursorStyle, Dimensions, LayoutParam, LayoutParams, MeasureSpec, MeasureSpecs,
    RequestedDimension, RequestedDimensions, View,
};
use uuid::Uuid;
use xmpp_parsers::jid::{BareJid, Jid};

use radix_trie::Trie;

use crate::color::id_to_rgb;
use crate::command::Command;
use crate::config::{Config, Theme};
use crate::conversation::{Channel, Chat, Conversation};
use crate::core::{Aparte, Event, ModTrait, UIMode as Mode};
use crate::i18n;
use crate::message::{Direction, Message, MessageView, XmppMessageType};
use crate::mods::bookmarks::BookmarksMod;
use crate::mods::messages::MessagesMod;
use crate::mods::omemo::OmemoEvent;
use crate::{contact, conversation};

#[derive(Clone, Debug)]
enum NormalCommand {
    SelectNext,
    SelectPrev,
    ScrollToTop,
    ScrollToBottom,
    SearchFirst(String),
    SearchNext,
    SearchPrev,
    SearchCancel,
}

fn build_normal_command_trie() -> Trie<String, NormalCommand> {
    let mut trie = Trie::new();
    trie.insert("j".to_string(), NormalCommand::SelectNext);
    trie.insert("k".to_string(), NormalCommand::SelectPrev);
    trie.insert("gg".to_string(), NormalCommand::ScrollToTop);
    trie.insert("G".to_string(), NormalCommand::ScrollToBottom);
    trie.insert("n".to_string(), NormalCommand::SearchNext);
    trie.insert("N".to_string(), NormalCommand::SearchPrev);
    trie
}

#[allow(clippy::large_enum_variant)]
enum UIEvent {
    Core(Event),
    Validate(Rc<RefCell<Option<(String, bool)>>>),
    GetInput(Rc<RefCell<Option<(String, Cursor, bool)>>>),
    AddWindow(
        String,
        Option<String>,
        Option<Box<dyn View<UIEvent, Theme>>>,
    ),
    ModeChange(Mode),
    NormalCommand(NormalCommand),
    CommandBufferUpdate(String),
    SetInput(String),
    ReduceHighlight(String, u64, u64),
}

fn insert_message(view: &mut ScrollWin<UIEvent, MessageView, Theme>, msg_view: MessageView) {
    let pred_date = view
        .predecessor(&msg_view)
        .map(|p| p.message.timestamp().with_timezone(&LocalTz).date_naive());
    let msg_date = msg_view
        .message
        .timestamp()
        .with_timezone(&LocalTz)
        .date_naive();
    msg_view.set_show_date_sep(pred_date.is_some_and(|d| d != msg_date));

    if let Some(succ) = view.successor(&msg_view) {
        let succ_date = succ
            .message
            .timestamp()
            .with_timezone(&LocalTz)
            .date_naive();
        succ.set_show_date_sep(msg_date != succ_date);
    }

    view.replace(msg_view);
}

struct TitleBar {
    current_jid: Option<String>,
    display_name: Option<String>,
    display_names: HashMap<String, String>,
    mode: Mode,
    connection: Option<String>,
    subjects: HashMap<String, HashMap<String, String>>,
    dimensions: Option<Dimensions>,
    preferred_langs: Vec<String>,
    encrypted_jids: HashSet<String>,
}

impl TitleBar {
    fn new(preferred_langs: Vec<String>) -> Self {
        Self {
            current_jid: None,
            display_name: None,
            display_names: HashMap::new(),
            mode: Mode::Insert,
            connection: None,
            subjects: HashMap::new(),
            dimensions: None,
            preferred_langs,
            encrypted_jids: HashSet::new(),
        }
    }

    fn set_window(&mut self, jid: &str) {
        self.current_jid = Some(jid.to_string());
        self.display_name = self
            .display_names
            .get(jid)
            .cloned()
            .or_else(|| Some(jid.to_string()));
        self.subjects.entry(jid.to_string()).or_default();
    }

    fn add_subjects(&mut self, jid: String, subjects: HashMap<String, String>) {
        self.subjects.insert(jid, subjects);
    }
}

impl View<UIEvent, Theme> for TitleBar {
    fn measure(&self, _measure_specs: &MeasureSpecs) -> RequestedDimensions {
        RequestedDimensions {
            height: RequestedDimension::Absolute(1),
            width: RequestedDimension::ExpandMax,
        }
    }

    fn layout(&mut self, dimensions: &Dimensions) {
        log::debug!("layout {} {:?}", std::any::type_name::<Self>(), dimensions);
        self.dimensions.replace(dimensions.clone());
    }

    fn render(&self, mut frame: ScreenFrame, config: &Theme) {
        log::debug!(
            "rendering {} at {:?}",
            std::any::type_name::<Self>(),
            self.dimensions
        );

        frame.set_background(config.title_bar.bg);
        frame.set_foreground(config.title_bar.fg);

        let mut frame_space = frame.width();

        let mode_label = match self.mode {
            Mode::Insert => " INSERT ",
            Mode::Normal => " NORMAL ",
            Mode::Command => " COMMAND ",
        };
        let mode_charxels = mode_label
            .with_style(Style::Bold)
            .with_color(&config.title_bar_mode);
        let mode_width = mode_charxels.display_width();
        if frame.width() >= mode_width {
            frame.write(&mode_charxels);
            frame_space -= mode_width;
        }

        if let Some(connection) = &self.connection {
            let connection = format!(" {} |", connection)
                .into_charxels()
                .with_color(&config.title_bar);
            let connection_width = connection.display_width();
            if frame_space > connection_width {
                frame.write(&connection);
                frame_space -= connection_width;
            }
        }

        if let Some(display) = &self.display_name {
            let is_encrypted = self
                .current_jid
                .as_deref()
                .map(|jid| self.encrypted_jids.contains(jid))
                .unwrap_or(false);
            let prefix = if is_encrypted { " 🔒 " } else { " " };
            let mut title = format!("{}{}", prefix, display).into_charxels();

            let subjects = self
                .current_jid
                .as_deref()
                .and_then(|jid| self.subjects.get(jid));
            if let Some(subjects) = subjects {
                if let Some((_lang, subject)) = i18n::get_best(
                    subjects,
                    self.preferred_langs.iter().map(|s| s.as_str()).collect(),
                ) {
                    if let Some(subject) = subject.lines().next() {
                        title.append(" – ");
                        title.append(subject);
                    }
                }
            }

            title.truncate(frame_space, "…");
            title = title
                .with_styles(&[Style::Bold])
                .with_color(&config.title_bar);
            frame.write(title);
        }
    }

    fn event(&mut self, event: &mut UIEvent) {
        match event {
            UIEvent::Core(Event::Connected(account, _)) => {
                self.connection = Some(terminus::clean_str(&account.to_string()));
            }
            UIEvent::Core(Event::ChangeWindow(name)) => {
                self.set_window(name);
            }
            UIEvent::AddWindow(jid, Some(name), _) => {
                self.display_names.insert(jid.clone(), name.clone());
            }
            UIEvent::Core(Event::Subject(_, jid, subjects)) => {
                let window: BareJid = jid.to_bare();
                self.add_subjects(
                    window.to_string(),
                    subjects
                        .iter()
                        .map(|(lang, subject)| (lang.clone(), terminus::clean_str(subject)))
                        .collect(),
                );
            }
            UIEvent::ModeChange(mode) => {
                self.mode = *mode;
            }
            UIEvent::Core(Event::Omemo(OmemoEvent::Enabled { jid, .. })) => {
                self.encrypted_jids.insert(jid.to_string());
            }
            _ => {}
        }
    }
}

struct WinBar {
    windows: Vec<String>,
    display_names: HashMap<String, String>,
    current_window: Option<String>,
    highlighted: HashMap<String, (u64, u64)>,
    dimensions: Option<Dimensions>,
    command_buffer: String,
}

impl WinBar {
    pub fn new() -> Self {
        Self {
            windows: Vec::new(),
            display_names: HashMap::new(),
            current_window: None,
            highlighted: HashMap::new(),
            dimensions: None,
            command_buffer: String::new(),
        }
    }

    pub fn add_window(&mut self, window: String) {
        self.windows.push(window);
    }

    pub fn del_window(&mut self, window: &str) {
        self.windows.retain(|win| win != window);
        self.highlighted.remove(window);
    }

    pub fn set_current_window(&mut self, window: &str) {
        self.current_window = Some(window.to_string());
        self.highlighted.remove(window);
    }

    pub fn highlight_window(&mut self, window: &str, important: bool) {
        if self.current_window.as_deref() != Some(window) {
            let state = self.highlighted.entry(window.to_string()).or_insert((0, 0));
            state.0 += 1;
            if important {
                state.1 += 1;
            }
        }
    }

    pub fn reduce_highlight(&mut self, window: &str, total: u64, important: u64) {
        if let Some(state) = self.highlighted.get_mut(window) {
            state.0 = state.0.saturating_sub(total);
            state.1 = state.1.saturating_sub(important);
            if state.0 == 0 {
                self.highlighted.remove(window);
            }
        }
    }
}

impl View<UIEvent, Theme> for WinBar {
    fn measure(&self, _measure_specs: &MeasureSpecs) -> RequestedDimensions {
        RequestedDimensions {
            height: RequestedDimension::Absolute(1),
            width: RequestedDimension::ExpandMax,
        }
    }

    fn layout(&mut self, dimensions: &Dimensions) {
        log::debug!("layout {} {:?}", std::any::type_name::<Self>(), dimensions);
        self.dimensions.replace(dimensions.clone());
    }

    fn render(&self, mut frame: ScreenFrame, config: &Theme) {
        log::debug!(
            "rendering {} at {:?}",
            std::any::type_name::<Self>(),
            self.dimensions
        );

        let mut frame_space = frame.width();

        let mut first = true;
        let mut remaining = self.highlighted.len();

        let mut sorted = self.highlighted.iter().collect::<Vec<_>>();
        sorted.sort_by(|(_, (_, a)), (_, (_, b))| b.partial_cmp(a).unwrap());

        if !sorted.is_empty() {
            frame.write(" ");
            frame_space -= 3; // Subtract space and enclosing []

            for (window, state) in sorted {
                // Ensure at all time that we can close hl and add remaining info
                let remaining_charxels = if remaining > 0 {
                    format!("+{}", remaining).into_charxels()
                } else {
                    Charxels::default()
                };

                if !first {
                    frame.write(" | ");
                    frame_space -= 2;
                }

                let label = self
                    .display_names
                    .get(window.as_str())
                    .map_or(window.as_str(), String::as_str);
                let highlighted = if state.1 > 0 {
                    let mut highlighted = label.with_style(Style::Bold);
                    highlighted.append(" (");
                    highlighted.append(format!("{}", state.1).with_style(Style::Bold));
                    highlighted.append(format!(", {})", state.0));
                    highlighted
                } else {
                    format!("{} ({})", label, state.0).into_charxels()
                };

                // Don't write current hl if we can't put remaining info afterward
                if highlighted.display_width() + remaining_charxels.display_width() >= frame_space {
                    // We are sure that previous hl has let us enough space for remaining info
                    frame.write(&remaining_charxels);
                    break;
                } else {
                    frame.write(&highlighted);
                    frame_space -= highlighted.display_width();
                }

                first = false;
                remaining -= 1;
            }
        }

        if !self.command_buffer.is_empty() {
            let cmd_buf = self.command_buffer.as_str().with_style(Style::Bold);
            let cmd_buf_width = cmd_buf.display_width();
            if frame.width() >= cmd_buf_width {
                frame.write_at((frame.width() - cmd_buf_width, 0u16), &cmd_buf);
            }
        }

        frame.set_background(config.win_bar.bg);
        frame.set_foreground(config.win_bar.fg);
    }

    fn event(&mut self, event: &mut UIEvent) {
        match event {
            UIEvent::Core(Event::ChangeWindow(name)) => {
                self.set_current_window(&terminus::clean_str(name));
            }
            UIEvent::AddWindow(jid, display_name, _) => {
                if let Some(name) = display_name {
                    self.display_names.insert(jid.clone(), name.clone());
                }
                self.add_window(terminus::clean_str(jid));
            }
            UIEvent::Core(Event::Close(window)) => {
                self.del_window(window);
            }
            UIEvent::Core(Event::Notification {
                conversation,
                important,
                ..
            }) => {
                self.highlight_window(&conversation.get_jid().to_string(), *important);
            }
            UIEvent::ReduceHighlight(window, total, important) => {
                self.reduce_highlight(window, *total, *important);
            }
            UIEvent::CommandBufferUpdate(buf) => {
                self.command_buffer = buf.clone();
            }
            _ => {}
        }
    }
}

impl CharxelDisplay<Theme> for contact::Group {
    fn colored_fmt(&self, config: &Theme) -> Charxels {
        self.0.clone().with_foreground(config.roster_group_fg)
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

impl CharxelDisplay<Theme> for RosterItem {
    fn colored_fmt(&self, config: &Theme) -> Charxels {
        match &self {
            Self::Contact(contact) => {
                let fg = match contact.presence {
                    contact::Presence::Available | contact::Presence::Chat => {
                        config.roster_available_fg
                    }
                    contact::Presence::Away
                    | contact::Presence::Dnd
                    | contact::Presence::Xa
                    | contact::Presence::Unavailable => config.roster_unavailable_fg,
                };

                let disp = match &contact.name {
                    Some(name) => format!(
                        "{} ({})",
                        terminus::clean_str(name),
                        terminus::clean_str(&contact.jid.to_string()),
                    ),
                    None => terminus::clean_str(&contact.jid.to_string()),
                };

                disp.with_foreground(fg)
            }

            Self::Bookmark(bookmark) => match &bookmark.name {
                Some(name) => name.into_charxels(),
                None => bookmark.jid.to_string().into_charxels(),
            },
            Self::Window(window) => window.into_charxels(),
        }
    }
}

impl CharxelDisplay<Theme> for conversation::Occupant {
    fn colored_fmt(&self, _config: &Theme) -> Charxels {
        self.nick
            .clone()
            .with_foreground(terminus::FgColor(id_to_rgb(&self.nick)))
    }
}

impl CharxelDisplay<Theme> for conversation::Role {
    fn colored_fmt(&self, config: &Theme) -> Charxels {
        let fg = config.roster_role_fg;
        match self {
            conversation::Role::Moderator => "Moderators",
            conversation::Role::Participant => "Participants",
            conversation::Role::Visitor => "Visitors",
            conversation::Role::None => "Others",
        }
        .with_foreground(fg)
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
        // Reset terminal state before printing so output is visible regardless
        // of whether this is a clean shutdown or a crash. These are no-ops when
        // the terminal was never set up (e.g. in tests).
        let _ = execute!(
            std::io::stdout(),
            crossterm::cursor::Show,
            terminal::LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();

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
    pub render_buffer: Arc<RwLock<OffscreenRenderBuffer>>,
    windows: Vec<String>,
    current_window: Option<String>,
    unread_windows: HashMap<String, VecDeque<(DateTime<FixedOffset>, bool)>>,
    conversations: HashMap<String, Conversation>,
    jid_to_name: HashMap<BareJid, String>,
    root: LinearLayout<UIEvent, Theme>,
    dirty: bool,
    password_command: Option<Command>,
    current_mode: Mode,
    outgoing_event_queue: Rc<RefCell<Vec<Event>>>,
    _panic_handler: PanicHandler, // Defining panic_handler last guarantee that it will be dropped last (after terminal restoration)
    dimensions: Dimensions,
}

impl UIMod {
    pub fn new(_config: &Config) -> Self {
        let screen = Arc::new(RwLock::new(OffscreenRenderBuffer::default()));
        let panic_handler = PanicHandler::new();
        let (width, height) = crossterm::terminal::size().unwrap();
        RwLock::write(&screen)
            .unwrap()
            .set_size((width, height).into());

        Self {
            render_buffer: screen,
            root: LinearLayout::new(Orientation::Vertical),
            windows: Vec::new(),
            unread_windows: HashMap::new(),
            current_window: None,
            conversations: HashMap::new(),
            jid_to_name: HashMap::new(),
            password_command: None,
            current_mode: Mode::Insert,
            outgoing_event_queue: Rc::new(RefCell::new(Vec::new())),
            _panic_handler: panic_handler,
            dirty: true,
            dimensions: Dimensions {
                top: 1,
                left: 1,
                height,
                width,
            },
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

    fn add_conversation(&mut self, aparte: &mut Aparte, conversation: Conversation) {
        let scheduler = self.get_scheduler();
        let selection_bg = aparte.config.theme.selected_message;
        let search_highlight_fg = aparte.config.theme.search_highlight_fg;
        let search_highlight_bg = aparte.config.theme.search_highlight_bg;
        match &conversation {
            Conversation::Chat(chat) => {
                let chat_for_event = chat.clone();
                let chatwin = ScrollWin::<UIEvent, MessageView, Theme>::new().with_event({
                    let mut aparte = aparte.proxy();
                    let mut mam_requested = false;
                    move |view, event| {
                        match event {
                            UIEvent::Core(Event::Message(_, Message::Xmpp(message))) => {
                                match message.direction {
                                    // TODO check to == us
                                    Direction::Incoming => {
                                        if message.from == chat_for_event.contact {
                                            insert_message(
                                                view,
                                                MessageView::new(
                                                    &mut aparte,
                                                    Message::Xmpp(message.clone()),
                                                ),
                                            );
                                            mam_requested = false;
                                        }
                                    }
                                    Direction::Outgoing => {
                                        // TODO check from == us
                                        if message.to == chat_for_event.contact {
                                            insert_message(
                                                view,
                                                MessageView::new(
                                                    &mut aparte,
                                                    Message::Xmpp(message.clone()),
                                                ),
                                            );
                                            mam_requested = false;
                                        }
                                    }
                                }
                            }
                            UIEvent::Core(Event::Key(KeyEvent {
                                code: KeyCode::PageUp,
                                ..
                            })) => {
                                let (at_top, old_sel, new_sel) = view.page_up();
                                if old_sel != new_sel {
                                    if let Some(i) = old_sel {
                                        if let Some(c) = view.child_at(i) {
                                            c.deselect();
                                        }
                                    }
                                    if let Some(i) = new_sel {
                                        if let Some(c) = view.child_at(i) {
                                            c.select(selection_bg);
                                        }
                                    }
                                }
                                if at_top && !mam_requested {
                                    mam_requested = true;
                                    let from =
                                        view.first().map(|message| message.message.timestamp());
                                    scheduler.schedule(Event::LoadChatHistory {
                                        account: chat_for_event.account.clone(),
                                        contact: chat_for_event.contact.clone(),
                                        from: from.cloned(),
                                    });
                                }
                            }
                            UIEvent::Core(Event::Key(KeyEvent {
                                code: KeyCode::PageDown,
                                ..
                            })) => {
                                let (old_sel, new_sel) = view.page_down();
                                if old_sel != new_sel {
                                    if let Some(i) = old_sel {
                                        if let Some(c) = view.child_at(i) {
                                            c.deselect();
                                        }
                                    }
                                    if let Some(i) = new_sel {
                                        if let Some(c) = view.child_at(i) {
                                            c.select(selection_bg);
                                        }
                                    }
                                }
                                mam_requested = false;
                            }
                            UIEvent::NormalCommand(cmd) => match cmd {
                                NormalCommand::SelectPrev => {
                                    let (old, new, at_top) = view.select_prev();
                                    if let Some(i) = old {
                                        if let Some(c) = view.child_at(i) {
                                            c.deselect();
                                        }
                                    }
                                    if let Some(i) = new {
                                        if let Some(c) = view.child_at(i) {
                                            c.select(selection_bg);
                                        }
                                    }
                                    if at_top && !mam_requested {
                                        mam_requested = true;
                                        let from =
                                            view.first().map(|message| message.message.timestamp());
                                        scheduler.schedule(Event::LoadChatHistory {
                                            account: chat_for_event.account.clone(),
                                            contact: chat_for_event.contact.clone(),
                                            from: from.cloned(),
                                        });
                                    }
                                }
                                NormalCommand::SelectNext => {
                                    let (old, new) = view.select_next();
                                    if let Some(i) = old {
                                        if let Some(c) = view.child_at(i) {
                                            c.deselect();
                                        }
                                    }
                                    if let Some(i) = new {
                                        if let Some(c) = view.child_at(i) {
                                            c.select(selection_bg);
                                        }
                                    }
                                }
                                NormalCommand::ScrollToTop => {
                                    let (old, new) = view.scroll_to_top();
                                    if let Some(i) = old {
                                        if let Some(c) = view.child_at(i) {
                                            c.deselect();
                                        }
                                    }
                                    if let Some(i) = new {
                                        if let Some(c) = view.child_at(i) {
                                            c.select(selection_bg);
                                        }
                                    }
                                    if !mam_requested {
                                        mam_requested = true;
                                        let from =
                                            view.first().map(|message| message.message.timestamp());
                                        scheduler.schedule(Event::LoadChatHistory {
                                            account: chat_for_event.account.clone(),
                                            contact: chat_for_event.contact.clone(),
                                            from: from.cloned(),
                                        });
                                    }
                                }
                                NormalCommand::ScrollToBottom => {
                                    let (old, new) = view.scroll_to_bottom();
                                    if let Some(i) = old {
                                        if let Some(c) = view.child_at(i) {
                                            c.deselect();
                                        }
                                    }
                                    if let Some(i) = new {
                                        if let Some(c) = view.child_at(i) {
                                            c.select(selection_bg);
                                        }
                                    }
                                    mam_requested = false;
                                }
                                NormalCommand::SearchFirst(query) => {
                                    let (old, new) = view.set_search(query);
                                    for child in view.children_iter() {
                                        child.set_highlight(Some((
                                            query.clone(),
                                            search_highlight_fg,
                                            search_highlight_bg,
                                        )));
                                    }
                                    if old != new {
                                        if let Some(i) = old {
                                            if let Some(c) = view.child_at(i) {
                                                c.deselect();
                                            }
                                        }
                                        if let Some(i) = new {
                                            if let Some(c) = view.child_at(i) {
                                                c.select(selection_bg);
                                            }
                                        }
                                    }
                                }
                                NormalCommand::SearchNext => {
                                    let (old, new) = view.search_next();
                                    if old != new {
                                        if let Some(i) = old {
                                            if let Some(c) = view.child_at(i) {
                                                c.deselect();
                                            }
                                        }
                                        if let Some(i) = new {
                                            if let Some(c) = view.child_at(i) {
                                                c.select(selection_bg);
                                            }
                                        }
                                    }
                                }
                                NormalCommand::SearchPrev => {
                                    let (old, new) = view.search_prev();
                                    if old != new {
                                        if let Some(i) = old {
                                            if let Some(c) = view.child_at(i) {
                                                c.deselect();
                                            }
                                        }
                                        if let Some(i) = new {
                                            if let Some(c) = view.child_at(i) {
                                                c.select(selection_bg);
                                            }
                                        }
                                    }
                                }
                                NormalCommand::SearchCancel => {
                                    view.clear_search();
                                    for child in view.children_iter() {
                                        child.set_highlight(None);
                                    }
                                }
                            },
                            UIEvent::ModeChange(Mode::Normal) if !view.has_selection() => {
                                let (_, new) = view.select_last_visible();
                                if let Some(i) = new {
                                    if let Some(c) = view.child_at(i) {
                                        c.select(selection_bg);
                                    }
                                }
                            }
                            UIEvent::ModeChange(Mode::Insert)
                            | UIEvent::ModeChange(Mode::Command) => {
                                if let Some(i) = view.clear_selection() {
                                    if let Some(c) = view.child_at(i) {
                                        c.deselect();
                                    }
                                }
                                for child in view.children_iter() {
                                    child.set_highlight(None);
                                }
                            }
                            _ => {}
                        }
                    }
                });

                self.add_window(chat.contact.to_string(), None, Box::new(chatwin));
                self.conversations
                    .insert(chat.contact.to_string(), conversation.clone());
            }
            Conversation::Channel(channel) => {
                let mut layout = LinearLayout::<UIEvent, Theme>::new(Orientation::Horizontal)
                    .with_event(|layout, event| {
                        for child in layout.iter_children_mut() {
                            child.event(event);
                        }
                    });

                let channel_for_event = channel.clone();
                let chanwin = ScrollWin::<UIEvent, MessageView, Theme>::new().with_event({
                    let mut aparte = aparte.proxy();
                    let mut mam_requested = false;
                    move |view, event| {
                        match event {
                            UIEvent::Core(Event::Message(_, Message::Xmpp(message))) => {
                                match message.direction {
                                    // TODO check to == us
                                    Direction::Incoming => {
                                        if message.from == channel_for_event.jid {
                                            insert_message(
                                                view,
                                                MessageView::new(
                                                    &mut aparte,
                                                    Message::Xmpp(message.clone()),
                                                ),
                                            );
                                            mam_requested = false;
                                        }
                                    }
                                    Direction::Outgoing => {
                                        // TODO check from == us
                                        if message.to == channel_for_event.jid {
                                            insert_message(
                                                view,
                                                MessageView::new(
                                                    &mut aparte,
                                                    Message::Xmpp(message.clone()),
                                                ),
                                            );
                                            mam_requested = false;
                                        }
                                    }
                                }
                            }
                            UIEvent::Core(Event::Key(KeyEvent {
                                code: KeyCode::PageUp,
                                ..
                            })) => {
                                let (at_top, old_sel, new_sel) = view.page_up();
                                if old_sel != new_sel {
                                    if let Some(i) = old_sel {
                                        if let Some(c) = view.child_at(i) {
                                            c.deselect();
                                        }
                                    }
                                    if let Some(i) = new_sel {
                                        if let Some(c) = view.child_at(i) {
                                            c.select(selection_bg);
                                        }
                                    }
                                }
                                if at_top && !mam_requested {
                                    mam_requested = true;
                                    let from =
                                        view.first().map(|message| message.message.timestamp());
                                    scheduler.schedule(Event::LoadChannelHistory {
                                        account: channel_for_event.account.clone(),
                                        jid: channel_for_event.jid.clone(),
                                        from: from.cloned(),
                                    });
                                }
                            }
                            UIEvent::Core(Event::Key(KeyEvent {
                                code: KeyCode::PageDown,
                                ..
                            })) => {
                                let (old_sel, new_sel) = view.page_down();
                                if old_sel != new_sel {
                                    if let Some(i) = old_sel {
                                        if let Some(c) = view.child_at(i) {
                                            c.deselect();
                                        }
                                    }
                                    if let Some(i) = new_sel {
                                        if let Some(c) = view.child_at(i) {
                                            c.select(selection_bg);
                                        }
                                    }
                                }
                                mam_requested = false;
                            }
                            UIEvent::NormalCommand(cmd) => match cmd {
                                NormalCommand::SelectPrev => {
                                    let (old, new, at_top) = view.select_prev();
                                    if let Some(i) = old {
                                        if let Some(c) = view.child_at(i) {
                                            c.deselect();
                                        }
                                    }
                                    if let Some(i) = new {
                                        if let Some(c) = view.child_at(i) {
                                            c.select(selection_bg);
                                        }
                                    }
                                    if at_top && !mam_requested {
                                        mam_requested = true;
                                        let from =
                                            view.first().map(|message| message.message.timestamp());
                                        scheduler.schedule(Event::LoadChannelHistory {
                                            account: channel_for_event.account.clone(),
                                            jid: channel_for_event.jid.clone(),
                                            from: from.cloned(),
                                        });
                                    }
                                }
                                NormalCommand::SelectNext => {
                                    let (old, new) = view.select_next();
                                    if let Some(i) = old {
                                        if let Some(c) = view.child_at(i) {
                                            c.deselect();
                                        }
                                    }
                                    if let Some(i) = new {
                                        if let Some(c) = view.child_at(i) {
                                            c.select(selection_bg);
                                        }
                                    }
                                }
                                NormalCommand::ScrollToTop => {
                                    let (old, new) = view.scroll_to_top();
                                    if let Some(i) = old {
                                        if let Some(c) = view.child_at(i) {
                                            c.deselect();
                                        }
                                    }
                                    if let Some(i) = new {
                                        if let Some(c) = view.child_at(i) {
                                            c.select(selection_bg);
                                        }
                                    }
                                    if !mam_requested {
                                        mam_requested = true;
                                        let from =
                                            view.first().map(|message| message.message.timestamp());
                                        scheduler.schedule(Event::LoadChannelHistory {
                                            account: channel_for_event.account.clone(),
                                            jid: channel_for_event.jid.clone(),
                                            from: from.cloned(),
                                        });
                                    }
                                }
                                NormalCommand::ScrollToBottom => {
                                    let (old, new) = view.scroll_to_bottom();
                                    if let Some(i) = old {
                                        if let Some(c) = view.child_at(i) {
                                            c.deselect();
                                        }
                                    }
                                    if let Some(i) = new {
                                        if let Some(c) = view.child_at(i) {
                                            c.select(selection_bg);
                                        }
                                    }
                                    mam_requested = false;
                                }
                                NormalCommand::SearchFirst(query) => {
                                    let (old, new) = view.set_search(query);
                                    for child in view.children_iter() {
                                        child.set_highlight(Some((
                                            query.clone(),
                                            search_highlight_fg,
                                            search_highlight_bg,
                                        )));
                                    }
                                    if old != new {
                                        if let Some(i) = old {
                                            if let Some(c) = view.child_at(i) {
                                                c.deselect();
                                            }
                                        }
                                        if let Some(i) = new {
                                            if let Some(c) = view.child_at(i) {
                                                c.select(selection_bg);
                                            }
                                        }
                                    }
                                }
                                NormalCommand::SearchNext => {
                                    let (old, new) = view.search_next();
                                    if old != new {
                                        if let Some(i) = old {
                                            if let Some(c) = view.child_at(i) {
                                                c.deselect();
                                            }
                                        }
                                        if let Some(i) = new {
                                            if let Some(c) = view.child_at(i) {
                                                c.select(selection_bg);
                                            }
                                        }
                                    }
                                }
                                NormalCommand::SearchPrev => {
                                    let (old, new) = view.search_prev();
                                    if old != new {
                                        if let Some(i) = old {
                                            if let Some(c) = view.child_at(i) {
                                                c.deselect();
                                            }
                                        }
                                        if let Some(i) = new {
                                            if let Some(c) = view.child_at(i) {
                                                c.select(selection_bg);
                                            }
                                        }
                                    }
                                }
                                NormalCommand::SearchCancel => {
                                    view.clear_search();
                                    for child in view.children_iter() {
                                        child.set_highlight(None);
                                    }
                                }
                            },
                            UIEvent::Core(Event::ChangeWindow(name))
                                if name == &channel_for_event.jid.to_string()
                                    && view.first().is_none()
                                    && !mam_requested =>
                            {
                                mam_requested = true;
                                scheduler.schedule(Event::LoadChannelHistory {
                                    account: channel_for_event.account.clone(),
                                    jid: channel_for_event.jid.clone(),
                                    from: None,
                                });
                            }
                            UIEvent::ModeChange(Mode::Normal) if !view.has_selection() => {
                                let (_, new) = view.select_last_visible();
                                if let Some(i) = new {
                                    if let Some(c) = view.child_at(i) {
                                        c.select(selection_bg);
                                    }
                                }
                            }
                            UIEvent::ModeChange(Mode::Insert)
                            | UIEvent::ModeChange(Mode::Command) => {
                                if let Some(i) = view.clear_selection() {
                                    if let Some(c) = view.child_at(i) {
                                        c.deselect();
                                    }
                                }
                                for child in view.children_iter() {
                                    child.set_highlight(None);
                                }
                            }
                            _ => {}
                        }
                    }
                });
                layout.push(chanwin, 7);

                let roster_jid = channel.jid.clone();
                let roster =
                    ListView::<UIEvent, conversation::Role, conversation::Occupant, Theme>::new()
                        .with_layout(LayoutParams {
                            width: LayoutParam::WrapContent,
                            height: LayoutParam::MatchParent,
                        })
                        .with_none_group()
                        .with_unique_item()
                        .with_sort_item()
                        .with_event(move |view, event| {
                            if let UIEvent::Core(Event::Occupant {
                                conversation,
                                occupant,
                                ..
                            }) = event
                            {
                                if roster_jid == *conversation {
                                    view.insert(occupant.clone(), Some(occupant.role));
                                }
                            }
                        });
                layout.push(roster, 3);

                let jid_str = channel.jid.to_string();
                if let Some(name) = &channel.name {
                    self.jid_to_name.insert(channel.jid.clone(), name.clone());
                }
                self.add_window(jid_str.clone(), channel.name.clone(), Box::new(layout));
                self.conversations.insert(jid_str, conversation.clone());
            }
        }
    }

    fn add_window(
        &mut self,
        jid: String,
        display: Option<String>,
        window: Box<dyn View<UIEvent, Theme>>,
    ) {
        self.windows.push(jid.clone());
        self.root
            .event(&mut UIEvent::AddWindow(jid, display, Some(window)));
    }

    pub fn change_window(&mut self, window: &str) {
        self.root
            .event(&mut UIEvent::Core(Event::ChangeWindow(window.to_string())));
        self.current_window = Some(window.to_string());
    }

    #[allow(unused)] // XXX Should be used when alt+arrow navigation is fixed
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

    #[allow(unused)] // XXX Should be used when alt+arrow navigation is fixed
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

    pub fn get_conversation(&self, window: &str) -> Option<&Conversation> {
        self.conversations.get(window)
    }

    pub fn get_display_name(&self, jid: &BareJid) -> Option<&str> {
        self.jid_to_name.get(jid).map(String::as_str)
    }

    pub fn current_window(&self) -> Option<&String> {
        self.current_window.as_ref()
    }

    /// Render the UI if the dirty flag is set. Intended to be called once per
    /// event batch from the main loop.
    pub fn render_if_dirty(&mut self, config: &Theme) -> bool {
        if !self.dirty {
            return false;
        }
        self.dirty = false;

        let before = Instant::now();
        let (width, height) = crossterm::terminal::size().unwrap();
        let measure_specs = MeasureSpecs {
            width: MeasureSpec::AtMost(width),
            height: MeasureSpec::AtMost(height),
        };
        let requested_dimensions = self.root.measure(&measure_specs);
        self.dimensions = Dimensions::reconcile(&measure_specs, &requested_dimensions, 0, 0);
        self.root.layout(&self.dimensions);

        let mut render_buffer = self.render_buffer.write().unwrap();
        render_buffer.set_size((width, height).into());
        render_buffer.clear();
        let frame = ScreenFrame::new(&mut render_buffer, &self.dimensions);
        self.root.render(frame, config);
        log::trace!("Mod::UI rendered in {:.2?}", before.elapsed());
        true
    }
}

impl ModTrait for UIMod {
    fn init(&mut self, aparte: &mut Aparte) -> Result<(), ()> {
        let (width, height) = crossterm::terminal::size().unwrap();
        log::debug!("Init UI on screen ({width}×{height})");

        // Indices into the root LinearLayout's children (push order below).
        const FRAME_LAYOUT_INDEX: usize = 1;
        const INPUT_INDEX: usize = 3;

        {
            let mut mode = Mode::Insert;
            let mut command_buffer = String::new();
            let mut saved_input = String::new();
            let mut timeout_generation: u64 = 0;
            let normal_commands = build_normal_command_trie();
            let mut aparte_proxy = aparte.proxy();
            let mut current_window = String::new();
            self.root = LinearLayout::<UIEvent, Theme>::new(Orientation::Vertical).with_event(
                move |layout, event| match event {
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Esc, ..
                    })) if mode == Mode::Insert => {
                        mode = Mode::Normal;
                        aparte_proxy.schedule(Event::UIMode(Mode::Normal));
                        command_buffer.clear();
                        layout.set_focus(FRAME_LAYOUT_INDEX);
                        for child in layout.iter_children_mut() {
                            child.event(&mut UIEvent::ModeChange(Mode::Normal));
                        }
                    }
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Char('i'),
                        ..
                    })) if mode == Mode::Normal => {
                        command_buffer.clear();
                        mode = Mode::Insert;
                        aparte_proxy.schedule(Event::UIMode(Mode::Insert));
                        layout.set_focus(INPUT_INDEX);
                        for child in layout.iter_children_mut() {
                            child.event(&mut UIEvent::CommandBufferUpdate(String::new()));
                            child.event(&mut UIEvent::ModeChange(Mode::Insert));
                        }
                    }
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Char(':'),
                        ..
                    })) if mode == Mode::Normal => {
                        mode = Mode::Command;
                        aparte_proxy.schedule(Event::UIMode(Mode::Command));
                        layout.set_focus(INPUT_INDEX);
                        // Save current input content so it can be restored on exit.
                        let result = Rc::new(RefCell::new(None));
                        for child in layout.iter_children_mut() {
                            child.event(&mut UIEvent::GetInput(Rc::clone(&result)));
                        }
                        saved_input = result
                            .borrow()
                            .as_ref()
                            .map(|(buf, _, _)| buf.clone())
                            .unwrap_or_default();
                        for child in layout.iter_children_mut() {
                            child.event(&mut UIEvent::ModeChange(Mode::Command));
                            child.event(&mut UIEvent::SetInput(":".to_string()));
                        }
                    }
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Char('/'),
                        ..
                    })) if mode == Mode::Normal => {
                        mode = Mode::Command;
                        aparte_proxy.schedule(Event::UIMode(Mode::Command));
                        layout.set_focus(INPUT_INDEX);
                        // Save current input content so it can be restored on exit.
                        let result = Rc::new(RefCell::new(None));
                        for child in layout.iter_children_mut() {
                            child.event(&mut UIEvent::GetInput(Rc::clone(&result)));
                        }
                        saved_input = result
                            .borrow()
                            .as_ref()
                            .map(|(buf, _, _)| buf.clone())
                            .unwrap_or_default();
                        for child in layout.iter_children_mut() {
                            child.event(&mut UIEvent::ModeChange(Mode::Command));
                            child.event(&mut UIEvent::SetInput("/".to_string()));
                        }
                    }
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Esc, ..
                    })) if mode == Mode::Command => {
                        mode = Mode::Normal;
                        aparte_proxy.schedule(Event::UIMode(Mode::Normal));
                        let saved = std::mem::take(&mut saved_input);
                        layout.set_focus(FRAME_LAYOUT_INDEX);
                        if let Some(focused) = layout.focused_child_mut() {
                            focused.event(&mut UIEvent::NormalCommand(NormalCommand::SearchCancel));
                        }
                        for child in layout.iter_children_mut() {
                            child.event(&mut UIEvent::ModeChange(Mode::Normal));
                            child.event(&mut UIEvent::SetInput(saved.clone()));
                            child.event(&mut UIEvent::CommandBufferUpdate(String::new()));
                        }
                    }
                    // All other keys in COMMAND mode go to the focused input widget,
                    // reusing its editing logic (Ctrl+A/B/E/F/H/W/U/K, arrows, Home/End,
                    // Delete, history).
                    UIEvent::Core(Event::Key(_)) if mode == Mode::Command => {
                        if let Some(focused) = layout.focused_child_mut() {
                            focused.event(event);
                        }
                    }
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Char(c),
                        ..
                    })) if mode == Mode::Normal => {
                        command_buffer.push(*c);

                        if let Some(cmd) = normal_commands.get(&command_buffer).cloned() {
                            command_buffer.clear();
                            if let Some(focused) = layout.focused_child_mut() {
                                focused.event(&mut UIEvent::NormalCommand(cmd));
                            }
                            for child in layout.iter_children_mut() {
                                child.event(&mut UIEvent::CommandBufferUpdate(String::new()));
                            }
                        } else if normal_commands
                            .get_raw_descendant(&command_buffer)
                            .is_some()
                        {
                            timeout_generation += 1;
                            let gen = timeout_generation;
                            let mut aparte_for_task = aparte_proxy.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                                aparte_for_task.schedule(Event::CommandTimeout(gen));
                            });
                            let buf = command_buffer.clone();
                            for child in layout.iter_children_mut() {
                                child.event(&mut UIEvent::CommandBufferUpdate(buf.clone()));
                            }
                        } else {
                            command_buffer.clear();
                            for child in layout.iter_children_mut() {
                                child.event(&mut UIEvent::CommandBufferUpdate(String::new()));
                            }
                        }
                    }
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Up, ..
                    })) if mode == Mode::Normal => {
                        if let Some(focused) = layout.focused_child_mut() {
                            focused.event(&mut UIEvent::NormalCommand(NormalCommand::SelectPrev));
                        }
                    }
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Down,
                        ..
                    })) if mode == Mode::Normal => {
                        if let Some(focused) = layout.focused_child_mut() {
                            focused.event(&mut UIEvent::NormalCommand(NormalCommand::SelectNext));
                        }
                    }
                    UIEvent::Core(Event::CommandTimeout(gen)) => {
                        if *gen == timeout_generation && !command_buffer.is_empty() {
                            command_buffer.clear();
                            for child in layout.iter_children_mut() {
                                child.event(&mut UIEvent::CommandBufferUpdate(String::new()));
                            }
                        }
                    }
                    // Scroll keys reach the message window in any mode.
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::PageUp | KeyCode::PageDown,
                        ..
                    })) if mode == Mode::Insert => {
                        for child in layout.iter_children_mut() {
                            child.event(event);
                        }
                    }
                    // All other keys in INSERT mode go only to the focused input.
                    UIEvent::Core(Event::Key(_)) if mode == Mode::Insert => {
                        if let Some(focused) = layout.focused_child_mut() {
                            focused.event(event);
                        }
                    }
                    // Enter in Command mode executes the typed command.
                    // (Enter is delivered as UIEvent::Validate by on_event, not as a Key event.)
                    UIEvent::Validate(result) if mode == Mode::Command => {
                        // Read the typed command from the input widget (source of truth).
                        let input_result = Rc::new(RefCell::new(None));
                        for child in layout.iter_children_mut() {
                            child.event(&mut UIEvent::GetInput(Rc::clone(&input_result)));
                        }
                        let cmd = input_result
                            .borrow()
                            .as_ref()
                            .map(|(buf, _, _)| buf.clone())
                            .unwrap_or_default();

                        mode = Mode::Normal;
                        aparte_proxy.schedule(Event::UIMode(Mode::Normal));
                        let saved = std::mem::take(&mut saved_input);
                        layout.set_focus(FRAME_LAYOUT_INDEX);
                        for child in layout.iter_children_mut() {
                            child.event(&mut UIEvent::ModeChange(Mode::Normal));
                            child.event(&mut UIEvent::SetInput(saved.clone()));
                            child.event(&mut UIEvent::CommandBufferUpdate(String::new()));
                        }
                        if let Some(query) = cmd.strip_prefix('/') {
                            let query = query.to_string();
                            if !query.is_empty() {
                                if let Some(focused) = layout.focused_child_mut() {
                                    focused.event(&mut UIEvent::NormalCommand(
                                        NormalCommand::SearchFirst(query),
                                    ));
                                }
                            }
                        } else if cmd.starts_with(':') {
                            aparte_proxy.schedule(Event::RawCommand(
                                aparte_proxy.current_account(),
                                current_window.clone(),
                                cmd,
                            ));
                        }
                        result.borrow_mut().replace((String::new(), false));
                    }
                    UIEvent::Core(Event::ChangeWindow(name)) => {
                        current_window = terminus::clean_str(name);
                        for child in layout.iter_children_mut() {
                            child.event(event);
                        }
                    }
                    _ => {
                        for child in layout.iter_children_mut() {
                            child.event(event);
                        }
                    }
                },
            );
        }

        let win_bar = WinBar::new();
        let frame =
            FrameLayout::<UIEvent, String, Theme>::new().with_event(|frame, event| match event {
                UIEvent::Core(Event::ChangeWindow(name)) => {
                    frame.set_current(name.to_string());
                    for child in frame.iter_children_mut() {
                        child.event(event);
                    }
                }
                UIEvent::AddWindow(jid, display_name, view) => {
                    let view = view.take().unwrap();
                    frame.insert_boxed(jid.to_string(), view);

                    // propagate AddWindow with jid only to each subview
                    // required at least for console view
                    for child in frame.iter_children_mut() {
                        child.event(&mut UIEvent::AddWindow(
                            jid.to_string(),
                            display_name.clone(),
                            None,
                        ));
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
                // Interaction events → current window only
                UIEvent::Core(Event::Key(_))
                | UIEvent::Core(Event::Completed(_, _))
                | UIEvent::Core(Event::ResetCompletion)
                | UIEvent::Core(Event::ReadPassword(_)) => {
                    if let Some(current) = frame.get_current_mut() {
                        current.event(event);
                    }
                }
                // Global events (Message, Notification, Subject, etc.) → all windows
                _ => {
                    for child in frame.iter_children_mut() {
                        child.event(event);
                    }
                }
            });
        let title_bar = TitleBar::new(aparte.config.preferred_langs.clone());
        let input = Input::new().with_event(|input, event| {
            if let UIEvent::Core(Event::Key(key)) = event {
                log::debug!("Input event: {:?}", key);
            }
            match event {
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Char(c),
                    modifiers: KeyModifiers::NONE,
                    ..
                })) => input.key(*c),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Char(c),
                    modifiers: KeyModifiers::SHIFT,
                    ..
                })) => input.key(*c),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Backspace,
                    ..
                })) => input.backspace(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Delete,
                    ..
                })) => input.delete(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Home,
                    ..
                })) => input.home(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::End, ..
                })) => input.end(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Up, ..
                })) => input.previous(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Down,
                    ..
                })) => input.next(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Left,
                    modifiers: KeyModifiers::NONE,
                    ..
                })) => input.left(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Right,
                    modifiers: KeyModifiers::NONE,
                    ..
                })) => input.right(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Char('a'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                })) => input.home(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Char('b'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                })) => input.left(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Char('e'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                })) => input.end(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Char('f'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                })) => input.right(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Char('h'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                })) => input.backspace(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Char('w'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                })) => input.backward_delete_word(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Char('u'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                })) => input.delete_from_cursor_to_start(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Char('k'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                })) => input.delete_from_cursor_to_end(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Left,
                    modifiers: KeyModifiers::CONTROL,
                    ..
                })) => input.word_left(),
                UIEvent::Core(Event::Key(KeyEvent {
                    code: KeyCode::Right,
                    modifiers: KeyModifiers::CONTROL,
                    ..
                })) => input.word_right(),
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
                }
                UIEvent::Core(Event::ReadPassword(_)) => input.password(),
                UIEvent::SetInput(text) => {
                    input.cursor =
                        Cursor::from_index(text, text.len()).unwrap_or_else(|_| Cursor::new(0));
                    input.buf = text.clone();
                }
                UIEvent::ModeChange(Mode::Normal) => {
                    input.set_cursor_style(CursorStyle::SteadyBlock);
                }
                UIEvent::ModeChange(Mode::Command) => {
                    input.set_show_cursor(true);
                    input.set_cursor_style(CursorStyle::SteadyBar);
                }
                UIEvent::ModeChange(Mode::Insert) => {
                    input.set_show_cursor(true);
                    input.set_cursor_style(CursorStyle::SteadyBar);
                }
                _ => {}
            }
        });

        self.root.push(win_bar, 0);
        self.root.push(frame, 1);
        self.root.push(title_bar, 0);
        self.root.push(input, 0);
        self.root.set_focus(INPUT_INDEX);

        let mut console = LinearLayout::<UIEvent, Theme>::new(Orientation::Horizontal).with_event(
            |layout, event| {
                for LayoutChild { child, .. } in layout.children.iter_mut() {
                    child.view.event(event);
                }
            },
        );
        console.push(
            ScrollWin::<UIEvent, MessageView, Theme>::new()
                .with_layout(LayoutParams {
                    width: LayoutParam::MatchParent,
                    height: LayoutParam::MatchParent,
                })
                .with_event({
                    let mut aparte = aparte.proxy();
                    let selection_bg = aparte.config.theme.selected_message;
                    let search_highlight_fg = aparte.config.theme.search_highlight_fg;
                    let search_highlight_bg = aparte.config.theme.search_highlight_bg;
                    move |view, event| match event {
                        UIEvent::Core(Event::Message(_, Message::Log(message))) => {
                            insert_message(
                                view,
                                MessageView::new(&mut aparte, Message::Log(message.clone())),
                            );
                        }
                        UIEvent::Core(Event::Key(KeyEvent {
                            code: KeyCode::PageUp,
                            ..
                        })) => {
                            let (_, old_sel, new_sel) = view.page_up();
                            if old_sel != new_sel {
                                if let Some(i) = old_sel {
                                    if let Some(c) = view.child_at(i) {
                                        c.deselect();
                                    }
                                }
                                if let Some(i) = new_sel {
                                    if let Some(c) = view.child_at(i) {
                                        c.select(selection_bg);
                                    }
                                }
                            }
                        }
                        UIEvent::Core(Event::Key(KeyEvent {
                            code: KeyCode::PageDown,
                            ..
                        })) => {
                            let (old_sel, new_sel) = view.page_down();
                            if old_sel != new_sel {
                                if let Some(i) = old_sel {
                                    if let Some(c) = view.child_at(i) {
                                        c.deselect();
                                    }
                                }
                                if let Some(i) = new_sel {
                                    if let Some(c) = view.child_at(i) {
                                        c.select(selection_bg);
                                    }
                                }
                            }
                        }
                        UIEvent::NormalCommand(cmd) => match cmd {
                            NormalCommand::SelectPrev => {
                                let (old, new, _) = view.select_prev();
                                if let Some(i) = old {
                                    if let Some(c) = view.child_at(i) {
                                        c.deselect();
                                    }
                                }
                                if let Some(i) = new {
                                    if let Some(c) = view.child_at(i) {
                                        c.select(selection_bg);
                                    }
                                }
                            }
                            NormalCommand::SelectNext => {
                                let (old, new) = view.select_next();
                                if let Some(i) = old {
                                    if let Some(c) = view.child_at(i) {
                                        c.deselect();
                                    }
                                }
                                if let Some(i) = new {
                                    if let Some(c) = view.child_at(i) {
                                        c.select(selection_bg);
                                    }
                                }
                            }
                            NormalCommand::ScrollToTop => {
                                let (old, new) = view.scroll_to_top();
                                if let Some(i) = old {
                                    if let Some(c) = view.child_at(i) {
                                        c.deselect();
                                    }
                                }
                                if let Some(i) = new {
                                    if let Some(c) = view.child_at(i) {
                                        c.select(selection_bg);
                                    }
                                }
                            }
                            NormalCommand::ScrollToBottom => {
                                let (old, new) = view.scroll_to_bottom();
                                if let Some(i) = old {
                                    if let Some(c) = view.child_at(i) {
                                        c.deselect();
                                    }
                                }
                                if let Some(i) = new {
                                    if let Some(c) = view.child_at(i) {
                                        c.select(selection_bg);
                                    }
                                }
                            }
                            NormalCommand::SearchFirst(query) => {
                                let (old, new) = view.set_search(query);
                                for child in view.children_iter() {
                                    child.set_highlight(Some((
                                        query.clone(),
                                        search_highlight_fg,
                                        search_highlight_bg,
                                    )));
                                }
                                if old != new {
                                    if let Some(i) = old {
                                        if let Some(c) = view.child_at(i) {
                                            c.deselect();
                                        }
                                    }
                                    if let Some(i) = new {
                                        if let Some(c) = view.child_at(i) {
                                            c.select(selection_bg);
                                        }
                                    }
                                }
                            }
                            NormalCommand::SearchNext => {
                                let (old, new) = view.search_next();
                                if old != new {
                                    if let Some(i) = old {
                                        if let Some(c) = view.child_at(i) {
                                            c.deselect();
                                        }
                                    }
                                    if let Some(i) = new {
                                        if let Some(c) = view.child_at(i) {
                                            c.select(selection_bg);
                                        }
                                    }
                                }
                            }
                            NormalCommand::SearchPrev => {
                                let (old, new) = view.search_prev();
                                if old != new {
                                    if let Some(i) = old {
                                        if let Some(c) = view.child_at(i) {
                                            c.deselect();
                                        }
                                    }
                                    if let Some(i) = new {
                                        if let Some(c) = view.child_at(i) {
                                            c.select(selection_bg);
                                        }
                                    }
                                }
                            }
                            NormalCommand::SearchCancel => {
                                view.clear_search();
                                for child in view.children_iter() {
                                    child.set_highlight(None);
                                }
                            }
                        },
                        UIEvent::ModeChange(Mode::Insert) | UIEvent::ModeChange(Mode::Command) => {
                            if let Some(i) = view.clear_selection() {
                                if let Some(c) = view.child_at(i) {
                                    c.deselect();
                                }
                            }
                            for child in view.children_iter() {
                                child.set_highlight(None);
                            }
                        }
                        _ => {}
                    }
                }),
            7,
        );
        let mut window_display: HashMap<String, String> = HashMap::new();
        let roster = ListView::<UIEvent, contact::Group, RosterItem, Theme>::new()
            .with_layout(LayoutParams {
                width: LayoutParam::WrapContent,
                height: LayoutParam::MatchParent,
            })
            .with_none_group()
            .with_sort_item()
            .with_event(move |view, event| match event {
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
                UIEvent::AddWindow(jid, display_name, _) => {
                    let group = contact::Group(String::from("Windows"));
                    let label = display_name.as_deref().unwrap_or(jid.as_str()).to_string();
                    window_display.insert(jid.clone(), label.clone());
                    view.insert(RosterItem::Window(label), Some(group));
                }
                UIEvent::Core(Event::Close(window)) => {
                    let group = contact::Group(String::from("Windows"));
                    let label = window_display
                        .remove(window.as_str())
                        .unwrap_or_else(|| window.clone());
                    let _ = view.remove(RosterItem::Window(label), Some(group));
                }
                _ => {}
            });
        console.push(roster, 3);

        self.add_window("console".to_string(), None, Box::new(console));
        self.change_window("console");

        // Measure, layout and render
        let measure_specs = terminus::MeasureSpecs {
            width: terminus::MeasureSpec::AtMost(width),
            height: terminus::MeasureSpec::AtMost(height),
        };
        let requested_dimensions = self.root.measure(&measure_specs);
        self.dimensions = Dimensions::reconcile(&measure_specs, &requested_dimensions, 0, 0);
        self.root.layout(&self.dimensions);

        {
            let mut render_buffer = self.render_buffer.write().unwrap();
            render_buffer.clear();
            let frame = ScreenFrame::new(&mut render_buffer, &self.dimensions);
            self.root.render(frame, &aparte.config.get_theme());
        }

        Ok(())
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        let before = Instant::now();

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
                let name = {
                    let bookmarks = aparte.get_mod::<BookmarksMod>();
                    bookmarks
                        .bookmarks_by_jid
                        .get(&Jid::from(bare.clone()))
                        .and_then(|&idx| bookmarks.bookmarks.get(idx))
                        .and_then(|b| b.name.clone())
                };
                let ch = Channel {
                    account: account.clone(),
                    jid: bare.clone(),
                    nick: channel.resource().to_string(),
                    name,
                    occupants: HashMap::new(),
                };
                let win_name = bare.to_string();
                if !self.windows.contains(&win_name) {
                    self.add_conversation(aparte, Conversation::Channel(ch));
                }
                if *user_request {
                    self.change_window(&win_name);
                }
            }
            Event::Win(window) => {
                if self.windows.contains(window) {
                    self.change_window(window);
                } else {
                    let jid = self
                        .jid_to_name
                        .iter()
                        .find(|(_, name)| name.as_str() == window.as_str())
                        .map(|(jid, _)| jid.to_string());
                    if let Some(jid) = jid {
                        self.change_window(&jid);
                    } else {
                        crate::info!(aparte, "Unknown window {window}");
                    }
                }
            }
            Event::WindowChange => {}
            Event::Close(window) => {
                if window != "console" {
                    if let Some(Conversation::Channel(channel)) =
                        self.conversations.get(window.as_str())
                    {
                        self.jid_to_name.remove(&channel.jid);
                    }
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
            Event::UIMode(mode) => {
                self.current_mode = *mode;
            }
            Event::Key(key) => {
                match key {
                    KeyEvent {
                        code: KeyCode::Tab, ..
                    } => {
                        let result = Rc::new(RefCell::new(None));

                        let (raw_buf, cursor, password) = {
                            self.root.event(&mut UIEvent::GetInput(Rc::clone(&result)));

                            let result = result.borrow_mut();
                            result.as_ref().unwrap().clone()
                        };

                        if password {
                            aparte.schedule(Event::Key(KeyEvent::new(
                                KeyCode::Tab,
                                KeyModifiers::NONE,
                            )));
                        } else {
                            match self.current_mode {
                                Mode::Normal => {}
                                Mode::Insert => {
                                    // Nick completion only — skip if the buffer looks like a command.
                                    if !raw_buf.starts_with(':') {
                                        let window = self.current_window.clone().unwrap();
                                        let account = match self.conversations.get(&window) {
                                            Some(Conversation::Chat(chat)) => {
                                                Some(chat.account.clone())
                                            }
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
                                Mode::Command => {
                                    let window = self.current_window.clone().unwrap();
                                    let account = match self.conversations.get(&window) {
                                        Some(Conversation::Chat(chat)) => {
                                            Some(chat.account.clone())
                                        }
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
                        }
                    }
                    KeyEvent {
                        code: KeyCode::Enter,
                        ..
                    } => {
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
                                                bodies,
                                                None,
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
                                                bodies,
                                                None,
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
                    KeyEvent {
                        code: KeyCode::Char('a'),
                        modifiers: KeyModifiers::ALT,
                        ..
                    } => {
                        if !self.unread_windows.is_empty() {
                            let next = {
                                let mut sorted = self.unread_windows.iter().collect::<Vec<_>>();
                                sorted.sort_by_key(|(_, b)| std::cmp::Reverse(b.len()));
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
                timestamp,
            } => {
                let win = conversation.get_jid().to_string();
                if Some(&win) != self.current_window.as_ref() && self.windows.contains(&win) {
                    self.unread_windows
                        .entry(win)
                        .or_default()
                        .push_back((*timestamp, *important));
                }
                if *important && aparte.config.bell {
                    self.render_buffer.read().unwrap().bell();
                }
                self.root.event(&mut UIEvent::Core(Event::Notification {
                    conversation: conversation.clone(),
                    important: *important,
                    timestamp: *timestamp,
                }));
            }
            Event::DisplayedMarker { account, jid, id } => {
                let win = jid.to_string();
                let cut_ts = {
                    let msgs = aparte.get_mod::<MessagesMod>();
                    let acct_key = Some(account.clone());
                    msgs.get_by_stanza_id(&acct_key, id)
                        .or_else(|| msgs.get(&acct_key, id))
                        .and_then(|m| {
                            if let Message::Xmpp(x) = m {
                                x.history.iter().max().map(|v| v.timestamp)
                            } else {
                                None
                            }
                        })
                };
                if let Some(cut_ts) = cut_ts {
                    if let Some(deque) = self.unread_windows.get_mut(&win) {
                        let n_important = deque
                            .iter()
                            .take_while(|(ts, _)| *ts <= cut_ts)
                            .filter(|(_, imp)| *imp)
                            .count() as u64;
                        let before = deque.len();
                        deque.retain(|(ts, _)| *ts > cut_ts);
                        let n_drained = (before - deque.len()) as u64;
                        if n_drained > 0 {
                            self.root.event(&mut UIEvent::ReduceHighlight(
                                win.clone(),
                                n_drained,
                                n_important,
                            ));
                        }
                        if deque.is_empty() {
                            self.unread_windows.remove(&win);
                        }
                    }
                }
            }
            Event::UIRender(_) => {
                log::debug!("Force render");
            }
            // Forward all unknown events
            event => self.root.event(&mut UIEvent::Core(event.clone())),
        }
        log::trace!("Mod::UI handled event in {:.2?}", before.elapsed());

        // Mark UI as needing a render; actual render happens once per event batch
        // in the main loop via render_if_dirty().
        self.dirty = true;

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

pub struct EventStream {
    inner: CrosstermEventStream,
}

impl EventStream {
    pub fn new() -> Self {
        Self {
            inner: CrosstermEventStream::new(),
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
            Poll::Ready(Some(Ok(CrosstermEvent::Key(key)))) => Poll::Ready(Some(Event::Key(key))),
            Poll::Ready(Some(Ok(_))) => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Poll::Ready(Some(Err(_))) | Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
