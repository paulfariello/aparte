/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::cell::Cell;

use unicode_segmentation::UnicodeSegmentation;

use crate::cursor::Cursor;
use crate::next_word;

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
            width: Cell::new(0),
        }
    }

    #[must_use]
    pub fn with_text(text: &str) -> Self {
        let mut editor = Self {
            buf: text.to_string(),
            cursor: Cursor::new(0),
            view: Cursor::new(0),
            width: Cell::new(0),
        };
        editor.end();
        editor
    }

    pub fn set_width(&self, width: usize) {
        self.width.set(width);
    }

    #[must_use]
    pub fn width(&self) -> usize {
        self.width.get()
    }

    pub fn key(&mut self, c: char) {
        let byte_index = self.cursor.index(&self.buf);
        self.buf.insert(byte_index, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
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
        self.buf.replace_range(0..self.cursor.index(&self.buf), "");
        self.cursor.set(0);
        self.view.set(0);
    }

    pub fn delete_from_cursor_to_end(&mut self) {
        self.buf.replace_range(self.cursor.index(&self.buf).., "");
    }

    pub fn delete(&mut self) {
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
}
