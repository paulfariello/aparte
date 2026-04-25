use std::convert::TryFrom;
use std::thread;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use rstest::fixture;
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use xmpp_parsers::jid::Jid;

use tokio_xmpp::xmlstream::{accept_stream, StreamHeader, Timeouts, XmppStreamElement};
use xmpp_parsers::{
    bind::{BindFeature, BindResponse},
    bookmarks::{Conference, Storage},
    carbons::{Received, Sent},
    forwarding::Forwarded,
    iq::Iq,
    jid::BareJid,
    mam,
    message::{Id, Message, MessageType},
    minidom::Element,
    muc::user::{Affiliation, Item as MucItem, MucUser, Role},
    ns,
    presence::{Presence, Show, Type as PresenceType},
    pubsub::{event as pubsub_event, NodeName},
    roster::{Ask, Item as RosterItem, Roster, Subscription},
    rsm::{First, SetResult},
    sasl::{Nonza as SaslNonza, Success},
    stream_features::{SaslMechanisms, StreamFeatures},
};

use super::Harness;
use super::wait_for_screen;

const BOUND_JID: &str = "user@localhost/aparte_test";

// ---------------------------------------------------------------------------
// Mock XMPP server
// ---------------------------------------------------------------------------

struct MockServer {
    inject_tx: mpsc::UnboundedSender<XmppStreamElement>,
}

impl MockServer {
    fn inject(&self, stanza: XmppStreamElement) {
        self.inject_tx.send(stanza).expect("inject channel closed");
    }
}

fn bind_feature() -> BindFeature {
    BindFeature::try_from(Element::bare("bind", ns::BIND)).expect("BindFeature")
}

fn bind_response(jid: &str) -> BindResponse {
    let elem = Element::builder("bind", ns::BIND)
        .append(Element::builder("jid", ns::BIND).append(jid).build())
        .build();
    BindResponse::try_from(elem).expect("BindResponse")
}

async fn run_mock_server(
    listener: TcpListener,
    bound_jid: String,
    mut inject_rx: mpsc::UnboundedReceiver<XmppStreamElement>,
    roster_contacts: Vec<String>,
    respond_to_disco: bool,
    mam_archive: Vec<(String, String, String, String)>,
) {
    // Real XMPP servers echo roster results with from=user_bare_jid, matching
    // the `to` field in the client's request. IqResponseTracker stores by
    // (to, id) and looks up by (from, id), so both must be the bare JID.
    let bare_str = bound_jid.split('/').next().unwrap();
    let user_bare_jid: Jid = Jid::from(BareJid::new(bare_str).expect("valid bare jid"));
    let (socket, _addr) = listener.accept().await.expect("mock: accept");
    let io = tokio::io::BufReader::new(socket);

    // ── Phase 1: first stream, offer SASL PLAIN ──────────────────────────────
    let accepted = accept_stream(io, "jabber:client", Timeouts::default())
        .await
        .expect("mock: accept_stream");

    let pending = accepted
        .send_header(StreamHeader {
            from: Some("localhost".into()),
            to: None,
            id: Some("mock-stream-1".into()),
        })
        .await
        .expect("mock: send_header 1");

    let sasl_features = StreamFeatures {
        sasl_mechanisms: SaslMechanisms {
            mechanisms: vec!["PLAIN".to_string()],
        },
        ..Default::default()
    };
    let mut stream = pending
        .send_features::<XmppStreamElement>(&sasl_features)
        .await
        .expect("mock: send_features 1");

    loop {
        match stream.next().await {
            Some(Ok(XmppStreamElement::Sasl(_))) => break,
            Some(Ok(_)) => continue,
            other => panic!("mock: expected SASL auth, got {:?}", other),
        }
    }

    let success = XmppStreamElement::Sasl(SaslNonza::Success(Success { data: vec![] }));
    let accepted = stream
        .accept_reset(&success)
        .await
        .expect("mock: accept_reset");

    // ── Phase 2: second stream, offer resource binding (no SM) ───────────────
    let pending = accepted
        .send_header(StreamHeader {
            from: Some("localhost".into()),
            to: None,
            id: Some("mock-stream-2".into()),
        })
        .await
        .expect("mock: send_header 2");

    let bind_features = StreamFeatures {
        bind: Some(bind_feature()),
        ..Default::default()
    };
    let mut stream = pending
        .send_features::<XmppStreamElement>(&bind_features)
        .await
        .expect("mock: send_features 2");

    let bind_req_id = loop {
        match stream.next().await {
            Some(Ok(XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(iq)))) => {
                break iq.id().to_string();
            }
            Some(Ok(_)) => continue,
            other => panic!("mock: expected bind IQ, got {:?}", other),
        }
    };

    let bind_result = Iq::from_result(bind_req_id, Some(bind_response(&bound_jid)));
    stream
        .send(&XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(
            bind_result,
        )))
        .await
        .expect("mock: send bind result");

    // ── Phase 3: stanza exchange loop ────────────────────────────────────────
    loop {
        tokio::select! {
            biased;

            Some(stanza) = inject_rx.recv() => {
                if stream.send(&stanza).await.is_err() {
                    break;
                }
            }

            msg = stream.next() => {
                match msg {
                    Some(Ok(XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(iq)))) => {
                        match &iq {
                            Iq::Get { id, payload, .. } => {
                                if payload.is("query", ns::ROSTER) {
                                    let refs: Vec<&str> =
                                        roster_contacts.iter().map(|s| s.as_str()).collect();
                                    let resp = roster_result_iq(id, &user_bare_jid, &refs);
                                    let _ = stream.send(&resp).await;
                                } else if payload.is("query", ns::DISCO_INFO) && respond_to_disco {
                                    let resp = Iq::Result {
                                        from: None,
                                        to: None,
                                        id: id.to_string(),
                                        payload: None,
                                    };
                                    let _ = stream.send(&XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(resp))).await;
                                } else {
                                    // Silently ignore unrecognised IQ-gets
                                }
                            }
                            Iq::Set { id, payload, .. } => {
                                if payload.is("query", ns::MAM) && !mam_archive.is_empty() {
                                    // Extract queryid from the MAM query
                                    if let Ok(query) = mam::Query::try_from(payload.clone()) {
                                        let queryid = query.queryid.clone();
                                        for (msg_from, msg_to, msg_id, body) in &mam_archive {
                                            let result = mam_result_message(
                                                queryid.as_ref().map(|q| q.0.as_str()).unwrap_or(""),
                                                msg_id,
                                                msg_from,
                                                msg_to,
                                                msg_id,
                                                body,
                                            );
                                            let _ = stream.send(&result).await;
                                        }
                                    }
                                    let fin = mam_fin_iq(id, true);
                                    let _ = stream.send(&fin).await;
                                } else {
                                    // Ack all other IQ-sets so the client doesn't stall
                                    let ack = Iq::Result {
                                        from: None,
                                        to: None,
                                        id: id.to_string(),
                                        payload: None,
                                    };
                                    let _ = stream.send(&XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(ack))).await;
                                }
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(_)) => {}
                    None | Some(Err(_)) => break,
                }
            }
        }
    }
}

