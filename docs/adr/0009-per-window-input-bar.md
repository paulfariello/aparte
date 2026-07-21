# ADR-0009: Per-window input bar with `> ` prompt

**Status:** Accepted  
**Amends:** ADR-0008 (input bar was a global root-layout child at index 2)

ADR-0008 placed the input bar as a global slot in the root vertical stack, shared across all windows. This ADR moves it inside each Chat/Channel window view and adds a visible `> ` prompt prefix.

## Context

The global input bar had two problems once per-window semantics were needed:

1. **Shared draft.** Switching windows wiped the in-progress message. Per-conversation drafts require the input widget to be owned by the window, not the root layout.
2. **Console ambiguity.** The console window is read-only, but it still displayed the input bar, wasting a screen row and implying that text could be sent.

A `> ` prompt was also requested so the input bar is immediately distinguishable from the messages above it.

## Decision

Move the `Input` widget out of the root layout and into each Chat and Channel window's own `LinearLayout`. Console has no `Input` child.

Root layout shrinks by one slot:

```
win_bar (0) / frame (1) / status_line (2) / command_bar (3)
```

`COMMAND_BAR_INDEX` changes from 4 to 3. `INPUT_INDEX` at the root level is removed entirely.

Each Chat/Channel window's `LinearLayout` is:

```
messages / roster pane (0) / input bar (1)
```

The input bar spans the full window width (under both messages and roster in Channel windows).

**`> ` prompt:** The `Input` widget gains an optional `prompt: Option<(String, Style)>` field set via `with_prompt`. The prompt renders before the text in `input_prompt` theme colour; cursor column = `prompt.len() + text_cursor`. The command bar's `Input` leaves it `None`.

**Per-window drafts:** Each window's `Input` widget owns its own buffer. No draft `HashMap` is needed; the widget state is the draft.

**`current_input` removed:** The mod-level `current_input` field is dropped. Any arm that needs the current input state sends `UIEvent::RequestInputState` to the frame; the window's `Input` responds synchronously with `InputChanged(buf, cursor, password)`.

**`dispatch_action` simplified:** The non-frame path (text motions sent to non-frame root children) is removed. All text actions route to `FRAME_LAYOUT_INDEX`; the window routes them internally to whichever child is focused.

**`o` key:** Emits `UIEvent::FocusInputBar` routed to `FRAME_LAYOUT_INDEX`. Each Chat/Channel window handles it by focusing its `Input` child. Console receives the event and ignores it — Insert mode is never entered because no insertable child gets focused.

**`i`/`a` on console:** These keys reach the frame, the frame forwards to its focused `ScrollWin`, the ScrollWin is not in an edit state and has no message selected, so the event goes unhandled. Mode stays Normal. No explicit window-type guard is needed.

## Considered options

- **Keep the global input bar, add per-window draft storage via `HashMap`** — rejected: the widget is still on console, wasting a row and implying editability. Saving/restoring buffers on every tab switch adds sync complexity and race risk.
- **Prompt as an external label in a horizontal `LinearLayout`** — rejected: the cursor column must be computed relative to the prompt. Computing it externally means every render site must know the prompt length. Putting the prompt inside `Input` keeps the cursor offset self-contained.
- **Explicit console guard at the mod level (check window type before entering Insert)** — rejected: fragile — must be updated every time a new window type is added. The natural fallback (no insertable child → no mode change) is more robust.

## Consequences

- Chrome on Chat/Channel windows is unchanged in total row count: the global input bar row moves inside the window, so the frame loses one content row but the bar stack loses one row. Net zero.
- Console gains one row of message content (input bar row is reclaimed).
- `current_input` removal touches every event arm that reads it — roughly a dozen call sites.
- Tests that assert `INPUT_ROW = ROWS - 3` remain valid: the input bar is still 3 rows from the bottom (inside the frame, which ends at `ROWS - 3`).
- The `input_prompt` theme key must be added to all built-in themes with a sensible default.
