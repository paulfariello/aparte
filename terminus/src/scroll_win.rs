/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::hash::Hash;
use std::rc::Rc;

use crate::ScreenFrame;

use super::Dimensions;
use super::{
    EventHandler, LayoutParam, LayoutParams, MeasureSpec, MeasureSpecs, RequestedDimension,
    RequestedDimensions, View,
};

const MISSING_DIMENSIONS: &str = "Missing dimensions";
const INVALID_VIEW: &str = "Invalid view detected";

#[derive(Debug)]
struct LayoutChild<I> {
    child: I,
    dimensions: Option<Dimensions>,
}

#[cfg(test)]
impl<I: PartialEq> PartialEq<I> for LayoutChild<I> {
    fn eq(&self, other: &I) -> bool {
        self.child.eq(other)
    }
}

impl<I> Ord for LayoutChild<I>
where
    I: Ord,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.child.cmp(&other.child)
    }
}

impl<I> PartialOrd for LayoutChild<I>
where
    I: PartialOrd,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.child.partial_cmp(&other.child)
    }
}

impl<I> Eq for LayoutChild<I> where I: Eq {}

impl<I> PartialEq for LayoutChild<I>
where
    I: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.child.eq(&other.child)
    }
}

/// Ordered vertical window giving ability to scroll
pub struct ScrollWin<E, I>
where
    I: View<E> + Hash + Eq + Ord,
{
    /// Index in children of last visible child (bottom child)
    bottom_visible_child_index: usize,
    event_handler: Option<EventHandler<Self, E>>,
    children: BTreeSet<LayoutChild<I>>,
    layouts: LayoutParams,
    dimensions: Option<Dimensions>,
}

impl<E, I> Default for ScrollWin<E, I>
where
    I: View<E> + Hash + Eq + Ord,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<E, I> ScrollWin<E, I>