fn start_mock_server(
    rt: &Runtime,
    bound_jid: &str,
    roster_contacts: &[&str],
    respond_to_disco: bool,
    mam_archive: Vec<(String, String, String, String)>,
) -> (MockServer, u16) {
    let listener = rt.block_on(async {
        TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock port")
    });
    let port = listener.local_addr().unwrap().port();
    let (inject_tx, inject_rx) = mpsc::unbounded_channel::<XmppStreamElement>();
    rt.spawn(run_mock_server(
        listener,
        bound_jid.to_string(),
        inject_rx,
        roster_contacts.iter().map(|s| s.to_string()).collect(),
        respond_to_disco,
        mam_archive,
    ));
    (MockServer { inject_tx }, port)
}

// ---------------------------------------------------------------------------
// Stanza builders
// ---------------------------------------------------------------------------

pub fn chat_message(from: &str, to: &str, id: &str, body: &str) -> XmppStreamElement {
    let mut msg = Message::chat(Some(Jid::new(to).unwrap()));
    msg.from = Some(Jid::new(from).unwrap());
    msg.id = Some(Id(id.to_string()));
    msg.bodies.insert(Default::default(), body.to_string());
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg))
}

fn roster_result_iq(req_id: &str, from: &Jid, contacts: &[&str]) -> XmppStreamElement {
    let items = contacts
        .iter()
        .map(|jid| RosterItem {
            jid: BareJid::new(*jid).expect("valid JID"),
            name: None,
            subscription: Subscription::Both,
            ask: Ask::None,
            groups: vec![],
        })
        .collect();
    let roster = Roster { ver: None, items };
    // Set `from` to the user's bare JID, mirroring real XMPP server behaviour
    // for self-addressed IQs. IqResponseTracker stores by (to, id) and looks
    // up by (from, id); both must be the bare JID for the response to match.
    let iq = Iq::Result {
        from: Some(from.clone()),
        to: None,
        id: req_id.to_string(),
        payload: Some(roster.into()),
    };
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(iq))
}

