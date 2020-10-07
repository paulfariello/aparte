/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::fmt;
use uuid::Uuid;
use xmpp_parsers::Element;
use xmpp_parsers::Jid;
use xmpp_parsers::data_forms::{DataForm, DataFormType, Field, FieldType};
use xmpp_parsers::iq::Iq;
use xmpp_parsers::mam;
use xmpp_parsers::ns;

use crate::core::{Plugin, Aparte, Event};

pub struct MamPlugin {
}

impl MamPlugin {
    fn query(&self, with: Jid) -> Element {
        let id = Uuid::new_v4().to_hyphenated().to_string();

        let mut fields = Vec::new();

        fields.push(Field {
            var: "with".to_string(),
            type_: FieldType::TextSingle,
            label: None,
            required: false,
            options: vec![],
            values: vec![with.to_string()],
            media: vec![],
        });

        let form = DataForm {
            type_: DataFormType::Submit,
            form_type: Some(String::from(ns::MAM)),
            title: None,
            instructions: None,
            fields: fields,
        };

        let query = mam::Query {
            queryid: None,
            node: None,
            form: Some(form),
            set: None
        };

        let iq = Iq::from_set(id, query);
        iq.into()
    }
}

impl Plugin for MamPlugin {
    fn new() -> MamPlugin {
        MamPlugin { }
    }

    fn init(&mut self, _aparte: &mut Aparte) -> Result<(), ()> {
        Ok(())
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::Join(jid, _) => aparte.send(self.query(Jid::Bare(jid.clone().into()))),
            Event::Chat(jid) => aparte.send(self.query(jid.clone().into())),
            Event::LoadHistory(jid) => aparte.send(self.query(jid.clone().into())),
            _ => {},
        }
    }
}

impl fmt::Display for MamPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0313: Message Archive Management")
    }
}

