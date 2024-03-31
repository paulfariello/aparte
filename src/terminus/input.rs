/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use crate::cursor::Cursor;
use std::cell::{Cell, RefCell};
use std::io::Write;
use std::os::fd::AsFd;
use std::rc::Rc;
use unicode_segmentation::UnicodeSegmentation;

use super::{
    next_word, term_string_visible_len, Dimension, LayoutParam, LayoutParams, Screen, View,
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
    pub event_handler: Option<Rc<RefCell<Box<dyn FnMut(&mut Self, &mut E)>>>>,
    pub dirty: Cell<bool>,
    width: Cell<usize>,
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
            dirty: Cell::new(true),
            width: Cell::new(0),
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

        if !self.password {
            self.dirty.set(true);
        }
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
        if !self.password {
            self.dirty.set(true);
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
        if !self.password {
            self.dirty.set(true);
        }
    }

    pub fn delete_from_cursor_to_start(&mut self) {
        self.buf.replace_range(0..self.cursor.index(&self.buf), "");
        self.cursor.set(0);
        self.view.set(0);
        if !self.password {
            self.dirty.set(true);
        }
    }

    pub fn delete_from_cursor_to_end(&mut self) {
        self.buf.replace_range(self.cursor.index(&self.buf).., "");
        if !self.password {
            self.dirty.set(true);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.buf.graphemes(true).count() {
            let byte_index = self.cursor.index(&self.buf);

            self.buf.remove(byte_index);
            while !self.buf.is_char_boundary(byte_index) {
                self.buf.remove(byte_index);
            }
        }
        if !self.password {
            self.dirty.set(true);
        }
    }

    pub fn home(&mut self) {
        self.cursor.set(0);
        self.view.set(0);
        if !self.password {
            self.dirty.set(true);
        }
    }

    pub fn end(&mut self) {
        self.cursor.set(self.buf.graphemes(true).count());
        if self.cursor > self.width.get() - 1 {
            self.view = &self.cursor - (self.width.get() - 1);
        } else {
            self.view.set(0);
        }
        if !self.password {
            self.dirty.set(true);
        }
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.cursor.set(0);
        self.view.set(0);
        let _ = self.tmp_buf.take();
        self.password = false;
        self.dirty.set(true);
    }

    pub fn left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
        if !self.password {
            self.dirty.set(true);
        }
    }

    pub fn right(&mut self) {
        if self.cursor < term_string_visible_len(&self.buf) {
            self.cursor += 1;
        }
        if !self.password {
            self.dirty.set(true);
        }
    }

    pub fn word_left(&mut self) {
        let iter = self.buf[..self.cursor.index(&self.buf)].chars().rev();
        self.cursor -= next_word(iter);

        if !self.password {
            self.dirty.set(true);
        }
    }

    pub fn word_right(&mut self) {
        let iter = self.buf[self.cursor.index(&self.buf)..].chars();
        self.cursor += next_word(iter);

        if !self.password {
            self.dirty.set(true);
        }
    }

    pub fn password(&mut self) {
        self.password = true;
        self.dirty.set(true);
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
        self.dirty.set(true);
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
        self.dirty.set(true);
    }
}

impl<E, W> View<E, W> for Input<E>
where
    W: Write + AsFd,
{
    fn render(&self, dimension: &Dimension, screen: &mut Screen<W>) {
        self.width.set(dimension.width as usize);
        match self.password {
            true => {
                super::goto!(screen, dimension.x, dimension.y);
                super::vprint!(screen, "password: ");
                super::flush!(screen);

                self.dirty.set(false);
            }
            false => {
                // Max displayable size is view width less 1 for cursor
                let max_size = (dimension.width - 1) as usize;

                // cursor must always be inside the view
                if self.cursor < self.view {
                    if self.cursor < max_size {
                        self.view.set(0);
                    } else {
                        self.view
                            .update(&self.cursor - (dimension.width as usize - 1));
                    }
                } else if self.cursor > &self.view + (dimension.width as usize - 1) {
                    self.view
                        .update(&self.cursor - (dimension.width as usize - 1));
                }
                assert!(self.cursor >= self.view);
                assert!(self.cursor <= &self.view + (max_size + 1));

                let start_index = self.view.index(&self.buf);
                let end_index = (&self.view + max_size).index(&self.buf);
                let buf = &self.buf[start_index..end_index];
                let cursor = &self.cursor - &self.view;

                super::goto!(screen, dimension.x, dimension.y);
                for _ in 0..max_size {
                    super::vprint!(screen, " ");
                }

                super::goto!(screen, dimension.x, dimension.y);
                super::vprint!(screen, "{}", buf);
                super::goto!(screen, dimension.x + cursor.get() as u16, dimension.y);

                super::flush!(screen);

                self.dirty.set(false);
            }
        }
    }

    fn is_layout_dirty(&self) -> bool {
        // Content can never change Input dimension
        false
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn event(&mut self, event: &mut E) {
        if let Some(handler) = &self.event_handler {
            let handler = Rc::clone(handler);
            let handler = &mut *handler.borrow_mut();
            handler(self, event);
        }
    }

    fn get_layout(&self) -> LayoutParams {
        LayoutParams {
            width: LayoutParam::MatchParent,
            height: LayoutParam::Absolute(1),
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
