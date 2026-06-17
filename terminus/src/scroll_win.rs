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

pub trait Searchable {
    fn matches(&self, query: &str) -> bool;
}

#[derive(Debug, Clone)]
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
pub struct ScrollWin<E, I, C = ()>
where
    I: View<E, C> + Hash + Eq + Ord,
{
    /// Index in children of last visible child (bottom child)
    bottom_visible_child_index: usize,
    /// Index in children of first visible child (top child); updated after layout
    first_visible_child_index: usize,
    /// Index of the currently selected child, if any
    selected_child_index: Option<usize>,
    /// Background colour applied to the selected child during render via `View::select()`.
    /// When `None` the selection visual is suppressed (useful for views where selection
    /// is managed externally or not needed).
    selection_bg: Option<super::BgColor>,
    event_handler: Option<EventHandler<Self, E>>,
    focus_change_handler: Option<Box<dyn FnMut(&mut Self, bool)>>,
    children: BTreeSet<LayoutChild<I>>,
    layouts: LayoutParams,
    dimensions: Option<Dimensions>,
    search_query: Option<String>,
    remember_position: bool,
    remembered_position: Option<(usize, Option<usize>)>,
}

impl<E, I, C> Default for ScrollWin<E, I, C>
where
    I: View<E, C> + Hash + Eq + Ord,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<E, I, C> ScrollWin<E, I, C>
