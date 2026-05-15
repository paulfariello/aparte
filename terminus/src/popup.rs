/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use crate::charxel::{Charxel, Grapheme};
use crate::rendering::{OffscreenRenderBuffer, ScreenFrame};
use crate::{ColorTuple, Dimensions, MeasureSpec, MeasureSpecs, RequestedDimension, View};

pub trait PopupColors {
    fn popup_colors(&self) -> ColorTuple;
}

impl PopupColors for () {
    fn popup_colors(&self) -> ColorTuple {
        ColorTuple::default()
    }
}

/// Popup overlay: centered, bordered content drawn on top of another view.
pub struct Popup<E, C = ()> {
    pub(crate) content: Option<Box<dyn View<E, C>>>,
    pub(crate) title: Option<String>,
    /// Outer box dimensions (border included), in absolute buffer coordinates.
    /// `None` when hidden or parent is too small.
    pub(crate) content_dims: Option<Dimensions>,
}

impl<E, C> Default for Popup<E, C> {
    fn default() -> Self {
        Self {
            content: None,
            title: None,
            content_dims: None,
        }
    }
}

impl<E, C> Popup<E, C> {
    pub fn show(&mut self, content: Box<dyn View<E, C>>, title: Option<String>) {
        self.content = Some(content);
        self.title = title;
    }

    pub fn hide(&mut self) {
        self.content = None;
        self.title = None;
        self.content_dims = None;
    }

    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.content.is_some()
    }

    pub fn content_mut(&mut self) -> Option<&mut Box<dyn View<E, C>>> {
        self.content.as_mut()
    }

    /// Compute and store the popup's box dimensions given the parent area.
    /// Must be called after `show()` and whenever the parent is laid out.
    pub fn layout(&mut self, parent: &Dimensions) {
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

impl<E, C: PopupColors> Popup<E, C> {
    /// Draw the popup border and content onto `buf`. No-op when hidden.
    pub fn render(&self, buf: &mut OffscreenRenderBuffer, config: &C) {
        if let (Some(content), Some(outer)) = (&self.content, &self.content_dims) {
            draw_border(buf, outer, &config.popup_colors(), self.title.as_deref());
            if outer.width >= 2 && outer.height >= 2 {
                let inner = Dimensions {
                    top: outer.top + 1,
                    left: outer.left + 1,
                    width: outer.width - 2,
                    height: outer.height - 2,
                };
                content.render(ScreenFrame::new(buf, &inner), config);
            }
        }
    }
}

fn draw_border(
    buf: &mut OffscreenRenderBuffer,
    dims: &Dimensions,
    color: &ColorTuple,
    title: Option<&str>,
) {
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
        buf[row][col].set_color(color);
    }

    // Top border: render "─ Title ─...─" if title fits, otherwise plain "─...─".
    let inner_w = (right - left).saturating_sub(1) as usize;
    let title_graphemes: Vec<String> = title
        .and_then(|t| {
            if t.chars().count() + 4 <= inner_w {
                Some(format!("─ {t} ─"))
            } else {
                None
            }
        })
        .map(|s| s.chars().map(|c| c.to_string()).collect())
        .unwrap_or_default();

    for (i, col) in (left + 1..right).enumerate() {
        let g: &str = title_graphemes.get(i).map_or("─", String::as_str);
        buf[top][col].grapheme = Grapheme::from(g);
        buf[top][col].set_color(color);
    }

    for col in left + 1..right {
        buf[bot][col].grapheme = Grapheme::from("─");
        buf[bot][col].set_color(color);
    }

    for row in top + 1..bot {
        buf[row][left].grapheme = Grapheme::from("│");
        buf[row][left].set_color(color);
        buf[row][right].grapheme = Grapheme::from("│");
        buf[row][right].set_color(color);
        for col in left + 1..right {
            buf[row][col] = Charxel::default();
            buf[row][col].set_background(color.bg);
        }
    }
}
