/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::cell::RefCell;
use std::rc::Rc;

use crate::popup::{Popup, PopupColors};
use crate::rendering::ScreenFrame;
use crate::{Dimensions, EventHandler, MeasureSpecs, RequestedDimensions, View};

/// Root compositor: renders a background view and optionally overlays a popup.
///
/// When no popup is visible all events are forwarded to the background.  When
/// one is shown, keyboard events are captured by the popup content.  An
/// explicit `with_event` closure overrides all default routing.
pub struct Root<E, C = ()> {
    background: Box<dyn View<E, C>>,
    pub(crate) popup: Popup<E, C>,
    /// Parent dimensions from the last `layout()` call, used to lay out a
    /// popup immediately on `show()`.
    dimensions: Option<Dimensions>,
    event_handler: Option<EventHandler<Self, E>>,
}

impl<E, C> Root<E, C> {
    pub fn new(background: impl View<E, C> + 'static) -> Self {
        Self {
            background: Box::new(background),
            popup: Popup::default(),
            dimensions: None,
            event_handler: None,
        }
    }

    /// Show `content` as a centered popup overlay with an optional title.
    /// Lays out immediately if parent dimensions are known.
    pub fn show(&mut self, content: Box<dyn View<E, C>>, title: Option<String>) {
        self.popup.show(content, title);
        if let Some(dims) = self.dimensions.clone() {
            self.popup.layout(&dims);
        }
    }

    pub fn hide(&mut self) {
        self.popup.hide();
    }

    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.popup.is_visible()
    }

    pub fn background_mut(&mut self) -> &mut dyn View<E, C> {
        self.background.as_mut()
    }

    pub fn content_mut(&mut self) -> Option<&mut Box<dyn View<E, C>>> {
        self.popup.content_mut()
    }

    #[must_use]
    pub fn with_event<F>(mut self, event_handler: F) -> Self
    where
        F: FnMut(&mut Self, &mut E) + 'static,
    {
        self.event_handler = Some(Rc::new(RefCell::new(Box::new(event_handler))));
        self
    }
}

impl<E, C: PopupColors> View<E, C> for Root<E, C> {
    fn measure(&self, measure_specs: &MeasureSpecs) -> RequestedDimensions {
        self.background.measure(measure_specs)
    }

    fn layout(&mut self, dimensions: &Dimensions) {
        self.dimensions = Some(dimensions.clone());
        self.background.layout(dimensions);
        self.popup.layout(dimensions);
    }

    fn render(&self, frame: ScreenFrame<'_>, config: &C) {
        // Reborrow so the mutable reference can be reused for the popup layer.
        let parent_dims = frame.dimensions;
        let buf = frame.offscreen;
        self.background
            .render(ScreenFrame::new(&mut *buf, parent_dims), config);
        self.popup.render(&mut *buf, config);
    }

