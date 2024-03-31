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
    term_string_visible_len, term_string_visible_truncate, Dimensions, LayoutParam, LayoutParams,
    MeasureSpec, MeasureSpecs, RequestedDimension, RequestedDimensions, Screen, View,
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
    layouts: LayoutParams,
    dimensions: Option<Dimensions>,
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
    fn measure(&self, measure_specs: &MeasureSpecs) -> RequestedDimensions {
        let max_width = match self.layouts.width {
            LayoutParam::MatchParent | LayoutParam::WrapContent => self
                .items
                .iter()
                .flat_map(|(group, items)| {
                    let indent = match group {
                        Some(_) => "  ",
                        None => "",
                    };

                    group
                        .iter()
                        .map(|group| term_string_visible_len(&format!("{group}")) as u16)
                        .chain(items.iter().map(move |item| {
                            term_string_visible_len(&format!("{indent}{item}")) as u16
                        }))
                })
                .max()
                .unwrap_or(0),
            LayoutParam::Absolute(_) => 0, // We don't care,
        };

        let max_height = match self.layouts.height {
            LayoutParam::MatchParent | LayoutParam::WrapContent => self
                .items
                .iter()
                .flat_map(|(group, items)| group.iter().map(|_| ()).chain(items.iter().map(|_| ())))
                .count() as u16,
            LayoutParam::Absolute(_) => 0, // We don't care,
        };

        RequestedDimensions {
            width: RequestedDimension::Absolute(match (self.layouts.width, measure_specs.width) {
                (LayoutParam::MatchParent, MeasureSpec::Unspecified) => max_width,
                (LayoutParam::MatchParent, MeasureSpec::AtMost(at_most_width)) => at_most_width,
                (LayoutParam::WrapContent, MeasureSpec::Unspecified) => max_width,
                (LayoutParam::WrapContent, MeasureSpec::AtMost(at_most_width)) => {
                    std::cmp::min(max_width, at_most_width)
                }
                (LayoutParam::Absolute(absolute_width), MeasureSpec::Unspecified) => absolute_width,
                (LayoutParam::Absolute(absolute_width), MeasureSpec::AtMost(at_most_width)) => {
                    cmp::min(absolute_width, at_most_width)
                }
            }),
            height: RequestedDimension::Absolute(
                match (self.layouts.height, measure_specs.height) {
                    (LayoutParam::MatchParent, MeasureSpec::Unspecified) => max_height,
                    (LayoutParam::MatchParent, MeasureSpec::AtMost(at_most_height)) => {
                        at_most_height
                    }
                    (LayoutParam::WrapContent, MeasureSpec::Unspecified) => max_height,
                    (LayoutParam::WrapContent, MeasureSpec::AtMost(at_most_height)) => {
                        std::cmp::min(max_height, at_most_height)
                    }
                    (LayoutParam::Absolute(absolute_height), MeasureSpec::Unspecified) => {
                        absolute_height
                    }
                    (
                        LayoutParam::Absolute(absolute_height),
                        MeasureSpec::AtMost(at_most_height),
                    ) => cmp::min(absolute_height, at_most_height),
                },
            ),
        }
    }

    fn layout(&mut self, dimensions: &Dimensions) {
        log::debug!("layout {} {:?}", std::any::type_name::<Self>(), dimensions);
        self.dimensions.replace(dimensions.clone());
    }

    fn render(&self, screen: &mut Screen<W>) {
        log::debug!(
            "rendering {} at {:?}",
            std::any::type_name::<Self>(),
            self.dimensions
        );
        super::save_cursor!(screen);
        let dimensions = self.dimensions.as_ref().unwrap();

        // Clean space
        for top in dimensions.top..dimensions.top + dimensions.height {
            super::goto!(screen, dimensions.left, top);
            super::vprint!(screen, "{: <1$}", "", dimensions.width as usize);
        }

        // Draw items
        let mut top = dimensions.top;
        let usize_width = dimensions.width as usize;

        for (group, items) in &self.items {
            if top > dimensions.top + dimensions.height {
                break;
            }

            super::goto!(screen, dimensions.left, top);

            if group.is_some() {
                let mut disp = format!("{}", group.as_ref().unwrap());
                if term_string_visible_len(&disp) > usize_width {
                    disp = term_string_visible_truncate(&disp, usize_width, Some("…"));
                }
                super::vprint!(screen, "{}", disp);
                top += 1;
            }

            let mut items = items.iter().collect::<Vec<&V>>();
            if let Some(sort) = &self.sort_item {
                items.sort_by(|a, b| sort(*a, *b));
            }

            for item in items {
                if top > dimensions.top + dimensions.height {
                    break;
                }

                super::goto!(screen, dimensions.left, top);

                let mut disp = match group {
                    Some(_) => format!("  {item}"),
                    None => format!("{item}"),
                };
                if term_string_visible_len(&disp) > usize_width {
                    disp = term_string_visible_truncate(&disp, usize_width, Some("…"));
                }
                super::vprint!(screen, "{}", disp);

                top += 1;
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

    fn is_layout_dirty(&self) -> bool {
        (matches!(self.layouts.width, LayoutParam::WrapContent)
            || matches!(self.layouts.height, LayoutParam::WrapContent))
            && self.is_dirty()
    }

    fn is_dirty(&self) -> bool {
        self.dirty.get()
    }
}
