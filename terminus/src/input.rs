/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use crate::rendering::ScreenFrame;
use crate::text_editor::TextEditor;
use crate::CursorPos;
use crate::CursorStyle;
use std::cell::RefCell;
use std::rc::Rc;
use unicode_display_width;
use unicode_segmentation::UnicodeSegmentation;

use super::{
    Dimensions, EventHandler, MeasureSpecs, RequestedDimension, RequestedDimensions, View,
};

pub struct Input<E> {
    pub editor: TextEditor,
    pub tmp_buf: Option<String>,
    pub password: bool,
    pub history: Vec<String>,
    pub history_index: usize,
    pub event_handler: Option<EventHandler<Self, E>>,
    pub show_cursor: bool,
    pub cursor_style: CursorStyle,
    dimensions: Option<Dimensions>,
}

impl<E> Default for Input<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> Input<E> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            editor: TextEditor::new(),
            tmp_buf: None,
            password: false,
            history: Vec::new(),
            history_index: 0,
            event_handler: None,
            show_cursor: true,
            cursor_style: CursorStyle::SteadyBar,
            dimensions: None,
        }
    }

    pub fn set_show_cursor(&mut self, visible: bool) {
        self.show_cursor = visible;
    }

    pub fn set_cursor_style(&mut self, style: CursorStyle) {
        self.cursor_style = style;
    }

    #[must_use]
    pub fn with_event<F>(mut self, event_handler: F) -> Self
    where
        F: FnMut(&mut Self, &mut E) + 'static,
    {
        self.event_handler = Some(Rc::new(RefCell::new(Box::new(event_handler))));
        self
    }

    pub fn key(&mut self, c: char) {
        self.editor.key(c);
    }

    pub fn backspace(&mut self) {
        self.editor.backspace();
    }

    pub fn backward_delete_word(&mut self) {
        self.editor.backward_delete_word();
    }

    pub fn delete_from_cursor_to_start(&mut self) {
        self.editor.delete_from_cursor_to_start();
    }

    pub fn delete_from_cursor_to_end(&mut self) {
        self.editor.delete_from_cursor_to_end();
    }

    pub fn delete(&mut self) {
        self.editor.delete();
    }

    pub fn home(&mut self) {
        self.editor.home();
    }

    pub fn end(&mut self) {
        self.editor.end();
    }

    pub fn clear(&mut self) {
        self.editor.clear();
        let _ = self.tmp_buf.take();
        self.password = false;
        self.show_cursor = true;
    }

    pub fn left(&mut self) {
        self.editor.left();
    }

    pub fn right(&mut self) {
        self.editor.right();
    }

    pub fn word_left(&mut self) {
        self.editor.word_left();
    }

    pub fn word_right(&mut self) {
        self.editor.word_right();
    }

    pub fn password(&mut self) {
        self.password = true;
        self.show_cursor = false;
    }

    pub fn validate(&mut self) -> (String, bool) {
        if !self.password {
            self.history.push(self.editor.buf.clone());
            self.history_index = self.history.len();
        }
        let buf = self.editor.buf.clone();
        let password = self.password;
        self.clear();
        (buf, password)
    }

    pub fn previous(&mut self) {
        if self.history_index == 0 {
            return;
        }

        if self.tmp_buf.is_none() {
            self.tmp_buf = Some(self.editor.buf.clone());
        }

        self.history_index -= 1;
        self.editor.buf = self.history[self.history_index].clone();
        self.editor.end();
    }

    /// # Panics
    ///
    /// Panics if `history_index == history.len()` and `tmp_buf` is `None`.
    pub fn next(&mut self) {
        if self.history_index == self.history.len() {
            return;
        }

        self.history_index += 1;
        if self.history_index == self.history.len() {
            self.editor.buf = self.tmp_buf.take().unwrap();
        } else {
            self.editor.buf = self.history[self.history_index].clone();
        }
        self.editor.end();
    }
}

