# ADR-0006: Reanchor terminal cursor after ZWJ graphemes and at line boundaries

**Status:** Accepted

## Context

Some terminal emulators advance their internal *tracking* cursor by
`unicode_display_width` after printing a grapheme, but physically place the next
character using a separate *placement* cursor that may differ for unrecognised ZWJ
sequences. For a grapheme such as 🙂‍↔️ (U+1F642 + U+200D + U+2194 + U+FE0F),
`unicode_display_width` returns 2, so both aparte's buffer model and the terminal's
CPR (DSR → cursor position report) agree on width=2. However the terminal renders the
sequence as three visual columns because the ZWJ is unrecognised: 🙂 (2 cols) +
↔ (1 col). The placement cursor ends up at col+3 instead of col+2, shifting every
subsequent character on the same row by one column to the right.

A CPR-based runtime probe was attempted but abandoned: the probe correctly measured
the tracking cursor (col=2) which matches our model, and therefore cannot detect the
placement divergence. The same rendering artefact was reproduced in vim and other
software, confirming it is a terminal-side limitation with no reliable probe strategy.

## Decision

Contain placement-cursor drift in the two render output paths without touching the
buffer model or layout layer:

**`full_render`** emits an explicit `MoveTo(0, row)` before each line instead of
relying on the terminal to wrap naturally after the previous line. This prevents
end-of-line ZWJ drift from shifting the start of the next row.

**`render_line`** (called by `full_render`) and **`render_diff`** detect any emitted
grapheme whose UTF-8 encoding contains U+200D (ZERO WIDTH JOINER). Immediately after
writing such a grapheme they emit `MoveTo(model_col, row)`, where `model_col` is the
column our buffer model expects the next character to occupy. This resets the
placement cursor to the model position, containing drift to the ZWJ cell itself.

**Scope deliberately limited to the render paths.** Pane boundaries are a layout
concept and are not visible to `OffscreenRenderBuffer` at render time. Annotating the
buffer with pane boundary markers to emit extra `MoveTo` calls was rejected as
inappropriate coupling between rendering and layout.

## Consequences

- Any grapheme containing U+200D receives one extra `MoveTo` escape in the output.
  For typical renders (few changed cells per frame) this cost is negligible.
- The ZWJ emoji's rightmost visual column may be overwritten by the following
  character when the terminal's rendering exceeds the model width. This is acceptable:
  the emoji was already rendering incorrectly on terminals that do not recognise the
  ZWJ sequence; the fix trades one corrupted column of the emoji for correct placement
  of all subsequent content on the line.
- Terminals that correctly render ZWJ sequences (tracking and placement cursors agree)
  receive an extra `MoveTo` that is a no-op: the cursor is already at that position.
