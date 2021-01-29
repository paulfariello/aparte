/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use chrono::{DateTime, FixedOffset};
use std::convert::TryFrom;
use std::fmt;
use uuid::Uuid;
use xmpp_parsers::data_forms::{DataForm, DataFormType, Field, FieldType};
use xmpp_parsers::iq::{Iq, IqType};
use xmpp_parsers::mam;
use xmpp_parsers::ns;
use xmpp_parsers::rsm::SetQuery;
use xmpp_parsers::Element;
use xmpp_parsers::Jid;

use crate::account::Account;
use crate::core::{Aparte, Event, Plugin};

pub struct MamPlugin {}

impl MamPlugin {
    fn query(
        &self,
        jid: Jid,
        start: Option<DateTime<FixedOffset>>,
        end: Option<DateTime<FixedOffset>>,
        before: Option<String>,
        after: Option<String>,
    ) -> Element {
        let id = Uuid::new_v4().to_hyphenated().to_string();

        let mut fields = Vec::new();

        if let Some(end) = end {
            let datetime = end.to_rfc3339();
            fields.push(Field {
                var: "end".to_string(),
                type_: FieldType::default(),
                label: None,
                required: false,
                options: vec![],
                values: vec![datetime],
                media: vec![],
            });
        }

        if let Some(start) = start {
            let datetime = start.to_rfc3339();
            fields.push(Field {
                var: "start".to_string(),
                type_: FieldType::default(),
                label: None,
                required: false,
                options: vec![],
                values: vec![datetime],
                media: vec![],
            });
        }

        let form = DataForm {
            type_: DataFormType::Submit,
            form_type: Some(String::from(ns::MAM)),
            title: None,
            instructions: None,
            fields: fields,
        };

        // TODO first query should have a <before/>
        let set = SetQuery {
            max: Some(100),
            after,
            before,
            index: None,
        };

        let query = mam::Query {
            queryid: Some(mam::QueryId(id.clone())),
            node: None,
            form: Some(form),
            set: Some(set),
        };

        let iq = Iq::from_set(id, query).with_to(jid);
        iq.into()
    }

    fn handle_result(&self, aparte: &mut Aparte, account: &Account, result: mam::Result_) {
        match (result.forwarded.delay, result.forwarded.stanza) {
            (Some(delay), Some(message)) => {
                aparte.handle_message(account.clone(), message, Some(delay));
            }
            _ => {}
        }
    }

    fn handle_fin(&self, aparte: &mut Aparte, account: &Account, from: Jid, fin: mam::Fin) {
        if fin.complete == mam::Complete::False {
            if let Some(start) = fin.set.first {
                info!("Continuing MAM retrieval for {}", from);
                aparte.send(account, self.query(from, None, None, Some(start), None));
            }
        }
    }
}

impl Plugin for MamPlugin {
    fn new() -> MamPlugin {
        MamPlugin {}
    }

    fn init(&mut self, _aparte: &mut Aparte) -> Result<(), ()> {
        Ok(())
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            //Event::Join {
            //    account, channel, ..
            //} => aparte.send(account, self.query(Jid::Bare(channel.clone().into()), None)),
            //Event::Chat { account, contact } => {
            //    aparte.send(account, self.query(contact.clone().into(), None))
            //}
            Event::LoadHistory { account, jid, from } => aparte.send(
                account,
                self.query(
                    jid.clone().into(),
                    None,
                    from.clone(),
                    Some("".to_string()),
                    None,
                ),
            ),
            Event::MessagePayload(account, payload, _delay) => {
                if let Ok(result) = mam::Result_::try_from(payload.clone()) {
                    self.handle_result(aparte, account, result);
                }
            }
            Event::Iq(account, iq) => {
                // TODO match query id
                if let IqType::Result(Some(payload)) = &iq.payload {
                    if let (Some(from), Ok(fin)) = (&iq.from, mam::Fin::try_from(payload.clone())) {
                        self.handle_fin(aparte, account, from.clone(), fin);
                    }
                }
            }
            _ => {}
        }
    }
}

impl fmt::Display for MamPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0313: Message Archive Management")
    }
}