impl<E, C> View<E, C> for Input<E> {
    fn on_focus_change(&mut self, _focused: bool) {
        // Cursor visibility is managed by mode changes, not by focus transitions.
    }

    fn insertable(&self) -> bool {
        true
    }

    fn measure(&self, _measure_specs: &MeasureSpecs) -> RequestedDimensions {
        RequestedDimensions {
            width: RequestedDimension::ExpandMax,
            height: RequestedDimension::Absolute(1),
        }
    }

    fn layout(&mut self, dimensions: &Dimensions) {
        log::debug!("layout {} {:?}", std::any::type_name::<Self>(), dimensions);
        if self.dimensions.as_ref() != Some(dimensions) {
            self.dimensions.replace(dimensions.clone());
        }
    }

    fn render(&self, mut frame: ScreenFrame, _config: &C) {
        log::debug!(
            "rendering {} at {:?}",
            std::any::type_name::<Self>(),
            self.dimensions
        );

        self.editor.set_width(frame.dimensions.width as usize);
        if self.password {
            let prompt = "password: ";
            frame.write(prompt);
            frame.set_cursor_with_priority(
                CursorPos {
                    top: frame.dimensions.top,
                    #[allow(clippy::cast_possible_truncation)]
                    left: frame.dimensions.left + prompt.len() as u16,
                },
                1,
            );
        } else {
            // Max displayable size is view width less 1 for cursor
            let max_size = (frame.dimensions.width - 1) as usize;

            // cursor must always be inside the view
            if self.editor.cursor < self.editor.view {
                if self.editor.cursor < max_size {
                    self.editor.view.set(0);
                } else {
                    self.editor
                        .view
                        .update(&self.editor.cursor - (frame.dimensions.width as usize - 1));
                }
            } else if self.editor.cursor > &self.editor.view + (frame.dimensions.width as usize - 1)
            {
                self.editor
                    .view
                    .update(&self.editor.cursor - (frame.dimensions.width as usize - 1));
            }
            assert!(self.editor.cursor >= self.editor.view);
            assert!(self.editor.cursor <= &self.editor.view + (max_size + 1));

            let start_index = self.editor.view.index(&self.editor.buf);
            let end_index = (&self.editor.view + max_size).index(&self.editor.buf);
            let buf = &self.editor.buf[start_index..end_index];

            frame.write(buf);

            let cursor_byte_index = self.editor.cursor.index(&self.editor.buf);
            #[allow(clippy::cast_possible_truncation)]
            let cursor_col: u16 = self.editor.buf[start_index..cursor_byte_index]
                .graphemes(true)
                .map(|g| unicode_display_width::width(g) as u16)
                .sum();
            frame.set_cursor_with_priority(
                CursorPos {
                    top: frame.dimensions.top,
                    left: frame.dimensions.left + cursor_col,
                },
                1,
            );
        }
        frame.set_cursor_style(self.cursor_style);
        frame.set_cursor_visible(self.show_cursor);
    }

    fn event(&mut self, event: &mut E) {
        if let Some(handler) = &self.event_handler {
            let handler = Rc::clone(handler);
            let handler = &mut *handler.borrow_mut();
            handler(self, event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::View;

    #[test]
    fn test_on_focus_change_does_not_affect_cursor() {
        let mut input = Input::<()>::new();
        // Cursor visibility must not change on focus loss — mode changes manage it.
        <Input<()> as View<(), ()>>::on_focus_change(&mut input, false);
        assert!(input.show_cursor);
    }

    #[test]
    fn test_on_focus_change_true_shows_cursor() {
        let mut input = Input::<()>::new();
        <Input<()> as View<(), ()>>::on_focus_change(&mut input, true);
        assert!(input.show_cursor);
    }

    #[test]
    fn test_input_backspace() {
        // Given
        let mut input = Input::<()>::new();

        // When
        input.key('a');
        input.key('b');
        input.key('c');
        input.backspace();

        // Then
        assert_eq!(input.editor.buf, "ab".to_string());
    }
}
