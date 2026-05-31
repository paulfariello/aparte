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
    root::Root,
    scroll_win::ScrollWin,
    Action, ActionParser, CursorStyle, Dimensions, FocusRouted, LayoutParam, LayoutParams,
    MeasureSpec, MeasureSpecs, Motion, Operator, ParseResult, RegisterValue, Registers,
    RequestedDimension, RequestedDimensions, View,
};
use unicode_segmentation::UnicodeSegmentation;
use uuid::Uuid;
use xmpp_parsers::jid::{BareJid, Jid};

use crate::account::Account;
use crate::color::id_to_rgb;
use crate::command::Command;
use crate::config::{Config, Theme};
use crate::conversation::{Channel, Chat, Conversation};
use crate::core::{Aparte, AparteAsync, Event, ModTrait, UIMode as Mode};
use crate::i18n;
use crate::message::{Direction, Message, MessageView, XmppMessageType};
use crate::mods::bookmarks::BookmarksMod;
use crate::mods::messages::MessagesMod;
use crate::mods::omemo::OmemoEvent;
use crate::{contact, conversation};

#[derive(Clone, Debug)]
enum NormalCommand {
    SelectNext(usize),
    SelectPrev(usize),
    ScrollToTop,
    ScrollToBottom,
    SearchFirst(String),
    SearchNext,
    SearchPrev,
    SearchCancel,
}

/// A single line in the popup, keyed by insertion index so duplicates are preserved.
#[derive(Debug, Clone)]
struct PopupLine {
    index: usize,
    text: String,
}

impl PartialEq for PopupLine {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}
impl Eq for PopupLine {}
impl PartialOrd for PopupLine {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PopupLine {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.index.cmp(&other.index)
    }
}
impl Hash for PopupLine {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
    }
}
impl<E, C> View<E, C> for PopupLine {
    fn measure(&self, _: &MeasureSpecs) -> RequestedDimensions {
        RequestedDimensions {
            width: RequestedDimension::Absolute(self.text.as_str().into_charxels().display_width()),
            height: RequestedDimension::Absolute(1),
        }
    }
    fn layout(&mut self, _: &Dimensions) {}
    fn render(&self, mut frame: ScreenFrame<'_>, _: &C) {
        frame.write_at((0u16, 0u16), &self.text);
    }
    fn event(&mut self, _: &mut E) {}
}

#[allow(clippy::large_enum_variant)]
enum UIEvent {
    Core(Event),
    Validate(Rc<RefCell<Option<(String, bool)>>>),
    /// Explicit "start editing the selected message" request. Dispatched by
    /// the top-level `i` key handler only when focus is on the message frame,
    /// so we don't accidentally start an in-place edit when the user just
    /// wants to type in the input bar.
    StartEdit,
    /// XEP-0308 in-place edit commit. If the focused conversation has an
    /// editing MessageView, the slot is filled with `(original_message_id,
    /// new_body)` and the edit state is cleared. Used by the Enter handler
    /// to detect and send a correction instead of a fresh message.
    ValidateEdit(Rc<RefCell<Option<(String, String)>>>),
    InputChanged(String, Cursor, bool),
    AddWindow(
        String,
        Option<String>,
        Option<Box<dyn View<UIEvent, Theme>>>,
    ),
    ModeChange(Mode),
    NormalCommand {
        cmd: NormalCommand,
        bubbled: bool,
    },
    CommandBufferUpdate(String),
    SetInput(String),
    /// Apply a Normal-mode text action to the input bar.  The operator result
    /// (yanked or deleted text) is written into the slot when present.
    ApplyTextAction(Action, Rc<RefCell<Option<RegisterValue>>>),
    /// Atomically set the input bar content and cursor (grapheme index).
    SetInputState(String, usize),
    /// Insert `text` at the current cursor position in the input bar.
    Paste(String),
    ReduceHighlight(String, u64, u64),
    ShowPopup {
        title: Option<String>,
        lines: Vec<String>,
    },
    ClosePopup,
    /// Signals that the current window auto-selected a message (Normal mode,
    /// follow_bottom=true).  Emitted by mutating the in-flight event so the
    /// outer LinearLayout can set focused_child_index = FRAME_LAYOUT_INDEX
    /// after the dispatch loop.
    FocusFrame,
}

impl FocusRouted for UIEvent {
    fn is_focus_routed(&self) -> bool {
        match self {
            UIEvent::Core(event) => event.is_focus_routed(),
            _ => false,
        }
    }
}

