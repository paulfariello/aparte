/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::Hash;
use std::io::Write;
use std::os::fd::AsFd;
use std::rc::Rc;

use super::{Dimension, DimensionSpec, LayoutParam, LayoutParams, Screen, View};

/// Component that can hold multiple children but display only one at a time
pub struct FrameLayout<E, W, K>
where
    K: Hash + Eq + Clone,
    W: Write + AsFd,
{
    children: HashMap<K, Box<dyn View<E, W>>>,
    current: Option<K>,
    event_handler: Option<Rc<RefCell<Box<dyn FnMut(&mut Self, &mut E)>>>>,
    dirty: Cell<bool>,
    layout: LayoutParams,
}

impl<E, W, K> FrameLayout<E, W, K>
where
    K: Hash + Eq + Clone,
    W: Write + AsFd,
{
    pub fn new() -> Self {
        Self {
            children: HashMap::new(),
            current: None,
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

    pub fn set_current(&mut self, key: K) {
        self.current = Some(key);
        self.dirty.set(true);
    }

    pub fn get_current_mut<'a>(&'a mut self) -> Option<&'a mut Box<dyn View<E, W>>> {
        if let Some(current) = &self.current {
            if let Some(child) = self.children.get_mut(current) {
                Some(child)
            } else {
                unreachable!();
            }
        } else {
            None
        }
    }

    #[allow(unused)]
    pub fn get_current<'a>(&'a self) -> Option<&'a Box<dyn View<E, W>>> {
        if let Some(current) = &self.current {
            if let Some(child) = self.children.get(current) {
                Some(child)
            } else {
                unreachable!();
            }
        } else {
            None
        }
    }

    #[allow(unused)]
    pub fn get_current_key<'a>(&'a self) -> Option<&'a K> {
        self.current.as_ref()
    }

    #[allow(unused)]
    pub fn insert<T>(&mut self, key: K, view: T)
    where
        T: View<E, W> + 'static,
    {
        self.children.insert(key, Box::new(view));
    }

    pub fn insert_boxed(&mut self, key: K, view: Box<dyn View<E, W> + 'static>) {
        self.children.insert(key, view);
    }

    pub fn remove(&mut self, key: &K) {
        self.children.remove(key);
        if Some(key) == self.current.as_ref() {
            self.current = self.children.keys().next().cloned();
        }
    }

    pub fn iter_children_mut<'a>(
        &'a mut self,
    ) -> impl Iterator<Item = &'a mut Box<dyn View<E, W>>> {
        self.children.iter_mut().map(|(_, child)| child)
    }

    #[allow(unused)]
    pub fn iter_children<'a>(&'a self) -> impl Iterator<Item = &'a Box<dyn View<E, W>>> {
        self.children.iter().map(|(_, child)| child)
    }
}

impl<E, W, K> View<E, W> for FrameLayout<E, W, K>
where
    K: Hash + Eq + Clone,
    W: Write + AsFd,
{
    fn measure(&mut self, dimension_spec: &mut DimensionSpec) {
        if let Some(child) = self.get_current_mut() {
            child.measure(dimension_spec);
        }
    }

    fn layout(&mut self, dimension_spec: &DimensionSpec, x: u16, y: u16) -> Dimension {
        if let Some(child) = self.get_current_mut() {
            child.layout(dimension_spec, x, y)
        } else {
            dimension_spec.layout(x, y)
        }
    }

    fn render(&self, dimension: &Dimension, screen: &mut Screen<W>) {
        if let Some(child) = self.get_current() {
            if self.dirty.get() || child.is_dirty() {
                child.render(dimension, screen);
            }
        }
        self.dirty.set(false);
    }

    fn is_layout_dirty(&self) -> bool {
        self.dirty.get()
            || self
                .get_current()
                .map_or(false, |child| child.is_layout_dirty())
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get() || self.get_current().map_or(false, |child| child.is_dirty())
    }

    fn event(&mut self, event: &mut E) {
        if let Some(handler) = &self.event_handler {
            let handler = Rc::clone(handler);
            let handler = &mut *handler.borrow_mut();
            handler(self, event);
        } else {
            for child in self.iter_children_mut() {
                child.event(event);
            }
        }
    }

    fn get_layout(&self) -> LayoutParams {
        self.layout.clone()
    }
}
