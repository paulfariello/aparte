/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::cell::RefCell;
use std::rc::Rc;

use crate::charxel::{Charxel, Grapheme};
use crate::rendering::{OffscreenRenderBuffer, ScreenFrame};
use crate::{
    ColorTuple, Dimensions, EventHandler, MeasureSpec, MeasureSpecs, RequestedDimension,
    RequestedDimensions, View,
};

pub trait PopupColors {
    fn popup_colors(&self) -> ColorTuple;
}

impl PopupColors for () {
    fn popup_colors(&self) -> ColorTuple {
        ColorTuple::default()
    }
}

/// A compositor view that renders a background view and optionally overlays a
/// centered, bordered popup on top.
///
/// When the popup is hidden (`content` is `None`) all events are forwarded to
/// the background view.  When it is visible, keyboard events are captured by
/// the popup content.  An explicit `with_event` closure overrides all default
/// routing.
pub struct PopupLayer<E, C = ()> {
    background: Box<dyn View<E, C>>,
    content: Option<Box<dyn View<E, C>>>,
    /// Outer dimensions of the drawn box (border included), in absolute buffer
    /// coordinates.  `None` when popup is hidden or parent is too small.
    content_dims: Option<Dimensions>,
    /// Parent dimensions stored from the last `layout()` call so that `show()`
    /// can immediately compute content dimensions.
    dimensions: Option<Dimensions>,
    event_handler: Option<EventHandler<Self, E>>,
}

impl<E, C> PopupLayer<E, C> {
    pub fn new(background: impl View<E, C> + 'static) -> Self {
        Self {
            background: Box::new(background),
            content: None,
            content_dims: None,
            dimensions: None,
            event_handler: None,
        }
    }

    /// Show `content` as a centered popup overlay, laying it out immediately
    /// if the parent dimensions are already known.
    pub fn show(&mut self, content: Box<dyn View<E, C>>) {
        self.content = Some(content);
        if let Some(dims) = self.dimensions.clone() {
            self.layout_content(&dims);
        }
    }

    pub fn hide(&mut self) {
        self.content = None;
        self.content_dims = None;
    }

    pub fn is_visible(&self) -> bool {
        self.content.is_some()
    }

    pub fn background_mut(&mut self) -> &mut dyn View<E, C> {
        self.background.as_mut()
    }

    pub fn content_mut(&mut self) -> Option<&mut Box<dyn View<E, C>>> {
        self.content.as_mut()
    }

    pub fn with_event<F>(mut self, event_handler: F) -> Self
    where
        F: FnMut(&mut Self, &mut E) + 'static,
    {
        self.event_handler = Some(Rc::new(RefCell::new(Box::new(event_handler))));
        self
    }

    fn layout_content(&mut self, parent: &Dimensions) {
        if self.content.is_none() {
            self.content_dims = None;
            return;
        }

        // Need at least 4×4 to fit a 1-cell border around a 2×2 content minimum.
        if parent.width < 4 || parent.height < 4 {
            self.content_dims = None;
            return;
        }

        let max_inner_w = (parent.width * 3 / 4).min(parent.width - 2).max(1);
        let max_inner_h = (parent.height * 3 / 4).min(parent.height - 2).max(1);

        let specs = MeasureSpecs {
            width: MeasureSpec::AtMost(max_inner_w),
            height: MeasureSpec::AtMost(max_inner_h),
        };
        let requested = self.content.as_ref().unwrap().measure(&specs);

        let inner_w = match requested.width {
            RequestedDimension::ExpandMax => max_inner_w,
            RequestedDimension::Absolute(v) => v.min(max_inner_w),
        }
        .max(1);
        let inner_h = match requested.height {
            RequestedDimension::ExpandMax => max_inner_h,
            RequestedDimension::Absolute(v) => v.min(max_inner_h),
        }
        .max(1);

        let outer_w = inner_w + 2;
        let outer_h = inner_h + 2;
        let outer_top = parent.top + parent.height.saturating_sub(outer_h) / 2;
        let outer_left = parent.left + parent.width.saturating_sub(outer_w) / 2;

        let inner_dims = Dimensions {
            top: outer_top + 1,
            left: outer_left + 1,
            width: inner_w,
            height: inner_h,
        };
        self.content.as_mut().unwrap().layout(&inner_dims);

        self.content_dims = Some(Dimensions {
            top: outer_top,
            left: outer_left,
            width: outer_w,
            height: outer_h,
        });
    }
}