/// Route a key event to a `MessageView` that is currently being edited in
/// place (XEP-0308 correction).
fn dispatch_edit_key(msg: &mut MessageView, key: &KeyEvent) {
    let Some(editor) = msg.edit.as_mut() else {
        return;
    };
    editor.handle_key_event(key);
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
            mode: Mode::Normal,
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
    fn focusable(&self) -> bool {
        false
    }

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
            let connection = format!(" {connection} |")
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
                .is_some_and(|jid| self.encrypted_jids.contains(jid));
            let prefix = if is_encrypted { " 🔒 " } else { " " };
            let mut title = format!("{prefix}{display}").into_charxels();

            let subjects = self
                .current_jid
                .as_deref()
                .and_then(|jid| self.subjects.get(jid));
            if let Some(subjects) = subjects {
                if let Some((_lang, subject)) = i18n::get_best(
                    subjects,
                    self.preferred_langs
                        .iter()
                        .map(std::string::String::as_str)
                        .collect(),
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
    fn focusable(&self) -> bool {
        false
    }

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
                    format!("+{remaining}").into_charxels()
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
                }
                frame.write(&highlighted);
                frame_space -= highlighted.display_width();

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
                self.command_buffer.clone_from(buf);
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
        }
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
    root: Root<UIEvent, Theme>,
    dirty: bool,
    password_command: Option<Command>,
    current_mode: Mode,
    popup_saved_mode: Option<Mode>,
    outgoing_event_queue: Rc<RefCell<Vec<Event>>>,
    _panic_handler: PanicHandler, // Defining panic_handler last guarantee that it will be dropped last (after terminal restoration)
    dimensions: Dimensions,
    /// Set after the first Enter press on a /command input. The next Enter send is allowed.
    slash_warned: bool,
    current_input: (String, Cursor, bool),
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
            root: Root::new(LinearLayout::<UIEvent, Theme>::new(Orientation::Vertical)),
            windows: Vec::new(),
            unread_windows: HashMap::new(),
            current_window: None,
            conversations: HashMap::new(),
            jid_to_name: HashMap::new(),
            password_command: None,
            current_mode: Mode::Normal,
            popup_saved_mode: None,
            outgoing_event_queue: Rc::new(RefCell::new(Vec::new())),
            _panic_handler: panic_handler,
            dirty: true,
            slash_warned: false,
            current_input: (String::new(), Cursor::new(0), false),
            dimensions: Dimensions {
                top: 1,
                left: 1,
                height,
                width,
            },
        }
    }

    #[allow(clippy::unused_self)]
    pub fn event_stream(&self) -> EventStream {
        EventStream::default()
    }

    fn get_scheduler(&self) -> Scheduler {
        Scheduler {
            queue: self.outgoing_event_queue.clone(),
        }
    }

    /// Send a XEP-0308 correction for the message identified by
    /// `original_id`, with the new body. Applies the correction to the local
    /// message store immediately and pushes the updated message into the UI
    /// synchronously, then schedules the outgoing stanza.
    fn send_correction(&mut self, aparte: &mut Aparte, original_id: &str, new_body: String) {
        let Some(current_window) = self.current_window.clone() else {
            return;
        };
        let Some(conversation) = self.conversations.get(&current_window).cloned() else {
            return;
        };

        let new_id = Uuid::new_v4().to_string();
        let timestamp: DateTime<FixedOffset> = LocalTz::now().into();
        let mut bodies = HashMap::new();
        bodies.insert(String::new(), new_body.clone());

        let (account, mut correction): (Account, Message) = match &conversation {
            Conversation::Chat(chat) => {
                let from: Jid = chat.account.clone().into();
                let to: Jid = chat.contact.clone().into();
                let msg = Message::outgoing_chat(
                    new_id.clone(),
                    timestamp,
                    &from,
                    &to,
                    bodies,
                    None,
                    false,
                );
                (chat.account.clone(), msg)
            }
            Conversation::Channel(channel) => {
                let Ok(us) = channel.account.to_bare().with_resource_str(&channel.nick) else {
                    return;
                };
                let from: Jid = us.into();
                let to: Jid = channel.jid.clone().into();
                let msg = Message::outgoing_channel(
                    new_id.clone(),
                    timestamp,
                    &from,
                    &to,
                    bodies,
                    None,
                    false,
                );
                (channel.account.clone(), msg)
            }
        };
        correction.set_correcting_id(original_id.to_string());

        // Apply correction to the local store and push the updated original
        // into the UI immediately, so the edited message updates in place.
        let updated = {
            let mut msgs = aparte.get_mod_mut::<crate::mods::messages::MessagesMod>();
            if let Some(original) = msgs.get_mut(&Some(account.clone()), &original_id.to_string()) {
                original.apply_correction(new_id, new_body, timestamp);
                Some(original.clone())
            } else {
                None
            }
        };
        if let Some(updated_msg) = updated {
            self.root.event(&mut UIEvent::Core(Event::Message(
                Some(account.clone()),
                updated_msg,
            )));
        }

        aparte.schedule(Event::SendMessage(account, correction));
    }

    #[allow(clippy::too_many_lines, clippy::similar_names)]
    fn add_conversation(&mut self, aparte: &mut Aparte, conversation: &Conversation) {
        let scheduler = self.get_scheduler();
        let selection_bg = aparte.config.theme.selected_message;
        let search_highlight_fg = aparte.config.theme.search_highlight_fg;
        let search_highlight_bg = aparte.config.theme.search_highlight_bg;
        match conversation {
            Conversation::Chat(chat) => {
                let chat_for_event = chat.clone();
                let chatwin = ScrollWin::<UIEvent, MessageView, Theme>::new()
                    .with_selection_bg(selection_bg)
                    .with_event({
                        let mut aparte = aparte.proxy();
                        let mut mam_requested = false;
                        let mut current_mode = Mode::Normal;
                        let mut follow_bottom = true;
                        let mut is_current_window = false;
                        let mut first_visit = true;
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
                                                if current_mode == Mode::Normal && follow_bottom {
                                                    view.update_selected(|msg| {
                                                        if msg.is_editing() {
                                                            msg.cancel_edit();
                                                        }
                                                    });
                                                    view.select_last_visible();
                                                    view.update_selected(MessageView::start_cursor);
                                                    if is_current_window {
                                                        *event = UIEvent::FocusFrame;
                                                    }
                                                }
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
                                                if current_mode == Mode::Normal && follow_bottom {
                                                    view.update_selected(|msg| {
                                                        if msg.is_editing() {
                                                            msg.cancel_edit();
                                                        }
                                                    });
                                                    view.select_last_visible();
                                                    view.update_selected(MessageView::start_cursor);
                                                    if is_current_window {
                                                        *event = UIEvent::FocusFrame;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                UIEvent::Core(Event::Key(KeyEvent {
                                    code: KeyCode::PageUp,
                                    ..
                                })) => {
                                    follow_bottom = false;
                                    let (at_top, _, _) = view.page_up();
                                    if at_top && !mam_requested {
                                        mam_requested = true;
                                        let from =
                                            view.first().map(|message| message.message.timestamp());
                                        scheduler.schedule(Event::LoadChatHistory {
                                            account: chat_for_event.account.clone(),
                                            contact: chat_for_event.contact.clone(),
                                            from: from.copied(),
                                        });
                                    }
                                }
                                UIEvent::Core(Event::Key(KeyEvent {
                                    code: KeyCode::PageDown,
                                    ..
                                })) => {
                                    view.page_down();
                                    mam_requested = false;
                                }
                                UIEvent::NormalCommand { bubbled, cmd } => match cmd {
                                    NormalCommand::SelectPrev(count) => {
                                        follow_bottom = false;
                                        view.update_selected(|msg| {
                                            if msg.is_editing() {
                                                msg.cancel_edit();
                                            }
                                        });
                                        for _ in 0..*count {
                                            let (old, new, at_top) = view.select_prev();
                                            if old.is_some() && old == new {
                                                *bubbled = true;
                                                view.clear_selection();
                                                break;
                                            }
                                            if at_top && !mam_requested {
                                                mam_requested = true;
                                                let from = view
                                                    .first()
                                                    .map(|message| message.message.timestamp());
                                                scheduler.schedule(Event::LoadChatHistory {
                                                    account: chat_for_event.account.clone(),
                                                    contact: chat_for_event.contact.clone(),
                                                    from: from.copied(),
                                                });
                                                break;
                                            }
                                        }
                                        if !*bubbled {
                                            view.update_selected(MessageView::start_cursor);
                                        }
                                    }
                                    NormalCommand::SelectNext(count) => {
                                        follow_bottom = false;
                                        view.update_selected(|msg| {
                                            if msg.is_editing() {
                                                msg.cancel_edit();
                                            }
                                        });
                                        for _ in 0..*count {
                                            let (old, new) = view.select_next();
                                            if old.is_some() && old == new {
                                                *bubbled = true;
                                                view.clear_selection();
                                                break;
                                            }
                                        }
                                        if !*bubbled {
                                            view.update_selected(MessageView::start_cursor);
                                        }
                                    }
                                    NormalCommand::ScrollToTop => {
                                        follow_bottom = false;
                                        view.update_selected(|msg| {
                                            if msg.is_editing() {
                                                msg.cancel_edit();
                                            }
                                        });
                                        view.scroll_to_top();
                                        if !mam_requested {
                                            mam_requested = true;
                                            let from = view
                                                .first()
                                                .map(|message| message.message.timestamp());
                                            scheduler.schedule(Event::LoadChatHistory {
                                                account: chat_for_event.account.clone(),
                                                contact: chat_for_event.contact.clone(),
                                                from: from.copied(),
                                            });
                                        }
                                    }
                                    NormalCommand::ScrollToBottom => {
                                        follow_bottom = true;
                                        view.update_selected(|msg| {
                                            if msg.is_editing() {
                                                msg.cancel_edit();
                                            }
                                        });
                                        view.scroll_to_bottom();
                                        view.update_selected(MessageView::start_cursor);
                                        mam_requested = false;
                                    }
                                    NormalCommand::SearchFirst(query) => {
                                        follow_bottom = false;
                                        view.set_search(query);
                                        for child in view.children_iter() {
                                            child.set_highlight(Some((
                                                query.clone(),
                                                search_highlight_fg,
                                                search_highlight_bg,
                                            )));
                                        }
                                    }
                                    NormalCommand::SearchNext => {
                                        follow_bottom = false;
                                        view.search_next();
                                    }
                                    NormalCommand::SearchPrev => {
                                        follow_bottom = false;
                                        view.search_prev();
                                    }
                                    NormalCommand::SearchCancel => {
                                        view.clear_search();
                                        for child in view.children_iter() {
                                            child.set_highlight(None);
                                        }
                                    }
                                },
                                UIEvent::Core(Event::ChangeWindow(name)) => {
                                    is_current_window = name == &chat_for_event.contact.to_string();
                                    if is_current_window && first_visit {
                                        first_visit = false;
                                        view.clear_selection();
                                    }
                                }
                                UIEvent::ModeChange(Mode::Normal) => {
                                    let was_command = current_mode == Mode::Command;
                                    current_mode = Mode::Normal;
                                    let editor_normal_mode = view
                                        .selected()
                                        .and_then(|msg| msg.edit.as_ref())
                                        .map(|e| e.normal_mode);
                                    match editor_normal_mode {
                                        Some(true) if !was_command => {
                                            view.update_selected(|msg| msg.cancel_edit());
                                        }
                                        Some(false) => {
                                            view.update_selected(|msg| msg.set_normal_mode(true));
                                        }
                                        _ => {}
                                    }
                                }
                                UIEvent::ModeChange(Mode::Command) => {
                                    current_mode = Mode::Command;
                                    // Keep selection alive so the cursor returns to the
                                    // message after Esc, at the start of message content.
                                    for child in view.children_iter() {
                                        child.set_highlight(None);
                                    }
                                }
                                UIEvent::ModeChange(Mode::Insert) => {
                                    current_mode = Mode::Insert;
                                    if is_current_window {
                                        follow_bottom = true;
                                        let editing =
                                            view.selected().is_some_and(MessageView::is_editing);
                                        if !editing {
                                            view.clear_selection();
                                        } else {
                                            view.update_selected(|msg| {
                                                msg.set_normal_mode(false);
                                            });
                                        }
                                    }
                                    for child in view.children_iter() {
                                        child.set_highlight(None);
                                    }
                                }
                                // Begin an in-place edit on the selected outgoing message.
                                // 'i' on a message: switch to Insert mode for outgoing
                                // messages; incoming messages stay in Normal mode (read-only).
                                UIEvent::StartEdit => {
                                    view.update_selected(|msg| {
                                        let is_outgoing = matches!(&msg.message,
                                            Message::Xmpp(m) if m.direction == Direction::Outgoing);
                                        if is_outgoing {
                                            if !msg.is_editing() {
                                                msg.start_cursor();
                                            }
                                            msg.set_normal_mode(false);
                                        }
                                    });
                                }
                                UIEvent::ApplyTextAction(action, slot) => {
                                    if view.selected().is_some_and(MessageView::is_editing) {
                                        let rv =
                                            view.update_selected(|msg| msg.apply_action(action));
                                        if let Some(Some(rv)) = rv {
                                            *slot.borrow_mut() = Some(rv);
                                        }
                                    }
                                }
                                // Route INSERT-mode key events to the selected
                                // message when it's being edited in place.
                                UIEvent::Core(Event::Key(key))
                                    if current_mode == Mode::Insert
                                        && view.selected().is_some_and(MessageView::is_editing) =>
                                {
                                    view.update_selected(|msg| dispatch_edit_key(msg, key));
                                }
                                // Commit an in-place edit: if the selected
                                // MessageView is editing, fill the slot with
                                // (original_id, new_body) and clear the edit
                                // state.
                                UIEvent::ValidateEdit(result) => {
                                    let pair = view.update_selected(|msg| {
                                        if !msg.is_insert_editing() {
                                            return None;
                                        }
                                        let id = match &msg.message {
                                            Message::Xmpp(m) => Some(m.id.clone()),
                                            Message::Log(_) => None,
                                        };
                                        let body = msg.take_edit();
                                        id.zip(body)
                                    });
                                    if let Some(Some(p)) = pair {
                                        *result.borrow_mut() = Some(p);
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
                let chanwin = ScrollWin::<UIEvent, MessageView, Theme>::new()
                    .with_selection_bg(selection_bg)
                    .with_event({
                        let mut aparte = aparte.proxy();
                        let mut mam_requested = false;
                        let mut current_mode = Mode::Normal;
                        let mut follow_bottom = true;
                        let mut is_current_window = false;
                        let mut first_visit = true;
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
                                                if current_mode == Mode::Normal && follow_bottom {
                                                    view.update_selected(|msg| {
                                                        if msg.is_editing() {
                                                            msg.cancel_edit();
                                                        }
                                                    });
                                                    view.select_last_visible();
                                                    view.update_selected(MessageView::start_cursor);
                                                    if is_current_window {
                                                        *event = UIEvent::FocusFrame;
                                                    }
                                                }
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
                                                if current_mode == Mode::Normal && follow_bottom {
                                                    view.update_selected(|msg| {
                                                        if msg.is_editing() {
                                                            msg.cancel_edit();
                                                        }
                                                    });
                                                    view.select_last_visible();
                                                    view.update_selected(MessageView::start_cursor);
                                                    if is_current_window {
                                                        *event = UIEvent::FocusFrame;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                UIEvent::Core(Event::Key(KeyEvent {
                                    code: KeyCode::PageUp,
                                    ..
                                })) => {
                                    follow_bottom = false;
                                    let (at_top, _, _) = view.page_up();
                                    if at_top && !mam_requested {
                                        mam_requested = true;
                                        let from =
                                            view.first().map(|message| message.message.timestamp());
                                        scheduler.schedule(Event::LoadChannelHistory {
                                            account: channel_for_event.account.clone(),
                                            jid: channel_for_event.jid.clone(),
                                            from: from.copied(),
                                        });
                                    }
                                }
                                UIEvent::Core(Event::Key(KeyEvent {
                                    code: KeyCode::PageDown,
                                    ..
                                })) => {
                                    view.page_down();
                                    mam_requested = false;
                                }
                                UIEvent::NormalCommand { bubbled, cmd } => match cmd {
                                    NormalCommand::SelectPrev(count) => {
                                        follow_bottom = false;
                                        view.update_selected(|msg| {
                                            if msg.is_editing() {
                                                msg.cancel_edit();
                                            }
                                        });
                                        for _ in 0..*count {
                                            let (old, new, at_top) = view.select_prev();
                                            if old.is_some() && old == new {
                                                *bubbled = true;
                                                view.clear_selection();
                                                break;
                                            }
                                            if at_top && !mam_requested {
                                                mam_requested = true;
                                                let from = view
                                                    .first()
                                                    .map(|message| message.message.timestamp());
                                                scheduler.schedule(Event::LoadChannelHistory {
                                                    account: channel_for_event.account.clone(),
                                                    jid: channel_for_event.jid.clone(),
                                                    from: from.copied(),
                                                });
                                                break;
                                            }
                                        }
                                        if !*bubbled {
                                            view.update_selected(MessageView::start_cursor);
                                        }
                                    }
                                    NormalCommand::SelectNext(count) => {
                                        follow_bottom = false;
                                        view.update_selected(|msg| {
                                            if msg.is_editing() {
                                                msg.cancel_edit();
                                            }
                                        });
                                        for _ in 0..*count {
                                            let (old, new) = view.select_next();
                                            if old.is_some() && old == new {
                                                *bubbled = true;
                                                view.clear_selection();
                                                break;
                                            }
                                        }
                                        if !*bubbled {
                                            view.update_selected(MessageView::start_cursor);
                                        }
                                    }
                                    NormalCommand::ScrollToTop => {
                                        follow_bottom = false;
                                        view.update_selected(|msg| {
                                            if msg.is_editing() {
                                                msg.cancel_edit();
                                            }
                                        });
                                        view.scroll_to_top();
                                        if !mam_requested {
                                            mam_requested = true;
                                            let from = view
                                                .first()
                                                .map(|message| message.message.timestamp());
                                            scheduler.schedule(Event::LoadChannelHistory {
                                                account: channel_for_event.account.clone(),
                                                jid: channel_for_event.jid.clone(),
                                                from: from.copied(),
                                            });
                                        }
                                    }
                                    NormalCommand::ScrollToBottom => {
                                        follow_bottom = true;
                                        view.update_selected(|msg| {
                                            if msg.is_editing() {
                                                msg.cancel_edit();
                                            }
                                        });
                                        view.scroll_to_bottom();
                                        view.update_selected(MessageView::start_cursor);
                                        mam_requested = false;
                                    }
                                    NormalCommand::SearchFirst(query) => {
                                        follow_bottom = false;
                                        view.set_search(query);
                                        for child in view.children_iter() {
                                            child.set_highlight(Some((
                                                query.clone(),
                                                search_highlight_fg,
                                                search_highlight_bg,
                                            )));
                                        }
                                    }
                                    NormalCommand::SearchNext => {
                                        follow_bottom = false;
                                        view.search_next();
                                    }
                                    NormalCommand::SearchPrev => {
                                        follow_bottom = false;
                                        view.search_prev();
                                    }
                                    NormalCommand::SearchCancel => {
                                        view.clear_search();
                                        for child in view.children_iter() {
                                            child.set_highlight(None);
                                        }
                                    }
                                },
                                UIEvent::Core(Event::ChangeWindow(name)) => {
                                    is_current_window = name == &channel_for_event.jid.to_string();
                                    if is_current_window && first_visit {
                                        first_visit = false;
                                        view.clear_selection();
                                    }
                                    if is_current_window && view.first().is_none() && !mam_requested
                                    {
                                        mam_requested = true;
                                        scheduler.schedule(Event::LoadChannelHistory {
                                            account: channel_for_event.account.clone(),
                                            jid: channel_for_event.jid.clone(),
                                            from: None,
                                        });
                                    }
                                }
                                UIEvent::ModeChange(Mode::Normal) => {
                                    let was_command = current_mode == Mode::Command;
                                    current_mode = Mode::Normal;
                                    let editor_normal_mode = view
                                        .selected()
                                        .and_then(|msg| msg.edit.as_ref())
                                        .map(|e| e.normal_mode);
                                    match editor_normal_mode {
                                        Some(true) if !was_command => {
                                            view.update_selected(|msg| msg.cancel_edit());
                                        }
                                        Some(false) => {
                                            view.update_selected(|msg| msg.set_normal_mode(true));
                                        }
                                        _ => {}
                                    }
                                }
                                UIEvent::ModeChange(Mode::Command) => {
                                    current_mode = Mode::Command;
                                    for child in view.children_iter() {
                                        child.set_highlight(None);
                                    }
                                }
                                UIEvent::ModeChange(Mode::Insert) => {
                                    current_mode = Mode::Insert;
                                    if is_current_window {
                                        follow_bottom = true;
                                        let editing =
                                            view.selected().is_some_and(MessageView::is_editing);
                                        if !editing {
                                            view.clear_selection();
                                        } else {
                                            view.update_selected(|msg| {
                                                msg.set_normal_mode(false);
                                            });
                                        }
                                    }
                                    for child in view.children_iter() {
                                        child.set_highlight(None);
                                    }
                                }
                                UIEvent::StartEdit => {
                                    view.update_selected(|msg| {
                                        let is_outgoing = matches!(&msg.message,
                                            Message::Xmpp(m) if m.direction == Direction::Outgoing);
                                        if is_outgoing {
                                            if !msg.is_editing() {
                                                msg.start_cursor();
                                            }
                                            msg.set_normal_mode(false);
                                        }
                                    });
                                }
                                UIEvent::ApplyTextAction(action, slot) => {
                                    if view.selected().is_some_and(MessageView::is_editing) {
                                        let rv =
                                            view.update_selected(|msg| msg.apply_action(action));
                                        if let Some(Some(rv)) = rv {
                                            *slot.borrow_mut() = Some(rv);
                                        }
                                    }
                                }
                                UIEvent::Core(Event::Key(key))
                                    if current_mode == Mode::Insert
                                        && view.selected().is_some_and(MessageView::is_editing) =>
                                {
                                    view.update_selected(|msg| dispatch_edit_key(msg, key));
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

fn dispatch_nav_command(
    cmd: NormalCommand,
    layout: &mut LinearLayout<UIEvent, Theme>,
    at_nav_bottom: &mut bool,
) {
    const FRAME_LAYOUT_INDEX: usize = 1;
    const INPUT_INDEX: usize = 3;
    let is_select_next = matches!(cmd, NormalCommand::SelectNext(_));
    let is_select_prev = matches!(cmd, NormalCommand::SelectPrev(_));

    // 'j' while already at the navigation bottom (input bar, after bubbling
    // past the last message) is a no-op — nowhere further to go.
    if is_select_next && *at_nav_bottom {
        return;
    }
    // Any nav command other than a repeated no-op clears the flag.
    *at_nav_bottom = false;

    let mut event = UIEvent::NormalCommand {
        cmd,
        bubbled: false,
    };
    if let Some(child) = layout.children.get_mut(FRAME_LAYOUT_INDEX) {
        child.child.view.event(&mut event);
    }
    let bubbled = matches!(event, UIEvent::NormalCommand { bubbled: true, .. });
    if bubbled {
        if is_select_next {
            if let Some(next) = layout
                .children
                .iter()
                .enumerate()
                .skip(FRAME_LAYOUT_INDEX + 1)
                .find(|(_, lc)| lc.child.view.insertable())
                .map(|(i, _)| i)
            {
                layout.set_focus(next);
                *at_nav_bottom = true;
            }
        } else if is_select_prev {
            layout.set_focus(INPUT_INDEX);
        }
    } else if is_select_next || is_select_prev {
        layout.set_focus(FRAME_LAYOUT_INDEX);
    }
}

fn dispatch_action(
    action: Action,
    layout: &mut LinearLayout<UIEvent, Theme>,
    registers: &mut Registers,
    mode: &mut Mode,
    aparte_proxy: &mut AparteAsync,
    at_nav_bottom: &mut bool,
    current_input: &mut (String, Cursor, bool),
) {
    const FRAME_LAYOUT_INDEX: usize = 1;
    if action.motion.is_navigation() {
        let cmd = match action.motion {
            Motion::Down => NormalCommand::SelectNext(action.count),
            Motion::Up => NormalCommand::SelectPrev(action.count),
            Motion::FileTop => NormalCommand::ScrollToTop,
            Motion::FileBottom => NormalCommand::ScrollToBottom,
            Motion::SearchNext => NormalCommand::SearchNext,
            Motion::SearchPrev => NormalCommand::SearchPrev,
            _ => return,
        };
        dispatch_nav_command(cmd, layout, at_nav_bottom);
    } else if matches!(action.motion, Motion::PasteAfter | Motion::PasteBefore) {
        let reg_name = action.register.unwrap_or(Registers::UNNAMED);
        if let Some(rv) = registers.get(reg_name) {
            let text = rv.text.clone();
            if matches!(action.motion, Motion::PasteAfter) {
                let move_action = Action {
                    count: 1,
                    register: None,
                    operator: Operator::Move,
                    motion: Motion::Right,
                };
                let slot = Rc::new(RefCell::new(None::<RegisterValue>));
                for child in layout.iter_children_mut() {
                    child.event(&mut UIEvent::ApplyTextAction(
                        move_action.clone(),
                        Rc::clone(&slot),
                    ));
                }
            }
            let mut paste_event = UIEvent::Paste(text.clone());
            for child in layout.iter_children_mut() {
                child.event(&mut paste_event);
            }
            if let UIEvent::InputChanged(ref buf, ref cursor, password) = paste_event {
                *current_input = (buf.clone(), cursor.clone(), password);
                aparte_proxy.schedule(Event::InputChanged(buf.clone(), cursor.clone(), password));
            }
        }
    } else {
        let slot = Rc::new(RefCell::new(None::<RegisterValue>));
        let frame_has_cursor = layout.focused_child_index == Some(FRAME_LAYOUT_INDEX);
        if frame_has_cursor {
            // Frame is focused: route to frame only.
            if let Some(child) = layout.children.get_mut(FRAME_LAYOUT_INDEX) {
                child.child.view.event(&mut UIEvent::ApplyTextAction(
                    action.clone(),
                    Rc::clone(&slot),
                ));
            }
        } else {
            // Focus on the input bar: send to non-frame children only.
            for (i, child) in layout.children.iter_mut().enumerate() {
                if i != FRAME_LAYOUT_INDEX {
                    child.child.view.event(&mut UIEvent::ApplyTextAction(
                        action.clone(),
                        Rc::clone(&slot),
                    ));
                }
            }
        }
        if let Some(rv) = slot.borrow_mut().take() {
            registers.yank(action.register, rv);
        }
        if matches!(action.operator, Operator::Change) {
            *mode = Mode::Insert;
            aparte_proxy.schedule(Event::UIMode(Mode::Insert));
            for child in layout.iter_children_mut() {
                child.event(&mut UIEvent::ModeChange(Mode::Insert));
            }
            if frame_has_cursor {
                layout.set_focus(FRAME_LAYOUT_INDEX);
            }
        }
    }
}

impl ModTrait for UIMod {
    #[allow(clippy::too_many_lines, clippy::similar_names)]
    fn init(&mut self, aparte: &mut Aparte) -> Result<(), ()> {
        // Indices into the root LinearLayout's children (push order below).
        const FRAME_LAYOUT_INDEX: usize = 1;
        const INPUT_INDEX: usize = 3;

        let (width, height) = crossterm::terminal::size().unwrap();
        log::debug!("Init UI on screen ({width}×{height})");

        let layout;
        {
            let mut mode = Mode::Normal;
            let mut timeout_generation: u64 = 0;
            let mut saved_input = String::new();
            let mut action_parser = ActionParser::new();
            let mut registers = Registers::new();
            let mut aparte_proxy = aparte.proxy();
            let mut current_window = String::new();
            let mut visited_windows: HashSet<String> = HashSet::new();
            let render_buffer_for_ctrl_l = std::sync::Arc::clone(&self.render_buffer);
            let mut at_nav_bottom = false;
            let mut message_cursor_active = false;
            let mut current_input: (String, Cursor, bool) = (String::new(), Cursor::new(0), false);
            layout = LinearLayout::<UIEvent, Theme>::new(Orientation::Vertical).with_event(
                move |layout, event| match event {
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Esc, ..
                    })) if mode == Mode::Insert => {
                        mode = Mode::Normal;
                        aparte_proxy.schedule(Event::UIMode(Mode::Normal));
                        action_parser.reset();
                        let focus_on_frame = layout.focused_child_index == Some(FRAME_LAYOUT_INDEX);
                        if focus_on_frame {
                            // Esc while editing: switch editor from Insert to Normal mode.
                            // A second Esc (in Normal mode) cancels the edit.
                            for child in layout.iter_children_mut() {
                                child.event(&mut UIEvent::ModeChange(Mode::Normal));
                            }
                        } else {
                            // Vim: when leaving Insert on the input bar, cursor
                            // moves back one if at the end of a non-empty buffer.
                            let (buf, cursor, _) = current_input.clone();
                            let len = buf.graphemes(true).count();
                            if cursor.get() == len && len > 0 {
                                let new_pos = len - 1;
                                let mut sis_event = UIEvent::SetInputState(buf.clone(), new_pos);
                                for child in layout.iter_children_mut() {
                                    child.event(&mut sis_event);
                                }
                                if let UIEvent::InputChanged(ref buf2, ref cursor2, password) =
                                    sis_event
                                {
                                    current_input = (buf2.clone(), cursor2.clone(), password);
                                    aparte_proxy.schedule(Event::InputChanged(
                                        buf2.clone(),
                                        cursor2.clone(),
                                        password,
                                    ));
                                }
                            }
                            for child in layout.iter_children_mut() {
                                child.event(&mut UIEvent::ModeChange(Mode::Normal));
                            }
                        }
                    }
                    // Esc in Normal mode: if focus is on the message frame,
                    // fire ModeChange(Normal) so the scroll win can cancel the edit.
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Esc, ..
                    })) if mode == Mode::Normal => {
                        let focus_on_frame = layout.focused_child_index == Some(FRAME_LAYOUT_INDEX);
                        if focus_on_frame {
                            for child in layout.iter_children_mut() {
                                child.event(&mut UIEvent::ModeChange(Mode::Normal));
                            }
                        }
                    }
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Char('i'),
                        ..
                    })) if mode == Mode::Normal && !action_parser.is_pending() => {
                        let focused_is_insertable = layout
                            .focused_child()
                            .map(|c| c.insertable())
                            .unwrap_or(false);
                        if focused_is_insertable {
                            // Only request an in-place edit when focus is on
                            // the message frame. Otherwise (focus on input
                            // bar), `i` just enters INSERT for typing — even
                            // if the auto-selection on NORMAL highlighted an
                            // outgoing message.
                            let focus_on_frame =
                                layout.focused_child_index == Some(FRAME_LAYOUT_INDEX);
                            if focus_on_frame {
                                if let Some(focused) = layout.focused_child_mut() {
                                    focused.event(&mut UIEvent::StartEdit);
                                }
                                message_cursor_active = true;
                            }
                            action_parser.reset();
                            mode = Mode::Insert;
                            aparte_proxy.schedule(Event::UIMode(Mode::Insert));
                            for child in layout.iter_children_mut() {
                                child.event(&mut UIEvent::CommandBufferUpdate(String::new()));
                                child.event(&mut UIEvent::ModeChange(Mode::Insert));
                            }
                        }
                        // If focused component is not insertable, deny INSERT mode silently.
                    }
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Char(':'),
                        ..
                    })) if mode == Mode::Normal => {
                        mode = Mode::Command;
                        aparte_proxy.schedule(Event::UIMode(Mode::Command));
                        action_parser.reset();
                        layout.set_focus(INPUT_INDEX);
                        saved_input = current_input.0.clone();
                        for child in layout.iter_children_mut() {
                            child.event(&mut UIEvent::ModeChange(Mode::Command));
                        }
                        let mut si_event = UIEvent::SetInput(":".to_string());
                        for child in layout.iter_children_mut() {
                            child.event(&mut si_event);
                        }
                        if let UIEvent::InputChanged(ref buf, ref cursor, password) = si_event {
                            current_input = (buf.clone(), cursor.clone(), password);
                            aparte_proxy.schedule(Event::InputChanged(
                                buf.clone(),
                                cursor.clone(),
                                password,
                            ));
                        }
                    }
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Char('/'),
                        ..
                    })) if mode == Mode::Normal => {
                        mode = Mode::Command;
                        aparte_proxy.schedule(Event::UIMode(Mode::Command));
                        action_parser.reset();
                        layout.set_focus(INPUT_INDEX);
                        saved_input = current_input.0.clone();
                        for child in layout.iter_children_mut() {
                            child.event(&mut UIEvent::ModeChange(Mode::Command));
                        }
                        let mut si_event = UIEvent::SetInput("/".to_string());
                        for child in layout.iter_children_mut() {
                            child.event(&mut si_event);
                        }
                        if let UIEvent::InputChanged(ref buf, ref cursor, password) = si_event {
                            current_input = (buf.clone(), cursor.clone(), password);
                            aparte_proxy.schedule(Event::InputChanged(
                                buf.clone(),
                                cursor.clone(),
                                password,
                            ));
                        }
                    }
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Esc, ..
                    })) if mode == Mode::Command => {
                        mode = Mode::Normal;
                        aparte_proxy.schedule(Event::UIMode(Mode::Normal));
                        action_parser.reset();
                        if message_cursor_active {
                            layout.set_focus(FRAME_LAYOUT_INDEX);
                        }
                        let saved = std::mem::take(&mut saved_input);
                        if let Some(frame) = layout.children.get_mut(FRAME_LAYOUT_INDEX) {
                            frame.child.view.event(&mut UIEvent::NormalCommand {
                                cmd: NormalCommand::SearchCancel,
                                bubbled: false,
                            });
                        }
                        for child in layout.iter_children_mut() {
                            child.event(&mut UIEvent::ModeChange(Mode::Normal));
                            child.event(&mut UIEvent::CommandBufferUpdate(String::new()));
                        }
                        let mut si_event = UIEvent::SetInput(saved.clone());
                        for child in layout.iter_children_mut() {
                            child.event(&mut si_event);
                        }
                        if let UIEvent::InputChanged(ref buf, ref cursor, password) = si_event {
                            current_input = (buf.clone(), cursor.clone(), password);
                            aparte_proxy.schedule(Event::InputChanged(
                                buf.clone(),
                                cursor.clone(),
                                password,
                            ));
                        }
                    }
                    // All other keys in COMMAND mode go to the focused input widget,
                    // reusing its editing logic (Ctrl+A/B/E/F/H/W/U/K, arrows, Home/End,
                    // Delete, history).
                    UIEvent::Core(Event::Key(_)) if mode == Mode::Command => {
                        layout.route_to_focused(event);
                        if let UIEvent::InputChanged(ref buf, ref cursor, password) = *event {
                            current_input = (buf.clone(), cursor.clone(), password);
                            aparte_proxy.schedule(Event::InputChanged(
                                buf.clone(),
                                cursor.clone(),
                                password,
                            ));
                        }
                    }
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Char(c),
                        modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
                        ..
                    })) if mode == Mode::Normal => {
                        match action_parser.feed(*c) {
                            ParseResult::Pending => {
                                // Show partial sequence; schedule a 1s timeout.
                                timeout_generation += 1;
                                let gen = timeout_generation;
                                let mut aparte_for_task = aparte_proxy.clone();
                                tokio::spawn(async move {
                                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                    aparte_for_task.schedule(Event::CommandTimeout(gen));
                                });
                                let buf = action_parser.pending().to_string();
                                for child in layout.iter_children_mut() {
                                    child.event(&mut UIEvent::CommandBufferUpdate(buf.clone()));
                                }
                            }
                            ParseResult::Complete(action) => {
                                for child in layout.iter_children_mut() {
                                    child.event(&mut UIEvent::CommandBufferUpdate(String::new()));
                                }
                                dispatch_action(
                                    action,
                                    layout,
                                    &mut registers,
                                    &mut mode,
                                    &mut aparte_proxy,
                                    &mut at_nav_bottom,
                                    &mut current_input,
                                );
                            }
                            ParseResult::Invalid => {
                                for child in layout.iter_children_mut() {
                                    child.event(&mut UIEvent::CommandBufferUpdate(String::new()));
                                }
                            }
                        }
                    }
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Up, ..
                    })) if mode == Mode::Normal => {
                        dispatch_nav_command(
                            NormalCommand::SelectPrev(1),
                            layout,
                            &mut at_nav_bottom,
                        );
                    }
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Down,
                        ..
                    })) if mode == Mode::Normal => {
                        dispatch_nav_command(
                            NormalCommand::SelectNext(1),
                            layout,
                            &mut at_nav_bottom,
                        );
                    }
                    UIEvent::Core(Event::CommandTimeout(gen)) => {
                        if *gen == timeout_generation && action_parser.is_pending() {
                            action_parser.reset();
                            for child in layout.iter_children_mut() {
                                child.event(&mut UIEvent::CommandBufferUpdate(String::new()));
                            }
                        }
                    }
                    // Ctrl+L forces a full clean repaint in any mode.
                    UIEvent::Core(Event::Key(KeyEvent {
                        code: KeyCode::Char('l'),
                        modifiers: KeyModifiers::CONTROL,
                        ..
                    })) => {
                        render_buffer_for_ctrl_l
                            .read()
                            .unwrap()
                            .request_full_render();
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
                        layout.route_to_focused(event);
                        if let UIEvent::InputChanged(ref buf, ref cursor, password) = *event {
                            current_input = (buf.clone(), cursor.clone(), password);
                            aparte_proxy.schedule(Event::InputChanged(
                                buf.clone(),
                                cursor.clone(),
                                password,
                            ));
                        }
                    }
                    // Enter in Command mode executes the typed command.
                    // (Enter is delivered as UIEvent::Validate by on_event, not as a Key event.)
                    UIEvent::Validate(result) if mode == Mode::Command => {
                        let cmd = current_input.0.clone();

                        mode = Mode::Normal;
                        aparte_proxy.schedule(Event::UIMode(Mode::Normal));
                        if message_cursor_active {
                            layout.set_focus(FRAME_LAYOUT_INDEX);
                        }
                        let saved = std::mem::take(&mut saved_input);
                        for child in layout.iter_children_mut() {
                            child.event(&mut UIEvent::ModeChange(Mode::Normal));
                            child.event(&mut UIEvent::CommandBufferUpdate(String::new()));
                        }
                        let mut si_event = UIEvent::SetInput(saved.clone());
                        for child in layout.iter_children_mut() {
                            child.event(&mut si_event);
                        }
                        if let UIEvent::InputChanged(ref buf, ref cursor, password) = si_event {
                            current_input = (buf.clone(), cursor.clone(), password);
                            aparte_proxy.schedule(Event::InputChanged(
                                buf.clone(),
                                cursor.clone(),
                                password,
                            ));
                        }
                        if let Some(query) = cmd.strip_prefix('/') {
                            let query = query.to_string();
                            if !query.is_empty() {
                                if let Some(frame) = layout.children.get_mut(FRAME_LAYOUT_INDEX) {
                                    frame.child.view.event(&mut UIEvent::NormalCommand {
                                        cmd: NormalCommand::SearchFirst(query),
                                        bubbled: false,
                                    });
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
                        let clean = terminus::clean_str(name);
                        let first_visit = visited_windows.insert(clean.clone());
                        current_window = clean;
                        for child in layout.iter_children_mut() {
                            child.event(event);
                        }
                        if first_visit {
                            layout.set_focus(INPUT_INDEX);
                        }
                    }
                    UIEvent::ModeChange(new_mode) => match *new_mode {
                        Mode::Normal => {
                            mode = Mode::Normal;
                            message_cursor_active = false;
                            aparte_proxy.schedule(Event::UIMode(Mode::Normal));
                            action_parser.reset();
                            for child in layout.iter_children_mut() {
                                child.event(&mut UIEvent::CommandBufferUpdate(String::new()));
                                child.event(&mut UIEvent::ModeChange(Mode::Normal));
                            }
                        }
                        Mode::Insert => {
                            let focused_is_insertable = layout
                                .focused_child()
                                .map(|c| c.insertable())
                                .unwrap_or(false);
                            if focused_is_insertable {
                                mode = Mode::Insert;
                                aparte_proxy.schedule(Event::UIMode(Mode::Insert));
                                for child in layout.iter_children_mut() {
                                    child.event(&mut UIEvent::CommandBufferUpdate(String::new()));
                                    child.event(&mut UIEvent::ModeChange(Mode::Insert));
                                }
                            }
                        }
                        Mode::Command => {
                            for child in layout.iter_children_mut() {
                                child.event(event);
                            }
                        }
                    },
                    _ => {
                        for child in layout.iter_children_mut() {
                            child.event(event);
                        }
                        if matches!(*event, UIEvent::FocusFrame) {
                            layout.set_focus(FRAME_LAYOUT_INDEX);
                        } else if let UIEvent::InputChanged(ref buf, ref cursor, password) = *event
                        {
                            current_input = (buf.clone(), cursor.clone(), password);
                            aparte_proxy.schedule(Event::InputChanged(
                                buf.clone(),
                                cursor.clone(),
                                password,
                            ));
                        }
                    }
                },
            );
        }

        let win_bar = WinBar::new();
        let frame =
            FrameLayout::<UIEvent, String, Theme>::new().with_event(|frame, event| match event {
                UIEvent::Core(Event::ChangeWindow(name)) => {
                    frame.set_current(name.clone());
                    for child in frame.iter_children_mut() {
                        child.event(event);
                    }
                }
                UIEvent::AddWindow(jid, display_name, view) => {
                    let view = view.take().unwrap();
                    frame.insert_boxed(jid.clone(), view);

                    // propagate AddWindow with jid only to each subview
                    // required at least for console view
                    for child in frame.iter_children_mut() {
                        child.event(&mut UIEvent::AddWindow(
                            jid.clone(),
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
                UIEvent::Core(
                    Event::Key(_)
                    | Event::Completed(_, _)
                    | Event::ResetCompletion
                    | Event::ReadPassword(_),
                )
                // NormalCommand must go to the current window only: all windows
                // get ModeChange(Normal) and call select_last_visible(), so any
                // non-current window that is already at its last selection would
                // set `bubbled = true` on the shared event, making the outer
                // layout think navigation failed when it actually succeeded.
                | UIEvent::NormalCommand { .. }
                | UIEvent::StartEdit
                // Text actions go to the focused window only.
                | UIEvent::ApplyTextAction(..) => frame.route_to_focused(event),
                // Global events (Message, Notification, Subject, etc.) → all windows
                _ => {
                    for child in frame.iter_children_mut() {
                        child.event(event);
                    }
                }
            });
        let title_bar = TitleBar::new(aparte.config.preferred_langs.clone());
        let input = Input::new().with_event(|input, event| match event {
            UIEvent::Core(Event::Key(key)) => {
                log::debug!("Input event: {:?}", key);
                match key.code {
                    KeyCode::Up => input.previous(),
                    KeyCode::Down => input.next(),
                    _ => {
                        input.editor.handle_key_event(key);
                    }
                }
                *event = UIEvent::InputChanged(
                    input.editor.buf.clone(),
                    input.editor.cursor.clone(),
                    input.password,
                );
            }
            UIEvent::Validate(result) => {
                let mut result = result.borrow_mut();
                result.replace(input.validate());
            }
            UIEvent::Core(Event::Completed(raw_buf, cursor)) => {
                input.editor.buf.clone_from(raw_buf);
                input.editor.cursor.clone_from(cursor);
                *event = UIEvent::InputChanged(
                    input.editor.buf.clone(),
                    input.editor.cursor.clone(),
                    input.password,
                );
            }
            UIEvent::Core(Event::ReadPassword(_)) => input.password(),
            UIEvent::SetInput(text) => {
                input.editor.cursor =
                    Cursor::from_index(text, text.len()).unwrap_or_else(|_| Cursor::new(0));
                input.editor.buf.clone_from(text);
                *event = UIEvent::InputChanged(
                    input.editor.buf.clone(),
                    input.editor.cursor.clone(),
                    input.password,
                );
            }
            UIEvent::ApplyTextAction(action, slot) => {
                if let Some(rv) = input.editor.apply_action(action) {
                    *slot.borrow_mut() = Some(rv);
                }
            }
            UIEvent::SetInputState(content, cursor_pos) => {
                input.editor.buf = content.clone();
                input.editor.cursor = Cursor::new(*cursor_pos);
                *event = UIEvent::InputChanged(
                    input.editor.buf.clone(),
                    input.editor.cursor.clone(),
                    input.password,
                );
            }
            UIEvent::Paste(text) => {
                for c in text.chars() {
                    input.key(c);
                }
                *event = UIEvent::InputChanged(
                    input.editor.buf.clone(),
                    input.editor.cursor.clone(),
                    input.password,
                );
            }
            UIEvent::ModeChange(Mode::Normal) => {
                input.set_show_cursor(true);
                input.set_cursor_style(CursorStyle::SteadyBlock);
                input.set_cursor_priority(1);
            }
            UIEvent::ModeChange(Mode::Command) => {
                input.set_show_cursor(true);
                input.set_cursor_style(CursorStyle::SteadyBar);
                // Priority 3 beats the message-selection cursor (priority 2) so
                // the terminal cursor moves to the command bar.
                input.set_cursor_priority(3);
            }
            UIEvent::ModeChange(Mode::Insert) => {
                input.set_show_cursor(true);
                input.set_cursor_style(CursorStyle::SteadyBar);
                input.set_cursor_priority(1);
            }
            _ => {}
        });

        let mut layout = layout;
        layout.push(win_bar, 0);
        layout.push(frame, 1);
        layout.push(title_bar, 0);
        layout.push(input, 0);
        layout.set_focus(INPUT_INDEX);

        self.root = Root::new(layout).with_event(|root, event| match event {
            UIEvent::ShowPopup { title, lines } => {
                let mut scroll_win = ScrollWin::<UIEvent, PopupLine, Theme>::new()
                    .with_layout(LayoutParams {
                        width: LayoutParam::WrapContent,
                        height: LayoutParam::WrapContent,
                    })
                    .with_event(|view, event| match event {
                        UIEvent::Core(Event::Key(KeyEvent {
                            code: KeyCode::PageUp,
                            ..
                        })) => {
                            view.page_up();
                        }
                        UIEvent::Core(Event::Key(KeyEvent {
                            code: KeyCode::PageDown,
                            ..
                        })) => {
                            view.page_down();
                        }
                        _ => {}
                    });
                for (i, line) in lines.iter().enumerate() {
                    scroll_win.insert(PopupLine {
                        index: i,
                        text: line.clone(),
                    });
                }
                // Start at the top: reset to index 0, then clear auto-selection.
                scroll_win.scroll_to_top();
                scroll_win.clear_selection();
                root.show(Box::new(scroll_win), title.clone());
            }
            UIEvent::ClosePopup => {
                root.hide();
            }
            // Key events route to popup (when visible) or background.
            UIEvent::Core(Event::Key(_)) => root.route_to_focused(event),
            // All other events always go to the background.
            _ => root.background_mut().event(event),
        });

        let mut console = LinearLayout::<UIEvent, Theme>::new(Orientation::Horizontal).with_event(
            |layout, event| {
                for LayoutChild { child, .. } in &mut layout.children {
                    child.view.event(event);
                }
            },
        );
        let console_selection_bg = aparte.config.theme.selected_message;
        let console_search_highlight_fg = aparte.config.theme.search_highlight_fg;
        let console_search_highlight_bg = aparte.config.theme.search_highlight_bg;
        console.push(
            ScrollWin::<UIEvent, MessageView, Theme>::new()
                .with_layout(LayoutParams {
                    width: LayoutParam::MatchParent,
                    height: LayoutParam::MatchParent,
                })
                .with_selection_bg(console_selection_bg)
                .with_event({
                    let mut aparte = aparte.proxy();
                    let search_highlight_fg = console_search_highlight_fg;
                    let search_highlight_bg = console_search_highlight_bg;
                    let mut is_current_window = false;
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
                            view.page_up();
                        }
                        UIEvent::Core(Event::Key(KeyEvent {
                            code: KeyCode::PageDown,
                            ..
                        })) => {
                            view.page_down();
                        }
                        UIEvent::NormalCommand { bubbled, cmd } => match cmd {
                            NormalCommand::SelectPrev(count) => {
                                for _ in 0..*count {
                                    let (old, new, _) = view.select_prev();
                                    if old.is_some() && old == new {
                                        *bubbled = true;
                                        view.clear_selection();
                                        break;
                                    }
                                }
                            }
                            NormalCommand::SelectNext(count) => {
                                for _ in 0..*count {
                                    let (old, new) = view.select_next();
                                    if old.is_some() && old == new {
                                        *bubbled = true;
                                        view.clear_selection();
                                        break;
                                    }
                                }
                            }
                            NormalCommand::ScrollToTop => {
                                view.scroll_to_top();
                            }
                            NormalCommand::ScrollToBottom => {
                                view.scroll_to_bottom();
                            }
                            NormalCommand::SearchFirst(query) => {
                                view.set_search(query);
                                for child in view.children_iter() {
                                    child.set_highlight(Some((
                                        query.clone(),
                                        search_highlight_fg,
                                        search_highlight_bg,
                                    )));
                                }
                            }
                            NormalCommand::SearchNext => {
                                view.search_next();
                            }
                            NormalCommand::SearchPrev => {
                                view.search_prev();
                            }
                            NormalCommand::SearchCancel => {
                                view.clear_search();
                                for child in view.children_iter() {
                                    child.set_highlight(None);
                                }
                            }
                        },
                        UIEvent::Core(Event::ChangeWindow(name)) => {
                            is_current_window = name == "console";
                        }
                        UIEvent::ModeChange(Mode::Insert | Mode::Command) => {
                            if is_current_window {
                                view.clear_selection();
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
                UIEvent::Core(Event::Contact(_, contact) | Event::ContactUpdate(_, contact)) => {
                    if contact.groups.is_empty() {
                        let group = contact::Group(String::from("Contacts"));
                        view.insert(RosterItem::Contact(contact.clone()), Some(group));
                    } else {
                        for group in &contact.groups {
                            view.insert(RosterItem::Contact(contact.clone()), Some(group.clone()));
                        }
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

        // Broadcast the initial mode so views (cursor style, title bar, etc.) are consistent.
        self.root.event(&mut UIEvent::ModeChange(Mode::Normal));

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

    #[allow(clippy::too_many_lines)]
    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        let before = Instant::now();

        match event {
            Event::ReadPassword(command) => {
                self.password_command = Some(command.clone());
                self.root.event(&mut UIEvent::ModeChange(Mode::Insert));
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

                            self.add_conversation(aparte, &conversation);
                        }
                    }
                    Message::Log(_message) => {}
                }

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
                        &Conversation::Chat(Chat {
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
                    self.add_conversation(aparte, &Conversation::Channel(ch));
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
                        .event(&mut UIEvent::Core(Event::Close(window.clone())));
                }
            }
            Event::UIMode(mode) => {
                self.current_mode = *mode;
            }
            Event::InputChanged(buf, cursor, password) => {
                self.current_input = (buf.clone(), cursor.clone(), *password);
            }
            Event::Key(key) => {
                match key {
                    KeyEvent {
                        code: KeyCode::Tab, ..
                    } => {
                        let (raw_buf, cursor, password) = self.current_input.clone();

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
                        // First, see whether a MessageView is being edited in
                        // place — if so, commit it as a XEP-0308 correction
                        // rather than running the input-bar send path.
                        let edit_result: Rc<RefCell<Option<(String, String)>>> =
                            Rc::new(RefCell::new(None));
                        self.root
                            .event(&mut UIEvent::ValidateEdit(Rc::clone(&edit_result)));
                        if let Some((original_id, new_body)) = edit_result.borrow_mut().take() {
                            self.send_correction(aparte, &original_id, new_body);
                            // Return to NORMAL mode; the layout closure listens
                            // for ModeChange and updates its captured `mode`.
                            self.root.event(&mut UIEvent::ModeChange(Mode::Normal));
                            aparte.schedule(Event::UIMode(Mode::Normal));
                            return;
                        }

                        let result = Rc::new(RefCell::new(None));
                        // TODO avoid direct send to root, should go back to main event loop
                        self.root.event(&mut UIEvent::Validate(Rc::clone(&result)));

                        let result = result.borrow_mut();
                        let (raw_buf, password) = result.as_ref().unwrap();
                        let raw_buf = raw_buf.clone();

                        let looks_like_cmd = !password
                            && !raw_buf.is_empty()
                            && ((raw_buf.starts_with('/') && !raw_buf.starts_with("/me "))
                                || raw_buf.starts_with(':'));

                        if *password {
                            let mut command = self.password_command.take().unwrap();
                            command.args.push(raw_buf);
                            aparte.schedule(Event::Command(command));
                            self.root.event(&mut UIEvent::ModeChange(Mode::Normal));
                        } else if looks_like_cmd && !self.slash_warned {
                            self.slash_warned = true;
                            aparte.schedule(Event::ShowPopup {
                                title: None,
                                lines: vec![
                                    format!(
                                        "\"{}\" looks like a command but is not recognized.",
                                        raw_buf
                                    ),
                                    String::new(),
                                    "Commands are entered in normal mode (Esc, then :command)."
                                        .to_string(),
                                    "Press Enter again to send as plain text, or ESC to edit."
                                        .to_string(),
                                ],
                            });
                        } else if !raw_buf.is_empty() {
                            self.slash_warned = false;
                            aparte.schedule(Event::ClosePopup);
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
                                            bodies.insert(String::new(), raw_buf);
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
                                            bodies.insert(String::new(), raw_buf);
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
                    KeyEvent {
                        code: KeyCode::Esc, ..
                    } if self.root.is_visible() => {
                        self.root.event(&mut UIEvent::ClosePopup);
                        if let Some(saved) = self.popup_saved_mode.take() {
                            self.root.event(&mut UIEvent::ModeChange(saved));
                        }
                    }
                    _ => {
                        // Reset the slash-command warning only when the popup is closed
                        // (while the popup is visible, keys are absorbed by its content).
                        if !self.root.is_visible() {
                            self.slash_warned = false;
                        }
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
            Event::ShowPopup { title, lines } => {
                self.popup_saved_mode = Some(self.current_mode);
                self.root.event(&mut UIEvent::ShowPopup {
                    title: title.clone(),
                    lines: lines.clone(),
                });
                self.root.event(&mut UIEvent::ModeChange(Mode::Normal));
            }
            Event::ClosePopup => {
                self.root.event(&mut UIEvent::ClosePopup);
                if let Some(saved) = self.popup_saved_mode.take() {
                    self.root.event(&mut UIEvent::ModeChange(saved));
                }
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
            Poll::Ready(Some(Err(_)) | None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
