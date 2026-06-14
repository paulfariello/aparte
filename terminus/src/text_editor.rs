/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::cell::Cell;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

use crate::cursor::Cursor;
use crate::motion::{Action, Motion, Operator};
use crate::next_word;
use crate::registers::{MotionType, RegisterValue};

/// Reusable text-editing state: a string buffer with a cursor and a scrolling
/// view window. All edit operations (insert/delete/cursor movement/word ops)
/// mutate this state in place.
///
/// `Input` composes a `TextEditor` for the input bar; `MessageView` composes
/// one to support in-place message correction (XEP-0308).
#[derive(Debug, Clone)]
pub struct TextEditor {
    pub buf: String,
    pub cursor: Cursor,
    pub view: Cursor,
    /// Whether the editor is in Normal mode (cursor capped at `len-1`,
    /// block cursor style) vs Insert mode (cursor may sit at `len`).
    pub normal_mode: bool,
    /// Set on the first keystroke that modifies the buffer. Distinguishes a
    /// real Edit Session from a navigation cursor that hasn't been touched.
    pub dirty: bool,
    width: Cell<usize>,
}

impl Default for TextEditor {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEditor {
    #[must_use]
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            cursor: Cursor::new(0),
            view: Cursor::new(0),
            normal_mode: false,
            dirty: false,
            width: Cell::new(0),
        }
    }

    #[must_use]
    pub fn with_text(text: &str) -> Self {
        let mut editor = Self {
            buf: text.to_string(),
            cursor: Cursor::new(0),
            view: Cursor::new(0),
            normal_mode: false,
            dirty: false,
            width: Cell::new(0),
        };
        editor.end();
        editor
    }

    /// Switch between Insert and Normal mode, capping the cursor when entering
    /// Normal mode (cursor must not sit past the last character).
    pub fn set_normal_mode(&mut self, normal: bool) {
        self.normal_mode = normal;
        if normal {
            let len = self.buf.graphemes(true).count();
            if len > 0 && self.cursor.get() >= len {
                self.cursor.set(len - 1);
            }
        }
    }

    pub fn set_width(&self, width: usize) {
        self.width.set(width);
    }

    #[must_use]
    pub fn width(&self) -> usize {
        self.width.get()
    }

    pub fn key(&mut self, c: char) {
        self.dirty = true;
        let byte_index = self.cursor.index(&self.buf);
        self.buf.insert(byte_index, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        self.dirty = true;
        if self.cursor > Cursor::new(0) {
            self.cursor -= 1;
            let mut byte_index = self.cursor.index(&self.buf);
            if byte_index == self.buf.len() {
                byte_index -= 1;
            }
            self.buf.remove(byte_index);
            // TODO work on grapheme
            while !self.buf.is_char_boundary(byte_index) {
                self.buf.remove(byte_index);
            }
        }
    }

    pub fn backward_delete_word(&mut self) {
        self.dirty = true;
        let iter = self.buf[..self.cursor.index(&self.buf)].chars().rev();
        let mut word_start = self.cursor.clone();
        word_start -= next_word(iter);
        self.buf.replace_range(
            word_start.index(&self.buf)..self.cursor.index(&self.buf),
            "",
        );
        self.cursor = word_start;
    }

    pub fn delete_from_cursor_to_start(&mut self) {
        self.dirty = true;
        self.buf.replace_range(0..self.cursor.index(&self.buf), "");
        self.cursor.set(0);
        self.view.set(0);
    }

    pub fn delete_from_cursor_to_end(&mut self) {
        self.dirty = true;
        self.buf.replace_range(self.cursor.index(&self.buf).., "");
    }

    pub fn delete(&mut self) {
        self.dirty = true;
        if self.cursor < self.buf.graphemes(true).count() {
            let byte_index = self.cursor.index(&self.buf);

            self.buf.remove(byte_index);
            while !self.buf.is_char_boundary(byte_index) {
                self.buf.remove(byte_index);
            }
        }
    }

    pub fn home(&mut self) {
        self.cursor.set(0);
        self.view.set(0);
    }

    pub fn end(&mut self) {
        self.cursor.set(self.buf.graphemes(true).count());
        let width = self.width.get();
        if width > 0 && self.cursor > width - 1 {
            self.view = &self.cursor - (width - 1);
        } else {
            self.view.set(0);
        }
    }

    pub fn clear(&mut self) {
        self.dirty = true;
        self.buf.clear();
        self.cursor.set(0);
        self.view.set(0);
    }

    pub fn left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn right(&mut self) {
        if self.cursor < self.buf.graphemes(true).count() {
            self.cursor += 1;
        }
    }

    pub fn word_left(&mut self) {
        let iter = self.buf[..self.cursor.index(&self.buf)].chars().rev();
        self.cursor -= next_word(iter);
    }

    pub fn word_right(&mut self) {
        let iter = self.buf[self.cursor.index(&self.buf)..].chars();
        self.cursor += next_word(iter);
    }

    /// Dispatch a key event to the appropriate editing operation.
    /// Covers all standard readline/emacs-style bindings shared across every
    /// editable widget (input bar, in-place message correction, …).
    pub fn handle_key_event(&mut self, key: &KeyEvent) {
        match (key.code, key.modifiers) {
            (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => self.key(c),
            (KeyCode::Backspace, _) | (KeyCode::Char('h'), KeyModifiers::CONTROL) => {
                self.backspace();
            }
            (KeyCode::Delete, _) => self.delete(),
            (KeyCode::Home, _) | (KeyCode::Char('a'), KeyModifiers::CONTROL) => self.home(),
            (KeyCode::End, _) | (KeyCode::Char('e'), KeyModifiers::CONTROL) => self.end(),
            (KeyCode::Left, KeyModifiers::NONE) | (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
                self.left();
            }
            (KeyCode::Right, KeyModifiers::NONE) | (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                self.right();
            }
            (KeyCode::Left, KeyModifiers::ALT | KeyModifiers::CONTROL)
            | (KeyCode::Char('b'), KeyModifiers::ALT) => self.word_left(),
            (KeyCode::Right, KeyModifiers::ALT | KeyModifiers::CONTROL)
            | (KeyCode::Char('f'), KeyModifiers::ALT) => self.word_right(),
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => self.backward_delete_word(),
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => self.delete_from_cursor_to_start(),
            (KeyCode::Char('k'), KeyModifiers::CONTROL) => self.delete_from_cursor_to_end(),
            _ => {}
        }
    }

    // ── Vim Normal-mode motion support ───────────────────────────────────────

    /// Grapheme position the cursor would move to after applying `motion`
    /// `count` times.  Capped at `len-1` (Normal-mode semantics: cursor cannot
    /// sit past the last character).
    pub fn motion_target(&self, motion: &Motion, count: usize) -> usize {
        let len = self.buf.graphemes(true).count();
        let mut pos = self.cursor.get();
        for _ in 0..count {
            pos = self.motion_once_raw(motion, pos);
        }
        if len == 0 {
            0
        } else {
            pos.min(len - 1)
        }
    }

    /// Apply a complete [`Action`]: move cursor, delete, change, or yank.
    /// Returns the yanked [`RegisterValue`] when the operator produces one
    /// (`Delete`, `Change`, `Yank`); returns `None` for a plain `Move`.
    pub fn apply_action(&mut self, action: &Action) -> Option<RegisterValue> {
        match action.operator {
            Operator::Move => {
                let target = self.motion_target(&action.motion, action.count);
                self.cursor.set(target);
                None
            }
            Operator::Yank => Some(self.yank_motion(&action.motion, action.count)),
            Operator::Delete => Some(self.delete_motion(&action.motion, action.count)),
            Operator::Change => {
                // Pre-compute lo before deletion: delete_motion caps the cursor at
                // new_len-1 (Normal-mode rule), but Change enters Insert mode where
                // the cursor must sit at lo even when lo == new_len (end of buffer).
                let (lo, _) = self.grapheme_range(&action.motion, action.count);
                let rv = self.delete_motion(&action.motion, action.count);
                self.cursor.set(lo);
                Some(rv)
            }
        }
    }

    /// Delete the text covered by `motion × count`. Returns the deleted text.
    /// Cursor is left at the start of the deleted range (or end of buffer if
    /// the deletion reached the end).
    pub fn delete_motion(&mut self, motion: &Motion, count: usize) -> RegisterValue {
        self.dirty = true;
        let (lo, hi) = self.grapheme_range(motion, count);
        if lo >= hi {
            return RegisterValue::new("", MotionType::Char);
        }
        let text = extract_grapheme_range(&self.buf, lo, hi);
        let byte_lo = Cursor::new(lo).index(&self.buf);
        let byte_hi = Cursor::new(hi).index(&self.buf);
        self.buf.replace_range(byte_lo..byte_hi, "");
        let new_len = self.buf.graphemes(true).count();
        let new_cursor = if new_len == 0 { 0 } else { lo.min(new_len - 1) };
        self.cursor.set(new_cursor);
        RegisterValue::new(text, MotionType::Char)
    }

    /// Return the text covered by `motion × count` without modifying the
    /// buffer or the cursor.
    pub fn yank_motion(&self, motion: &Motion, count: usize) -> RegisterValue {
        let (lo, hi) = self.grapheme_range(motion, count);
        RegisterValue::new(extract_grapheme_range(&self.buf, lo, hi), MotionType::Char)
    }

    // ── private helpers ───────────────────────────────────────────────────────

    /// (lo, hi) grapheme range covered by `motion × count`, already normalised
    /// so that `lo ≤ hi`.  Used for both delete and yank.
    fn grapheme_range(&self, motion: &Motion, count: usize) -> (usize, usize) {
        let len = self.buf.graphemes(true).count();
        let pos = self.cursor.get();
        match motion {
            Motion::WholeLine => (0, len),
            // h and l use direct arithmetic so deletion always covers exactly
            // `count` chars regardless of Normal-mode cursor caps.
            Motion::Left => (pos.saturating_sub(count), pos),
            Motion::Right => (pos, (pos + count).min(len)),
            Motion::AWord => {
                if self.buf.is_empty() {
                    return (0, 0);
                }
                let spans = word_spans(&self.buf);
                let idx = spans
                    .partition_point(|&(_, e, _)| e <= pos)
                    .min(spans.len() - 1);
                let (lo, hi) = (spans[idx].0, spans[idx].1);
                match spans[idx].2 {
                    CharClass::Space => {
                        // whitespace span: consume it + the following word/punct span
                        if let Some(&(_, nhi, _)) = spans.get(idx + 1) {
                            (lo, nhi)
                        } else {
                            (lo, hi)
                        }
                    }
                    _ => {
                        // word/punct span: prefer trailing space, else consume leading space
                        if let Some(&(_, nhi, CharClass::Space)) = spans.get(idx + 1) {
                            (lo, nhi)
                        } else if idx > 0 {
                            if let Some(&(nlo, _, CharClass::Space)) = spans.get(idx - 1) {
                                (nlo, hi)
                            } else {
                                (lo, hi)
                            }
                        } else {
                            (lo, hi)
                        }
                    }
                }
            }
            Motion::IWord => {
                if self.buf.is_empty() {
                    return (0, 0);
                }
                let spans = word_spans(&self.buf);
                let idx = spans
                    .partition_point(|&(_, e, _)| e <= pos)
                    .min(spans.len() - 1);
                let (lo, hi) = (spans[idx].0, spans[idx].1);
                (lo, hi)
            }
            _ => {
                let mut target = pos;
                for _ in 0..count {
                    target = self.motion_once_raw(motion, target);
                }
                target = target.min(len);
                let (lo, hi) = if target >= pos {
                    (pos, target)
                } else {
                    (target, pos)
                };
                if motion.is_inclusive() {
                    (lo, (hi + 1).min(len))
                } else {
                    (lo, hi)
                }
            }
        }
    }

    /// Single-step raw motion: returns the target grapheme position WITHOUT
    /// applying Normal-mode cursor caps.  May return values up to `len`.
    fn motion_once_raw(&self, motion: &Motion, pos: usize) -> usize {
        let len = self.buf.graphemes(true).count();
        match motion {
            Motion::Left => pos.saturating_sub(1),
            Motion::Right => (pos + 1).min(len),
            Motion::LineStart => 0,
            Motion::LineEnd => len.saturating_sub(1),
            Motion::FirstNonBlank => self
                .buf
                .graphemes(true)
                .position(|g| !g.chars().next().map(is_space).unwrap_or(true))
                .unwrap_or(0),
            Motion::WordForward => vim_word_forward(&self.buf, pos),
            Motion::WordBackward => vim_word_backward(&self.buf, pos),
            Motion::WordEnd => vim_word_end(&self.buf, pos),
            Motion::BigWordForward => vim_bigword_forward(&self.buf, pos),
            Motion::BigWordBackward => vim_bigword_backward(&self.buf, pos),
            Motion::BigWordEnd => vim_bigword_end(&self.buf, pos),
            // Navigation / special motions are handled by the UI layer.
            _ => pos,
        }
    }
}

// ── word character classification ─────────────────────────────────────────────

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_space(c: char) -> bool {
    c == ' ' || c == '\t'
}

fn first_char(g: &str) -> char {
    g.chars().next().unwrap_or(' ')
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CharClass {
    Word,
    Space,
    Other,
}

fn char_class(c: char) -> CharClass {
    if is_word(c) {
        CharClass::Word
    } else if is_space(c) {
        CharClass::Space
    } else {
        CharClass::Other
    }
}

/// Split `buf` into contiguous runs of the same [`CharClass`] (vim word definition).
/// Returns `(start, end, class)` in grapheme-index space (end exclusive).
///
/// Uses `split_word_bounds` from `unicode_segmentation` for the initial
/// segmentation, then merges adjacent segments sharing the same vim char class.
/// Merging is necessary because Unicode splits e.g. "hello123" at the
/// letter/digit boundary while vim treats alphanumerics+underscore as one word.
fn word_spans(buf: &str) -> Vec<(usize, usize, CharClass)> {
    let mut spans: Vec<(usize, usize, CharClass)> = Vec::new();
    let mut pos = 0usize;
    for segment in buf.split_word_bounds() {
        let n = segment.graphemes(true).count();
        let cls = segment
            .graphemes(true)
            .next()
            .and_then(|g| g.chars().next())
            .map(char_class)
            .unwrap_or(CharClass::Other);
        if let Some(last) = spans.last_mut().filter(|s| s.2 == cls) {
            last.1 += n;
        } else {
            spans.push((pos, pos + n, cls));
        }
        pos += n;
    }
    spans
}

// ── vim word motions (operate on grapheme positions) ─────────────────────────

/// `w` — move to the start of the next word.
fn vim_word_forward(buf: &str, pos: usize) -> usize {
    let gs: Vec<&str> = buf.graphemes(true).collect();
    let len = gs.len();
    if pos >= len {
        return pos;
    }
    let mut i = pos;
    let c = first_char(gs[i]);
    if is_word(c) {
        while i < len && is_word(first_char(gs[i])) {
            i += 1;
        }
    } else if !is_space(c) {
        while i < len && !is_word(first_char(gs[i])) && !is_space(first_char(gs[i])) {
            i += 1;
        }
    }
    // skip spaces
    while i < len && is_space(first_char(gs[i])) {
        i += 1;
    }
    i
}

/// `b` — move to the start of the current/previous word.
fn vim_word_backward(buf: &str, pos: usize) -> usize {
    let gs: Vec<&str> = buf.graphemes(true).collect();
    if pos == 0 {
        return 0;
    }
    let mut i = pos - 1;
    // skip spaces backwards
    while i > 0 && is_space(first_char(gs[i])) {
        i -= 1;
    }
    if is_space(first_char(gs[i])) {
        return 0;
    }
    let word_type = is_word(first_char(gs[i]));
    // skip same-type chars backwards
    while i > 0 {
        let prev = first_char(gs[i - 1]);
        if is_word(prev) != word_type || is_space(prev) {
            break;
        }
        i -= 1;
    }
    i
}

/// `e` — move to the end of the current/next word (inclusive position).
fn vim_word_end(buf: &str, pos: usize) -> usize {
    let gs: Vec<&str> = buf.graphemes(true).collect();
    let len = gs.len();
    if len == 0 || pos + 1 >= len {
        return len.saturating_sub(1);
    }
    let mut i = pos + 1;
    // skip spaces
    while i < len && is_space(first_char(gs[i])) {
        i += 1;
    }
    if i >= len {
        return len - 1;
    }
    let word_type = is_word(first_char(gs[i]));
    // advance to end of token
    while i + 1 < len {
        let next = first_char(gs[i + 1]);
        let matches = if word_type {
            is_word(next)
        } else {
            !is_space(next) && !is_word(next)
        };
        if matches {
            i += 1;
        } else {
            break;
        }
    }
    i
}

/// `W` — move to the start of the next WORD (non-blank sequence).
fn vim_bigword_forward(buf: &str, pos: usize) -> usize {
    let gs: Vec<&str> = buf.graphemes(true).collect();
    let len = gs.len();
    let mut i = pos;
    // skip non-blank
    while i < len && !is_space(first_char(gs[i])) {
        i += 1;
    }
    // skip spaces
    while i < len && is_space(first_char(gs[i])) {
        i += 1;
    }
    i
}

/// `B` — move to the start of the current/previous WORD.
fn vim_bigword_backward(buf: &str, pos: usize) -> usize {
    let gs: Vec<&str> = buf.graphemes(true).collect();
    if pos == 0 {
        return 0;
    }
    let mut i = pos - 1;
    // skip spaces backwards
    while i > 0 && is_space(first_char(gs[i])) {
        i -= 1;
    }
    if is_space(first_char(gs[i])) {
        return 0;
    }
    // skip non-blank backwards
    while i > 0 && !is_space(first_char(gs[i - 1])) {
        i -= 1;
    }
    i
}

/// `E` — move to the end of the current/next WORD (inclusive position).
fn vim_bigword_end(buf: &str, pos: usize) -> usize {
    let gs: Vec<&str> = buf.graphemes(true).collect();
    let len = gs.len();
    if len == 0 || pos + 1 >= len {
        return len.saturating_sub(1);
    }
    let mut i = pos + 1;
    // skip spaces
    while i < len && is_space(first_char(gs[i])) {
        i += 1;
    }
    if i >= len {
        return len - 1;
    }
    // advance to end of WORD
    while i + 1 < len && !is_space(first_char(gs[i + 1])) {
        i += 1;
    }
    i
}

fn extract_grapheme_range(buf: &str, lo: usize, hi: usize) -> String {
    if lo >= hi {
        return String::new();
    }
    buf.graphemes(true).skip(lo).take(hi - lo).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_editor_is_empty_with_cursor_at_zero() {
        let editor = TextEditor::new();
        assert_eq!(editor.buf, "");
        assert_eq!(editor.cursor.get(), 0);
        assert_eq!(editor.view.get(), 0);
    }

    #[test]
    fn with_text_initializes_buffer_and_puts_cursor_at_end() {
        let editor = TextEditor::with_text("hello");
        assert_eq!(editor.buf, "hello");
        assert_eq!(editor.cursor.get(), 5);
    }

    #[test]
    fn key_inserts_at_cursor_and_advances() {
        let mut editor = TextEditor::new();
        editor.key('a');
        editor.key('b');
        editor.key('c');
        assert_eq!(editor.buf, "abc");
        assert_eq!(editor.cursor.get(), 3);
    }

    #[test]
    fn key_inserts_in_middle() {
        let mut editor = TextEditor::with_text("ac");
        editor.left();
        editor.key('b');
        assert_eq!(editor.buf, "abc");
        assert_eq!(editor.cursor.get(), 2);
    }

    #[test]
    fn backspace_removes_char_before_cursor() {
        let mut editor = TextEditor::new();
        editor.key('a');
        editor.key('b');
        editor.key('c');
        editor.backspace();
        assert_eq!(editor.buf, "ab");
        assert_eq!(editor.cursor.get(), 2);
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut editor = TextEditor::with_text("abc");
        editor.home();
        editor.backspace();
        assert_eq!(editor.buf, "abc");
        assert_eq!(editor.cursor.get(), 0);
    }

    #[test]
    fn delete_removes_char_at_cursor() {
        let mut editor = TextEditor::with_text("abc");
        editor.home();
        editor.delete();
        assert_eq!(editor.buf, "bc");
        assert_eq!(editor.cursor.get(), 0);
    }

    #[test]
    fn delete_at_end_is_noop() {
        let mut editor = TextEditor::with_text("abc");
        editor.delete();
        assert_eq!(editor.buf, "abc");
    }

    #[test]
    fn left_moves_cursor_back_within_bounds() {
        let mut editor = TextEditor::with_text("ab");
        editor.left();
        assert_eq!(editor.cursor.get(), 1);
        editor.left();
        assert_eq!(editor.cursor.get(), 0);
        editor.left();
        assert_eq!(editor.cursor.get(), 0);
    }

    #[test]
    fn right_moves_cursor_forward_within_bounds() {
        let mut editor = TextEditor::with_text("ab");
        editor.home();
        editor.right();
        assert_eq!(editor.cursor.get(), 1);
        editor.right();
        assert_eq!(editor.cursor.get(), 2);
        editor.right();
        assert_eq!(editor.cursor.get(), 2);
    }

    #[test]
    fn home_and_end_jump_cursor() {
        let mut editor = TextEditor::with_text("hello");
        editor.home();
        assert_eq!(editor.cursor.get(), 0);
        editor.end();
        assert_eq!(editor.cursor.get(), 5);
    }

    #[test]
    fn word_right_jumps_past_word() {
        let mut editor = TextEditor::with_text("foo bar");
        editor.home();
        editor.word_right();
        assert_eq!(editor.cursor.get(), 3);
    }

    #[test]
    fn word_left_jumps_back_a_word() {
        let mut editor = TextEditor::with_text("foo bar");
        editor.word_left();
        assert_eq!(editor.cursor.get(), 4);
    }

    #[test]
    fn backward_delete_word_removes_previous_word() {
        let mut editor = TextEditor::with_text("foo bar");
        editor.backward_delete_word();
        assert_eq!(editor.buf, "foo ");
    }

    #[test]
    fn delete_from_cursor_to_start() {
        let mut editor = TextEditor::with_text("hello world");
        // place cursor after "hello "
        editor.home();
        for _ in 0..6 {
            editor.right();
        }
        editor.delete_from_cursor_to_start();
        assert_eq!(editor.buf, "world");
        assert_eq!(editor.cursor.get(), 0);
    }

    #[test]
    fn delete_from_cursor_to_end() {
        let mut editor = TextEditor::with_text("hello world");
        editor.home();
        for _ in 0..5 {
            editor.right();
        }
        editor.delete_from_cursor_to_end();
        assert_eq!(editor.buf, "hello");
    }

    #[test]
    fn clear_resets_everything() {
        let mut editor = TextEditor::with_text("hello");
        editor.clear();
        assert_eq!(editor.buf, "");
        assert_eq!(editor.cursor.get(), 0);
        assert_eq!(editor.view.get(), 0);
    }

    #[test]
    fn key_with_multibyte_codepoint() {
        let mut editor = TextEditor::new();
        editor.key('🍺');
        editor.key('a');
        assert_eq!(editor.buf, "🍺a");
        assert_eq!(editor.cursor.get(), 2);
    }

    #[test]
    fn backspace_on_multibyte_codepoint() {
        let mut editor = TextEditor::with_text("🍺a");
        editor.backspace();
        assert_eq!(editor.buf, "🍺");
        assert_eq!(editor.cursor.get(), 1);
        editor.backspace();
        assert_eq!(editor.buf, "");
        assert_eq!(editor.cursor.get(), 0);
    }

    // ── motion_target ─────────────────────────────────────────────────────────

    fn at(text: &str, pos: usize) -> TextEditor {
        let ed = TextEditor::with_text(text);
        ed.cursor.set(pos);
        ed
    }

    #[test]
    fn motion_left_moves_back() {
        assert_eq!(at("foo bar", 3).motion_target(&Motion::Left, 1), 2);
    }

    #[test]
    fn motion_left_floors_at_zero() {
        assert_eq!(at("foo bar", 0).motion_target(&Motion::Left, 1), 0);
    }

    #[test]
    fn motion_right_moves_forward() {
        assert_eq!(at("foo bar", 3).motion_target(&Motion::Right, 1), 4);
    }

    #[test]
    fn motion_right_caps_at_last_char() {
        // "foo bar" has len=7, last grapheme index = 6
        assert_eq!(at("foo bar", 6).motion_target(&Motion::Right, 1), 6);
    }

    #[test]
    fn motion_line_start() {
        assert_eq!(at("foo bar", 4).motion_target(&Motion::LineStart, 1), 0);
    }

    #[test]
    fn motion_line_end() {
        assert_eq!(at("foo bar", 0).motion_target(&Motion::LineEnd, 1), 6);
    }

    #[test]
    fn motion_first_non_blank_skips_leading_spaces() {
        assert_eq!(at("  foo", 4).motion_target(&Motion::FirstNonBlank, 1), 2);
    }

    #[test]
    fn motion_first_non_blank_on_no_leading_spaces() {
        assert_eq!(at("foo", 2).motion_target(&Motion::FirstNonBlank, 1), 0);
    }

    #[test]
    fn motion_word_forward_to_next_word() {
        // "foo bar": w from 'f'(0) → 'b'(4)
        assert_eq!(at("foo bar", 0).motion_target(&Motion::WordForward, 1), 4);
    }

    #[test]
    fn motion_word_forward_skips_extra_spaces() {
        // "foo  bar": w from 0 → 5
        assert_eq!(at("foo  bar", 0).motion_target(&Motion::WordForward, 1), 5);
    }

    #[test]
    fn motion_word_forward_at_last_word_caps() {
        // "foo bar": w from 'b'(4) has no next word → caps at len-1=6
        assert_eq!(at("foo bar", 4).motion_target(&Motion::WordForward, 1), 6);
    }

    #[test]
    fn motion_word_forward_count_2() {
        // "foo bar baz": 2w from 0 → 8 ('b' in "baz")
        assert_eq!(
            at("foo bar baz", 0).motion_target(&Motion::WordForward, 2),
            8
        );
    }

    #[test]
    fn motion_word_backward_to_word_start() {
        // "foo bar": b from 'r'(6) → 'b'(4)
        assert_eq!(at("foo bar", 6).motion_target(&Motion::WordBackward, 1), 4);
    }

    #[test]
    fn motion_word_backward_from_start_of_word() {
        // "foo bar": b from 'b'(4) → 'f'(0)
        assert_eq!(at("foo bar", 4).motion_target(&Motion::WordBackward, 1), 0);
    }

    #[test]
    fn motion_word_end_to_end_of_word() {
        // "foo bar": e from 'f'(0) → 'o'(2)
        assert_eq!(at("foo bar", 0).motion_target(&Motion::WordEnd, 1), 2);
    }

    #[test]
    fn motion_word_end_from_end_jumps_to_next() {
        // "foo bar": e from 'o'(2) → 'r'(6)
        assert_eq!(at("foo bar", 2).motion_target(&Motion::WordEnd, 1), 6);
    }

    #[test]
    fn motion_bigword_forward_skips_punctuation() {
        // "foo,bar baz": W from 0 → 8 ('b' in "baz")
        assert_eq!(
            at("foo,bar baz", 0).motion_target(&Motion::BigWordForward, 1),
            8
        );
    }

    #[test]
    fn motion_bigword_backward() {
        // "foo,bar baz": B from 8 → 0
        assert_eq!(
            at("foo,bar baz", 8).motion_target(&Motion::BigWordBackward, 1),
            0
        );
    }

    #[test]
    fn motion_bigword_end_skips_punctuation() {
        // "foo,bar baz": E from 0 → 6 (last char of "foo,bar")
        assert_eq!(
            at("foo,bar baz", 0).motion_target(&Motion::BigWordEnd, 1),
            6
        );
    }

    // ── delete_motion ─────────────────────────────────────────────────────────

    #[test]
    fn delete_word_forward_exclusive() {
        let mut ed = at("foo bar", 0);
        let rv = ed.delete_motion(&Motion::WordForward, 1);
        assert_eq!(ed.buf, "bar");
        assert_eq!(ed.cursor.get(), 0);
        assert_eq!(rv.text, "foo ");
    }

    #[test]
    fn delete_word_end_inclusive() {
        let mut ed = at("foo bar", 0);
        let rv = ed.delete_motion(&Motion::WordEnd, 1);
        assert_eq!(ed.buf, " bar");
        assert_eq!(ed.cursor.get(), 0);
        assert_eq!(rv.text, "foo");
    }

    #[test]
    fn delete_to_line_end_inclusive() {
        let mut ed = at("foo bar", 4);
        let rv = ed.delete_motion(&Motion::LineEnd, 1);
        assert_eq!(ed.buf, "foo ");
        assert_eq!(rv.text, "bar");
        // cursor at end of remaining buf (len-1 = 3)
        assert_eq!(ed.cursor.get(), 3);
    }

    #[test]
    fn delete_left() {
        let mut ed = at("foo bar", 4);
        let rv = ed.delete_motion(&Motion::Left, 1);
        assert_eq!(ed.buf, "foobar");
        assert_eq!(rv.text, " ");
        assert_eq!(ed.cursor.get(), 3);
    }

    #[test]
    fn delete_right() {
        let mut ed = at("foo bar", 0);
        let rv = ed.delete_motion(&Motion::Right, 1);
        assert_eq!(ed.buf, "oo bar");
        assert_eq!(rv.text, "f");
        assert_eq!(ed.cursor.get(), 0);
    }

    #[test]
    fn delete_right_at_last_char() {
        let mut ed = at("foo", 2);
        let rv = ed.delete_motion(&Motion::Right, 1);
        assert_eq!(ed.buf, "fo");
        assert_eq!(rv.text, "o");
        assert_eq!(ed.cursor.get(), 1);
    }

    #[test]
    fn delete_whole_line() {
        let mut ed = at("foo bar", 3);
        let rv = ed.delete_motion(&Motion::WholeLine, 1);
        assert_eq!(ed.buf, "");
        assert_eq!(rv.text, "foo bar");
        assert_eq!(ed.cursor.get(), 0);
    }

    #[test]
    fn delete_word_backward() {
        let mut ed = at("foo bar", 7);
        ed.cursor.set(7); // past end (Insert-mode position)
        let rv = ed.delete_motion(&Motion::WordBackward, 1);
        assert_eq!(ed.buf, "foo ");
        assert_eq!(rv.text, "bar");
    }

    // ── yank_motion ───────────────────────────────────────────────────────────

    #[test]
    fn yank_word_does_not_modify_buffer() {
        let ed = at("foo bar", 0);
        let rv = ed.yank_motion(&Motion::WordForward, 1);
        assert_eq!(ed.buf, "foo bar");
        assert_eq!(ed.cursor.get(), 0);
        assert_eq!(rv.text, "foo ");
    }

    #[test]
    fn yank_whole_line() {
        let ed = at("foo bar", 3);
        let rv = ed.yank_motion(&Motion::WholeLine, 1);
        assert_eq!(rv.text, "foo bar");
    }

    // ── apply_action ──────────────────────────────────────────────────────────

    #[test]
    fn apply_action_move() {
        use crate::motion::{Action, Operator};
        let mut ed = at("foo bar", 0);
        let rv = ed.apply_action(&Action {
            count: 1,
            register: None,
            operator: Operator::Move,
            motion: Motion::WordForward,
        });
        assert!(rv.is_none());
        assert_eq!(ed.cursor.get(), 4);
    }

    #[test]
    fn apply_action_delete_returns_register_value() {
        use crate::motion::{Action, Operator};
        let mut ed = at("foo bar", 0);
        let rv = ed.apply_action(&Action {
            count: 1,
            register: None,
            operator: Operator::Delete,
            motion: Motion::WordForward,
        });
        assert!(rv.is_some());
        assert_eq!(rv.unwrap().text, "foo ");
        assert_eq!(ed.buf, "bar");
    }

    #[test]
    fn apply_action_yank_does_not_modify() {
        use crate::motion::{Action, Operator};
        let mut ed = at("foo bar", 0);
        let rv = ed.apply_action(&Action {
            count: 1,
            register: None,
            operator: Operator::Yank,
            motion: Motion::WholeLine,
        });
        assert_eq!(rv.unwrap().text, "foo bar");
        assert_eq!(ed.buf, "foo bar");
    }

    // ── apply_action / Change ─────────────────────────────────────────────────

    #[test]
    fn apply_action_change_cursor_at_end_of_buffer() {
        // caw on the last word must leave cursor at new_len (Insert-mode position),
        // not at new_len-1 (the last char of the preceding word).
        use crate::motion::{Action, Operator};
        let mut ed = at("hello world", 6);
        ed.apply_action(&Action {
            count: 1,
            register: None,
            operator: Operator::Change,
            motion: Motion::AWord,
        });
        assert_eq!(ed.buf, "hello");
        assert_eq!(ed.cursor.get(), 5); // Insert mode: right after 'o'
    }

    #[test]
    fn apply_action_change_returns_register_value() {
        use crate::motion::{Action, Operator};
        let mut ed = at("foo bar", 0);
        let rv = ed.apply_action(&Action {
            count: 1,
            register: None,
            operator: Operator::Change,
            motion: Motion::WordForward,
        });
        assert!(rv.is_some());
        assert_eq!(rv.unwrap().text, "foo ");
        assert_eq!(ed.buf, "bar");
    }

    #[test]
    fn apply_action_change_whole_line() {
        use crate::motion::{Action, Operator};
        let mut ed = at("foo bar", 3);
        let rv = ed.apply_action(&Action {
            count: 1,
            register: None,
            operator: Operator::Change,
            motion: Motion::WholeLine,
        });
        assert_eq!(rv.unwrap().text, "foo bar");
        assert_eq!(ed.buf, "");
        assert_eq!(ed.cursor.get(), 0);
    }

    #[test]
    fn apply_action_change_stores_to_unnamed_register() {
        use crate::motion::{Action, Operator};
        use crate::registers::Registers;
        let mut ed = at("foo bar", 0);
        let mut registers = Registers::new();
        let action = Action {
            count: 1,
            register: None,
            operator: Operator::Change,
            motion: Motion::WordForward,
        };
        let rv = ed.apply_action(&action).unwrap();
        registers.yank(action.register, rv);
        assert_eq!(registers.get(Registers::UNNAMED).unwrap().text, "foo ");
    }

    #[test]
    fn apply_action_change_stores_to_named_register() {
        use crate::motion::{Action, Operator};
        use crate::registers::Registers;
        let mut ed = at("hello world", 0);
        let mut registers = Registers::new();
        let action = Action {
            count: 1,
            register: Some('a'),
            operator: Operator::Change,
            motion: Motion::AWord,
        };
        let rv = ed.apply_action(&action).unwrap();
        registers.yank(action.register, rv);
        assert_eq!(registers.get('a').unwrap().text, "hello ");
        assert_eq!(registers.get(Registers::UNNAMED).unwrap().text, "hello ");
    }

    // ── word_spans ────────────────────────────────────────────────────────────

    #[test]
    fn word_spans_empty() {
        assert!(word_spans("").is_empty());
    }

    #[test]
    fn word_spans_single_word() {
        assert_eq!(word_spans("hello"), vec![(0, 5, CharClass::Word)]);
    }

    #[test]
    fn word_spans_word_space_word() {
        assert_eq!(
            word_spans("hello world"),
            vec![
                (0, 5, CharClass::Word),
                (5, 6, CharClass::Space),
                (6, 11, CharClass::Word),
            ]
        );
    }

    #[test]
    fn word_spans_leading_space() {
        assert_eq!(
            word_spans(" hello"),
            vec![(0, 1, CharClass::Space), (1, 6, CharClass::Word)]
        );
    }

    #[test]
    fn word_spans_trailing_space() {
        assert_eq!(
            word_spans("hello "),
            vec![(0, 5, CharClass::Word), (5, 6, CharClass::Space)]
        );
    }

    #[test]
    fn word_spans_merges_alnum() {
        // Unicode splits "hello123" at the letter/digit boundary;
        // word_spans must merge both segments into one Word span.
        assert_eq!(word_spans("hello123"), vec![(0, 8, CharClass::Word)]);
    }

    #[test]
    fn word_spans_punctuation() {
        assert_eq!(
            word_spans("hello-world"),
            vec![
                (0, 5, CharClass::Word),
                (5, 6, CharClass::Other),
                (6, 11, CharClass::Word),
            ]
        );
    }

    #[test]
    fn word_spans_multiple_spaces() {
        assert_eq!(
            word_spans("foo  bar"),
            vec![
                (0, 3, CharClass::Word),
                (3, 5, CharClass::Space),
                (5, 8, CharClass::Word),
            ]
        );
    }

    // ── delete_motion / AWord ─────────────────────────────────────────────────

    #[test]
    fn aword_first_word_with_trailing_space() {
        let mut ed = at("hello world", 0);
        let rv = ed.delete_motion(&Motion::AWord, 1);
        assert_eq!(rv.text, "hello ");
        assert_eq!(ed.buf, "world");
        assert_eq!(ed.cursor.get(), 0);
    }

    #[test]
    fn aword_last_word_with_leading_space() {
        let mut ed = at("hello world", 6);
        let rv = ed.delete_motion(&Motion::AWord, 1);
        assert_eq!(rv.text, " world");
        assert_eq!(ed.buf, "hello");
        assert_eq!(ed.cursor.get(), 4);
    }

    #[test]
    fn aword_single_word_no_spaces() {
        let mut ed = at("hello", 2);
        let rv = ed.delete_motion(&Motion::AWord, 1);
        assert_eq!(rv.text, "hello");
        assert_eq!(ed.buf, "");
        assert_eq!(ed.cursor.get(), 0);
    }

    #[test]
    fn aword_cursor_inside_word() {
        let mut ed = at("hello world", 2);
        let rv = ed.delete_motion(&Motion::AWord, 1);
        assert_eq!(rv.text, "hello ");
        assert_eq!(ed.buf, "world");
        assert_eq!(ed.cursor.get(), 0);
    }

    #[test]
    fn aword_on_space() {
        // cursor on the space between "hello" and "world"
        let mut ed = at("hello world", 5);
        let rv = ed.delete_motion(&Motion::AWord, 1);
        assert_eq!(rv.text, " world");
        assert_eq!(ed.buf, "hello");
    }

    // ── delete_motion / IWord ─────────────────────────────────────────────────

    #[test]
    fn iword_first_word() {
        let mut ed = at("hello world", 0);
        let rv = ed.delete_motion(&Motion::IWord, 1);
        assert_eq!(rv.text, "hello");
        assert_eq!(ed.buf, " world");
        assert_eq!(ed.cursor.get(), 0);
    }

    #[test]
    fn iword_last_word() {
        let mut ed = at("hello world", 6);
        let rv = ed.delete_motion(&Motion::IWord, 1);
        assert_eq!(rv.text, "world");
        assert_eq!(ed.buf, "hello ");
    }

    #[test]
    fn iword_cursor_inside() {
        let mut ed = at("hello world", 2);
        let rv = ed.delete_motion(&Motion::IWord, 1);
        assert_eq!(rv.text, "hello");
        assert_eq!(ed.buf, " world");
        assert_eq!(ed.cursor.get(), 0);
    }

    #[test]
    fn iword_on_space() {
        let mut ed = at("hello world", 5);
        let rv = ed.delete_motion(&Motion::IWord, 1);
        assert_eq!(rv.text, " ");
        assert_eq!(ed.buf, "helloworld");
    }
}
