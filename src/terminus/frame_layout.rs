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
    Dimensions, EventHandler, LayoutParam, LayoutParams, MeasureSpecs, RequestedDimension,
    RequestedDimensions, Screen, View,
};

/// Component that can hold multiple children but display only one at a time
pub struct FrameLayout<E, W, K>
where
    K: Hash + Eq + Clone,
    W: Write + AsFd,
{
    children: HashMap<K, Box<dyn View<E, W>>>,
    current: Option<K>,
    event_handler: Option<EventHandler<Self, E>>,
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
        if self.current.as_ref() != Some(&key) {
            self.current = Some(key);
            self.set_dirty();
        }
    }

    pub fn get_current_mut(&mut self) -> Option<&mut Box<dyn View<E, W>>> {
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

    pub fn get_current(&self) -> Option<&dyn View<E, W>> {
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

    #[allow(unused)]
    pub fn get_current_key(&self) -> Option<&K> {
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

    pub fn iter_children_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn View<E, W>>> {
        self.children.iter_mut().map(|(_, child)| child)
    }

    #[allow(unused)]
    pub fn iter_children(&self) -> impl Iterator<Item = &Box<dyn View<E, W>>> {
        self.children.values()
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
        if self.dimensions.as_ref() != Some(dimensions) {
            self.dirty.set(true);
            self.dimensions.replace(dimensions.clone());
        }
        if let Some(child) = self.get_current_mut() {
            child.layout(dimensions)
        }
    }

    fn render(&self, screen: &mut Screen<W>) {
        log::debug!("rendering {}", std::any::type_name::<Self>(),);
        let was_dirty = self.dirty.replace(false);
        if was_dirty {
            clear_screen(self.dimensions.as_ref().unwrap(), screen);
        }

        if let Some(child) = self.get_current() {
            if was_dirty || child.is_dirty() {
                child.render(screen);
            }
        }
    }

    fn set_dirty(&mut self) {
        self.dirty.set(true);
        self.get_current_mut()
            .iter_mut()
            .for_each(|current| current.set_dirty());
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

#[cfg(test)]
mod tests {
    use std::fs::File;
    use test_log::test;

    use super::*;
    use crate::terminus::MockView;

    #[test]
    fn test_set_dirty_to_current_child() {
        // Given
        let mut layout = FrameLayout::<(), File, u8>::new();

        // Then
        let first_view = MockView::new();
        let mut second_view = MockView::new();
        second_view.expect_set_dirty().times(2).return_const(());

        // When
        layout.insert(1, first_view);
        layout.insert(2, second_view);

        // Set current triggers dirty once
        layout.set_current(2);

        // Set dirty triggers dirty once again
        layout.set_dirty();
    }

    #[test]
    fn test_set_child_dirty_when_changing_current() {
        // Given
        let mut layout = FrameLayout::<(), File, u8>::new();

        // Then
        let first_view = MockView::new();
        let mut second_view = MockView::new();
        second_view.expect_set_dirty().times(1).return_const(());

        // When
        layout.insert(1, first_view);
        layout.insert(2, second_view);

        layout.set_current(2);
        // Not changing current shouldn't trigger dirty
        layout.set_current(2);
    }
}
