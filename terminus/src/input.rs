/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use crate::charxel::IntoCharxels;
use crate::color::ColorTuple;
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
    pub cursor_style: CursorStyle,
    dimensions: Option<Dimensions>,
    prompt: Option<(String, ColorTuple)>,
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
            cursor_style: CursorStyle::SteadyBar,
            dimensions: None,
            prompt: None,
        }
    }

    #[must_use]
    pub fn with_prompt(mut self, text: impl Into<String>, color: ColorTuple) -> Self {
        self.prompt = Some((text.into(), color));
        self
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
            // No cursor in password mode — leave it hidden.
        } else {
            #[allow(clippy::cast_possible_truncation)]
            let prompt_width: u16 = self
                .prompt
                .as_ref()
                .map(|(text, _)| unicode_display_width::width(text.as_str()) as u16)
                .unwrap_or(0);

            // Max displayable size is view width less prompt width less 1 for cursor
            let max_size = (frame.dimensions.width.saturating_sub(prompt_width + 1)) as usize;

            // cursor must always be inside the view
            let cursor = self.editor.cursor.get();
            let view = self.editor.view.get();
            if cursor < view || cursor > view + max_size {
                self.editor.view.set(cursor.saturating_sub(max_size));
            }
            assert!(self.editor.cursor >= self.editor.view);
            assert!(self.editor.cursor <= &self.editor.view + max_size);

            if let Some((text, color)) = &self.prompt {
                frame.write(text.as_str().with_color(color));
            }

            let start_index = self.editor.view.index(&self.editor.buf);
            let end_index = (&self.editor.view + max_size).index(&self.editor.buf);
            let buf = &self.editor.buf[start_index..end_index];

            frame.write(buf);

            let cursor_byte_index = self.editor.cursor.index(&self.editor.buf);
            #[allow(clippy::cast_possible_truncation)]
            let text_cursor_col: u16 = self.editor.buf[start_index..cursor_byte_index]
                .graphemes(true)
                .map(|g| unicode_display_width::width(g) as u16)
                .sum();
            frame.set_cursor(CursorPos {
                top: 0,
                left: prompt_width + text_cursor_col,
            });
        }
        frame.set_cursor_style(self.cursor_style);
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
        assert_eq!(input.editor.buf, "ab".to_string());
    }

    fn render_input(input: &Input<()>, width: u16) -> crate::rendering::OffscreenRenderBuffer {
        use crate::rendering::{OffscreenRenderBuffer, ScreenSize};
        let mut buf = OffscreenRenderBuffer::default();
        buf.set_size(ScreenSize::from((width, 1u16)));
        let dims = Dimensions {
            top: 0,
            left: 0,
            width,
            height: 1,
        };
        let frame = ScreenFrame::new(&mut buf, &dims, true);
        View::<(), ()>::render(input, frame, &());
        buf
    }

    #[test]
    fn typing_past_the_view_width_with_prompt_does_not_panic() {
        let width = 80;
        let mut input = Input::<()>::new().with_prompt("> ", ColorTuple::default());
        for _ in 0..(width as usize * 3) {
            input.key('a');
            render_input(&input, width);
        }
    }

    #[test]
    fn typing_in_a_burst_past_the_view_width_with_prompt_does_not_panic() {
        let width = 80;
        let mut input = Input::<()>::new().with_prompt("> ", ColorTuple::default());
        for _ in 0..240 {
            input.key('a');
        }
        render_input(&input, width);
    }
}
