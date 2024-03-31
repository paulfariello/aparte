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
    Dimensions, LayoutParam, LayoutParams, MeasureSpecs, RequestedDimension, RequestedDimensions,
    Screen, View,
};

/// Ordered vertical window giving ability to scroll
pub struct ScrollWin<E, W, I>
where
    W: Write + AsFd,
    I: View<E, W> + Hash + Eq + Ord,
{
    next_line: u16,
    view: usize,
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
            next_line: 0,
            children: BTreeSet::new(),
            view: 0,
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
        // We don't care about rendered buffer, we avoid computation here at cost of false positive
        // (set dirty while in fact it shouldn't)
        // XXX We should care
        self.children.insert(item);
        self.dirty.set(true);
    }

    /// PageUp the window, return true if top is reached
    pub fn page_up(&mut self) -> bool {
        todo!()
        // let buffers = self.get_rendered_items();
        // let count = buffers.len();

        // if count < self.height {
        //     return true;
        // }

        // self.dirty = true;

        // let max = count - self.height;

        // if self.view + self.height < max {
        //     self.view += self.height;
        //     false
        // } else {
        //     self.view = max;
        //     true
        // }
    }

    /// PageDown the window, return true if bottom is reached
    pub fn page_down(&mut self) -> bool {
        todo!()
        //self.dirty.set(true);
        //if self.view > self.height {
        //    self.view -= self.height;
        //    false
        //} else {
        //    self.view = 0;
        //    true
        //}
    }

    pub fn visible_children<'a>(&'a self) -> impl Iterator<Item = &'a I> {
        // TODO filter
        self.children.iter().filter(|_| true)
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
        let total_child_height = heights.into_iter().sum();

        let requested_width = match self.layouts.width {
            LayoutParam::MatchParent => RequestedDimension::ExpandMax,
            LayoutParam::WrapContent => max_child_width,
            LayoutParam::Absolute(absolute_width) => RequestedDimension::Absolute(absolute_width),
        };

        let requested_height = match self.layouts.height {
            LayoutParam::MatchParent => RequestedDimension::ExpandMax,
            LayoutParam::WrapContent => total_child_height,
            LayoutParam::Absolute(absolute_height) => RequestedDimension::Absolute(absolute_height),
        };

        RequestedDimensions {
            width: requested_width,   // We let parent reconcile
            height: requested_height, // We let parent reconcile
        }
    }

    fn layout(&mut self, dimensions: &Dimensions) {
        log::debug!("layout {} {:?}", std::any::type_name::<Self>(), dimensions);
        // TODO should layout only visible children
        let mut child_top = dimensions.top;
        let measure_specs: MeasureSpecs = dimensions.into();

        let children: Vec<_> = std::mem::replace(&mut self.children, BTreeSet::new())
            .into_iter()
            .collect();
        for mut child in children.into_iter() {
            let requested_dimensions = child.measure(&measure_specs);
            let mut child_dimensions = Dimensions::reconcile(
                &measure_specs,
                &requested_dimensions,
                child_top,
                dimensions.left,
            );

            // Force full width
            child_dimensions.width = dimensions.width;
            child.layout(&child_dimensions);

            child_top += child_dimensions.height;
            self.children.insert(child);
        }

        self.dimensions = Some(dimensions.clone());
    }

    fn render(&self, screen: &mut Screen<W>) {
        log::debug!(
            "rendering {} at {:?}",
            std::any::type_name::<Self>(),
            self.dimensions
        );
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
