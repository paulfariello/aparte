/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::Hash;
use std::io::Write;
use std::os::fd::AsFd;
use std::rc::Rc;

use crate::terminus::clear_screen;

use super::{
    Dimensions, LayoutParam, LayoutParams, MeasureSpecs, RequestedDimension, RequestedDimensions,
    Screen, View,
};

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
    layouts: LayoutParams,
    dimensions: Option<Dimensions>,
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
    pub fn with_layout(mut self, layout: LayoutParams) -> Self {
        self.layouts = layout;
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
    fn measure(&self, measure_specs: &MeasureSpecs) -> RequestedDimensions {
        // TODO take self.layouts into account
        if let Some(child) = self.get_current() {
            child.measure(measure_specs)
        } else {
            RequestedDimensions {
                width: RequestedDimension::Absolute(0),
                height: RequestedDimension::Absolute(0),
            }
        }
    }

    fn layout(&mut self, dimensions: &Dimensions) {
        log::debug!("layout {} {:?}", std::any::type_name::<Self>(), dimensions);
        self.dimensions.replace(dimensions.clone());
        if let Some(child) = self.get_current_mut() {
            child.layout(dimensions)
        }
    }

    fn render(&self, screen: &mut Screen<W>) {
        log::debug!("rendering {}", std::any::type_name::<Self>(),);
        if self.dirty.get() {
            clear_screen(self.dimensions.as_ref().unwrap(), screen)
        }
        if let Some(child) = self.get_current() {
            if self.dirty.get() || child.is_dirty() {
                child.render(screen);
            }
        }
        self.dirty.set(false);
    }

    fn is_layout_dirty(&self) -> bool {
        match (self.layouts.width, self.layouts.height) {
            (_, LayoutParam::WrapContent) | (LayoutParam::WrapContent, _) => {
                self.dirty.get()
                    || self
                        .get_current()
                        .map_or(false, |child| child.is_layout_dirty())
            }
            _ => false,
        }
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
}