fn draw_border(buf: &mut OffscreenRenderBuffer, dims: &Dimensions, color: &ColorTuple) {
    if dims.width < 2 || dims.height < 2 {
        return;
    }

    let top = dims.top;
    let left = dims.left;
    let right = dims.left + dims.width - 1;
    let bot = dims.top + dims.height - 1;

    for (row, col, grapheme) in [
        (top, left, "┌"),
        (top, right, "┐"),
        (bot, left, "└"),
        (bot, right, "┘"),
    ] {
        buf[row][col].grapheme = Grapheme::from(grapheme);
        buf[row][col].set_color(color.clone());
    }

    for col in left + 1..right {
        buf[top][col].grapheme = Grapheme::from("─");
        buf[top][col].set_color(color.clone());
        buf[bot][col].grapheme = Grapheme::from("─");
        buf[bot][col].set_color(color.clone());
    }

    for row in top + 1..bot {
        buf[row][left].grapheme = Grapheme::from("│");
        buf[row][left].set_color(color.clone());
        buf[row][right].grapheme = Grapheme::from("│");
        buf[row][right].set_color(color.clone());
        for col in left + 1..right {
            buf[row][col] = Charxel::default();
            buf[row][col].set_background(color.bg);
        }
    }
}

impl<E, C: PopupColors> View<E, C> for PopupLayer<E, C> {
    fn measure(&self, measure_specs: &MeasureSpecs) -> RequestedDimensions {
        self.background.measure(measure_specs)
    }

    fn layout(&mut self, dimensions: &Dimensions) {
        self.dimensions = Some(dimensions.clone());
        self.background.layout(dimensions);
        self.layout_content(dimensions);
    }