where
    I: View<E, C> + Hash + Eq + Ord,
{
    #[must_use]
    pub fn new() -> Self {
        Self {
            children: BTreeSet::new(),
            bottom_visible_child_index: 0,
            first_visible_child_index: 0,
            selected_child_index: None,
            selection_bg: None,
            event_handler: None,
            focus_change_handler: None,
            layouts: LayoutParams {
                width: LayoutParam::MatchParent,
                height: LayoutParam::MatchParent,
            },
            dimensions: None,
            search_query: None,
            remember_position: false,
            remembered_position: None,
        }
    }

    /// Set the background colour used to highlight the selected child during render.
    /// Must be called for selection highlighting and cursor positioning to work.
    #[must_use]
    pub fn with_selection_bg(mut self, bg: super::BgColor) -> Self {
        self.selection_bg = Some(bg);
        self
    }

    #[must_use]
    pub fn with_event<F>(mut self, event_handler: F) -> Self
    where
        F: FnMut(&mut Self, &mut E) + 'static,
    {
        self.event_handler = Some(Rc::new(RefCell::new(Box::new(event_handler))));
        self
    }

    #[must_use]
    pub fn with_focus_change<F>(mut self, handler: F) -> Self
    where
        F: FnMut(&mut Self, bool) + 'static,
    {
        self.focus_change_handler = Some(Box::new(handler));
        self
    }

    #[must_use]
    pub fn with_position_memory(mut self) -> Self {
        self.remember_position = true;
        self
    }

    #[must_use]
    pub fn with_layout(mut self, layouts: LayoutParams) -> Self {
        self.layouts = layouts;
        self
    }

    #[must_use]
    pub fn first(&self) -> Option<&I> {
        self.children
            .iter()
            .next()
            .map(|LayoutChild { child, .. }| child)
    }

    /// Returns a reference to the child at the given sorted index (O(n)).
    #[must_use]
    pub fn child_at(&self, index: usize) -> Option<&I> {
        self.children
            .iter()
            .nth(index)
            .map(|LayoutChild { child, .. }| child)
    }

    pub fn children_iter(&self) -> impl Iterator<Item = &I> {
        self.children.iter().map(|LayoutChild { child, .. }| child)
    }

    pub fn selected(&self) -> Option<&I> {
        self.selected_child_index.and_then(|i| self.child_at(i))
    }

    /// Apply a closure to a mutable reference of the currently selected
    /// child. Uses a take-then-reinsert pattern because children are stored
    /// in a `BTreeSet`. The closure MUST NOT change fields that affect the
    /// child's `Ord` (since the reinsert position depends on that key).
    pub fn update_selected<F, R>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut I) -> R,
        I: Clone,
    {
        let idx = self.selected_child_index?;
        let key = self.children.iter().nth(idx)?.clone();
        let mut lc = self.children.take(&key)?;
        let result = f(&mut lc.child);
        self.children.insert(lc);
        Some(result)
    }

    pub fn predecessor(&self, item: &I) -> Option<&I> {
        let mut prev: Option<&I> = None;
        for LayoutChild { child, .. } in &self.children {
            if child >= item {
                break;
            }
            prev = Some(child);
        }
        prev
    }

    pub fn successor(&self, item: &I) -> Option<&I> {
        self.children
            .iter()
            .map(|LayoutChild { child, .. }| child)
            .find(|child| *child > item)
    }

    #[must_use]
    pub fn current_search(&self) -> Option<&str> {
        self.search_query.as_deref()
    }

    /// Moves selection to the previous (older) child.
    /// Returns `(old_index, new_index, at_top)`.
    pub fn select_prev(&mut self) -> (Option<usize>, Option<usize>, bool) {
        let old = self.selected_child_index;
        let new = match old {
            None => self.bottom_visible_child_index,
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.selected_child_index = Some(new);
        if new < self.first_visible_child_index && self.bottom_visible_child_index > 0 {
            self.bottom_visible_child_index -= 1;
        }
        (old, Some(new), new == 0)
    }

    /// Moves selection to the next (newer) child.
    /// Returns `(old_index, new_index)`.
    pub fn select_next(&mut self) -> (Option<usize>, Option<usize>) {
        let last = self.children.len().saturating_sub(1);
        let old = self.selected_child_index;
        let new = match old {
            None => self.bottom_visible_child_index,
            Some(i) => (i + 1).min(last),
        };
        self.selected_child_index = Some(new);
        if new > self.bottom_visible_child_index {
            self.bottom_visible_child_index = new;
        }
        (old, Some(new))
    }

    /// Clears the selection and returns the previously selected index.
    pub fn clear_selection(&mut self) -> Option<usize> {
        self.selected_child_index.take()
    }

    #[must_use]
    pub fn has_selection(&self) -> bool {
        self.selected_child_index.is_some()
    }

    /// Selects the last visible child without changing the scroll position.
    /// Returns `(old_index, new_index)`.
    pub fn select_last_visible(&mut self) -> (Option<usize>, Option<usize>) {
        let old = self.selected_child_index;
        if self.children.is_empty() {
            self.selected_child_index = None;
            (old, None)
        } else {
            let new = self.bottom_visible_child_index;
            self.selected_child_index = Some(new);
            (old, Some(new))
        }
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

    /// Remove the item that compares equal to `key`.
    /// Adjusts scroll and selection indices. No-op if the key is not found.
    pub fn remove_by_key(&mut self, key: &I)
    where
        I: Clone,
    {
        let probe = LayoutChild {
            child: key.clone(),
            dimensions: None,
        };
        let Some(removed_lc) = self.children.take(&probe) else {
            return;
        };
        // Determine the index of the removed element before removal.
        let removed_index = self
            .children
            .iter()
            .position(|lc| lc >= &removed_lc)
            .unwrap_or(self.children.len());

        // Adjust bottom_visible_child_index.
        if self.children.is_empty() {
            self.bottom_visible_child_index = 0;
        } else if removed_index <= self.bottom_visible_child_index
            && self.bottom_visible_child_index > 0
        {
            self.bottom_visible_child_index -= 1;
        }

        // Adjust or clear the selection.
        match self.selected_child_index {
            Some(sel) if sel == removed_index => {
                self.selected_child_index = None;
            }
            Some(sel) if sel > removed_index => {
                self.selected_child_index = Some(sel - 1);
            }
            _ => {}
        }
    }

    /// Insert or replace an item. If an element with the same Ord key already
    /// exists it is removed first so the new element (which may carry updated
    /// content, e.g. reactions or a correction) replaces it.
    pub fn replace(&mut self, item: I) {
        let new_lc = LayoutChild {
            child: item,
            dimensions: None,
        };
        let stick_to_bottom = self.bottom_visible_child_index + 1 == self.children.len();
        // Remove the old element if present (same Ord key = same timestamp+id).
        let was_present = self.children.remove(&new_lc);
        let inserted = self.children.insert(new_lc);
        // Only bump the scroll index when this is a genuinely new element.
        if inserted && !was_present && stick_to_bottom {
            self.bottom_visible_child_index += 1;
        }
    }

    /// `PageUp` the window.
    /// Returns `(at_top, old_selected, new_selected)`.
    /// If a message was selected and scrolled off the bottom of the new viewport,
    /// the selection is clamped to the last visible message.
    ///
    /// # Panics
    ///
    /// Panics if the dimensions have not been set yet.
    pub fn page_up(&mut self) -> (bool, Option<usize>, Option<usize>) {
        log::debug!("Page up");
        let dimensions = self.dimensions.as_ref().expect(MISSING_DIMENSIONS);
        let measure_specs = MeasureSpecs::from(dimensions);

        let total_children_height: u32 = self
            .children
            .iter()
            .map(
                |LayoutChild { child, .. }| match child.measure(&measure_specs).height {
                    RequestedDimension::ExpandMax => u32::from(dimensions.height),
                    RequestedDimension::Absolute(child_height) => u32::from(child_height),
                },
            )
            .sum();

        if total_children_height < u32::from(dimensions.height) {
            // All children fits in the current dimensions
            // don't bother to page up
            let sel = self.selected_child_index;
            return (true, sel, sel);
        }

        // Look for children index that correspond to a page up
        let mut remaining_height = dimensions.height;

        let range = if let Some(bottom_visible_child) = self.bottom_visible_child() {
            self.children.range(..=bottom_visible_child)
        } else {
            self.children.range(..)
        };

        let old_selected = self.selected_child_index;
        let initial_index = self.bottom_visible_child_index;
        for (i, LayoutChild { child, .. }) in range.rev().enumerate() {
            let child_height = match child.measure(&measure_specs).height {
                RequestedDimension::ExpandMax => dimensions.height,
                RequestedDimension::Absolute(child_height) => child_height,
            };
            if child_height > remaining_height {
                // If i==0, the current child is taller than the screen: move at least 1
                let step = if i == 0 { 1 } else { i };
                self.bottom_visible_child_index =
                    self.bottom_visible_child_index.saturating_sub(step);
                break;
            }
            remaining_height -= child_height;
        }

        if let Some(sel) = self.selected_child_index {
            if sel > self.bottom_visible_child_index {
                self.selected_child_index = Some(self.bottom_visible_child_index);
            }
        }

        log::debug!("View at: {}", self.bottom_visible_child_index);
        let at_top = self.bottom_visible_child_index == 0
            || self.bottom_visible_child_index == initial_index;
        (at_top, old_selected, self.selected_child_index)
    }

    /// Scroll to the very top (first messages visible).
    /// Returns `(old_selected, new_selected)` indices.
    pub fn scroll_to_top(&mut self) -> (Option<usize>, Option<usize>) {
        let old = self.selected_child_index;
        if self.children.is_empty() {
            self.selected_child_index = None;
            return (old, None);
        }
        let dimensions = if let Some(d) = &self.dimensions {
            d.clone()
        } else {
            self.bottom_visible_child_index = 0;
            self.selected_child_index = Some(0);
            return (old, Some(0));
        };
        let measure_specs = MeasureSpecs::from(&dimensions);
        let mut accumulated: u32 = 0;
        let mut bottom = 0usize;

        for (i, LayoutChild { child, .. }) in self.children.iter().enumerate() {
            let h = match child.measure(&measure_specs).height {
                RequestedDimension::ExpandMax => u32::from(dimensions.height),
                RequestedDimension::Absolute(h) => u32::from(h),
            };
            accumulated += h;
            bottom = i;
            if accumulated >= u32::from(dimensions.height) {
                break;
            }
        }

        self.bottom_visible_child_index = bottom;
        self.selected_child_index = Some(0);
        (old, Some(0))
    }

    /// Scroll to the very bottom (last messages visible).
    /// Returns `(old_selected, new_selected)` indices.
    pub fn scroll_to_bottom(&mut self) -> (Option<usize>, Option<usize>) {
        let old = self.selected_child_index;
        if self.children.is_empty() {
            self.selected_child_index = None;
            (old, None)
        } else {
            let last = self.children.len() - 1;
            self.bottom_visible_child_index = last;
            self.selected_child_index = Some(last);
            (old, Some(last))
        }
    }

    /// `PageDown` the window.
    /// Returns `(old_selected, new_selected)`.
    /// If a message was selected and scrolled off the top of the new viewport,
    /// the selection is clamped to the first visible message.
    ///
    /// # Panics
    ///
    /// Panics if the dimensions have not been set yet.
    pub fn page_down(&mut self) -> (Option<usize>, Option<usize>) {
        log::debug!("Page down");
        let dimensions = self.dimensions.as_ref().expect(MISSING_DIMENSIONS);
        let measure_specs = MeasureSpecs::from(dimensions);

        let total_children_height: u32 = self
            .children
            .iter()
            .map(
                |LayoutChild { child, .. }| match child.measure(&measure_specs).height {
                    RequestedDimension::ExpandMax => u32::from(dimensions.height),
                    RequestedDimension::Absolute(child_height) => {
                        log::debug!("Child height: {}", child_height);
                        u32::from(child_height)
                    }
                },
            )
            .sum();

        let old_selected = self.selected_child_index;

        if total_children_height < u32::from(dimensions.height) {
            // All children fits in the current dimensions
            // don't bother to page down
            return (old_selected, old_selected);
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

        if let Some(sel) = self.selected_child_index {
            let new_first = self.compute_first_visible();
            if sel < new_first {
                self.selected_child_index = Some(new_first);
            }
        }

        log::debug!("View at: {}", self.bottom_visible_child_index);
        (old_selected, self.selected_child_index)
    }

    fn bottom_visible_child(&self) -> Option<&LayoutChild<I>> {
        if self.children.is_empty() {
            None
        } else {
            let index = self.bottom_visible_child_index.min(self.children.len() - 1);
            self.children.iter().nth(index)
        }
    }

    /// Compute the index of the first (topmost) visible child for the current
    /// `bottom_visible_child_index`, using live `measure()` calls.
    fn compute_first_visible(&self) -> usize {
        let Some(dimensions) = &self.dimensions else {
            return 0;
        };
        let measure_specs = MeasureSpecs::from(dimensions);
        let Some(bottom_child) = self.bottom_visible_child() else {
            return 0;
        };
        let mut remaining = dimensions.height;
        let mut count = 0usize;
        for LayoutChild { child, .. } in self.children.range(..=bottom_child).rev() {
            let h = match child.measure(&measure_specs).height {
                RequestedDimension::ExpandMax => dimensions.height,
                RequestedDimension::Absolute(h) => h,
            };
            if h > remaining {
                break;
            }
            remaining -= h;
            count += 1;
        }
        self.bottom_visible_child_index + 1 - count.max(1)
    }

    /// Count visible children from bottom using pre-measured heights (no `measure()` calls).
    fn visible_children_count(&self, dimensions: &Dimensions, heights: &[u16]) -> usize {
        let mut remaining_height = dimensions.height;
        let bottom = self.bottom_visible_child_index;
        let range_end = std::cmp::min(bottom + 1, heights.len());

        heights[..range_end]
            .iter()
            .rev()
            .take_while(move |&&child_height| {
                if remaining_height > 0 {
                    remaining_height -= std::cmp::min(remaining_height, child_height);
                    true
                } else {
                    false
                }
            })
            .count()
    }

    #[cfg(test)]
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

    fn layout_from_bottom(&mut self, dimensions: &Dimensions, heights: &[u16]) {
        // Start layout at bottom of the view
        let mut child_top = dimensions.top + dimensions.height;
        let mut measure_specs: MeasureSpecs = dimensions.into();

        let visible_children_count = self.visible_children_count(dimensions, heights);

        // Empty the BTreeSet so we can mutate children
        let mut children: Vec<_> = std::mem::take(&mut self.children).into_iter().collect();

        // Layout only visible children
        let last_visible_child_index = self.bottom_visible_child_index;
        // If we have only 1 visible children, first is 0 and last is also 0
        let first_visible_child_index = last_visible_child_index + 1 - visible_children_count;
        self.first_visible_child_index = first_visible_child_index;
        for (layout_child, &child_height) in children
            [first_visible_child_index..=last_visible_child_index]
            .iter_mut()
            .zip(heights[first_visible_child_index..=last_visible_child_index].iter())
            .rev()
        {
            // Clamp to remaining space (topmost visible child may be partial)
            let MeasureSpec::AtMost(measure_spec_height) = measure_specs.height else {
                unreachable!()
            };
            let actual_height = std::cmp::min(child_height, measure_spec_height);

            let mut child_dimensions = Dimensions {
                top: child_top,
                left: dimensions.left,
                width: dimensions.width,
                height: actual_height,
            };

            // Fix top
            child_top -= actual_height;
            child_dimensions.top = child_top;

            // Update measure_specs
            measure_specs.height = MeasureSpec::AtMost(measure_spec_height - actual_height);

            layout_child.dimensions = Some(child_dimensions);

            // Finally layout child with correct dimensions
            layout_child
                .child
                .layout(layout_child.dimensions.as_ref().unwrap());
        }

        // Insert back all children in the BTreeSet
        for child in children {
            self.children.insert(child);
        }
    }

    fn layout_from_top(&mut self, dimensions: &Dimensions, heights: &[u16]) {
        let mut child_top = dimensions.top;

        // Count forward from index 0: how many children fit starting at the top.
        // (visible_children_count counts backward from bottom_visible_child_index,
        // which breaks when bottom_visible_child_index is 0 or misplaced near the top.)
        let mut remaining = dimensions.height;
        let mut count = 0usize;
        for &h in heights {
            if h > remaining {
                break;
            }
            remaining -= h;
            count += 1;
        }
        let count = count.max(1);

        let first_visible_child_index = 0;
        let last_visible_child_index = count - 1;
        self.first_visible_child_index = first_visible_child_index;
        self.bottom_visible_child_index = last_visible_child_index;

        // Empty the BTreeSet so we can mutate children
        let mut children: Vec<_> = std::mem::take(&mut self.children).into_iter().collect();

        for (layout_child, &child_height) in children
            [first_visible_child_index..=last_visible_child_index]
            .iter_mut()
            .zip(heights[first_visible_child_index..=last_visible_child_index].iter())
        {
            let child_dimensions = Dimensions {
                top: child_top,
                left: dimensions.left,
                width: dimensions.width,
                height: child_height,
            };

            child_top += child_height;

            layout_child.dimensions = Some(child_dimensions);

            // Finally layout child with correct dimensions
            layout_child
                .child
                .layout(layout_child.dimensions.as_ref().unwrap());
        }

        // Insert back all children in the BTreeSet
        for child in children {
            self.children.insert(child);
        }
    }
}

impl<E, I, C> View<E, C> for ScrollWin<E, I, C>
where
    I: View<E, C> + Hash + Eq + Ord,
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

        // Measure all children once; reuse heights in layout helpers
        let heights: Vec<u16> = self
            .children
            .iter()
            .map(
                |LayoutChild { child, .. }| match child.measure(&measure_specs).height {
                    RequestedDimension::ExpandMax => dimensions.height,
                    RequestedDimension::Absolute(h) => h,
                },
            )
            .collect();

        let total_children_height: u32 = heights.iter().map(|&h| u32::from(h)).sum();

        let height_to_bottom: u32 = heights[..=self.bottom_visible_child_index]
            .iter()
            .map(|&h| u32::from(h))
            .sum();

        if total_children_height < u32::from(dimensions.height)
            || height_to_bottom < u32::from(dimensions.height)
        {
            self.layout_from_top(dimensions, &heights);
        } else {
            self.layout_from_bottom(dimensions, &heights);
        }
    }

    fn render(&self, frame: ScreenFrame, config: &C) {
        log::debug!("rendering {}", std::any::type_name::<Self>(),);

        if self.children.is_empty() {
            return;
        }

        let ScreenFrame {
            offscreen,
            cursor_granted,
            ..
        } = frame;
        for (idx, LayoutChild { child, dimensions }) in self.children.iter().enumerate() {
            if idx < self.first_visible_child_index {
                continue;
            }
            if idx > self.bottom_visible_child_index {
                break;
            }
            let dims = dimensions.as_ref().expect(MISSING_DIMENSIONS);
            if Some(idx) == self.selected_child_index {
                if let Some(bg) = self.selection_bg {
                    child.select(bg);
                }
            } else {
                child.deselect();
            }
            let child_cursor_granted = cursor_granted && (Some(idx) == self.selected_child_index);
            let child_frame = ScreenFrame::new(offscreen, dims, child_cursor_granted);
            child.render(child_frame, config);
        }
    }

    fn on_focus_change(&mut self, focused: bool) {
        if self.remember_position {
            if focused {
                if let Some((bottom, sel)) = self.remembered_position.take() {
                    self.bottom_visible_child_index = bottom;
                    self.selected_child_index = sel;
                } else {
                    self.selected_child_index = Some(self.first_visible_child_index);
                }
            } else {
                self.remembered_position =
                    Some((self.bottom_visible_child_index, self.selected_child_index));
            }
        }
        if let Some(mut handler) = self.focus_change_handler.take() {
            handler(self, focused);
            self.focus_change_handler = Some(handler);
        }
    }

    fn event(&mut self, event: &mut E) {
        if let Some(handler) = &self.event_handler {
            let handler = Rc::clone(handler);
            let handler = &mut *handler.borrow_mut();
            handler(self, event);
        }
    }

    fn insertable(&self) -> bool {
        self.selected().map(View::insertable).unwrap_or(false)
    }
}

