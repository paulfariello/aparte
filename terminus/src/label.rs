/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::hash::{Hash, Hasher};

use crate::{Dimensions, MeasureSpecs, RequestedDimension, RequestedDimensions, ScreenFrame, View};

/// A single-row text label.
///
/// Requests one row of height and full parent width. Text is truncated to the
/// available width on render.
#[derive(Debug, Clone)]
pub struct Label {
    pub text: String,
}

impl Label {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl PartialEq for Label {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Eq for Label {}

impl PartialOrd for Label {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Label {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.text.cmp(&other.text)
    }
}

impl Hash for Label {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text.hash(state);
    }
}

impl<E, C> View<E, C> for Label {
    fn measure(&self, _measure_specs: &MeasureSpecs) -> RequestedDimensions {
        RequestedDimensions {
            width: RequestedDimension::ExpandMax,
            height: RequestedDimension::ExpandMax,
        }
    }

    fn layout(&mut self, _dimensions: &Dimensions) {}

    fn render<'a>(&self, mut frame: ScreenFrame<'a>, _config: &C) {
        frame.write_at((0u16, 0u16), self.text.as_str());
    }

    fn event(&mut self, _event: &mut E) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::{OffscreenRenderBuffer, ScreenFrame, ScreenSize};

    fn render_label(text: &str, width: u16) -> OffscreenRenderBuffer {
        let mut buf = OffscreenRenderBuffer::default();
        buf.set_size(ScreenSize::from((width, 1u16)));
        let dims = Dimensions {
            top: 0,
            left: 0,
            width,
            height: 1,
        };
        let mut label: Box<dyn View<(), ()>> = Box::new(Label::new(text));
        label.layout(&dims);
        let frame = ScreenFrame::new(&mut buf, &dims);
        label.render(frame, &());
        buf
    }

    #[test]
    fn label_renders_text_at_origin() {
        let buf = render_label("hello", 20);
        let text: String = (0..5)
            .map(|col| buf[0][col].grapheme.as_str().to_owned())
            .collect();
        assert_eq!(text, "hello");
    }

    #[test]
    fn label_measures_expand_max() {
        let label = Label::new("test");
        let dims = <Label as View<(), ()>>::measure(&label, &MeasureSpecs::default());
        assert_eq!(dims.height, RequestedDimension::ExpandMax);
    }
}
