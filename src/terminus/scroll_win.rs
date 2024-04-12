/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::hash::Hash;
use std::io::Write;
use std::os::fd::AsFd;
use std::rc::Rc;

use super::{
    Dimensions, LayoutParam, LayoutParams, MeasureSpec, MeasureSpecs, RequestedDimension,
    RequestedDimensions, Screen, View,
};

const MISSING_DIMENSIONS: &'static str = "Missing dimensions";
const INVALID_VIEW: &'static str = "Invalid view detected";

/// Ordered vertical window giving ability to scroll
pub struct ScrollWin<E, W, I>
where
    W: Write + AsFd,
    I: View<E, W> + Hash + Eq + Ord,
{
    /// Index in children of last visible child (bottom child)
    bottom_visible_child_index: usize,
    event_handler: Option<Rc<RefCell<Box<dyn FnMut(&mut Self, &mut E)>>>>,
    dirty: Cell<bool>,
    children: BTreeSet<I>,
    dimensions: Option<Dimensions>,
    layouts: LayoutParams,
}

impl<E, W, I> ScrollWin<E, W, I>
where
    W: Write + AsFd,
    I: View<E, W> + Hash + Eq + Ord,
{
    pub fn new() -> Self {
        Self {
            children: BTreeSet::new(),
            bottom_visible_child_index: 0,
            event_handler: None,
            dirty: Cell::new(true),
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

    #[allow(unused)]
    pub fn with_layout(mut self, layouts: LayoutParams) -> Self {
        self.layouts = layouts;
        self
    }

    #[allow(dead_code)]
    pub fn first<'a>(&'a self) -> Option<&'a I> {
        self.children.iter().next()
    }

    pub fn insert(&mut self, item: I) {
        // If view index is on last child, then keep it there
        let stick_to_bottom = self.bottom_visible_child_index + 1 == self.children.len();
        if self.children.insert(item) && stick_to_bottom {
            self.bottom_visible_child_index += 1;
        }

        // We don't care about rendered buffer, we avoid computation here at cost of false positive
        // (set dirty while in fact it shouldn't)
        // XXX We should care
        self.dirty.set(true);
    }

    /// PageUp the window, return true if top is reached
    pub fn page_up(&mut self) -> bool {
        log::debug!("Page up");
        let dimensions = self.dimensions.as_ref().expect(MISSING_DIMENSIONS);
        let measure_specs = MeasureSpecs::from(dimensions);

        let total_children_height: u16 = self
            .children
            .iter()
            .map(|child| match child.measure(&measure_specs).height {
                RequestedDimension::ExpandMax => dimensions.height,
                RequestedDimension::Absolute(child_height) => child_height,
            })
            .sum();

        if total_children_height < dimensions.height {
            // All children fits in the current dimensions
            // don't bother to page up
            return true;
        }

        self.dirty.set(true);

        // Look for children index that correspond to a page up
        let mut remaining_height = dimensions.height;
        for (i, child) in self
            .children
            .range(..=self.bottom_visible_child())
            .rev()
            .enumerate()
        {
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
            .map(|child| match child.measure(&measure_specs).height {
                RequestedDimension::ExpandMax => dimensions.height,
                RequestedDimension::Absolute(child_height) => {
                    log::debug!("Child height: {}", child_height);
                    child_height
                }
            })
            .sum();

        if total_children_height < dimensions.height {
            // All children fits in the current dimensions
            // don't bother to page down
            return true;
        }

        self.dirty.set(true);

        // Look for children index that correspond to a page down

        let mut remaining_height = dimensions.height;
        match self
            .children
            .range(self.bottom_visible_child()..) // We want to keep last visible child on top
            .enumerate()
            .find_map(|(i, child)| {
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

    fn bottom_visible_child<'a>(&'a self) -> &'a I {
        self.children
            .iter()
            .nth(self.bottom_visible_child_index)
            .expect(INVALID_VIEW)
    }

    /// List visible children starting from bottom
    pub fn visible_children<'a>(&'a self) -> impl Iterator<Item = &'a I> {
        let dimensions = self.dimensions.as_ref().expect(MISSING_DIMENSIONS);
        let measure_specs = MeasureSpecs::from(dimensions);
        let mut remaining_height = dimensions.height;
        self.children
            .range(..=self.bottom_visible_child())
            .rev()
            .take_while(move |child| {
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

    fn layout_from_bottom(&mut self) {
        let dimensions = self.dimensions.as_ref().unwrap();

        // Start layout at bottom of the view
        let mut child_top = dimensions.top + dimensions.height;
        let mut measure_specs: MeasureSpecs = dimensions.into();

        let visible_children_count = self.visible_children().count();

        // Empty the BTreeSet so we can mutate children
        let mut children: Vec<_> = std::mem::replace(&mut self.children, BTreeSet::new())
            .into_iter()
            .collect();

        // Layout only visiable children
        let last_visible_child_index = self.bottom_visible_child_index;
        // If we have only 1 visible children, first is 0 and last is also 0
        let first_visible_child_index = last_visible_child_index + 1 - visible_children_count;
        for child in children[first_visible_child_index..=last_visible_child_index]
            .iter_mut()
            .rev()
        {
            let requested_dimensions = child.measure(&measure_specs);
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

            // Finally layout child with correct dimensions
            child.layout(&child_dimensions);
        }

        // Insert back all children in the BTreeSet
        for child in children.into_iter() {
            self.children.insert(child);
        }
    }

    fn layout_from_top(&mut self) {
        let dimensions = self.dimensions.as_ref().unwrap();

        // Start layout at bottom of the view
        let mut child_top = dimensions.top;
        let measure_specs: MeasureSpecs = dimensions.into();

        let visible_children_count = self.visible_children().count();

        // Empty the BTreeSet so we can mutate children
        let mut children: Vec<_> = std::mem::replace(&mut self.children, BTreeSet::new())
            .into_iter()
            .collect();

        // Layout only visiable children
        let last_visible_child_index = self.bottom_visible_child_index;
        let first_visible_child_index = last_visible_child_index + 1 - visible_children_count;
        for child in children[first_visible_child_index..=last_visible_child_index].iter_mut() {
            let requested_dimensions = child.measure(&measure_specs);
            let mut child_dimensions = Dimensions::reconcile(
                &measure_specs,
                &requested_dimensions,
                child_top,
                dimensions.left,
            );

            // Force full width
            child_dimensions.width = dimensions.width;

            // Finally layout child with correct dimensions
            child.layout(&child_dimensions);

            child_top += child_dimensions.height;
        }

        // Insert back all children in the BTreeSet
        for child in children.into_iter() {
            self.children.insert(child);
        }
    }
}

impl<E, W, I> View<E, W> for ScrollWin<E, W, I>
where
    W: Write + AsFd,
    I: View<E, W> + Hash + Eq + Ord,
{
    fn measure(&self, measure_specs: &MeasureSpecs) -> RequestedDimensions {
        // Should we measure only visible children?
        // Mesure max width of each children
        let (widths, heights): (Vec<RequestedDimension>, Vec<RequestedDimension>) = self
            .children
            .iter()
            .map(|child| {
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

        self.dimensions.replace(dimensions.clone());

        if self.children.is_empty() {
            // Don't bother
            return;
        }

        let measure_specs = MeasureSpecs::from(dimensions);

        let total_children_height: u16 = self
            .children
            .iter()
            .map(|child| match child.measure(&measure_specs).height {
                RequestedDimension::ExpandMax => dimensions.height,
                RequestedDimension::Absolute(child_height) => child_height,
            })
            .sum();

        if total_children_height < dimensions.height {
            self.layout_from_top();
        } else {
            self.layout_from_bottom();
        }
    }

    fn render(&self, screen: &mut Screen<W>) {
        log::debug!(
            "rendering {} at {:?}",
            std::any::type_name::<Self>(),
            self.dimensions
        );

        if self.children.is_empty() {
            // Don't bother
            return;
        }

        super::save_cursor!(screen);

        for child in self.visible_children() {
            // child dimension can be unwrapped since it must have been set during the measure
            // phase
            child.render(screen);
        }

        super::restore_cursor!(screen);

        self.dirty.set(false);
    }

    fn event(&mut self, event: &mut E) {
        if let Some(handler) = &self.event_handler {
            let handler = Rc::clone(handler);
            let handler = &mut *handler.borrow_mut();
            handler(self, event);
        }
    }

    fn is_layout_dirty(&self) -> bool {
        (matches!(self.layouts.width, LayoutParam::WrapContent)
            || matches!(self.layouts.height, LayoutParam::WrapContent))
            && self.is_dirty()
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use test_log::test;

    use super::*;

    #[derive(Debug, Clone, Default)]
    struct MockView {
        ord: usize,
        height: u16,
        dimensions: Option<Dimensions>,
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

    impl<E, W> View<E, W> for MockView
    where
        W: std::io::Write + std::os::fd::AsFd,
    {
        fn measure(&self, _measure_specs: &MeasureSpecs) -> RequestedDimensions {
            RequestedDimensions {
                height: RequestedDimension::Absolute(self.height),
                width: RequestedDimension::ExpandMax,
            }
        }

        fn layout(&mut self, dimensions: &Dimensions) {
            self.dimensions.replace(dimensions.clone());
        }

        fn render(&self, _screen: &mut Screen<W>) {
            unreachable!()
        }

        fn is_layout_dirty(&self) -> bool {
            unreachable!()
        }

        fn is_dirty(&self) -> bool {
            unreachable!()
        }

        fn event(&mut self, _event: &mut E) {
            unreachable!()
        }
    }

    #[test]
    fn test_visible_children() {
        // Given
        let mut scroll_win = ScrollWin::<(), File, MockView>::new();

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
        scroll_win.layout(&Dimensions {
            width: 100,
            height: 20,
            top: 1,
            left: 1,
        });

        // Then
        let visible_children = scroll_win.visible_children().collect::<Vec<_>>();
        assert_eq!(visible_children, vec![&third_view, &second_view]);
    }

    #[test]
    fn test_visible_children_page_down() {
        // Given
        let mut scroll_win = ScrollWin::<(), File, MockView>::new();

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

        scroll_win.layout(&Dimensions {
            width: 100,
            height: 20,
            top: 1,
            left: 1,
        });
        scroll_win.page_up();

        // When
        scroll_win.page_down();

        // Then
        let visible_children = scroll_win.visible_children().collect::<Vec<_>>();
        assert_eq!(visible_children, vec![&fourth_view, &third_view]);
    }

    #[test]
    fn test_visible_children_page_up() {
        // Given
        let mut scroll_win = ScrollWin::<(), File, MockView>::new();

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
        scroll_win.layout(&Dimensions {
            width: 100,
            height: 20,
            top: 1,
            left: 1,
        });

        // When
        scroll_win.page_up();

        // Then
        let visible_children = scroll_win.visible_children().collect::<Vec<_>>();
        assert_eq!(visible_children, vec![&second_view, &first_view]);
    }

    #[test]
    fn test_visible_children_page_up_then_down_unaligned() {
        // Given
        let mut scroll_win = ScrollWin::<(), File, MockView>::new();

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

        scroll_win.layout(&Dimensions {
            width: 100,
            height: 20,
            top: 1,
            left: 1,
        });

        // When
        scroll_win.page_up();

        // Then
        let visible_children = scroll_win.visible_children().collect::<Vec<_>>();
        assert_eq!(visible_children, vec![&second_view, &first_view]);

        // When
        scroll_win.insert(fifth_view.clone());
        scroll_win.page_down();

        // Then
        let visible_children = scroll_win.visible_children().collect::<Vec<_>>();
        assert_eq!(visible_children, vec![&fourth_view, &third_view]);

        // When
        scroll_win.page_down();

        // Then
        let visible_children = scroll_win.visible_children().collect::<Vec<_>>();
        assert_eq!(visible_children, vec![&fifth_view, &fourth_view]);
    }

    /// Ensure children are layout at the top of the view if they cannot fill it
    #[test]
    fn test_layout_children_at_top_if_they_dont_fill_it() {
        // Given
        let mut scroll_win = ScrollWin::<(), File, MockView>::new();

        let first_view = MockView {
            ord: 0,
            height: 10,
            ..Default::default()
        };

        scroll_win.insert(first_view.clone());

        // When
        scroll_win.layout(&Dimensions {
            width: 100,
            height: 20,
            top: 1,
            left: 1,
        });

        // Then
        let visible_children = scroll_win.visible_children().collect::<Vec<_>>();
        assert_eq!(visible_children, vec![&first_view]);
        assert_eq!(
            visible_children[0].dimensions.as_ref().map(|d| d.top),
            Some(1)
        );
    }

    #[test]
    fn test_layout_children_respecting_order() {
        // Given
        let mut scroll_win = ScrollWin::<(), File, MockView>::new();

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
        scroll_win.layout(&Dimensions {
            width: 100,
            height: 20,
            top: 1,
            left: 1,
        });

        // Then
        let visible_children = scroll_win.visible_children().collect::<Vec<_>>();
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
        let mut scroll_win = ScrollWin::<(), File, MockView>::new();

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
        let mut scroll_win = ScrollWin::<(), File, MockView>::new();

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
        scroll_win.layout(&Dimensions {
            width: 100,
            height: 30,
            top: 1,
            left: 1,
        });

        // Then
        let visible_children = scroll_win.visible_children().collect::<Vec<_>>();
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
