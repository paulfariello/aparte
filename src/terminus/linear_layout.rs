/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::cell::{Cell, RefCell};
use std::cmp;
use std::io::Write;
use std::os::fd::AsFd;
use std::rc::Rc;

use super::{Dimension, DimensionSpec, LayoutParam, LayoutParams, Screen, View};

#[derive(Debug, Clone, PartialEq)]
pub enum Orientation {
    Vertical,
    Horizontal,
}

pub struct Child<E, W> {
    dimension: Option<Dimension>,
    pub view: Box<dyn View<E, W>>,
    weight: u16,
}

/// Component with multiple ordered children sharing the same space.
/// Each child is placed according to the orientation of the component.
pub struct LinearLayout<E, W> {
    pub orientation: Orientation,
    pub children: Vec<Child<E, W>>,
    pub event_handler: Option<Rc<RefCell<Box<dyn FnMut(&mut Self, &mut E)>>>>,
    pub dirty: Cell<bool>,
    layouts: LayoutParams,
}

impl<E, W> LinearLayout<E, W>
where
    W: Write + AsFd,
{
    pub fn new(orientation: Orientation) -> Self {
        Self {
            orientation,
            children: Vec::new(),
            event_handler: None,
            dirty: Cell::new(true),
            layouts: LayoutParams {
                width: LayoutParam::MatchParent,
                height: LayoutParam::MatchParent,
            },
        }
    }

    pub fn push<T>(&mut self, view: T, weight: u16)
    where
        T: View<E, W> + 'static,
    {
        self.children.push(Child {
            dimension: None,
            view: Box::new(view),
            weight,
        });
    }

    pub fn with_event<F>(mut self, event_handler: F) -> Self
    where
        F: FnMut(&mut Self, &mut E) + 'static,
    {
        self.event_handler = Some(Rc::new(RefCell::new(Box::new(event_handler))));
        self
    }

    #[allow(unused)]
    pub fn with_layouts(mut self, layouts: LayoutParams) -> Self {
        self.layouts = layouts;
        self
    }

    pub fn iter_children_mut<'a>(
        &'a mut self,
    ) -> impl Iterator<Item = &'a mut Box<dyn View<E, W>>> {
        self.children.iter_mut().map(|child| &mut child.view)
    }

    #[allow(unused)]
    pub fn iter_children<'a>(&'a self) -> impl Iterator<Item = &'a Box<dyn View<E, W>>> {
        self.children.iter().map(|child| &child.view)
    }
}

