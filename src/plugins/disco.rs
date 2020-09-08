/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::fmt;

use crate::core::{Plugin, Aparte, Event};

#[allow(non_camel_case_types)]
pub struct Disco<'a> {
    features: Vec<&'a str>,
}

impl<'a> Disco<'a> {
    pub fn add_feature(&mut self, feature: &'a str) -> Result<(), ()> {
        debug!("Adding `{}` feature", feature);
        self.features.push(feature);

        Ok(())
    }
}

impl<'a> Plugin for Disco<'a> {
    fn new() -> Disco<'a> {
        Disco { features: Vec::new() }
    }

    fn init(&mut self, _aparte: &mut Aparte) -> Result<(), ()> {
        Ok(())
    }

    fn on_event(&mut self, _aparte: &mut Aparte, _event: &Event) {
    }
}

impl<'a> fmt::Display for Disco<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0030: Service Discovery")
    }
}
