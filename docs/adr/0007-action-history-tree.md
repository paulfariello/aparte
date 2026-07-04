# Action history as a per-window undo tree

Status: proposed

The application needs a way to track editing actions so that `u`/`U` can undo and
redo changes vim-style, and so that future features (`.`-repeat, macros) have a
record to replay.

## Decisions made

**Scope of "action":** Normal-mode `Action` values (operator + motion) and
Insert-mode change sessions (one record per `i … Esc` block). Commands (`:join`,
`:win`) and pure navigation are excluded. This matches vim's model and keeps the
tree free of side-effectful, non-invertible events.

**Semantics of `u`/`U`:** Undo/redo in the vim sense — `u` restores the prior
buffer state, `U` moves forward on the most-recently-visited branch. Not a
read-only history browser.

**Tree shape:** Going back and then making a new edit creates a branch, preserving
the abandoned future rather than discarding it. This is vim's undo-tree model, not
linear undo/redo.

**Scope:** Per-window input bar. Each conversation window owns an independent undo
tree. Switching windows switches to that window's tree.

## Open question: tree lifetime across sends

Whether the undo tree resets when a message is sent is unresolved. Three positions
are on the table:

1. **Reset on send** — simple; undo never reaches past-a-sent-message buffer
   states. Clean invariant: the tree covers only the current unsent draft.

2. **Persist across sends, send node is opaque** — the tree accumulates across
   multiple messages. Send events are recorded as non-undoable markers; `u` can
   cross them to restore pre-send buffer states. Challenged because: crossing a
   send node IS a visible buffer change ("hello world" reappears), the redo
   direction is ambiguous (re-send?), and it is unclear whether post-boundary
   editing creates a genuinely new branch or a confusing resurrection of sent
   content.

3. **Orthogonal message recall** — the undo tree resets on send; a separate
   ordered structure (analogous to shell history, `Alt+Up`/`Alt+Down`) lets the
   user recall previously sent message bodies for reuse. This separates "undo
   current draft" from "recall old content" without conflating them.

This decision is deferred. The implementation should make the reset-on-send
boundary a single, replaceable policy so option 2 or 3 can be layered on later
without restructuring the tree.
