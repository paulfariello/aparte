# ADR-0004: Keyboard model and suspend semantics for in-place message correction

**Status:** Accepted

## Context

Aparte supports XEP-0308 last-message correction: the user can navigate to an outgoing message, enter Insert mode, edit it in place, and send the revised body as a correction stanza.

The design space has three competing pressures:

1. **Vim muscle memory.** Users in a modal TUI hit `Esc` reflexively — to ensure they are in Normal mode, to abort half-typed commands, as a safety reflex. Any design that assigns a commit or cancel meaning to `Esc` will produce accidental corrections or accidental discards.

2. **In-session navigation.** A common use case is: start editing a message, realise you need to copy text from another message or window, navigate away, then return to finish the edit. A model that auto-discards on any navigation destroys this workflow.

3. **Explicitness.** Corrections are network-visible stanzas. An accidental commit sends a spurious XEP-0308 message to the server and all recipients. The commit path must be deliberate.

### Alternatives considered

| Approach | Problem |
|---|---|
| `Esc` commits | Esc is a safety reflex; every Insert→Normal cycle during editing sends a correction. |
| `Esc` (2nd press) cancels | Users mash Esc to confirm they are in Normal mode; double-Esc accidentally discards edits. |
| Block navigation with a popup | Prevents the copy-from-another-message workflow entirely. |
| Auto-commit on navigation away | Sends partial/mid-thought corrections silently. |

## Decision

### Keyboard bindings

| Key | Insert mode | Normal mode |
|---|---|---|
| `Enter` | commit | commit |
| `Ctrl+C` | discard | discard |
| `Esc` | → Normal mode | no-op |

`Esc` is reserved exclusively for vim mode management. It never commits and never discards an Edit Session.

### Dirty flag

`TextEditor` carries an explicit `dirty: bool` flag, set on the first keystroke that modifies the buffer. This distinguishes a real Edit Session from a navigation cursor that happens to have `edit = Some(_)`.

### Suspend semantics

- **Dirty Edit Sessions survive navigation** — switching windows, pressing `j`/`k`, jumping with `G`. The buffer is stored on the `MessageView` and persists until committed or discarded.
- **Clean cursors are discarded on navigation** — unchanged selection state is ephemeral.
- **Multiple suspended edits are allowed** — there is no cap on concurrent dirty Edit Sessions across messages or windows.
- **Window close silently discards** all suspended edits in that window. Closing is an explicit destructive act; no prompt is shown.

### Visual indication

- The message row always renders the live edit buffer, distinguishing it visually from committed text.
- The `WinBar` shows a per-window indicator when at least one dirty Edit Session exists in that window, mirroring the existing unread-count mechanism.

## Consequences

- Users can navigate freely during editing without losing work.
- The commit path (`Enter`) is identical whether the cursor is in Insert or Normal mode on the message, and is consistent with how sending a new message works.
- `Ctrl+C` is a well-known "abort" signal; its meaning here is unambiguous.
- The dirty flag requires a small addition to `TextEditor` but removes the ambiguity between navigation cursors and real edits throughout the codebase.
- The WinBar indicator requires a new event type to propagate dirty-state changes upward.