    fn event(&mut self, event: &mut E) {
        if let Some(handler) = &self.event_handler {
            let handler = Rc::clone(handler);
            let handler = &mut *handler.borrow_mut();
            handler(self, event);
        } else if let Some(content) = self.popup.content_mut() {
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
    use crate::rendering::{OffscreenRenderBuffer, ScreenSize};
    use crate::stories::{render_view_into, row_text};
    use crate::Color;

    const W: u16 = 40;
    const H: u16 = 10;

    fn make_root() -> Root<(), ()> {
        Root::new(Label::new("background"))
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
        let mut root = make_root();
        let buf = render(&mut root, W, H);
        let text = row_text(&buf, 0, W);
        assert!(
            text.contains("background"),
            "expected 'background' text in row 0, got: {text:?}"
        );
    }

    #[test]
    fn popup_content_dims_none_when_hidden() {
        let mut root = make_root();
        let dims = Dimensions {
            top: 0,
            left: 0,
            width: W,
            height: H,
        };
        root.layout(&dims);
        assert!(root.popup.content_dims.is_none());
    }

    #[test]
    fn popup_show_sets_content_dims() {
        let mut root = make_root();
        let dims = Dimensions {
            top: 0,
            left: 0,
            width: W,
            height: H,
        };
        root.layout(&dims);
        root.show(Box::new(Label::new("hello")), None);
        assert!(root.popup.content_dims.is_some());
    }

    #[test]
    fn popup_content_is_centered() {
        let mut root = make_root();
        root.show(Box::new(Label::new("hello")), None);
        let buf = render(&mut root, W, H);

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
        let mut root = make_root();
        root.show(Box::new(Label::new("hidden text")), None);
        root.hide();
        let buf = render(&mut root, W, H);

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
        let mut root = make_root();
        assert!(!root.is_visible());
        root.show(Box::new(Label::new("x")), None);
        assert!(root.is_visible());
        root.hide();
        assert!(!root.is_visible());
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
                width: crate::RequestedDimension::ExpandMax,
                height: crate::RequestedDimension::ExpandMax,
            }
        }
        fn layout(&mut self, _: &Dimensions) {}
        fn render(&self, _: ScreenFrame<'_>, _: &()) {}
        fn event(&mut self, event: &mut u32) {
            self.count += 1;
            *event += 1;
        }
    }

    #[test]
    fn popup_default_event_routes_to_background_when_hidden() {
        let mut root: Root<u32, ()> = Root::new(Counter::new());
        let mut ev = 0u32;
        root.event(&mut ev);
        assert_eq!(ev, 1, "event should reach background when popup is hidden");
    }

    #[test]
    fn popup_default_event_routes_to_content_when_visible() {
        let mut root: Root<u32, ()> = Root::new(Counter::new());
        root.show(Box::new(Counter::new()), None);
        let mut ev = 0u32;
        root.event(&mut ev);
        assert_eq!(ev, 1, "event should reach content when popup is visible");
    }

    #[test]
    fn popup_height_equals_content_lines_plus_border() {
        let mut root = make_root();
        let mut content = LinearLayout::new(Orientation::Vertical);
        content.push(Label::new("line one"), 1);
        content.push(Label::new("line two"), 1);
        content.push(Label::new("line three"), 1);
        root.show(Box::new(content), None);
        let dims = Dimensions {
            top: 0,
            left: 0,
            width: W,
            height: H,
        };
        root.layout(&dims);

        let outer = root.popup.content_dims.clone().unwrap();
        assert_eq!(
            outer.height, 5,
            "popup should be exactly 3 content lines + 2 border rows"
        );
    }

    #[test]
    fn popup_width_equals_longest_line_plus_border() {
        let mut root = make_root();
        let mut content = LinearLayout::new(Orientation::Vertical);
        content.push(Label::new("short"), 1);
        content.push(Label::new("much longer line"), 1);
        root.show(Box::new(content), None);
        let dims = Dimensions {
            top: 0,
            left: 0,
            width: W,
            height: H,
        };
        root.layout(&dims);

        // "much longer line" is 16 chars + 2 border = 18
        let outer = root.popup.content_dims.clone().unwrap();
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
        let mut root: Root<(), ThemedConfig> = Root::new(Label::new("background"));
        root.show(Box::new(Label::new("hello")), None);

        let dims = Dimensions {
            top: 0,
            left: 0,
            width: W,
            height: H,
        };
        let mut buf = OffscreenRenderBuffer::default();
        buf.set_size(ScreenSize::from((W, H)));
        root.layout(&dims);
        let frame = ScreenFrame::new(&mut buf, &dims);
        root.render(frame, &ThemedConfig);

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

    #[test]
    fn popup_title_appears_in_top_border() {
        // W=40, H=10, Label("hello") → outer_w=7, outer_h=3
        // outer_top=3, outer_left=16
        // Top border inner runs col 17..22 (left+1..right).
        // "─ hi ─" is 6 chars, inner_w = right-left-1 = 5, 2+2+4=6 > 5 so title should be skipped...
        // Use a wider popup. Label("much longer line") → inner_w=16, outer_w=18
        // outer_left = (40-18)/2 = 11, outer_right = 28
        // Top border inner: col 12..28
        // "─ hi ─" = 6 chars ≤ 16 → fits
        let mut root = make_root();
        let mut content = LinearLayout::new(Orientation::Vertical);
        content.push(Label::new("much longer line"), 1);
        root.show(Box::new(content), Some("hi".to_string()));
        let buf = render(&mut root, W, H);

        // The top border row should contain the title text "hi"
        let top_border = row_text(&buf, 3, W);
        assert!(
            top_border.contains("hi"),
            "expected title 'hi' in top border row, got: {top_border:?}"
        );
        // And still have the corner glyphs
        assert!(
            top_border.contains('┌'),
            "expected top-left corner in border row, got: {top_border:?}"
        );
    }

    #[test]
    fn popup_title_too_long_falls_back_to_plain_border() {
        // inner_w for Label("hello") = 5; "─ " + title + " ─" needs title.len()+4 ≤ 5
        // So any title with len ≥ 2 won't fit.
        let mut root = make_root();
        root.show(Box::new(Label::new("hello")), Some("toolong".to_string()));
        let buf = render(&mut root, W, H);

        // Top border should not contain "toolong" — only box-drawing chars
        let top_border = row_text(&buf, 3, W);
        assert!(
            !top_border.contains("toolong"),
            "title that doesn't fit should not appear in border, got: {top_border:?}"
        );
        assert!(
            top_border.contains('┌'),
            "corner should still be present, got: {top_border:?}"
        );
    }

    #[test]
    fn popup_hide_clears_title() {
        let mut root = make_root();
        root.show(Box::new(Label::new("hello")), Some("Title".to_string()));
        assert_eq!(root.popup.title, Some("Title".to_string()));
        root.hide();
        assert_eq!(root.popup.title, None);
    }
}