pub fn carbon_received(
    outer_from: &str,
    outer_to: &str,
    inner_from: &str,
    inner_to: &str,
    id: &str,
    body: &str,
) -> XmppStreamElement {
    let mut inner = Message::chat(Some(Jid::new(inner_to).unwrap()));
    inner.from = Some(Jid::new(inner_from).unwrap());
    inner.id = Some(Id(id.to_string()));
    inner.bodies.insert(Default::default(), body.to_string());

    let received = Received {
        forwarded: Forwarded {
            delay: None,
            message: inner,
        },
    };

    let mut outer = Message::new_with_type(MessageType::Normal, Some(Jid::new(outer_to).unwrap()));
    outer.from = Some(Jid::new(outer_from).unwrap());
    outer.payloads.push(received.into());
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(outer))
}

pub fn carbon_sent(
    outer_from: &str,
    outer_to: &str,
    inner_from: &str,
    inner_to: &str,
    id: &str,
    body: &str,
) -> XmppStreamElement {
    let mut inner = Message::chat(Some(Jid::new(inner_to).unwrap()));
    inner.from = Some(Jid::new(inner_from).unwrap());
    inner.id = Some(Id(id.to_string()));
    inner.bodies.insert(Default::default(), body.to_string());

    let sent = Sent {
        forwarded: Forwarded {
            delay: None,
            message: inner,
        },
    };

    let mut outer = Message::new_with_type(MessageType::Normal, Some(Jid::new(outer_to).unwrap()));
    outer.from = Some(Jid::new(outer_from).unwrap());
    outer.payloads.push(sent.into());
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(outer))
}

pub fn contact_presence(
    from: &str,
    to: &str,
    show: Option<Show>,
    status: Option<&str>,
) -> XmppStreamElement {
    let mut presence = Presence::new(PresenceType::None);
    presence.from = Some(Jid::new(from).unwrap());
    presence.to = Some(Jid::new(to).unwrap());
    presence.show = show;
    if let Some(s) = status {
        presence.statuses.insert(Default::default(), s.to_string());
    }
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Presence(presence))
}

pub fn contact_offline_presence(from: &str, to: &str) -> XmppStreamElement {
    let mut presence = Presence::new(PresenceType::Unavailable);
    presence.from = Some(Jid::new(from).unwrap());
    presence.to = Some(Jid::new(to).unwrap());
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Presence(presence))
}

pub fn muc_join_presence(from: &str, to: &str, affiliation: Affiliation, role: Role) -> XmppStreamElement {
    let muc_user = MucUser {
        status: vec![],
        items: vec![MucItem::new(affiliation, role)],
        invite: None,
        decline: None,
    };
    let mut presence = Presence::new(PresenceType::None);
    presence.from = Some(Jid::new(from).unwrap());
    presence.to = Some(Jid::new(to).unwrap());
    presence.add_payload(muc_user);
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Presence(presence))
}

pub fn groupchat_message(from: &str, to: &str, id: &str, body: &str) -> XmppStreamElement {
    let mut msg = Message::new_with_type(MessageType::Groupchat, Some(Jid::new(to).unwrap()));
    msg.from = Some(Jid::new(from).unwrap());
    msg.id = Some(Id(id.to_string()));
    msg.bodies.insert(Default::default(), body.to_string());
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg))
}

pub fn room_subject_message(from: &str, to: &str, id: &str, subject: &str) -> XmppStreamElement {
    let mut msg = Message::new_with_type(MessageType::Groupchat, Some(Jid::new(to).unwrap()));
    msg.from = Some(Jid::new(from).unwrap());
    msg.id = Some(Id(id.to_string()));
    msg.subjects.insert(Default::default(), subject.to_string());
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg))
}

pub fn corrected_chat_message(
    from: &str,
    to: &str,
    new_id: &str,
    original_id: &str,
    body: &str,
) -> XmppStreamElement {
    use xmpp_parsers::message_correct::Replace;
    let mut msg = Message::chat(Some(Jid::new(to).unwrap()));
    msg.from = Some(Jid::new(from).unwrap());
    msg.id = Some(Id(new_id.to_string()));
    msg.bodies.insert(Default::default(), body.to_string());
    msg.payloads
        .push(Replace { id: Id(original_id.to_string()) }.into());
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg))
}

pub fn bookmarks_v1_push_event(
    from: &str,
    to: &str,
    rooms: &[(&str, &str, bool)],
) -> XmppStreamElement {
    let conferences: Vec<_> = rooms
        .iter()
        .map(|(jid, name, autojoin)| Conference {
            autojoin: *autojoin,
            jid: BareJid::new(*jid).unwrap(),
            name: Some(name.to_string()),
            nick: None,
            password: None,
        })
        .collect();
    let storage = Storage { conferences, urls: vec![] };
    let storage_elem: Element = storage.into();

    let item = pubsub_event::Item {
        id: None,
        publisher: None,
        payload: Some(storage_elem),
    };
    let event = pubsub_event::Event {
        payload: pubsub_event::Payload::Items {
            node: NodeName(ns::BOOKMARKS.to_string()),
            published: vec![item],
            retracted: vec![],
        },
    };
    let event_elem: Element = event.into();

    let mut msg = Message::new_with_type(MessageType::Headline, Some(Jid::new(to).unwrap()));
    msg.from = Some(Jid::new(from).unwrap());
    msg.payloads.push(event_elem);
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg))
}

