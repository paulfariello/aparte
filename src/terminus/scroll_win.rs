/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::hash::Hash;
use std::io::Write;
use std::os::fd::AsFd;
use std::rc::Rc;

use super::{Dimension, DimensionSpec, LayoutParam, LayoutParams, Screen, View};

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
    children: BTreeMap<I, Option<Dimension>>,
    layout: LayoutParams,
}

impl<E, W, I> ScrollWin<E, W, I>
where
    W: Write + AsFd,
    I: View<E, W> + Hash + Eq + Ord,
{
    pub fn new() -> Self {
        Self {
            next_line: 0,
            children: BTreeMap::new(),
            view: 0,
            event_handler: None,
            dirty: Cell::new(true),
            layout: LayoutParams {
                width: LayoutParam::MatchParent,
                height: LayoutParam::MatchParent,
            },
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
    pub fn with_layout(mut self, layout: LayoutParams) -> Self {
        self.layout = layout;
        self
    }

    #[allow(dead_code)]
    pub fn first<'a>(&'a self) -> Option<&'a I> {
        self.children.keys().next()
    }

    pub fn insert(&mut self, item: I) {
        // We don't care about rendered buffer, we avoid computation here at cost of false positive
        // (set dirty while in fact it shouldn't)
        // XXX We should care
        self.children.insert(item, None);
        self.dirty.set(true)
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
}

impl<E, W, I> View<E, W> for ScrollWin<E, W, I>
where
    W: Write + AsFd,
    I: View<E, W> + Hash + Eq + Ord,
{
    fn measure(&mut self, dimension_spec: &mut DimensionSpec) {
        // Should we measure only visible children?
        if dimension_spec.width.is_none() {
            // Mesure max width of each children
            let children: Vec<_> = std::mem::replace(&mut self.children, BTreeMap::new())
                .into_keys()
                .collect();
            let max_width = children
                .into_iter()
                .map(|mut child| {
                    let mut child_dimension_spec = dimension_spec.clone();
                    child.measure(&mut child_dimension_spec);
                    self.children.insert(child, None);
                    child_dimension_spec.width
                })
                .max()
                .flatten();
            dimension_spec.width = max_width;
        }
    }

    fn layout(&mut self, dimension_spec: &DimensionSpec, x: u16, y: u16) -> Dimension {
        // TODO should layout only visible children
        let mut child_y = y;

        let children: Vec<_> = std::mem::replace(&mut self.children, BTreeMap::new())
            .into_iter()
            .collect();
        for (mut child, _) in children.into_iter() {
            let dimension = child.layout(dimension_spec, x, child_y);

            child_y += dimension.height;
            self.children.insert(child, Some(dimension));
        }

        dimension_spec.layout(x, y)
    }

    fn render(&self, _dimension: &Dimension, screen: &mut Screen<W>) {
        super::save_cursor!(screen);

        for (child, child_dimension) in self.children.iter() {
            // child dimension can be unwrapped since it must have been set during the measure
            // phase
            child.render(child_dimension.as_ref().unwrap(), screen);
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

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    fn get_layout(&self) -> LayoutParams {
        self.layout.clone()
    }
}
