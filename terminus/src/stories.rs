/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//! Storybook infrastructure for terminus.

use crate::label::Label;
use crate::linear_layout::{LinearLayout, Orientation};
use crate::rendering::{OffscreenRenderBuffer, ScreenFrame, ScreenSize};
use crate::{Dimensions, View};

/// Layout and render `view` into `buf` at the given `dims`.
pub fn render_view_into(
    buf: &mut OffscreenRenderBuffer,
    view: &mut dyn View<(), ()>,
    dims: &Dimensions,
) {
    view.layout(dims);
    let frame = ScreenFrame::new(buf, dims);
    view.render(frame, &());
}

/// Render `view` into a fresh offscreen buffer of `(width, height)` cells.
pub fn render_to_buffer(
    view: &mut dyn View<(), ()>,
    width: u16,
    height: u16,
) -> OffscreenRenderBuffer {
    let mut buf = OffscreenRenderBuffer::default();
    buf.set_size(ScreenSize::from((width, height)));
    let dims = Dimensions {
        top: 0,
        left: 0,
        width,
        height,
    };
    render_view_into(&mut buf, view, &dims);
    buf
}

/// Extract the visible text of `row` from `buf` as a plain `String`.
pub fn row_text(buf: &OffscreenRenderBuffer, row: u16, width: u16) -> String {
    (0..width)
        .map(|col| buf[row][col].grapheme.as_str().to_owned())
        .collect()
}

/// A named terminus story.
pub struct Story {
    pub name: &'static str,
    pub description: &'static str,
    build_fn: fn() -> Box<dyn View<(), ()>>,
}

impl Story {
    /// Build a fresh view for this story.
    #[must_use]
    pub fn build(&self) -> Box<dyn View<(), ()>> {
        (self.build_fn)()
    }

    /// Render this story to a new offscreen buffer.
    #[must_use]
    pub fn render(&self, width: u16, height: u16) -> OffscreenRenderBuffer {
        let mut view = self.build();
        render_to_buffer(view.as_mut(), width, height)
    }
}

// ---------------------------------------------------------------------------
// Story definitions
// ---------------------------------------------------------------------------

fn build_label() -> Box<dyn View<(), ()>> {
    Box::new(Label::new("Hello, terminus!"))
}

fn build_vertical_layout() -> Box<dyn View<(), ()>> {
    let mut layout = LinearLayout::<()>::new(Orientation::Vertical);
    layout.push(Label::new("Top section"), 1);
    layout.push(Label::new("Bottom section"), 1);
    Box::new(layout)
}

fn build_horizontal_layout() -> Box<dyn View<(), ()>> {
    let mut layout = LinearLayout::<()>::new(Orientation::Horizontal);
    layout.push(Label::new("Left"), 1);
    layout.push(Label::new("Right"), 1);
    Box::new(layout)
}

fn build_nested_layout() -> Box<dyn View<(), ()>> {
    let mut outer = LinearLayout::<()>::new(Orientation::Vertical);

    let mut top_row = LinearLayout::<()>::new(Orientation::Horizontal);
    top_row.push(Label::new("TL"), 1);
    top_row.push(Label::new("TR"), 1);

    let mut bot_row = LinearLayout::<()>::new(Orientation::Horizontal);
    bot_row.push(Label::new("BL"), 1);
    bot_row.push(Label::new("BR"), 1);

    outer.push(top_row, 1);
    outer.push(bot_row, 1);
    Box::new(outer)
}

/// All stories in display order.
#[must_use]
pub fn all_stories() -> Vec<Story> {
    vec![
        Story {
            name: "label",
            description: "A single text label",
            build_fn: build_label,
        },
        Story {
            name: "vertical_layout",
            description: "Two labels stacked vertically",
            build_fn: build_vertical_layout,
        },
        Story {
            name: "horizontal_layout",
            description: "Two labels side by side",
            build_fn: build_horizontal_layout,
        },
        Story {
            name: "nested_layout",
            description: "Horizontal rows nested inside a vertical layout",
            build_fn: build_nested_layout,
        },
    ]
}

// ---------------------------------------------------------------------------
// Automated tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const W: u16 = 40;
    const H: u16 = 10;

    fn render(build: fn() -> Box<dyn View<(), ()>>, w: u16, h: u16) -> OffscreenRenderBuffer {
        let mut view = build();
        render_to_buffer(view.as_mut(), w, h)
    }

    #[test]
    fn story_label_renders_text() {
        let buf = render(build_label, W, H);
        let text = row_text(&buf, 0, W);
        assert!(
            text.contains("Hello, terminus!"),
            "expected 'Hello, terminus!' in row 0, got: {text:?}"
        );
    }

    #[test]
    fn story_vertical_layout_places_top_label() {
        let buf = render(build_vertical_layout, W, H);
        let top = row_text(&buf, 0, W);
        assert!(
            top.contains("Top section"),
            "expected 'Top section' in row 0, got: {top:?}"
        );
    }

    #[test]
    fn story_vertical_layout_places_bottom_label() {
        let buf = render(build_vertical_layout, W, H);
        // Labels report Absolute(1) height, so "Bottom section" is at row 1.
        let bot = row_text(&buf, 1, W);
        assert!(
            bot.contains("Bottom section"),
            "expected 'Bottom section' in row 1, got: {bot:?}"
        );
    }

    #[test]
    fn story_horizontal_layout_places_both_labels_on_same_row() {
        let buf = render(build_horizontal_layout, W, H);
        let row = row_text(&buf, 0, W);
        assert!(row.contains("Left"), "expected 'Left': {row:?}");
        assert!(row.contains("Right"), "expected 'Right': {row:?}");
    }

    #[test]
    fn story_nested_layout_places_four_quadrants() {
        let buf = render(build_nested_layout, W, H);
        let top_row = row_text(&buf, 0, W);
        // Labels report Absolute(1) height, so each inner LinearLayout is 1 row tall.
        let bot_row = row_text(&buf, 1, W);
        assert!(
            top_row.contains("TL"),
            "expected TL in top row: {top_row:?}"
        );
        assert!(
            top_row.contains("TR"),
            "expected TR in top row: {top_row:?}"
        );
        assert!(
            bot_row.contains("BL"),
            "expected BL in bot row: {bot_row:?}"
        );
        assert!(
            bot_row.contains("BR"),
            "expected BR in bot row: {bot_row:?}"
        );
    }
}
