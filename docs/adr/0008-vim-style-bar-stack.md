# ADR-0008: Vim-style bar stack — input bar above status line and command bar

**Status:** Accepted

## Context

The input bar was a single widget serving three roles: message composition (Insert mode), `:` command and `/` search entry (Command mode), and masked password prompts. Entering Command mode stashed the in-progress message (`saved_input`) and restored it afterwards; message and command history shared one Up/Down ring; a message could not visually be told apart from a command being typed. Command errors were only visible in the console window.

Every conventional chat client (irssi, weechat, profanity) puts the typing line at the very bottom of the screen. Vim instead puts the buffer above the statusline, with the cmdline as the bottom row.

## Decision

Adopt the vim shape. Global vertical stack, top to bottom:

1. **Tab bar** — all open windows in order, current emphasized, per-window activity counts (previously the bar showed only windows with pending highlights). Names only, no ordinals; switching stays name/fuzzy-based.
2. **Frame** — per-window content. Channel windows gain a **topic bar** (channel name and XEP-0045 subject) above messages/roster; Chat windows have none.
3. **Input bar** — message composition only, with its own history.
4. **Status line** — mode first, then connected account, then indicators such as suspended dirty Edit Sessions.
5. **Command bar** — all transient meta-input: `:` commands, `/` search, password prompts. Pending normal-mode keys render right-aligned. Command errors echo here until the next keypress (still logged to console).

The old `TitleBar` widget disappears: its title/subject content moves to the per-Channel topic bar, its mode/connection content to the status line.

## Considered options

- **Keep one shared input widget, only reorder bars** — rejected: does not fix the mixing the change exists for (shared history, stash/restore, ambiguous typing target).
- **Command line overlays the status line when active** — rejected: saves one row but hides mode/account exactly when a command is being typed, and adds show/hide reflow.
- **Input bar at the very bottom, command bar above it** — rejected: the two editable lines become adjacent and confusable; the status line between them is the separator that makes the split legible.

## Consequences

- Chrome grows from 3 to 5 rows (topic bar and command bar are net new); Chat windows pay only 4.
- The input bar is not the bottom row, contrary to chat-client convention — deliberate, per the vim metaphor (buffer / statusline / cmdline). Do not "fix" this.
- `saved_input` stash/restore and the input widget's password mode are removed; Insert-mode `:` and `/` remain literal text.
- Message history and command/search history are separate.
- Command errors become visible outside the console window for the first time.
- On overflow the tab bar must keep the current window visible and truncate with an ellipsis.
- PTY-level rendering tests that assert row positions of bars all change.