/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use chrono::{DateTime, FixedOffset};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use uuid::Uuid;
use xmpp_parsers::data_forms::{DataForm, DataFormType, Field, FieldType};
use xmpp_parsers::delay::Delay;
use xmpp_parsers::iq::{Iq, IqType};
use xmpp_parsers::mam;
use xmpp_parsers::message::Message as XmppParsersMessage;
use xmpp_parsers::ns;
use xmpp_parsers::rsm::SetQuery;
use xmpp_parsers::{BareJid, Jid};

use crate::account::Account;
use crate::core::{Aparte, Event, ModTrait};

struct Query {
    jid: BareJid,
    with: Option<BareJid>,
    from: Option<DateTime<FixedOffset>>,
    count: usize,
}

impl Query {
    pub fn start(&self) -> (String, Iq) {
        // Start with before set to empty string in order to force xmpp_parser to generate a
        // <before/> element and to ensure we get last page first
        self.query(Some("".to_string()))
    }

    pub fn cont(&self, before: String) -> (String, Iq) {
        self.query(Some(before))
    }

    fn query(&self, before: Option<String>) -> (String, Iq) {
        let mut fields = Vec::new();

        if let Some(end) = self.from {
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

        if let Some(with) = &self.with {
            fields.push(Field {
                var: "with".to_string(),
                type_: FieldType::default(),
                label: None,
                required: false,
                options: vec![],
                values: vec![with.to_string()],
                media: vec![],
            });
        }

        let form = DataForm {
            type_: DataFormType::Submit,
            form_type: Some(String::from(ns::MAM)),
            title: None,
            instructions: None,
            fields,
        };

        let set = SetQuery {
            max: Some(self.count),
            after: None,
            before,
            index: None,
        };

        let queryid = Uuid::new_v4().hyphenated().to_string();
        let query = mam::Query {
            queryid: Some(mam::QueryId(queryid.clone())),
            node: None,
            form: Some(form),
            set: Some(set),
            flip_page: false,
        };

        let id = Uuid::new_v4().hyphenated().to_string();
        (
            queryid,
            Iq::from_set(id, query).with_to(Jid::from(self.jid.clone())),
        )
    }
}

#[derive(Default)]
pub struct MamMod {
    /// Queries indexed by queryid
    queries: HashMap<String, Query>,

    /// Mapping between iq ids and query ids
    iq2id: HashMap<String, String>,
}

impl MamMod {
    fn query(&mut self, aparte: &mut Aparte, account: &Account, query: Query) {
        let (queryid, iq) = query.start();
        self.queries.insert(queryid.clone(), query);
        self.iq2id.insert(iq.id.clone(), queryid);
        aparte.send(account, iq);
    }

    fn handle_result(&mut self, aparte: &mut Aparte, account: &Account, result: mam::Result_) {
        if let Some(id) = &result.queryid {
            if let Some(query) = self.queries.get_mut(&id.0) {
                query.count -= 1;
                if let (Some(delay), Some(message)) =
                    (result.forwarded.delay, result.forwarded.stanza)
                {
                    aparte.schedule(Event::RawMessage {
                        account: account.clone(),
                        message,
                        delay: Some(delay),
                        archive: true,
                    });
                }
            }
        }
    }

    fn handle_fin(&mut self, aparte: &mut Aparte, account: &Account, query: Query, fin: mam::Fin) {
        if fin.complete == mam::Complete::False {
            if let Some(start) = fin.set.first {
                log::info!(
                    "Continuing MAM retrieval for {} with {:?} from {:?}",
                    query.jid,
                    query.with.clone().map(|jid| jid.to_string()),
                    query.from
                );
                let (queryid, iq) = query.cont(start);
                self.queries.insert(queryid.clone(), query);
                self.iq2id.insert(iq.id.clone(), queryid);
                aparte.send(account, iq);
            }
        }
    }
}

impl ModTrait for MamMod {
    fn init(&mut self, _aparte: &mut Aparte) -> Result<(), ()> {
        Ok(())
    }

    fn can_handle_xmpp_message(
        &mut self,
        _aparte: &mut Aparte,
        _account: &Account,
        message: &XmppParsersMessage,
        _delay: &Option<Delay>,
    ) -> f64 {
        for payload in message.payloads.iter() {
            if mam::Result_::try_from(payload.clone()).is_ok() {
                return 1f64;
            }
        }
        0f64
    }

    fn handle_xmpp_message(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        message: &XmppParsersMessage,
        _delay: &Option<Delay>,
        _archive: bool,
    ) {
        for payload in message.payloads.iter() {
            if let Ok(result) = mam::Result_::try_from(payload.clone()) {
                self.handle_result(aparte, account, result);
            }
        }
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::Join {
                account, channel, ..
            } => {
                let query = Query {
                    jid: channel.to_bare(),
                    with: None,
                    from: None,
                    count: 100,
                };
                self.query(aparte, account, query);
            }
            Event::Chat { account, contact } => {
                let query = Query {
                    jid: account.to_bare(),
                    with: Some(contact.clone()),
                    from: None,
                    count: 100,
                };
                self.query(aparte, account, query);
            }
            Event::LoadChannelHistory { account, jid, from } => {
                let query = Query {
                    jid: jid.clone(),
                    with: None,
                    from: *from,
                    count: 100,
                };
                self.query(aparte, account, query);
            }
            Event::LoadChatHistory {
                account,
                contact,
                from,
            } => {
                let query = Query {
                    jid: account.to_bare(),
                    with: Some(contact.clone()),
                    from: *from,
                    count: 100,
                };
                self.query(aparte, account, query);
            }
            Event::Iq(account, iq) => {
                if let Some(id) = self.iq2id.remove(&iq.id) {
                    if let Some(query) = self.queries.remove(&id) {
                        if let IqType::Result(Some(payload)) = &iq.payload {
                            if let Ok(fin) = mam::Fin::try_from(payload.clone()) {
                                self.handle_fin(aparte, account, query, fin);
                            } else {
                                log::warn!("Incorrect IQ response for MAM query");
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

impl fmt::Display for MamMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0313: Message Archive Management")
    }
}