    fn render<'a>(&self, frame: ScreenFrame<'a>, config: &C) {
        // Extract the buffer reference and parent dimensions from the frame.
        // We use reborrows (&mut *buf) so the mutable reference is not consumed
        // and can be reused for the popup overlay.
        let parent_dims = frame.dimensions;
        let buf = frame.offscreen;

        self.background
            .render(ScreenFrame::new(&mut *buf, parent_dims), config);

        if let (Some(content), Some(outer)) = (&self.content, &self.content_dims) {
            draw_border(&mut *buf, outer, &config.popup_colors());
            if outer.width >= 2 && outer.height >= 2 {
                let inner = Dimensions {
                    top: outer.top + 1,
                    left: outer.left + 1,
                    width: outer.width - 2,
                    height: outer.height - 2,
                };
                content.render(ScreenFrame::new(&mut *buf, &inner), config);
            }
        }
    }

    fn event(&mut self, event: &mut E) {
        if let Some(handler) = &self.event_handler {
            let handler = Rc::clone(handler);
            let handler = &mut *handler.borrow_mut();
            handler(self, event);
        } else if let Some(content) = &mut self.content {
            content.event(event);
        } else {
            self.background.event(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{BgColor, FgColor, NamedColor};
    use crate::label::Label;
    use crate::linear_layout::{LinearLayout, Orientation};
    use crate::rendering::ScreenSize;
    use crate::stories::{render_view_into, row_text};

    const W: u16 = 40;
    const H: u16 = 10;

    fn make_popup() -> PopupLayer<(), ()> {
        PopupLayer::new(Label::new("background"))
    }

    fn render(view: &mut dyn View<(), ()>, w: u16, h: u16) -> OffscreenRenderBuffer {
        let dims = Dimensions {
            top: 0,
            left: 0,
            width: w,
            height: h,
        };
        let mut buf = OffscreenRenderBuffer::default();
        buf.set_size(ScreenSize::from((w, h)));
        render_view_into(&mut buf, view, &dims);
        buf
    }

    #[test]
    fn popup_renders_background_when_hidden() {
        let mut popup = make_popup();
        let buf = render(&mut popup, W, H);
        let text = row_text(&buf, 0, W);
        assert!(
            text.contains("background"),
            "expected 'background' text in row 0, got: {text:?}"
        );
    }

    #[test]
    fn popup_content_dims_none_when_hidden() {
        let mut popup = make_popup();
        let dims = Dimensions {
            top: 0,
            left: 0,
            width: W,
            height: H,
        };
        popup.layout(&dims);
        assert!(popup.content_dims.is_none());
    }

    #[test]
    fn popup_show_sets_content_dims() {
        let mut popup = make_popup();
        let dims = Dimensions {
            top: 0,
            left: 0,
            width: W,
            height: H,
        };
        popup.layout(&dims);
        popup.show(Box::new(Label::new("hello")));
        assert!(popup.content_dims.is_some());
    }

    #[test]
    fn popup_content_is_centered() {
        let mut popup = make_popup();
        popup.show(Box::new(Label::new("hello")));
        let buf = render(&mut popup, W, H);

        // With W=40, H=10, Label("hello") = Absolute(5) x Absolute(1):
        // inner_w=5, inner_h=1, outer_w=7, outer_h=3
        // outer_top = (10 - 3) / 2 = 3, outer_left = (40 - 7) / 2 = 16
        // border at row 3, content at row 4
        let top_border_row = row_text(&buf, 3, W);
        assert!(
            top_border_row.contains('┌'),
            "expected top-left corner in row 3, got: {top_border_row:?}"
        );

        let content_row = row_text(&buf, 4, W);
        assert!(
            content_row.contains("hello"),
            "expected popup content 'hello' in row 4, got: {content_row:?}"
        );
    }

    #[test]
    fn popup_hide_removes_content() {
        let mut popup = make_popup();
        popup.show(Box::new(Label::new("hidden text")));
        popup.hide();
        let buf = render(&mut popup, W, H);

        let row0 = row_text(&buf, 0, W);
        assert!(
            !row0.contains("hidden text"),
            "popup content should not appear after hide: {row0:?}"
        );
        assert!(
            row0.contains("background"),
            "background should be visible after hide: {row0:?}"
        );
    }

    #[test]
    fn popup_is_visible_tracks_state() {
        let mut popup = make_popup();
        assert!(!popup.is_visible());
        popup.show(Box::new(Label::new("x")));
        assert!(popup.is_visible());
        popup.hide();
        assert!(!popup.is_visible());
    }

    /// A view that counts how many events it receives.
    struct Counter {
        count: u32,
    }

    impl Counter {
        fn new() -> Self {
            Self { count: 0 }
        }
    }

    impl View<u32, ()> for Counter {
        fn measure(&self, _: &MeasureSpecs) -> RequestedDimensions {
            RequestedDimensions {
                width: RequestedDimension::ExpandMax,
                height: RequestedDimension::ExpandMax,
            }
        }
        fn layout(&mut self, _: &Dimensions) {}
        fn render<'a>(&self, _: ScreenFrame<'a>, _: &()) {}
        fn event(&mut self, event: &mut u32) {
            self.count += 1;
            *event += 1;
        }
    }

    #[test]
    fn popup_default_event_routes_to_background_when_hidden() {
        let mut popup: PopupLayer<u32, ()> = PopupLayer::new(Counter::new());
        let mut ev = 0u32;
        popup.event(&mut ev);
        assert_eq!(ev, 1, "event should reach background when popup is hidden");
    }

    #[test]
    fn popup_default_event_routes_to_content_when_visible() {
        let bg = Counter::new();
        let mut popup: PopupLayer<u32, ()> = PopupLayer::new(bg);
        popup.show(Box::new(Counter::new()));
        let mut ev = 0u32;
        popup.event(&mut ev);
        assert_eq!(ev, 1, "event should reach content when popup is visible");
    }

    #[test]
    fn popup_height_equals_content_lines_plus_border() {
        let mut popup = make_popup();
        let mut content = LinearLayout::new(Orientation::Vertical);
        content.push(Label::new("line one"), 1);
        content.push(Label::new("line two"), 1);
        content.push(Label::new("line three"), 1);
        popup.show(Box::new(content));
        let dims = Dimensions {
            top: 0,
            left: 0,
            width: W,
            height: H,
        };
        popup.layout(&dims);

        let outer = popup.content_dims.unwrap();
        assert_eq!(
            outer.height, 5,
            "popup should be exactly 3 content lines + 2 border rows"
        );
    }

    #[test]
    fn popup_width_equals_longest_line_plus_border() {
        let mut popup = make_popup();
        let mut content = LinearLayout::new(Orientation::Vertical);
        content.push(Label::new("short"), 1);
        content.push(Label::new("much longer line"), 1);
        popup.show(Box::new(content));
        let dims = Dimensions {
            top: 0,
            left: 0,
            width: W,
            height: H,
        };
        popup.layout(&dims);

        // "much longer line" is 16 chars + 2 border = 18
        let outer = popup.content_dims.unwrap();
        assert_eq!(
            outer.width, 18,
            "popup width should match the longest line + 2 border cols"
        );
    }

    struct ThemedConfig;

    impl PopupColors for ThemedConfig {
        fn popup_colors(&self) -> ColorTuple {
            ColorTuple {
                bg: BgColor(Color::Named(NamedColor::Blue)),
                fg: FgColor(Color::Named(NamedColor::White)),
            }
        }
    }

    #[test]
    fn popup_border_cells_have_theme_colors() {
        let mut popup: PopupLayer<(), ThemedConfig> = PopupLayer::new(Label::new("background"));
        popup.show(Box::new(Label::new("hello")));

        let dims = Dimensions {
            top: 0,
            left: 0,
            width: W,
            height: H,
        };
        let mut buf = OffscreenRenderBuffer::default();
        buf.set_size(ScreenSize::from((W, H)));
        popup.layout(&dims);
        let frame = ScreenFrame::new(&mut buf, &dims);
        popup.render(frame, &ThemedConfig);

        // Border top-left corner is at row 3, col 16 (see popup_content_is_centered)
        let corner = &buf[3][16];
        assert_eq!(
            corner.background,
            BgColor(Color::Named(NamedColor::Blue)),
            "border corner should have popup background color"
        );
        assert_eq!(
            corner.foreground,
            FgColor(Color::Named(NamedColor::White)),
            "border corner should have popup foreground color"
        );
    }
}