impl<E, W> View<E, W> for LinearLayout<E, W>
where
    W: Write + AsFd,
{
    fn measure(&mut self, dimension_spec: &mut DimensionSpec) {
        let layouts = self.get_layout();
        dimension_spec.width = match layouts.width {
            LayoutParam::MatchParent => dimension_spec.width,
            LayoutParam::WrapContent => {
                todo!()
            }
            LayoutParam::Absolute(width) => match dimension_spec.width {
                Some(width_spec) => Some(cmp::min(width, width_spec)),
                None => Some(width),
            },
        };

        dimension_spec.height = match layouts.height {
            LayoutParam::MatchParent => dimension_spec.height,
            LayoutParam::WrapContent => {
                todo!()
            }
            LayoutParam::Absolute(height) => match dimension_spec.height {
                Some(height_spec) => Some(cmp::min(height, height_spec)),
                None => Some(height),
            },
        };
    }

    fn layout(&mut self, dimension_spec: &DimensionSpec, x: u16, y: u16) -> Dimension {
        /* Layout with the following steps:
         *
         *  - Get dimension from spec
         *  - Compute min dimension from children
         *  - Split remaining space for each child that don't have strong size requirement
         *    (answered 0 to first measure pass)
         *  - Set dimension for each children
         *
         *  Algorithm should be:
         *
         *  - Satisfy hard behavior first (thus in the following order):
         *    - Absolute -> give them what they want
         *    - WrapContent -> give them at min(what they want, what's available)
         *    - MatchParent -> give them what's available
         */

        let max_width = dimension_spec.width;
        let max_height = dimension_spec.height;

        let mut children_infos: Vec<(LayoutParams, DimensionSpec, &mut Option<Dimension>, &mut _)> =
            self.children
                .iter_mut()
                .map(|child| {
                    let mut child_dimension_spec = dimension_spec.clone();
                    child.view.measure(&mut child_dimension_spec);
                    (
                        child.view.get_layout(),
                        child_dimension_spec,
                        &mut child.dimension,
                        &mut child.view,
                    )
                })
                .collect();

        // Gather requested sizes
        let mut min_width = 0;
        let mut min_height = 0;
        let mut free_width_count = 0;
        let mut free_height_count = 0;

        log::debug!("layout LinearLayout");
        for (child_layouts, child_dimension_spec, _, _) in children_infos.iter_mut() {
            child_dimension_spec.width = match &child_layouts.width {
                LayoutParam::WrapContent => todo!(),
                LayoutParam::MatchParent => None,
                LayoutParam::Absolute(n) => Some(*n),
            };
            log::debug!(
                "computed requested_width: {:?} ({:?})",
                child_dimension_spec.width,
                child_layouts.width
            );

            if child_dimension_spec.width.is_none() {
                free_width_count += 1;
            }

            child_dimension_spec.height = match &child_layouts.height {
                LayoutParam::WrapContent => todo!(),
                LayoutParam::MatchParent => None,
                LayoutParam::Absolute(n) => Some(*n),
            };

            if child_dimension_spec.height.is_none() {
                free_height_count += 1;
            }

            match self.orientation {
                Orientation::Vertical => {
                    if let Some(requested_width) = child_dimension_spec.width {
                        min_width = cmp::max(min_width, requested_width);
                    }
                    if let Some(requested_height) = child_dimension_spec.height {
                        min_height += requested_height;
                    }
                }
                Orientation::Horizontal => {
                    if let Some(requested_width) = child_dimension_spec.width {
                        min_width += requested_width;
                    }
                    if let Some(requested_height) = child_dimension_spec.height {
                        min_height = cmp::max(min_height, requested_height);
                    }
                }
            }
        }

        // Compute remaining free sizes
        let remaining_width = match max_width {
            Some(max_width) => max_width - min_width,
            None => 0,
        };

        let remaining_height = match max_height {
            Some(max_height) => max_height - min_height,
            None => 0,
        };

        // Split remaining space to children that don't know their size
        let splitted_width = match self.orientation {
            Orientation::Vertical => max_width,
            Orientation::Horizontal => Some(match free_width_count {
                0 => 0,
                count => remaining_width / count as u16,
            }),
        };
        let splitted_height = match self.orientation {
            Orientation::Vertical => Some(match free_height_count {
                0 => 0,
                count => remaining_height / count as u16,
            }),
            Orientation::Horizontal => max_height,
        };

        // Layout children
        let mut child_x = x;
        let mut child_y = y;
        for (_, child_dimension_spec, child_dimension, child_view) in children_infos.iter_mut() {
            log::debug!("Layout LinearLayout requested: {:?}", child_dimension_spec);
            let width_spec = match child_dimension_spec.width {
                Some(w) => Some(w),
                None => splitted_width,
            };
            log::debug!("width_spec: {:?}", width_spec);

            child_dimension_spec.width = match child_dimension_spec.width {
                Some(w) => Some(w),
                None => splitted_width,
            };
            child_dimension_spec.height = match child_dimension_spec.height {
                Some(h) => Some(h),
                None => splitted_height,
            };

            log::debug!(
                "Layout LinearLayout child with spec: {:?}",
                child_dimension_spec
            );
            child_dimension.replace(child_view.layout(child_dimension_spec, child_x, child_y));
            match self.orientation {
                Orientation::Vertical => child_y += child_dimension.as_ref().unwrap().height,
                Orientation::Horizontal => child_x += child_dimension.as_ref().unwrap().width,
            }
        }

        assert!(child_y <= y + dimension_spec.height.unwrap());
        assert!(child_x <= x + dimension_spec.width.unwrap());

        // Set dirty to ensure all children are rendered on next render
        self.dirty.set(true);

        dimension_spec.layout(x, y)
    }

    fn render(&self, _dimension: &Dimension, screen: &mut Screen<W>) {
        for child in self.children.iter() {
            if self.dirty.get() || child.view.is_dirty() {
                child
                    .view
                    .render(&child.dimension.as_ref().unwrap(), screen);
            }
        }
        self.dirty.set(false);
    }

    fn is_layout_dirty(&self) -> bool {
        match self.dirty.get() {
            true => true,
            _ => {
                let mut dirty = false;
                for child in self.children.iter() {
                    dirty |= child.view.is_layout_dirty()
                }
                dirty
            }
        }
    }

    fn is_dirty(&self) -> bool {
        match self.dirty.get() {
            true => true,
            _ => {
                let mut dirty = false;
                for child in self.children.iter() {
                    dirty |= child.view.is_dirty()
                }
                dirty
            }
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

    fn get_layout(&self) -> LayoutParams {
        self.layouts.clone()
    }
}

#[cfg(test)]
mod tests {
    use mockall::predicate::*;
    use std::fs::File;

    use super::*;
    use crate::terminus::MockView;

    #[test]
    fn test_linear_layout_layout_children_evenly() {
        // Given
        let mut layout = LinearLayout::<(), File>::new(Orientation::Vertical);

        let mut first_view = MockView::new();
        first_view
            .expect_layout()
            .with(eq(DimensionSpec::default()), eq(1), eq(1));
        first_view
            .expect_measure()
            .with(eq(DimensionSpec::default()));
        let mut second_view = MockView::new();
        second_view
            .expect_layout()
            .with(eq(DimensionSpec::default()), eq(1), eq(1));
        second_view
            .expect_measure()
            .with(eq(DimensionSpec::default()));

        layout.push(first_view, 1);
        layout.push(second_view, 1);

        let dimension_spec = DimensionSpec::default();

        // When
        layout.layout(&dimension_spec, 1, 1);

        //Then
    }
}
