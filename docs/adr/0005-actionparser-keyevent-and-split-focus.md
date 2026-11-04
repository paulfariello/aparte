# ADR-0005: ActionParser takes KeyEvent; split layouts route commands to focused pane

**Status:** Accepted

## Context

Two related problems required a coordinated design decision.

### 1. Vim split navigation (`Ctrl+W h`/`l`)

`ActionParser` (in `terminus`) previously consumed only `char` values — the printable character extracted from a `crossterm` `KeyEvent`. This was sufficient for sequences like `gg`, `5j`, or `"adw`, which are entirely composed of printable characters.

`Ctrl+W h`/`l` (vim's split-navigation chord) begins with a control key (`KeyCode::Char('w')` with `KeyModifiers::CONTROL`), which cannot be represented as a `char`. The two alternatives were:

- **External leader flag**: intercept `Ctrl+W` in the `on_event` key handler with a `ctrl_w_pending: bool` flag, outside the parser.
- **Extend ActionParser to `KeyEvent`**: change `feed(&mut self, c: char)` to `feed(&mut self, key: KeyEvent)`, add a `CtrlWPrefix` parser state alongside the existing `GPrefix`, and add `Motion::FocusPaneLeft` / `Motion::FocusPaneRight`.

`crossterm` is already a `terminus` dependency. The external flag would bypass the parser's timeout, `pending()` display, and reset logic — duplicating infrastructure. Every future multi-key chord involving a control key would need its own external flag.

### 2. Generic command dispatch on split layouts

The console window (`LinearLayout` with horizontal orientation: message `ScrollWin` on the left, roster panel on the right) previously broadcast every event to both children. Normal-mode navigation commands (`NormalCommand::SelectNext`, `SearchFirst`, etc.) reached both panels simultaneously; the roster panel ignored them because `ListView` had no such handlers.

Adding a scrollable roster requires routing navigation commands to whichever panel is currently focused, not to both.

## Decision

### ActionParser takes KeyEvent

`ActionParser::feed` is changed from `feed(&mut self, c: char)` to `feed(&mut self, key: KeyEvent)`. Internally:

- Printable characters with `NONE` or `SHIFT` modifiers follow the existing char-based state machine unchanged.
- `KeyCode::Char('w')` with `CONTROL` modifier enters a new `CtrlWPrefix` parser state.
- From `CtrlWPrefix`, `h` (plain) emits `Motion::FocusPaneLeft`; `l` (plain) emits `Motion::FocusPaneRight`; anything else is `Invalid`.

`Motion::FocusPaneLeft` and `Motion::FocusPaneRight` are added to the `Motion` enum and return `true` from `is_navigation()`. The call site in `ui.rs` widens its outer match arm to include `KeyModifiers::CONTROL` so these keys reach the parser.

`dispatch_action` maps `FocusPaneLeft`/`Right` to `NormalCommand::FocusPaneLeft`/`FocusPaneRight` and sends them to the active frame (same path as other navigation commands).

### Split layouts route to focused pane

The console layout's event handler changes from unconditional broadcast to focus-aware routing:

- A `focused_pane: usize` variable is captured in the closure (0 = message view, 1 = roster). Default is 0.
- `NormalCommand::FocusPaneLeft` sets `focused_pane = 0`; `NormalCommand::FocusPaneRight` sets `focused_pane = 1`.
- All other `NormalCommand` variants are dispatched only to the focused pane.
- Data events (contacts, bookmarks, window add/remove, mode changes) are still broadcast to all children.

### Roster panel: ScrollWin\<RosterRow\> replaces ListView

The roster `ListView` is replaced with a `ScrollWin<RosterRow>`. `RosterRow` is an enum with two variants:
- `GroupHeader(contact::Group)` — renders a group title, not selectable.
- `Item(RosterItem)` — a contact, bookmark, or window entry; carries the data needed to open the conversation on Enter.

`RosterRow` implements `View`, `Hash`, `Eq`, `Ord`, and `Searchable`. The `Ord` key encodes group index and within-group sort position so that group headers sort before their items and groups appear in the fixed order Windows → Contacts → Bookmarks. The roster `ScrollWin` handles `NormalCommand` events identically to the message `ScrollWin`. Pressing Enter on a selected item schedules `Event::Win(<jid>)`.

## Consequences

- `ActionParser`'s public API changes from `feed(char)` to `feed(KeyEvent)`. All callers (currently one site in `ui.rs`) must update.
- `Motion` gains two navigation variants that do not operate on text. `is_navigation()` returns true for them; no text-edit path reaches them.
- Future `Ctrl+W` sequences (e.g., `Ctrl+W v` for a new vertical split) extend naturally via new `CtrlWPrefix` branches and new `Motion` variants — no new external state needed.
- The console layout's broadcast contract is broken: child views that previously received all events now receive only the events appropriate for their focus state. Any future child added to the console horizontal split must be compatible with focus-routed `NormalCommand` delivery.
