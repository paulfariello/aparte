/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;

use crate::rendering::ScreenFrame;

use super::{
    Dimensions, EventHandler, LayoutParam, LayoutParams, MeasureSpecs, RequestedDimension,
    RequestedDimensions, View,
};

/// Component that can hold multiple children but display only one at a time
pub struct FrameLayout<E, K>
where
    K: Hash + Eq + Clone,
{
    children: HashMap<K, Box<dyn View<E>>>,
    current: Option<K>,
    event_handler: Option<EventHandler<Self, E>>,
    layouts: LayoutParams,
    dimensions: Option<Dimensions>,
}

impl<E, K> Default for FrameLayout<E, K>
where
    K: Hash + Eq + Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<E, K> FrameLayout<E, K>
where
    K: Hash + Eq + Clone,
{
    pub fn new() -> Self {
        Self {
            children: HashMap::new(),
            current: None,
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

    pub fn with_layout(mut self, layout: LayoutParams) -> Self {
        self.layouts = layout;
        self
    }

    pub fn set_current(&mut self, key: K) {
        if self.current.as_ref() != Some(&key) {
            self.current = Some(key);
        }
    }

    pub fn get_current_mut(&mut self) -> Option<&mut Box<dyn View<E>>> {
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

    pub fn get_current(&self) -> Option<&dyn View<E>> {
        if let Some(current) = &self.current {
            if let Some(child) = self.children.get(current) {
                Some(child.as_ref())
            } else {
                unreachable!();
            }
        } else {
            None
        }
    }

    pub fn get_current_key(&self) -> Option<&K> {
        self.current.as_ref()
    }

    pub fn insert<T>(&mut self, key: K, view: T)
    where
        T: View<E> + 'static,
    {
        self.children.insert(key, Box::new(view));
    }

    pub fn insert_boxed(&mut self, key: K, view: Box<dyn View<E> + 'static>) {
        self.children.insert(key, view);
    }

    pub fn remove(&mut self, key: &K) {
        self.children.remove(key);
        if Some(key) == self.current.as_ref() {
            self.current = self.children.keys().next().cloned();
        }
    }

    pub fn iter_children_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn View<E>>> {
        self.children.iter_mut().map(|(_, child)| child)
    }

    pub fn iter_children(&self) -> impl Iterator<Item = &Box<dyn View<E>>> {
        self.children.values()
    }
}

impl<E, K> View<E> for FrameLayout<E, K>
where
    K: Hash + Eq + Clone,
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

    fn render(&self, frame: ScreenFrame) {
        log::debug!("rendering {}", std::any::type_name::<Self>(),);

        if let Some(child) = self.get_current() {
            child.render(frame);
        }
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
