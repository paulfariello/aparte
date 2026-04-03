/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use crate::cursor::Cursor;
use crate::rendering::ScreenFrame;
use crate::CursorPos;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use unicode_display_width;
use unicode_segmentation::UnicodeSegmentation;

use super::{
    next_word, Dimensions, EventHandler, MeasureSpecs, RequestedDimension,
    RequestedDimensions, View,
};

pub struct Input<E> {
    pub buf: String,
    pub tmp_buf: Option<String>,
    pub password: bool,
    pub history: Vec<String>,
    pub history_index: usize,
    // Used to index code points in buf (don't use it to directly index buf)
    pub cursor: Cursor,
    // start index (in code points) of the view inside the buffer
    // |-----------------------|
    // | buffer text           |
    // |-----------------------|
    //     |-----------|
    //     | view      |
    //     |-----------|
    pub view: Cursor,
    pub event_handler: Option<EventHandler<Self, E>>,
    width: Cell<usize>,
    dimensions: Option<Dimensions>,
}

impl<E> Default for Input<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> Input<E> {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            tmp_buf: None,
            password: false,
            history: Vec::new(),
            history_index: 0,
            cursor: Cursor::new(0),
            view: Cursor::new(0),
            event_handler: None,
            width: Cell::new(0),
            dimensions: None,
        }
    }

    pub fn with_event<F>(mut self, event_handler: F) -> Self
    where
        F: FnMut(&mut Self, &mut E) + 'static,
    {
        self.event_handler = Some(Rc::new(RefCell::new(Box::new(event_handler))));
        self
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
        if self.cursor > self.width.get() - 1 {
            self.view = &self.cursor - (self.width.get() - 1);
        } else {
            self.view.set(0);
        }
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.cursor.set(0);
        self.view.set(0);
        let _ = self.tmp_buf.take();
        self.password = false;
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

    pub fn password(&mut self) {
        self.password = true;
    }

    pub fn validate(&mut self) -> (String, bool) {
        if !self.password {
            self.history.push(self.buf.clone());
            self.history_index = self.history.len();
        }
        let buf = self.buf.clone();
        let password = self.password;
        self.clear();
        (buf, password)
    }

    pub fn previous(&mut self) {
        if self.history_index == 0 {
            return;
        }

        if self.tmp_buf.is_none() {
            self.tmp_buf = Some(self.buf.clone());
        }

        self.history_index -= 1;
        self.buf = self.history[self.history_index].clone();
        self.end();
    }

    pub fn next(&mut self) {
        if self.history_index == self.history.len() {
            return;
        }

        self.history_index += 1;
        if self.history_index == self.history.len() {
            self.buf = self.tmp_buf.take().unwrap();
        } else {
            self.buf = self.history[self.history_index].clone();
        }
        self.end();
    }
}

impl<E> View<E> for Input<E> {
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

    fn render(&self, mut frame: ScreenFrame) {
        log::debug!(
            "rendering {} at {:?}",
            std::any::type_name::<Self>(),
            self.dimensions
        );

        self.width.set(frame.dimensions.width as usize);
        match self.password {
            true => {
                let prompt = "password: ";
                frame.write(prompt);
                frame.set_cursor(CursorPos {
                    top: frame.dimensions.top,
                    left: frame.dimensions.left + prompt.len() as u16,
                });
            }
            false => {
                // Max displayable size is view width less 1 for cursor
                let max_size = (frame.dimensions.width - 1) as usize;

                // cursor must always be inside the view
                if self.cursor < self.view {
                    if self.cursor < max_size {
                        self.view.set(0);
                    } else {
                        self.view
                            .update(&self.cursor - (frame.dimensions.width as usize - 1));
                    }
                } else if self.cursor > &self.view + (frame.dimensions.width as usize - 1) {
                    self.view
                        .update(&self.cursor - (frame.dimensions.width as usize - 1));
                }
                assert!(self.cursor >= self.view);
                assert!(self.cursor <= &self.view + (max_size + 1));

                let start_index = self.view.index(&self.buf);
                let end_index = (&self.view + max_size).index(&self.buf);
                let buf = &self.buf[start_index..end_index];

                frame.write(buf);

                let cursor_byte_index = self.cursor.index(&self.buf);
                let cursor_col: u16 = self.buf[start_index..cursor_byte_index]
                    .graphemes(true)
                    .map(|g| unicode_display_width::width(g) as u16)
                    .sum();
                frame.set_cursor(CursorPos {
                    top: frame.dimensions.top,
                    left: frame.dimensions.left + cursor_col,
                });
            }
        }
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
        assert_eq!(input.buf, "ab".to_string());
    }
}
