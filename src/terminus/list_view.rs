/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use linked_hash_map::{Entry, LinkedHashMap};
use std::cell::{Cell, RefCell};
use std::cmp;
use std::collections::HashSet;
use std::fmt::{self};
use std::hash::Hash;
use std::io::Write;
use std::os::fd::AsFd;
use std::rc::Rc;

use super::{
    term_string_visible_len, term_string_visible_truncate, Dimension, DimensionSpec, LayoutParam,
    LayoutParams, Screen, View,
};

pub struct ListView<E, W, G, V>
where
    G: fmt::Display + Hash + Eq,
    V: fmt::Display + Hash + Eq,
{
    items: LinkedHashMap<Option<G>, HashSet<V>>,
    unique: bool,
    sort_item: Option<Box<dyn Fn(&V, &V) -> cmp::Ordering>>,
    #[allow(dead_code)]
    sort_group: Option<Box<dyn FnMut(&G, &G) -> cmp::Ordering>>,
    event_handler: Option<Rc<RefCell<Box<dyn FnMut(&mut Self, &mut E)>>>>,
    dirty: Cell<bool>,
    layout: LayoutParams,
}

impl<E, W, G, V> ListView<E, W, G, V>
where
    G: fmt::Display + Hash + Eq,
    V: fmt::Display + Hash + Eq,
{
    pub fn new() -> Self {
        Self {
            items: LinkedHashMap::new(),
            unique: false,
            sort_item: None,
            sort_group: None,
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

    pub fn with_layout(mut self, layout: LayoutParams) -> Self {
        self.layout = layout;
        self
    }

    pub fn with_none_group(mut self) -> Self {
        if let Entry::Vacant(vacant) = self.items.entry(None) {
            vacant.insert(HashSet::new());
        }
        self
    }

    pub fn with_unique_item(mut self) -> Self {
        self.unique = true;
        self
    }

    pub fn with_sort_item(mut self) -> Self
    where
        V: Ord,
    {
        self.sort_item = Some(Box::new(|a, b| a.cmp(b)));
        self
    }

    #[allow(unused)] // XXX Should be removed once terminus is in its own crate
    pub fn with_sort_item_by<F>(mut self, compare: F) -> Self
    where
        F: Fn(&V, &V) -> cmp::Ordering + 'static,
    {
        self.sort_item = Some(Box::new(compare));
        self
    }

    #[allow(unused)] // XXX Should be removed once terminus is in its own crate
    pub fn with_sort_group(mut self) -> Self
    where
        G: Ord,
    {
        self.sort_group = Some(Box::new(|a, b| a.cmp(b)));
        self
    }

    #[allow(unused)] // XXX Should be removed once terminus is in its own crate
    pub fn with_sort_group_by<F>(mut self, compare: F) -> Self
    where
        F: FnMut(&G, &G) -> cmp::Ordering + 'static,
    {
        self.sort_group = Some(Box::new(compare));
        self
    }

    #[allow(unused)] // XXX Should be removed once terminus is in its own crate
    pub fn add_group(&mut self, group: G) {
        if let Entry::Vacant(vacant) = self.items.entry(Some(group)) {
            vacant.insert(HashSet::new());
        }

        self.dirty.set(true);
    }

    pub fn insert(&mut self, item: V, group: Option<G>) {
        if self.unique {
            for (_, items) in self.items.iter_mut() {
                items.remove(&item);
            }
        }
        match self.items.entry(group) {
            Entry::Vacant(vacant) => {
                let mut items = HashSet::new();
                items.insert(item);
                vacant.insert(items);
            }
            Entry::Occupied(mut occupied) => {
                occupied.get_mut().replace(item);
            }
        }

        self.dirty.set(true);
    }

    pub fn remove(&mut self, item: V, group: Option<G>) -> Result<(), ()> {
        match self.items.entry(group) {
            Entry::Vacant(_) => Err(()),
            Entry::Occupied(mut occupied) => {
                self.dirty
                    .set(self.dirty.get() || occupied.get_mut().remove(&item));
                Ok(())
            }
        }
    }
}

impl<E, W, G, V> View<E, W> for ListView<E, W, G, V>
where
    W: Write + AsFd,
    G: fmt::Display + Hash + Eq,
    V: fmt::Display + Hash + Eq,
{
    fn measure(&mut self, dimension_spec: &mut DimensionSpec) {
        let layouts = self.get_layout();
        dimension_spec.width = match layouts.width {
            LayoutParam::MatchParent => dimension_spec.width,
            LayoutParam::WrapContent => {
                let mut width: u16 = 0;
                for (group, items) in &self.items {
                    if let Some(group) = group {
                        width =
                            cmp::max(width, term_string_visible_len(&format!("{group}")) as u16);
                    }

                    let indent = match group {
                        Some(_) => "  ",
                        None => "",
                    };

                    for item in items {
                        width = cmp::max(
                            width,
                            term_string_visible_len(&format!("{indent}{item}")) as u16,
                        );
                    }
                }

                match dimension_spec.width {
                    Some(width_spec) => Some(cmp::min(width_spec, width)),
                    None => Some(width),
                }
            }
            LayoutParam::Absolute(width) => match dimension_spec.width {
                Some(width_spec) => Some(cmp::min(width, width_spec)),
                None => Some(width),
            },
        };

        dimension_spec.height = match layouts.height {
            LayoutParam::MatchParent => dimension_spec.height,
            LayoutParam::WrapContent => {
                let mut height: u16 = 0;
                for (group, items) in &self.items {
                    if group.is_some() {
                        height += 1;
                    }

                    height += items.len() as u16;
                }

                match dimension_spec.height {
                    Some(height_spec) => Some(cmp::min(height_spec, height)),
                    None => Some(height),
                }
            }
            LayoutParam::Absolute(height) => match dimension_spec.height {
                Some(height_spec) => Some(cmp::min(height, height_spec)),
                None => Some(height),
            },
        };
    }

    fn render(&self, dimension: &Dimension, screen: &mut Screen<W>) {
        super::save_cursor!(screen);

        let mut y = dimension.y;
        let width: usize = dimension.width.into();

        for y in dimension.y..dimension.y + dimension.height {
            super::goto!(screen, dimension.x, y);
            for _ in dimension.x..dimension.x + dimension.width {
                super::vprint!(screen, " ");
            }

            super::goto!(screen, dimension.x, y);
        }

        for (group, items) in &self.items {
            if y > dimension.y + dimension.height {
                break;
            }

            super::goto!(screen, dimension.x, y);

            if group.is_some() {
                let mut disp = format!("{}", group.as_ref().unwrap());
                if term_string_visible_len(&disp) > width {
                    disp = term_string_visible_truncate(&disp, width, Some("…"));
                }
                super::vprint!(screen, "{}", disp);
                y += 1;
            }

            let mut items = items.iter().collect::<Vec<&V>>();
            if let Some(sort) = &self.sort_item {
                items.sort_by(|a, b| sort(*a, *b));
            }

            for item in items {
                if y > dimension.y + dimension.height {
                    break;
                }

                super::goto!(screen, dimension.x, y);

                let mut disp = match group {
                    Some(_) => format!("  {item}"),
                    None => format!("{item}"),
                };
                if term_string_visible_len(&disp) > width {
                    disp = term_string_visible_truncate(&disp, width, Some("…"));
                }
                super::vprint!(screen, "{}", disp);

                y += 1;
            }
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