fn mam_result_message(
    queryid: &str,
    archive_id: &str,
    from: &str,
    to: &str,
    msg_id: &str,
    body: &str,
) -> XmppStreamElement {
    let mut inner = Message::chat(Some(Jid::new(to).unwrap()));
    inner.from = Some(Jid::new(from).unwrap());
    inner.id = Some(Id(msg_id.to_string()));
    inner.bodies.insert(Default::default(), body.to_string());

    let result = mam::Result_ {
        id: archive_id.to_string(),
        queryid: if queryid.is_empty() {
            None
        } else {
            Some(mam::QueryId(queryid.to_string()))
        },
        forwarded: Forwarded {
            delay: None,
            message: inner,
        },
    };

    let mut outer = Message::new_with_type(MessageType::Normal, Some(Jid::new(to).unwrap()));
    outer.payloads.push(result.into());
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(outer))
}

fn mam_fin_iq(req_id: &str, complete: bool) -> XmppStreamElement {
    let fin = mam::Fin {
        complete,
        set: SetResult {
            first: Some(First {
                index: Some(0),
                item: "a1".to_string(),
            }),
            last: Some("a1".to_string()),
            count: None,
        },
    };
    let iq = Iq::from_result(req_id.to_string(), Some(fin));
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(iq))
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

pub struct XmppFixture {
    rt: Option<Runtime>,
    mock: MockServer,
    harness: Option<Harness>,
}

impl XmppFixture {
    pub fn new(roster: &[&str]) -> Self {
        Self::new_impl(roster, false, vec![])
    }

    pub fn new_with_disco(roster: &[&str]) -> Self {
        Self::new_impl(roster, true, vec![])
    }

    pub fn new_with_mam(
        roster: &[&str],
        archive: &[(&str, &str, &str, &str)],
    ) -> Self {
        let mam_archive = archive
            .iter()
            .map(|(f, t, i, b)| (f.to_string(), t.to_string(), i.to_string(), b.to_string()))
            .collect();
        Self::new_impl(roster, true, mam_archive)
    }

    fn new_impl(
        roster: &[&str],
        respond_to_disco: bool,
        mam_archive: Vec<(String, String, String, String)>,
    ) -> Self {
        let rt = Runtime::new().unwrap();
        let (mock, port) = start_mock_server(&rt, BOUND_JID, roster, respond_to_disco, mam_archive);
        let config = format!(
            "[accounts.test]\n\
             jid = \"user@localhost\"\n\
             server = \"127.0.0.1\"\n\
             port = {port}\n\
             autoconnect = true\n\
             password = \"test\"\n"
        );
        let harness = Harness::spawn(&config, &[("APARTE_INSECURE_XMPP", "1")]);
        assert!(
            wait_for_screen(&harness, "Connected as", Duration::from_secs(15)),
            "aparte did not connect within 15s",
        );
        Self { rt: Some(rt), mock, harness: Some(harness) }
    }

    pub fn inject(&self, stanza: XmppStreamElement) {
        self.mock.inject(stanza);
    }

    pub fn send_command(&self, cmd: &str) {
        self.harness.as_ref().unwrap().send_command(cmd);
    }

    pub fn wait_for(&self, needle: &str, timeout: Duration) -> bool {
        wait_for_screen(self.harness.as_ref().unwrap(), needle, timeout)
    }

    pub fn switch_window(&self, name: &str) {
        thread::sleep(Duration::from_millis(400));
        self.harness.as_ref().unwrap().send_command(&format!("/win {name}"));
    }

    pub fn snapshot(&self) -> vt100::Parser {
        self.harness.as_ref().unwrap().snapshot()
    }
}

impl Drop for XmppFixture {
    fn drop(&mut self) {
        if let Some(h) = self.harness.take() {
            h.shutdown();
        }
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

#[fixture]
pub fn xmpp() -> XmppFixture {
    XmppFixture::new(&[])
}

#[fixture]
pub fn xmpp_with_contact() -> XmppFixture {
    XmppFixture::new(&["contact@localhost"])
}

#[fixture]
pub fn xmpp_with_disco() -> XmppFixture {
    XmppFixture::new_with_disco(&[])
}

#[fixture]
pub fn xmpp_with_contact_and_disco() -> XmppFixture {
    XmppFixture::new_with_disco(&["contact@localhost"])
}
