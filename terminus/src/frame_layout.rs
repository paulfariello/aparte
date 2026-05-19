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
pub struct FrameLayout<E, K, C = ()>
where
    K: Hash + Eq + Clone,
{
    children: HashMap<K, Box<dyn View<E, C>>>,
    current: Option<K>,
    event_handler: Option<EventHandler<Self, E>>,
    layouts: LayoutParams,
    dimensions: Option<Dimensions>,
    focused: bool,
}

impl<E, K, C> Default for FrameLayout<E, K, C>
where
    K: Hash + Eq + Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<E, K, C> FrameLayout<E, K, C>
where
    K: Hash + Eq + Clone,
{
    #[must_use]
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
            focused: false,
        }
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
    pub fn with_layout(mut self, layout: LayoutParams) -> Self {
        self.layouts = layout;
        self
    }

    pub fn set_current(&mut self, key: K) {
        if self.current.as_ref() == Some(&key) {
            return;
        }
        if self.focused {
            if let Some(ref old_key) = self.current {
                if let Some(child) = self.children.get_mut(old_key) {
                    child.on_focus_change(false);
                }
            }
        }
        self.current = Some(key);
        if self.focused {
            if let Some(ref new_key) = self.current {
                if let Some(child) = self.children.get_mut(new_key) {
                    if child.focusable() {
                        child.on_focus_change(true);
                    }
                }
            }
        }
    }

    pub fn route_to_focused(&mut self, event: &mut E) {
        if let Some(current) = self.get_current_mut() {
            current.event(event);
        }
    }

    pub fn get_current_mut(&mut self) -> Option<&mut Box<dyn View<E, C>>> {
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

    pub fn get_current(&self) -> Option<&dyn View<E, C>> {
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
        T: View<E, C> + 'static,
    {
        self.children.insert(key, Box::new(view));
    }

    pub fn insert_boxed(&mut self, key: K, view: Box<dyn View<E, C> + 'static>) {
        self.children.insert(key, view);
    }

    pub fn remove(&mut self, key: &K) {
        self.children.remove(key);
        if Some(key) == self.current.as_ref() {
            self.current = self.children.keys().next().cloned();
        }
    }

    pub fn iter_children_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn View<E, C>>> {
        self.children.iter_mut().map(|(_, child)| child)
    }

    pub fn iter_children(&self) -> impl Iterator<Item = &Box<dyn View<E, C>>> {
        self.children.values()
    }
}

impl<E, K, C> View<E, C> for FrameLayout<E, K, C>
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
            child.layout(dimensions);
        }
    }

    fn render(&self, frame: ScreenFrame, config: &C) {
        log::debug!("rendering {}", std::any::type_name::<Self>(),);

        if let Some(child) = self.get_current() {
            child.render(frame, config);
        }
    }

    fn on_focus_change(&mut self, focused: bool) {
        self.focused = focused;
        if let Some(ref key) = self.current {
            if let Some(child) = self.children.get_mut(key) {
                if child.focusable() {
                    child.on_focus_change(focused);
                }
            }
        }
    }

    fn event(&mut self, event: &mut E) {
        if let Some(handler) = &self.event_handler {
            let handler = Rc::clone(handler);
            let handler = &mut *handler.borrow_mut();
            handler(self, event);
        } else {
            self.route_to_focused(event);
        }
    }

    fn insertable(&self) -> bool {
        self.current
            .as_ref()
            .and_then(|key| self.children.get(key))
            .map(|child| child.insertable())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockView;
    type TestMockView = MockView<(), ()>;
    use crate::{RequestedDimension, RequestedDimensions};
    use mockall::predicate::*;

    fn make_mock_child() -> TestMockView {
        let mut v = TestMockView::new();
        v.expect_measure().return_const(RequestedDimensions {
            width: RequestedDimension::ExpandMax,
            height: RequestedDimension::ExpandMax,
        });
        v.expect_focusable().return_const(true);
        v
    }

    #[test]
    fn test_on_focus_change_cascades_to_current_child() {
        let mut frame = FrameLayout::<(), &str>::new();
        let mut child = make_mock_child();
        child
            .expect_on_focus_change()
            .with(eq(true))
            .times(1)
            .return_const(());
        frame.insert("a", child);
        frame.set_current("a");
        frame.on_focus_change(true);
    }

    #[test]
    fn test_on_focus_change_false_cascades_and_retains_current() {
        let mut frame = FrameLayout::<(), &str>::new();
        let mut child = make_mock_child();
        child
            .expect_on_focus_change()
            .with(eq(true))
            .times(1)
            .return_const(());
        child
            .expect_on_focus_change()
            .with(eq(false))
            .times(1)
            .return_const(());
        frame.insert("a", child);
        frame.set_current("a");
        frame.on_focus_change(true);
        frame.on_focus_change(false);
        assert_eq!(frame.current, Some("a"));
    }

    #[test]
    fn test_set_current_while_focused_notifies_old_and_new() {
        let mut frame = FrameLayout::<(), &str>::new();
        let mut child_a = make_mock_child();
        let mut child_b = make_mock_child();
        child_a
            .expect_on_focus_change()
            .with(eq(true))
            .times(1)
            .return_const(());
        child_a
            .expect_on_focus_change()
            .with(eq(false))
            .times(1)
            .return_const(());
        child_b
            .expect_on_focus_change()
            .with(eq(true))
            .times(1)
            .return_const(());
        frame.insert("a", child_a);
        frame.insert("b", child_b);
        frame.set_current("a");
        frame.on_focus_change(true); // frame focused → child_a notified
        frame.set_current("b"); // child_a loses, child_b gains
    }

    #[test]
    fn test_set_current_while_not_focused_no_notification() {
        let mut frame = FrameLayout::<(), &str>::new();
        let mut child_a = make_mock_child();
        child_a.expect_on_focus_change().times(0).return_const(());
        frame.insert("a", child_a);
        frame.set_current("a");
    }

    #[test]
    fn test_event_routes_to_current_when_no_handler() {
        let mut frame = FrameLayout::<(), &str>::new();
        let mut child_a = make_mock_child();
        let mut child_b = make_mock_child();
        child_a.expect_event().times(0).return_const(());
        child_b.expect_event().times(1).return_const(());
        frame.insert("a", child_a);
        frame.insert("b", child_b);
        frame.set_current("b");
        frame.event(&mut ());
    }
}