impl<E, I, C> ScrollWin<E, I, C>
where
    I: View<E, C> + Hash + Eq + Ord + Searchable,
{
    fn search_from(&mut self, query: &str, indices: &[usize]) -> (Option<usize>, Option<usize>) {
        let old = self.selected_child_index;
        let children: Vec<&LayoutChild<I>> = self.children.iter().collect();
        for &i in indices {
            if children[i].child.matches(query) {
                self.selected_child_index = Some(i);
                if i > self.bottom_visible_child_index {
                    self.bottom_visible_child_index = i;
                }
                return (old, Some(i));
            }
        }
        (old, old)
    }

    pub fn set_search(&mut self, query: &str) -> (Option<usize>, Option<usize>) {
        self.search_query = Some(query.to_string());
        let len = self.children.len();
        if len == 0 {
            return (self.selected_child_index, self.selected_child_index);
        }
        let anchor = self
            .selected_child_index
            .unwrap_or(self.bottom_visible_child_index);
        let before: Vec<usize> = (0..anchor).rev().collect();
        let after: Vec<usize> = (anchor..len).rev().collect();
        let indices: Vec<usize> = before.into_iter().chain(after).collect();
        self.search_from(query, &indices)
    }

    pub fn search_next(&mut self) -> (Option<usize>, Option<usize>) {
        let Some(query) = self.search_query.clone() else {
            return (self.selected_child_index, self.selected_child_index);
        };
        let len = self.children.len();
        if len == 0 {
            return (self.selected_child_index, self.selected_child_index);
        }
        let anchor = self
            .selected_child_index
            .unwrap_or(self.bottom_visible_child_index);
        let before: Vec<usize> = (0..anchor).rev().collect();
        let after: Vec<usize> = (anchor + 1..len).rev().collect();
        let indices: Vec<usize> = after.into_iter().chain(before).collect();
        self.search_from(&query, &indices)
    }

    pub fn search_prev(&mut self) -> (Option<usize>, Option<usize>) {
        let Some(query) = self.search_query.clone() else {
            return (self.selected_child_index, self.selected_child_index);
        };
        let len = self.children.len();
        if len == 0 {
            return (self.selected_child_index, self.selected_child_index);
        }
        let anchor = self
            .selected_child_index
            .unwrap_or(self.bottom_visible_child_index);
        let after: Vec<usize> = (anchor + 1..len).collect();
        let before: Vec<usize> = (0..=anchor).collect();
        let indices: Vec<usize> = after.into_iter().chain(before).collect();
        self.search_from(&query, &indices)
    }

    pub fn clear_search(&mut self) {
        self.search_query = None;
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;
    use crate::{BgColor, Color};

    #[derive(Debug, Clone, Default)]
    pub struct MockView {
        pub ord: usize,
        pub height: u16,
        pub dimensions: Option<Dimensions>,
        pub insertable: bool,
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

    impl<E, C> View<E, C> for MockView {
        fn measure(&self, _measure_specs: &MeasureSpecs) -> RequestedDimensions {
            RequestedDimensions {
                height: RequestedDimension::Absolute(self.height),
                width: RequestedDimension::ExpandMax,
            }
        }

        fn layout(&mut self, dimensions: &Dimensions) {
            self.dimensions.replace(dimensions.clone());
        }

        fn render(&self, _frame: ScreenFrame, _config: &C) {
            unreachable!()
        }

        fn event(&mut self, _event: &mut E) {
            unreachable!()
        }

        fn insertable(&self) -> bool {
            self.insertable
        }
    }

    impl Searchable for MockView {
        fn matches(&self, query: &str) -> bool {
            query == self.ord.to_string()
        }
    }

    #[test]
    fn test_search_first_finds_match() {
        let mut scroll_win = ScrollWin::<(), MockView>::new();
        for i in 0..5 {
            scroll_win.insert(MockView {
                ord: i,
                height: 5,
                ..Default::default()
            });
        }
        scroll_win.layout(&Dimensions {
            width: 100,
            height: 50,
            top: 0,
            left: 0,
        });
        scroll_win.selected_child_index = Some(4);

        let (old, new) = scroll_win.set_search("2");

        assert_eq!(old, Some(4));
        assert_eq!(new, Some(2));
        assert_eq!(scroll_win.selected_child_index, Some(2));
    }

    #[test]
    fn test_search_no_match_returns_same() {
        let mut scroll_win = ScrollWin::<(), MockView>::new();
        for i in 0..3 {
            scroll_win.insert(MockView {
                ord: i,
                height: 5,
                ..Default::default()
            });
        }
        scroll_win.layout(&Dimensions {
            width: 100,
            height: 50,
            top: 0,
            left: 0,
        });
        scroll_win.selected_child_index = Some(1);

        let (old, new) = scroll_win.set_search("99");

        assert_eq!(old, Some(1));
        assert_eq!(new, Some(1));
        assert_eq!(scroll_win.selected_child_index, Some(1));
    }

    #[test]
    fn test_search_next_wraps() {
        let mut scroll_win = ScrollWin::<(), MockView>::new();
        for i in 0..5 {
            scroll_win.insert(MockView {
                ord: i,
                height: 5,
                ..Default::default()
            });
        }
        scroll_win.layout(&Dimensions {
            width: 100,
            height: 50,
            top: 0,
            left: 0,
        });
        scroll_win.set_search("0");
        assert_eq!(scroll_win.selected_child_index, Some(0));

        // Already at index 0, next should wrap to find... no other "0". So stays.
        // Use a different query for a proper wrap test.
        scroll_win.set_search("2");
        assert_eq!(scroll_win.selected_child_index, Some(2));
        // next goes toward lower index (older), past 0, wraps to find 2 again
        let (_, new) = scroll_win.search_next();
        assert_eq!(new, Some(2)); // only one "2", wrap returns same
    }

    #[test]
    fn test_search_prev_direction() {
        let mut scroll_win = ScrollWin::<(), MockView>::new();
        for i in 0..5 {
            scroll_win.insert(MockView {
                ord: i,
                height: 5,
                ..Default::default()
            });
        }
        scroll_win.layout(&Dimensions {
            width: 100,
            height: 50,
            top: 0,
            left: 0,
        });
        scroll_win.selected_child_index = Some(2);
        scroll_win.search_query = Some("4".to_string());

        // prev goes toward newer (higher index)
        let (_, new) = scroll_win.search_prev();

        assert_eq!(new, Some(4));
        assert_eq!(scroll_win.selected_child_index, Some(4));
    }

    #[test]
    fn test_clear_search_disables_next() {
        let mut scroll_win = ScrollWin::<(), MockView>::new();
        for i in 0..3 {
            scroll_win.insert(MockView {
                ord: i,
                height: 5,
                ..Default::default()
            });
        }
        scroll_win.layout(&Dimensions {
            width: 100,
            height: 50,
            top: 0,
            left: 0,
        });
        scroll_win.selected_child_index = Some(1);
        scroll_win.search_query = Some("0".to_string());
        scroll_win.clear_search();

        let (old, new) = scroll_win.search_next();

        assert_eq!(old, new);
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

    #[test]
    fn test_layout_page_up_near_top() {
        // Given: 4 messages of height 4 (total 16), window height 10
        // After two page_ups we're near the top; first message must be at top, no blank space
        let mut scroll_win = ScrollWin::<(), MockView>::new();
        for ord in 0..4usize {
            scroll_win.insert(MockView {
                ord,
                height: 4,
                ..Default::default()
            });
        }
        let dimensions = Dimensions {
            width: 100,
            height: 10,
            top: 1,
            left: 1,
        };
        scroll_win.layout(&dimensions);
        scroll_win.page_up();
        scroll_win.layout(&dimensions);
        scroll_win.page_up();
        scroll_win.layout(&dimensions);

        // Then: first visible child must be at dimensions.top (no blank space at top)
        let visible: Vec<_> = scroll_win.visible_children(&dimensions).collect();
        let top_child = visible.last().unwrap(); // visible_children iterates bottom→top
        assert_eq!(
            top_child.dimensions.as_ref().map(|d| d.top),
            Some(dimensions.top),
            "first message should be flush with top of window"
        );
    }

    #[test]
    fn test_layout_page_up_to_index_zero_shows_multiple_children() {
        // Given: 10 messages of height 3 in a window of height 10 — page_up will
        // eventually drive bottom_visible_child_index to 0, after which
        // layout_from_top must still show all children that fit (not just one).
        let mut scroll_win = ScrollWin::<(), MockView>::new();
        for ord in 0..10usize {
            scroll_win.insert(MockView {
                ord,
                height: 3,
                ..Default::default()
            });
        }
        let dimensions = Dimensions {
            width: 100,
            height: 10,
            top: 0,
            left: 0,
        };
        scroll_win.layout(&dimensions);
        // Drive page_up until at_top
        for _ in 0..10 {
            let (at_top, _, _) = scroll_win.page_up();
            scroll_win.layout(&dimensions);
            if at_top {
                break;
            }
        }

        // Then: first visible child must be at top with no blank space
        let visible: Vec<_> = scroll_win.visible_children(&dimensions).collect();
        let top_child = visible.last().unwrap();
        assert_eq!(
            top_child.dimensions.as_ref().map(|d| d.top),
            Some(dimensions.top),
            "first message should be flush with top of window"
        );
        // And more than one child must be visible (3 fit in height 10 with height 3 each)
        assert!(
            visible.len() > 1,
            "multiple children should be visible at the top, got {}",
            visible.len()
        );
    }

    fn three_child_win() -> ScrollWin<(), MockView> {
        let mut w = ScrollWin::<(), MockView>::new();
        for ord in 0..3 {
            w.insert(MockView {
                ord,
                height: 10,
                ..Default::default()
            });
        }
        let dims = Dimensions {
            top: 0,
            left: 0,
            width: 80,
            height: 30,
        };
        w.layout(&dims);
        w
    }

    #[test]
    fn test_select_prev_initialises_at_bottom() {
        let mut w = three_child_win();
        let (old, new, _at_top) = w.select_prev();
        assert_eq!(old, None);
        assert_eq!(new, Some(w.bottom_visible_child_index));
    }

    #[test]
    fn test_select_next_initialises_at_bottom() {
        let mut w = three_child_win();
        let (old, new) = w.select_next();
        assert_eq!(old, None);
        assert_eq!(new, Some(w.bottom_visible_child_index));
    }

    #[test]
    fn test_select_prev_moves_up() {
        let mut w = three_child_win();
        w.select_prev(); // init at index 2
        let (old, new, at_top) = w.select_prev();
        assert_eq!(old, Some(2));
        assert_eq!(new, Some(1));
        assert!(!at_top);
    }

    #[test]
    fn test_select_next_moves_down() {
        let mut w = three_child_win();
        w.select_prev(); // init at 2
        w.select_prev(); // move to 1
        let (old, new) = w.select_next();
        assert_eq!(old, Some(1));
        assert_eq!(new, Some(2));
    }

    #[test]
    fn test_select_prev_clamped_at_zero() {
        let mut w = three_child_win();
        w.select_prev(); // 2
        w.select_prev(); // 1
        w.select_prev(); // 0 – at top
        let (old, new, at_top) = w.select_prev();
        assert_eq!(old, Some(0));
        assert_eq!(new, Some(0));
        assert!(at_top);
    }

    #[test]
    fn test_select_next_clamped_at_last() {
        let mut w = three_child_win();
        w.select_next(); // init at 2 (bottom)
        let (old, new) = w.select_next();
        assert_eq!(old, Some(2));
        assert_eq!(new, Some(2)); // already at last
    }

    #[test]
    fn test_clear_selection_returns_index() {
        let mut w = three_child_win();
        w.select_prev();
        w.select_prev(); // now at 1
        let prev = w.clear_selection();
        assert_eq!(prev, Some(1));
        // A subsequent select_prev initialises again from bottom
        let (old, new, _) = w.select_prev();
        assert_eq!(old, None);
        assert_eq!(new, Some(w.bottom_visible_child_index));
    }

    #[test]
    fn test_select_prev_scrolls_viewport_up() {
        // 4 children of height 10, viewport height 20 → only 2 visible at a time
        let mut w = ScrollWin::<(), MockView>::new();
        for ord in 0..4 {
            w.insert(MockView {
                ord,
                height: 10,
                ..Default::default()
            });
        }
        let dims = Dimensions {
            top: 0,
            left: 0,
            width: 80,
            height: 20,
        };
        w.layout(&dims);

        // bottom_visible_child_index should be 3, first_visible 2
        assert_eq!(w.bottom_visible_child_index, 3);
        assert_eq!(w.first_visible_child_index, 2);

        w.select_prev(); // init at 3
        w.select_prev(); // move to 2 (still visible)
        assert_eq!(w.bottom_visible_child_index, 3); // no scroll yet

        w.select_prev(); // move to 1 – now above first_visible(2), should scroll
        assert_eq!(w.bottom_visible_child_index, 2); // scrolled up by 1
    }

    #[test]
    fn test_selected_returns_none_when_no_selection() {
        let mut w = ScrollWin::<(), MockView>::new();
        for i in 0..3 {
            w.insert(MockView {
                ord: i,
                height: 5,
                ..Default::default()
            });
        }
        assert_eq!(w.selected(), None);
    }

    #[test]
    fn test_selected_returns_current_child() {
        let mut w = ScrollWin::<(), MockView>::new();
        for i in 0..5 {
            w.insert(MockView {
                ord: i,
                height: 5,
                ..Default::default()
            });
        }
        w.layout(&Dimensions {
            width: 100,
            height: 50,
            top: 0,
            left: 0,
        });
        w.select_last_visible();
        let idx = w.selected_child_index.unwrap();
        assert_eq!(w.selected(), w.child_at(idx));
    }

    #[test]
    fn test_select_last_visible_tracks_bottom_on_sequential_inserts() {
        // Simulates MAM backfill: messages arrive one-by-one and select_last_visible
        // should always return the current bottom_visible_child_index.
        let mut w = ScrollWin::<(), MockView>::new();
        for ord in 0..5 {
            w.insert(MockView {
                ord,
                height: 10,
                ..Default::default()
            });
            let (_, new) = w.select_last_visible();
            assert_eq!(
                new,
                Some(w.bottom_visible_child_index),
                "after insert {ord}: selection should equal bottom_visible_child_index"
            );
        }
    }

    #[test]
    fn test_bottom_visible_decreases_after_page_up() {
        // After page_up, bottom_visible_child_index moves up; the caller (ui.rs)
        // sets follow_bottom=false so auto-tracking stops.  This test verifies
        // page_up actually moves the bottom so follow_bottom logic is meaningful.
        let mut w = ScrollWin::<(), MockView>::new();
        for ord in 0..5 {
            w.insert(MockView {
                ord,
                height: 10,
                ..Default::default()
            });
        }
        let dims = Dimensions {
            top: 0,
            left: 0,
            width: 80,
            height: 20,
        };
        w.layout(&dims);
        w.select_last_visible(); // init at 4
        assert_eq!(w.bottom_visible_child_index, 4);
        w.page_up();
        assert!(
            w.bottom_visible_child_index < 4,
            "page_up should move bottom_visible_child_index below 4"
        );
    }

    #[test]
    fn insertable_false_when_no_child_selected() {
        let mut w = ScrollWin::<(), MockView>::new();
        w.insert(MockView {
            ord: 0,
            height: 1,
            insertable: true,
            ..Default::default()
        });
        // No selection yet
        assert!(!<ScrollWin<(), MockView> as View<(), ()>>::insertable(&w));
    }

    #[test]
    fn insertable_delegates_to_selected_child() {
        let mut w = ScrollWin::<(), MockView>::new();
        w.insert(MockView {
            ord: 0,
            height: 1,
            insertable: true,
            ..Default::default()
        });
        w.insert(MockView {
            ord: 1,
            height: 1,
            insertable: false,
            ..Default::default()
        });
        w.layout(&Dimensions {
            top: 0,
            left: 0,
            width: 80,
            height: 20,
        });
        w.selected_child_index = Some(0);
        assert!(<ScrollWin<(), MockView> as View<(), ()>>::insertable(&w));

        w.selected_child_index = Some(1);
        assert!(!<ScrollWin<(), MockView> as View<(), ()>>::insertable(&w));
    }

    #[test]
    fn focus_change_handler_fires_on_blur() {
        use std::cell::Cell;
        use std::rc::Rc;
        let fired = Rc::new(Cell::new(false));
        let fired2 = fired.clone();
        let mut sw = ScrollWin::<(), MockView>::new().with_focus_change(move |_, focused| {
            if !focused {
                fired2.set(true);
            }
        });
        assert!(!fired.get(), "handler must not fire before on_focus_change");
        sw.on_focus_change(false);
        assert!(fired.get(), "handler must fire when focus is lost");
    }

    #[test]
    fn focus_change_handler_not_called_when_focus_gained() {
        use std::cell::Cell;
        use std::rc::Rc;
        let fired = Rc::new(Cell::new(false));
        let fired2 = fired.clone();
        let mut sw = ScrollWin::<(), MockView>::new().with_focus_change(move |_, focused| {
            if !focused {
                fired2.set(true);
            }
        });
        sw.on_focus_change(true);
        assert!(!fired.get(), "blur handler must not fire on focus gained");
    }

    #[test]
    fn position_memory_selects_top_on_first_focus_with_no_remembered_position() {
        let bg = BgColor(Color::Rgb(0, 0, 0));
        let mut sw = ScrollWin::<(), MockView>::new()
            .with_selection_bg(bg)
            .with_position_memory();
        sw.insert(MockView {
            ord: 1,
            height: 1,
            ..Default::default()
        });
        sw.insert(MockView {
            ord: 2,
            height: 1,
            ..Default::default()
        });
        sw.insert(MockView {
            ord: 3,
            height: 1,
            ..Default::default()
        });
        // No prior on_focus_change(false) — remembered_position is None.
        sw.on_focus_change(true);
        assert_eq!(
            sw.selected().map(|v| v.ord),
            Some(1),
            "first focus with no remembered position must select the top (first visible) child"
        );
    }

    #[test]
    fn position_memory_restores_selection_after_blur_and_focus() {
        let bg = BgColor(Color::Rgb(0, 0, 0));
        let mut sw = ScrollWin::<(), MockView>::new()
            .with_selection_bg(bg)
            .with_position_memory()
            .with_focus_change(|view, focused| {
                if !focused {
                    view.clear_selection();
                }
            });
        sw.insert(MockView {
            ord: 1,
            height: 1,
            ..Default::default()
        });
        sw.insert(MockView {
            ord: 2,
            height: 1,
            ..Default::default()
        });
        sw.insert(MockView {
            ord: 3,
            height: 1,
            ..Default::default()
        });
        // After 3 inserts, bottom_visible_child_index = 2 (sticky bottom).
        // select_prev twice: first call anchors at bottom (index 2, ord=3), second moves to index 1 (ord=2).
        sw.select_prev();
        sw.select_prev(); // selected_child_index = 1 (ord=2)

        assert_eq!(sw.selected().map(|v| v.ord), Some(2));
        sw.on_focus_change(false);
        assert!(!sw.has_selection(), "selection must be cleared on blur");
        sw.on_focus_change(true);
        assert_eq!(
            sw.selected().map(|v| v.ord),
            Some(2),
            "selection must be restored after regaining focus"
        );
    }
}