where
    I: View<E> + Hash + Eq + Ord,
{
    pub fn new() -> Self {
        Self {
            children: BTreeSet::new(),
            bottom_visible_child_index: 0,
            event_handler: None,
            layouts: LayoutParams {
                width: LayoutParam::MatchParent,
                height: LayoutParam::MatchParent,
            },
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

    pub fn with_layout(mut self, layouts: LayoutParams) -> Self {
        self.layouts = layouts;
        self
    }

    pub fn first(&self) -> Option<&I> {
        self.children
            .iter()
            .next()
            .map(|LayoutChild { child, .. }| child)
    }

    pub fn insert(&mut self, item: I) {
        // If view index is on last child, then keep it there
        let stick_to_bottom = self.bottom_visible_child_index + 1 == self.children.len();
        if self.children.insert(LayoutChild {
            child: item,
            dimensions: None,
        }) && stick_to_bottom
        {
            self.bottom_visible_child_index += 1;
        }
    }

    /// PageUp the window, return true if top is reached
    pub fn page_up(&mut self) -> bool {
        log::debug!("Page up");
        let dimensions = self.dimensions.as_ref().expect(MISSING_DIMENSIONS);
        let measure_specs = MeasureSpecs::from(dimensions);

        let total_children_height: u16 = self
            .children
            .iter()
            .map(
                |LayoutChild { child, .. }| match child.measure(&measure_specs).height {
                    RequestedDimension::ExpandMax => dimensions.height,
                    RequestedDimension::Absolute(child_height) => child_height,
                },
            )
            .sum();

        if total_children_height < dimensions.height {
            // All children fits in the current dimensions
            // don't bother to page up
            return true;
        }

        // Look for children index that correspond to a page up
        let mut remaining_height = dimensions.height;

        let range = if let Some(bottom_visible_child) = self.bottom_visible_child() {
            self.children.range(..=bottom_visible_child)
        } else {
            self.children.range(..)
        };

        for (i, LayoutChild { child, .. }) in range.rev().enumerate() {
            let child_height = match child.measure(&measure_specs).height {
                RequestedDimension::ExpandMax => dimensions.height,
                RequestedDimension::Absolute(child_height) => child_height,
            };
            if child_height > remaining_height {
                self.bottom_visible_child_index -= i;
                break;
            }
            remaining_height -= child_height;
        }

        log::debug!("View at: {}", self.bottom_visible_child_index);
        true
    }

    /// PageDown the window, return true if bottom is reached
    pub fn page_down(&mut self) -> bool {
        log::debug!("Page down");
        let dimensions = self.dimensions.as_ref().expect(MISSING_DIMENSIONS);
        let measure_specs = MeasureSpecs::from(dimensions);

        let total_children_height: u16 = self
            .children
            .iter()
            .map(
                |LayoutChild { child, .. }| match child.measure(&measure_specs).height {
                    RequestedDimension::ExpandMax => dimensions.height,
                    RequestedDimension::Absolute(child_height) => {
                        log::debug!("Child height: {}", child_height);
                        child_height
                    }
                },
            )
            .sum();

        if total_children_height < dimensions.height {
            // All children fits in the current dimensions
            // don't bother to page down
            return true;
        }

        // Look for children index that correspond to a page down

        let mut remaining_height = dimensions.height;
        let range = if let Some(bottom_visible_child) = self.bottom_visible_child() {
            self.children.range(bottom_visible_child..)
        } else {
            self.children.range(..)
        };
        match range
            .enumerate()
            .find_map(|(i, LayoutChild { child, .. })| {
                let child_height = match child.measure(&measure_specs).height {
                    RequestedDimension::ExpandMax => dimensions.height,
                    RequestedDimension::Absolute(child_height) => child_height,
                };
                if child_height > remaining_height {
                    Some(i)
                } else {
                    remaining_height -= child_height;
                    None
                }
            }) {
            // Stopped before last children
            Some(i) => self.bottom_visible_child_index += i,
            // Reach bottom
            None => self.bottom_visible_child_index = self.children.len() - 1,
        }

        log::debug!("View at: {}", self.bottom_visible_child_index);
        true
    }

    fn bottom_visible_child(&self) -> Option<&LayoutChild<I>> {
        if self.children.is_empty() {
            None
        } else {
            Some(
                self.children
                    .iter()
                    .nth(self.bottom_visible_child_index)
                    .expect(INVALID_VIEW),
            )
        }
    }

    /// List visible children starting from bottom
    fn visible_children<'a, 'b>(
        &'a self,
        dimensions: &'b Dimensions,
    ) -> impl Iterator<Item = &'a LayoutChild<I>>
    where
        'b: 'a,
    {
        let measure_specs = MeasureSpecs::from(dimensions);
        let mut remaining_height = dimensions.height;
        let range = if let Some(bottom_visible_child) = self.bottom_visible_child() {
            self.children.range(..=bottom_visible_child)
        } else {
            self.children.range(..)
        };

        range.rev().take_while(move |LayoutChild { child, .. }| {
            if remaining_height > 0 {
                let child_height = match child.measure(&measure_specs).height {
                    RequestedDimension::ExpandMax => dimensions.height,
                    RequestedDimension::Absolute(child_height) => child_height,
                };
                remaining_height -= std::cmp::min(remaining_height, child_height);
                true
            } else {
                false
            }
        })
    }

    fn layout_from_bottom(&mut self, dimensions: &Dimensions) {
        // Start layout at bottom of the view
        let mut child_top = dimensions.top + dimensions.height;
        let mut measure_specs: MeasureSpecs = dimensions.into();

        let visible_children_count = self.visible_children(dimensions).count();

        // Empty the BTreeSet so we can mutate children
        let mut children: Vec<_> = std::mem::take(&mut self.children).into_iter().collect();

        // Layout only visible children
        let last_visible_child_index = self.bottom_visible_child_index;
        // If we have only 1 visible children, first is 0 and last is also 0
        let first_visible_child_index = last_visible_child_index + 1 - visible_children_count;
        for layout_child in children[first_visible_child_index..=last_visible_child_index]
            .iter_mut()
            .rev()
        {
            let requested_dimensions = layout_child.child.measure(&measure_specs);
            let mut child_dimensions = Dimensions::reconcile(
                &measure_specs,
                &requested_dimensions,
                child_top,
                dimensions.left,
            );

            // Force full width
            child_dimensions.width = dimensions.width;

            // Fix top
            child_top -= child_dimensions.height;
            child_dimensions.top = child_top;

            // Update measure_specs
            let MeasureSpec::AtMost(measure_spec_height) = measure_specs.height else {
                unreachable!()
            };
            measure_specs.height =
                MeasureSpec::AtMost(measure_spec_height - child_dimensions.height);

            layout_child.dimensions = Some(child_dimensions);

            // Finally layout child with correct dimensions
            layout_child
                .child
                .layout(layout_child.dimensions.as_ref().unwrap());
        }

        // Insert back all children in the BTreeSet
        for child in children.into_iter() {
            self.children.insert(child);
        }
    }

    fn layout_from_top(&mut self, dimensions: &Dimensions) {
        // Start layout at bottom of the view
        let mut child_top = dimensions.top;
        let measure_specs: MeasureSpecs = dimensions.into();

        let visible_children_count = self.visible_children(dimensions).count();

        // Empty the BTreeSet so we can mutate children
        let mut children: Vec<_> = std::mem::take(&mut self.children).into_iter().collect();

        // Layout only visible children
        let last_visible_child_index = self.bottom_visible_child_index;
        let first_visible_child_index = last_visible_child_index + 1 - visible_children_count;
        for layout_child in
            children[first_visible_child_index..=last_visible_child_index].iter_mut()
        {
            let requested_dimensions = layout_child.child.measure(&measure_specs);
            let mut child_dimensions = Dimensions::reconcile(
                &measure_specs,
                &requested_dimensions,
                child_top,
                dimensions.left,
            );

            // Force full width
            child_dimensions.width = dimensions.width;

            child_top += child_dimensions.height;

            layout_child.dimensions = Some(child_dimensions);

            // Finally layout child with correct dimensions
            layout_child
                .child
                .layout(layout_child.dimensions.as_ref().unwrap());
        }

        // Insert back all children in the BTreeSet
        for child in children.into_iter() {
            self.children.insert(child);
        }
    }
}

impl<E, I> View<E> for ScrollWin<E, I>
where
    I: View<E> + Hash + Eq + Ord,
{
    fn measure(&self, measure_specs: &MeasureSpecs) -> RequestedDimensions {
        // Should we measure only visible children?
        // Measure max width of each children
        let (widths, heights): (Vec<RequestedDimension>, Vec<RequestedDimension>) = self
            .children
            .iter()
            .map(|LayoutChild { child, .. }| {
                let child_measure_spec = measure_specs.clone();
                let requested_dimensions = child.measure(&child_measure_spec);
                (requested_dimensions.width, requested_dimensions.height)
            })
            .unzip();

        let max_child_width = widths
            .into_iter()
            .max()
            .unwrap_or(RequestedDimension::Absolute(0));
        let total_children_height = heights.into_iter().sum();

        let requested_width = match self.layouts.width {
            LayoutParam::MatchParent => RequestedDimension::ExpandMax,
            LayoutParam::WrapContent => max_child_width,
            LayoutParam::Absolute(absolute_width) => RequestedDimension::Absolute(absolute_width),
        };

        let requested_height = match self.layouts.height {
            LayoutParam::MatchParent => RequestedDimension::ExpandMax,
            LayoutParam::WrapContent => total_children_height,
            LayoutParam::Absolute(absolute_height) => RequestedDimension::Absolute(absolute_height),
        };

        RequestedDimensions {
            width: requested_width,   // We let parent reconcile
            height: requested_height, // We let parent reconcile
        }
    }

    fn layout(&mut self, dimensions: &Dimensions) {
        log::debug!("layout {} {:?}", std::any::type_name::<Self>(), dimensions);
        self.dimensions = Some(dimensions.clone());

        if self.children.is_empty() {
            // Don't bother
            return;
        }

        let measure_specs = MeasureSpecs::from(dimensions);

        let total_children_height: u16 = self
            .children
            .iter()
            .map(
                |LayoutChild { child, .. }| match child.measure(&measure_specs).height {
                    RequestedDimension::ExpandMax => dimensions.height,
                    RequestedDimension::Absolute(child_height) => child_height,
                },
            )
            .sum();

        if total_children_height < dimensions.height {
            self.layout_from_top(dimensions);
        } else {
            self.layout_from_bottom(dimensions);
        }
    }

    fn render(&self, frame: ScreenFrame) {
        log::debug!("rendering {}", std::any::type_name::<Self>(),);

        if self.children.is_empty() {
            // Don't bother
            return;
        }

        let ScreenFrame { offscreen, .. } = frame;
        for LayoutChild { child, dimensions } in
            self.visible_children(self.dimensions.as_ref().expect(MISSING_DIMENSIONS))
        {
            let frame = ScreenFrame::new(offscreen, dimensions.as_ref().expect(MISSING_DIMENSIONS));
            child.render(frame);
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
    use test_log::test;

    use super::*;

    #[derive(Debug, Clone, Default)]
    pub struct MockView {
        pub ord: usize,
        pub height: u16,
        pub dimensions: Option<Dimensions>,
    }

    impl PartialOrd for MockView {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            self.ord.partial_cmp(&other.ord)
        }
    }

    impl Ord for MockView {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.ord.cmp(&other.ord)
        }
    }

    impl PartialEq for MockView {
        fn eq(&self, other: &Self) -> bool {
            self.ord.eq(&other.ord)
        }
    }

    impl Eq for MockView {}

    impl Hash for MockView {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            self.ord.hash(state)
        }
    }

    impl<E> View<E> for MockView {
        fn measure(&self, _measure_specs: &MeasureSpecs) -> RequestedDimensions {
            RequestedDimensions {
                height: RequestedDimension::Absolute(self.height),
                width: RequestedDimension::ExpandMax,
            }
        }

        fn layout(&mut self, dimensions: &Dimensions) {
            self.dimensions.replace(dimensions.clone());
        }

        fn render<'a>(&self, _frame: ScreenFrame<'a>) {
            unreachable!()
        }

        fn event(&mut self, _event: &mut E) {
            unreachable!()
        }
    }

    #[test]
    fn test_visible_children() {
        // Given
        let mut scroll_win = ScrollWin::<(), MockView>::new();

        let first_view = MockView {
            ord: 0,
            height: 10,
            ..Default::default()
        };
        let second_view = MockView {
            ord: 1,
            height: 10,
            ..Default::default()
        };
        let third_view = MockView {
            ord: 2,
            height: 10,
            ..Default::default()
        };

        scroll_win.insert(first_view.clone());
        scroll_win.insert(second_view.clone());
        scroll_win.insert(third_view.clone());

        let dimensions = Dimensions {
            width: 100,
            height: 20,
            top: 1,
            left: 1,
        };

        // When
        scroll_win.layout(&dimensions);

        // Then
        let visible_children = scroll_win.visible_children(&dimensions).collect::<Vec<_>>();
        assert_eq!(visible_children, vec![&third_view, &second_view]);
    }

    #[test]
    fn test_visible_children_page_down() {
        // Given
        let mut scroll_win = ScrollWin::<(), MockView>::new();

        let first_view = MockView {
            ord: 0,
            height: 10,
            ..Default::default()
        };
        let second_view = MockView {
            ord: 1,
            height: 10,
            ..Default::default()
        };
        let third_view = MockView {
            ord: 2,
            height: 10,
            ..Default::default()
        };
        let fourth_view = MockView {
            ord: 3,
            height: 10,
            ..Default::default()
        };

        scroll_win.insert(first_view.clone());
        scroll_win.insert(second_view.clone());
        scroll_win.insert(third_view.clone());
        scroll_win.insert(fourth_view.clone());

        let dimensions = Dimensions {
            width: 100,
            height: 20,
            top: 1,
            left: 1,
        };
        scroll_win.layout(&dimensions);
        scroll_win.page_up();

        // When
        scroll_win.page_down();

        // Then
        let visible_children = scroll_win.visible_children(&dimensions).collect::<Vec<_>>();
        assert_eq!(visible_children, vec![&fourth_view, &third_view]);
    }

    #[test]
    fn test_visible_children_page_up() {
        // Given
        let mut scroll_win = ScrollWin::<(), MockView>::new();

        let first_view = MockView {
            ord: 0,
            height: 10,
            ..Default::default()
        };
        let second_view = MockView {
            ord: 1,
            height: 10,
            ..Default::default()
        };
        let third_view = MockView {
            ord: 2,
            height: 10,
            ..Default::default()
        };
        let fourth_view = MockView {
            ord: 3,
            height: 10,
            ..Default::default()
        };

        scroll_win.insert(first_view.clone());
        scroll_win.insert(second_view.clone());
        scroll_win.insert(third_view.clone());
        scroll_win.insert(fourth_view.clone());
        let dimensions = Dimensions {
            width: 100,
            height: 20,
            top: 1,
            left: 1,
        };
        scroll_win.layout(&dimensions);

        // When
        scroll_win.page_up();

        // Then
        let visible_children = scroll_win.visible_children(&dimensions).collect::<Vec<_>>();
        assert_eq!(visible_children, vec![&second_view, &first_view]);
    }

    #[test]
    fn test_visible_children_page_up_then_down_unaligned() {
        // Given
        let mut scroll_win = ScrollWin::<(), MockView>::new();

        let first_view = MockView {
            ord: 0,
            height: 10,
            ..Default::default()
        };
        let second_view = MockView {
            ord: 1,
            height: 10,
            ..Default::default()
        };
        let third_view = MockView {
            ord: 2,
            height: 10,
            ..Default::default()
        };
        let fourth_view = MockView {
            ord: 3,
            height: 10,
            ..Default::default()
        };
        let fifth_view = MockView {
            ord: 4,
            height: 10,
            ..Default::default()
        };

        scroll_win.insert(first_view.clone());
        scroll_win.insert(second_view.clone());
        scroll_win.insert(third_view.clone());
        scroll_win.insert(fourth_view.clone());

        let dimensions = Dimensions {
            width: 100,
            height: 20,
            top: 1,
            left: 1,
        };
        scroll_win.layout(&dimensions);

        // When
        scroll_win.page_up();

        // Then
        let visible_children = scroll_win.visible_children(&dimensions).collect::<Vec<_>>();
        assert_eq!(visible_children, vec![&second_view, &first_view]);

        // When
        scroll_win.insert(fifth_view.clone());
        scroll_win.page_down();

        // Then
        let visible_children = scroll_win.visible_children(&dimensions).collect::<Vec<_>>();
        assert_eq!(visible_children, vec![&fourth_view, &third_view]);

        // When
        scroll_win.page_down();

        // Then
        let visible_children = scroll_win.visible_children(&dimensions).collect::<Vec<_>>();
        assert_eq!(visible_children, vec![&fifth_view, &fourth_view]);
    }

    /// Ensure children are layout at the top of the view if they cannot fill it
    #[test]
    fn test_layout_children_at_top_if_they_dont_fill_it() {
        // Given
        let mut scroll_win = ScrollWin::<(), MockView>::new();

        let first_view = MockView {
            ord: 0,
            height: 10,
            ..Default::default()
        };

        scroll_win.insert(first_view.clone());

        // When
        let dimensions = Dimensions {
            width: 100,
            height: 20,
            top: 1,
            left: 1,
        };
        scroll_win.layout(&dimensions);

        // Then
        let visible_children = scroll_win.visible_children(&dimensions).collect::<Vec<_>>();
        assert_eq!(visible_children, vec![&first_view]);
        assert_eq!(
            visible_children[0].dimensions.as_ref().map(|d| d.top),
            Some(1)
        );
    }

    #[test]
    fn test_layout_children_respecting_order() {
        // Given
        let mut scroll_win = ScrollWin::<(), MockView>::new();

        let first_view = MockView {
            ord: 0,
            height: 10,
            ..Default::default()
        };
        let second_view = MockView {
            ord: 1,
            height: 10,
            ..Default::default()
        };
        let third_view = MockView {
            ord: 2,
            height: 10,
            ..Default::default()
        };

        scroll_win.insert(first_view.clone());
        scroll_win.insert(second_view.clone());
        scroll_win.insert(third_view.clone());

        // When
        let dimensions = Dimensions {
            width: 100,
            height: 20,
            top: 1,
            left: 1,
        };
        scroll_win.layout(&dimensions);

        // Then
        let visible_children = scroll_win.visible_children(&dimensions).collect::<Vec<_>>();
        assert_eq!(visible_children, vec![&third_view, &second_view]);
        assert_eq!(
            visible_children[0].dimensions.as_ref().map(|d| d.top),
            Some(11)
        );
        assert_eq!(
            visible_children[1].dimensions.as_ref().map(|d| d.top),
            Some(1)
        );
    }

    #[test]
    fn test_layout_without_children() {
        // Given
        let mut scroll_win = ScrollWin::<(), MockView>::new();

        // Then
        scroll_win.layout(&Dimensions {
            width: 100,
            height: 20,
            top: 1,
            left: 1,
        });
    }

    #[test]
    fn test_layout_partial_child() {
        // Given
        let mut scroll_win = ScrollWin::<(), MockView>::new();

        let first_view = MockView {
            ord: 0,
            height: 20,
            ..Default::default()
        };
        let second_view = MockView {
            ord: 1,
            height: 10,
            ..Default::default()
        };
        let third_view = MockView {
            ord: 2,
            height: 10,
            ..Default::default()
        };

        scroll_win.insert(first_view.clone());
        scroll_win.insert(second_view.clone());
        scroll_win.insert(third_view.clone());

        // When
        let dimensions = Dimensions {
            width: 100,
            height: 30,
            top: 1,
            left: 1,
        };
        scroll_win.layout(&dimensions);

        // Then
        let visible_children = scroll_win.visible_children(&dimensions).collect::<Vec<_>>();
        assert_eq!(
            visible_children,
            vec![&third_view, &second_view, &first_view]
        );
        assert_eq!(
            visible_children[0].dimensions.as_ref().map(|d| d.top),
            Some(21)
        );
        assert_eq!(
            visible_children[1].dimensions.as_ref().map(|d| d.top),
            Some(11)
        );
        assert_eq!(
            visible_children[2].dimensions.as_ref().map(|d| d.top),
            Some(1)
        );
        assert_eq!(
            visible_children[2].dimensions.as_ref().map(|d| d.height),
            Some(10)
        );
    }
}
