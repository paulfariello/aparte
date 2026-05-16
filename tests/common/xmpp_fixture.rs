use std::collections::HashMap;
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
    disco,
    forwarding::Forwarded,
    iq::Iq,
    jid::BareJid,
    legacy_omemo, mam,
    message::{Id, Message, MessageType},
    minidom::Element,
    muc::user::{Affiliation, Item as MucItem, MucUser, Role},
    ns,
    presence::{Presence, Show, Type as PresenceType},
    pubsub::{self, event as pubsub_event, ItemId, NodeName, PubSub},
    roster::{Ask, Item as RosterItem, Roster, Subscription},
    rsm::{First, SetResult},
    sasl::{Nonza as SaslNonza, Success},
    stream_features::{SaslMechanisms, StreamFeatures},
};

use super::wait_for_screen;
use super::Harness;

const BOUND_JID: &str = "user@localhost/aparte_test";

// ---------------------------------------------------------------------------
// OMEMO mock support
// ---------------------------------------------------------------------------

/// Config passed to the mock server enabling OMEMO-aware responses.
pub struct OmemoMockConfig {
    /// Fake devices per contact: bare JID string → Vec<(device_id, Bundle)>
    pub contact_devices: HashMap<String, Vec<(u32, legacy_omemo::Bundle)>>,
    /// Receives (device_id, Bundle) when aparte publishes its own bundle
    pub bundle_tx: mpsc::UnboundedSender<(u32, legacy_omemo::Bundle)>,
    /// Receives Message stanzas sent by aparte (for checking encryption)
    pub stanza_tx: mpsc::UnboundedSender<Message>,
}

/// Receive-side handles returned to the test when using OMEMO fixtures.
pub struct OmemoCapture {
    pub bundle_rx: mpsc::UnboundedReceiver<(u32, legacy_omemo::Bundle)>,
    pub stanza_rx: mpsc::UnboundedReceiver<Message>,
}

impl OmemoCapture {
    pub fn recv_bundle(&mut self, timeout: Duration) -> (u32, legacy_omemo::Bundle) {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Ok(b) = self.bundle_rx.try_recv() {
                return b;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for own bundle publish"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn recv_message_matching(
        &mut self,
        predicate: impl Fn(&Message) -> bool,
        timeout: Duration,
    ) -> Option<Message> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            while let Ok(msg) = self.stanza_rx.try_recv() {
                if predicate(&msg) {
                    return Some(msg);
                }
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

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
    omemo_cfg: Option<OmemoMockConfig>,
    mam_query_tx: Option<mpsc::UnboundedSender<()>>,
    muc_with_sid: Vec<String>,
    // Groupchat MAM archive for MUC queries: (stanza_id, from_full_jid, msg_id, body)
    muc_mam_archive: Vec<(String, String, String, String)>,
    // Chat reactions in MAM: (from, to, referenced_msg_id, emoji)
    mam_reaction_archive: Vec<(String, String, String, String)>,
    outgoing_msg_tx: mpsc::UnboundedSender<Message>,
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
                    Some(Ok(XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(xmpp_msg)))) => {
                        let _ = outgoing_msg_tx.send(xmpp_msg.clone());
                        if let Some(cfg) = &omemo_cfg {
                            let _ = cfg.stanza_tx.send(xmpp_msg);
                        }
                    }
                    Some(Ok(XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(iq)))) => {
                        match &iq {
                            Iq::Get { id, payload, to, .. } => {
                                if payload.is("query", ns::ROSTER) {
                                    let refs: Vec<&str> =
                                        roster_contacts.iter().map(|s| s.as_str()).collect();
                                    let resp = roster_result_iq(id, &user_bare_jid, &refs);
                                    let _ = stream.send(&resp).await;
                                } else if payload.is("query", ns::DISCO_INFO) {
                                    let to_str = to.as_ref().map(|j| j.to_string()).unwrap_or_default();
                                    if muc_with_sid.contains(&to_str) {
                                        // Respond with XEP-0359 (Stanza IDs) support for this MUC.
                                        // IqResponseTracker matches by (from, id); from must equal the request's to.
                                        let disco_result = disco::DiscoInfoResult {
                                            node: None,
                                            identities: vec![],
                                            features: vec![disco::Feature::new(ns::SID)],
                                            extensions: vec![],
                                        };
                                        let resp = Iq::Result {
                                            from: to.clone(),
                                            to: None,
                                            id: id.to_string(),
                                            payload: Some(disco_result.into()),
                                        };
                                        let _ = stream.send(&XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(resp))).await;
                                    } else if respond_to_disco {
                                        let resp = Iq::Result {
                                            from: None,
                                            to: None,
                                            id: id.to_string(),
                                            payload: None,
                                        };
                                        let _ = stream.send(&XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(resp))).await;
                                    }
                                } else if payload.is("pubsub", ns::PUBSUB) {
                                    if let Ok(PubSub::Items(items)) = PubSub::try_from(payload.clone()) {
                                        let node = items.node.0.as_str();
                                        let target = to.as_ref().map(|j| j.to_string()).unwrap_or_default();
                                        if node == ns::LEGACY_OMEMO_DEVICELIST {
                                            let resp = omemo_devicelist_iq(id, &items.node, &target, &omemo_cfg);
                                            let _ = stream.send(&resp).await;
                                        } else if node.starts_with(ns::LEGACY_OMEMO_BUNDLES) {
                                            let device_id: u32 = node.rsplit(':').next()
                                                .and_then(|s| s.parse().ok()).unwrap_or(0);
                                            let resp = omemo_bundle_iq(id, &items.node, &target, device_id, &omemo_cfg);
                                            let _ = stream.send(&resp).await;
                                        }
                                        // else: ignore other pubsub Gets
                                    }
                                }
                                // else: silently ignore other unrecognised IQ-gets
                            }
                            Iq::Set { id, payload, to, .. } => {
                                // IqResponseTracker matches by (from, id); echo request's `to` as `from`.
                                let from = to.clone();
                                if payload.is("query", ns::MAM) {
                                    if let Some(tx) = &mam_query_tx {
                                        let _ = tx.send(());
                                    }
                                    let target = to.as_ref().map(|j| j.to_string()).unwrap_or_default();
                                    let is_muc = muc_with_sid.contains(&target);
                                    if let Ok(query) = mam::Query::try_from(payload.clone()) {
                                        let queryid = query.queryid.clone();
                                        let qid = queryid.as_ref().map(|q| q.0.as_str()).unwrap_or("");
                                        if is_muc {
                                            for (archive_id, from_full, msg_id, body) in &muc_mam_archive {
                                                let result = mam_result_groupchat(
                                                    qid,
                                                    archive_id,
                                                    from_full,
                                                    &target,
                                                    msg_id,
                                                    archive_id,
                                                    body,
                                                );
                                                let _ = stream.send(&result).await;
                                            }
                                        } else {
                                            for (msg_from, msg_to, msg_id, body) in &mam_archive {
                                                let result = mam_result_message(
                                                    qid,
                                                    msg_id,
                                                    msg_from,
                                                    msg_to,
                                                    msg_id,
                                                    body,
                                                );
                                                let _ = stream.send(&result).await;
                                            }
                                            for (rxn_from, rxn_to, ref_id, emoji) in &mam_reaction_archive {
                                                let result = mam_result_chat_reaction(
                                                    qid,
                                                    rxn_from,
                                                    rxn_to,
                                                    ref_id,
                                                    emoji,
                                                );
                                                let _ = stream.send(&result).await;
                                            }
                                        }
                                    }
                                    let fin = mam_fin_iq(id, &target, true);
                                    let _ = stream.send(&fin).await;
                                } else if payload.is("pubsub", ns::PUBSUB) {
                                    if let Ok(pubsub) = PubSub::try_from(payload.clone()) {
                                        match pubsub {
                                            PubSub::Subscribe { subscribe: Some(sub), .. } => {
                                                let resp = omemo_subscription_iq(id, &sub, from.as_ref());
                                                let _ = stream.send(&resp).await;
                                            }
                                            PubSub::Publish { ref publish, .. }
                                                if publish.node.0.starts_with(ns::LEGACY_OMEMO_BUNDLES) =>
                                            {
                                                if let Some(cfg) = &omemo_cfg {
                                                    if let Some(item) = publish.items.first() {
                                                        if let Some(ref pl) = item.payload {
                                                            if let Ok(bundle) = legacy_omemo::Bundle::try_from(pl.clone()) {
                                                                let device_id: u32 = publish.node.0.rsplit(':').next()
                                                                    .and_then(|s| s.parse().ok()).unwrap_or(0);
                                                                let _ = cfg.bundle_tx.send((device_id, bundle));
                                                            }
                                                        }
                                                    }
                                                }
                                                let ack = Iq::Result { from, to: None, id: id.to_string(), payload: None };
                                                let _ = stream.send(&XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(ack))).await;
                                            }
                                            _ => {
                                                // Ack all other pubsub Sets
                                                let ack = Iq::Result { from, to: None, id: id.to_string(), payload: None };
                                                let _ = stream.send(&XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(ack))).await;
                                            }
                                        }
                                    } else {
                                        let ack = Iq::Result { from, to: None, id: id.to_string(), payload: None };
                                        let _ = stream.send(&XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(ack))).await;
                                    }
                                } else {
                                    // Ack all other IQ-sets so the client doesn't stall
                                    let ack = Iq::Result {
                                        from,
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
    omemo_cfg: Option<OmemoMockConfig>,
    mam_query_tx: Option<mpsc::UnboundedSender<()>>,
    muc_with_sid: Vec<String>,
    muc_mam_archive: Vec<(String, String, String, String)>,
    mam_reaction_archive: Vec<(String, String, String, String)>,
) -> (MockServer, u16, mpsc::UnboundedReceiver<Message>) {
    let listener = rt.block_on(async {
        TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock port")
    });
    let port = listener.local_addr().unwrap().port();
    let (inject_tx, inject_rx) = mpsc::unbounded_channel::<XmppStreamElement>();
    let (outgoing_msg_tx, outgoing_msg_rx) = mpsc::unbounded_channel::<Message>();
    rt.spawn(run_mock_server(
        listener,
        bound_jid.to_string(),
        inject_rx,
        roster_contacts.iter().map(|s| s.to_string()).collect(),
        respond_to_disco,
        mam_archive,
        omemo_cfg,
        mam_query_tx,
        muc_with_sid,
        muc_mam_archive,
        mam_reaction_archive,
        outgoing_msg_tx,
    ));
    (MockServer { inject_tx }, port, outgoing_msg_rx)
}

// ---------------------------------------------------------------------------
// OMEMO IQ response builders
// ---------------------------------------------------------------------------

fn omemo_devicelist_iq(
    req_id: &str,
    node: &NodeName,
    target_jid: &str,
    omemo_cfg: &Option<OmemoMockConfig>,
) -> XmppStreamElement {
    let devices: Vec<legacy_omemo::Device> = omemo_cfg
        .as_ref()
        .and_then(|c| c.contact_devices.get(target_jid))
        .map(|devs| {
            devs.iter()
                .map(|(id, _)| legacy_omemo::Device { id: *id })
                .collect()
        })
        .unwrap_or_default();
    let device_list = legacy_omemo::DeviceList { devices };
    let item = pubsub::pubsub::Item {
        id: Some(ItemId("current".into())),
        publisher: None,
        payload: Some(device_list.into()),
    };
    let pubsub_resp = PubSub::Items(pubsub::pubsub::Items {
        node: node.clone(),
        max_items: None,
        subid: None,
        items: vec![item],
    });
    // IqResponseTracker matches by (from, id); from must equal original request's to.
    let from = Jid::new(target_jid).ok();
    let iq = Iq::Result {
        from,
        to: None,
        id: req_id.to_string(),
        payload: Some(pubsub_resp.into()),
    };
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(iq))
}

fn omemo_bundle_iq(
    req_id: &str,
    node: &NodeName,
    target_jid: &str,
    device_id: u32,
    omemo_cfg: &Option<OmemoMockConfig>,
) -> XmppStreamElement {
    let bundle = omemo_cfg
        .as_ref()
        .and_then(|c| c.contact_devices.get(target_jid))
        .and_then(|devs| devs.iter().find(|(id, _)| *id == device_id))
        .map(|(_, b)| b.clone());
    // IqResponseTracker matches by (from, id); from must equal original request's to.
    let from = Jid::new(target_jid).ok();
    match bundle {
        Some(b) => {
            let item = pubsub::pubsub::Item {
                id: Some(ItemId("current".into())),
                publisher: None,
                payload: Some(b.into()),
            };
            let pubsub_resp = PubSub::Items(pubsub::pubsub::Items {
                node: node.clone(),
                max_items: None,
                subid: None,
                items: vec![item],
            });
            let iq = Iq::Result {
                from,
                to: None,
                id: req_id.to_string(),
                payload: Some(pubsub_resp.into()),
            };
            XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(iq))
        }
        None => {
            // Result(None) → OmemoEngine::get_bundle returns Ok(None) → triggers publish
            let iq = Iq::Result {
                from,
                to: None,
                id: req_id.to_string(),
                payload: None,
            };
            XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(iq))
        }
    }
}

fn omemo_subscription_iq(
    req_id: &str,
    sub: &pubsub::pubsub::Subscribe,
    to_jid: Option<&Jid>,
) -> XmppStreamElement {
    // Build raw XML string because SubscriptionElem has private fields in xmpp-parsers 0.22
    let node_str = sub.node.as_ref().map(|n| n.0.as_str()).unwrap_or("");
    let jid_str = sub.jid.to_string();
    let xml = format!(
        "<pubsub xmlns='{}'><subscription jid='{}' node='{}' subscription='subscribed'/></pubsub>",
        ns::PUBSUB,
        jid_str,
        node_str,
    );
    let pubsub_xml: Element = xml.parse().expect("valid subscription xml");
    // IqResponseTracker matches by (from, id); from must equal original request's to.
    let iq = Iq::Result {
        from: to_jid.cloned(),
        to: None,
        id: req_id.to_string(),
        payload: Some(pubsub_xml),
    };
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(iq))
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

pub fn carbon_sent_groupchat(
    outer_from: &str,
    outer_to: &str,
    inner_from: &str,
    inner_to: &str,
    id: &str,
    body: &str,
) -> XmppStreamElement {
    let mut inner =
        Message::new_with_type(MessageType::Groupchat, Some(Jid::new(inner_to).unwrap()));
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

pub fn carbon_received_groupchat(
    outer_from: &str,
    outer_to: &str,
    inner_from: &str,
    inner_to: &str,
    id: &str,
    body: &str,
) -> XmppStreamElement {
    let mut inner =
        Message::new_with_type(MessageType::Groupchat, Some(Jid::new(inner_to).unwrap()));
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

pub fn muc_join_presence(
    from: &str,
    to: &str,
    affiliation: Affiliation,
    role: Role,
) -> XmppStreamElement {
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

pub fn muc_join_presence_with_jid(
    from: &str,
    to: &str,
    affiliation: Affiliation,
    role: Role,
    real_jid: &str,
) -> XmppStreamElement {
    let mut item = MucItem::new(affiliation, role);
    item.jid = Jid::new(real_jid).unwrap().try_into_full().ok();
    let muc_user = MucUser {
        status: vec![],
        items: vec![item],
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
    msg.payloads.push(
        Replace {
            id: Id(original_id.to_string()),
        }
        .into(),
    );
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
    let storage = Storage {
        conferences,
        urls: vec![],
    };
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

/// A MAM result wrapping a chat reaction message (XEP-0444).
/// `ref_id` is the id of the original message being reacted to.
fn mam_result_chat_reaction(
    queryid: &str,
    from: &str,
    to: &str,
    ref_id: &str,
    emoji: &str,
) -> XmppStreamElement {
    use xmpp_parsers::minidom::Element;
    let reactions_elem: Element = format!(
        "<reactions xmlns='urn:xmpp:reactions:0' id='{ref_id}'><reaction>{emoji}</reaction></reactions>"
    )
    .parse()
    .expect("valid reactions element");

    let mut inner = Message::chat(Some(Jid::new(to).unwrap()));
    inner.from = Some(Jid::new(from).unwrap());
    inner.payloads.push(reactions_elem);

    let result = mam::Result_ {
        id: format!("rxn-{ref_id}"),
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

fn mam_fin_iq(req_id: &str, from_jid: &str, complete: bool) -> XmppStreamElement {
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
    let from = Jid::new(from_jid).ok();
    let iq = Iq::Result {
        from,
        to: None,
        id: req_id.to_string(),
        payload: Some(fin.into()),
    };
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(iq))
}

/// A MAM result wrapping a groupchat message with an embedded XEP-0359 stanza-id.
/// Used to simulate MUC MAM archives in tests.
fn mam_result_groupchat(
    queryid: &str,
    archive_id: &str,
    from_full: &str,
    room_bare: &str,
    msg_id: &str,
    stanza_id: &str,
    body: &str,
) -> XmppStreamElement {
    let sid_elem: Element =
        format!("<stanza-id xmlns='urn:xmpp:sid:0' by='{room_bare}' id='{stanza_id}'/>")
            .parse()
            .expect("valid stanza-id element");
    let mut inner =
        Message::new_with_type(MessageType::Groupchat, Some(Jid::new(room_bare).unwrap()));
    inner.from = Some(Jid::new(from_full).unwrap());
    inner.id = Some(Id(msg_id.to_string()));
    inner.bodies.insert(Default::default(), body.to_string());
    inner.payloads.push(sid_elem);

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

    let mut outer = Message::new_with_type(MessageType::Normal, Some(Jid::new(room_bare).unwrap()));
    outer.payloads.push(result.into());
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(outer))
}

/// A self-sent chat `<displayed>` marker (XEP-0333) referencing a message by ID.
/// Simulates another device (same JID, different resource) marking a chat message as read.
pub fn chat_displayed_marker(from: &str, to: &str, referenced_id: &str) -> XmppStreamElement {
    let marker: Element =
        format!("<displayed xmlns='urn:xmpp:chat-markers:0' id='{referenced_id}'/>",)
            .parse()
            .expect("valid displayed element");
    let mut msg = Message::chat(Some(Jid::new(to).unwrap()));
    msg.from = Some(Jid::new(from).unwrap());
    msg.payloads.push(marker);
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg))
}

/// A groupchat `<displayed>` marker (XEP-0333) from a specific occupant nick.
/// Simulates our own nick marking a MUC message as read (e.g. from another device).
pub fn groupchat_displayed_marker(from: &str, to: &str, referenced_id: &str) -> XmppStreamElement {
    let marker: Element =
        format!("<displayed xmlns='urn:xmpp:chat-markers:0' id='{referenced_id}'/>",)
            .parse()
            .expect("valid displayed element");
    let mut msg = Message::new_with_type(MessageType::Groupchat, Some(Jid::new(to).unwrap()));
    msg.from = Some(Jid::new(from).unwrap());
    msg.payloads.push(marker);
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg))
}

/// A groupchat message with an embedded XEP-0359 `<stanza-id>` element.
/// The stanza-id's `by` attribute is set to the room's bare JID (derived from `from`).
pub fn groupchat_message_with_stanza_id(
    from: &str,
    to: &str,
    id: &str,
    stanza_id: &str,
    body: &str,
) -> XmppStreamElement {
    let room_bare = Jid::new(from).unwrap().to_bare();
    let sid_elem: Element =
        format!("<stanza-id xmlns='urn:xmpp:sid:0' by='{room_bare}' id='{stanza_id}'/>",)
            .parse()
            .expect("valid stanza-id element");
    let mut msg = Message::new_with_type(MessageType::Groupchat, Some(Jid::new(to).unwrap()));
    msg.from = Some(Jid::new(from).unwrap());
    msg.id = Some(Id(id.to_string()));
    msg.bodies.insert(Default::default(), body.to_string());
    msg.payloads.push(sid_elem);
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg))
}

/// A chat message with an explicit XEP-0203 `<delay>` timestamp.
/// `stamp` must be an RFC 3339 / XEP-0082 timestamp string (e.g. `"2024-01-01T10:00:00Z"`).
pub fn chat_message_with_delay(
    from: &str,
    to: &str,
    id: &str,
    body: &str,
    stamp: &str,
) -> XmppStreamElement {
    let delay_elem: Element = format!("<delay xmlns='urn:xmpp:delay' stamp='{stamp}'/>")
        .parse()
        .expect("valid delay element");
    let mut msg = Message::chat(Some(Jid::new(to).unwrap()));
    msg.from = Some(Jid::new(from).unwrap());
    msg.id = Some(Id(id.to_string()));
    msg.bodies.insert(Default::default(), body.to_string());
    msg.payloads.push(delay_elem);
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg))
}

/// An OMEMO-encrypted chat message with no body and an optional `<delay>` timestamp.
/// The `encrypted_elem` should be an `<encrypted>` OMEMO element built from
/// `legacy_omemo::Encrypted`. Pass `delay_stamp=None` to omit the delay (the
/// receiver will use the current clock as the timestamp).
pub fn omemo_encrypted_chat_message(
    from: &str,
    to: &str,
    id: &str,
    encrypted_elem: xmpp_parsers::minidom::Element,
    delay_stamp: Option<&str>,
) -> XmppStreamElement {
    let mut msg = Message::chat(Some(Jid::new(to).unwrap()));
    msg.from = Some(Jid::new(from).unwrap());
    msg.id = Some(Id(id.to_string()));
    msg.payloads.push(encrypted_elem);
    if let Some(stamp) = delay_stamp {
        let delay_elem: Element = format!("<delay xmlns='urn:xmpp:delay' stamp='{stamp}'/>")
            .parse()
            .expect("valid delay element");
        msg.payloads.push(delay_elem);
    }
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg))
}

/// An OMEMO-encrypted groupchat message with no body (the server echo of a sent MUC OMEMO message).
/// `payloads` are copied verbatim from the sent stanza's payloads (the OMEMO `<encrypted>` element).
pub fn omemo_encrypted_groupchat_echo(
    from_full: &str,
    to: &str,
    id: &str,
    payloads: Vec<xmpp_parsers::minidom::Element>,
) -> XmppStreamElement {
    let mut msg = Message::new_with_type(MessageType::Groupchat, Some(Jid::new(to).unwrap()));
    msg.from = Some(Jid::new(from_full).unwrap());
    msg.id = Some(Id(id.to_string()));
    for p in payloads {
        msg.payloads.push(p);
    }
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg))
}

/// An OMEMO-encrypted chat message with no body, built from verbatim payloads.
/// Used to simulate a MAM replay of a sent 1:1 OMEMO message where the `from`
/// JID is our own account JID (aparte identifies it as an outgoing message).
pub fn omemo_encrypted_chat_replay(
    from: &str,
    to: &str,
    id: &str,
    payloads: Vec<xmpp_parsers::minidom::Element>,
    delay_stamp: Option<&str>,
) -> XmppStreamElement {
    let mut msg = Message::chat(Some(Jid::new(to).unwrap()));
    msg.from = Some(Jid::new(from).unwrap());
    msg.id = Some(Id(id.to_string()));
    for p in payloads {
        msg.payloads.push(p);
    }
    if let Some(stamp) = delay_stamp {
        let delay_elem: Element = format!("<delay xmlns='urn:xmpp:delay' stamp='{stamp}'/>")
            .parse()
            .expect("valid delay element");
        msg.payloads.push(delay_elem);
    }
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg))
}

/// A chat reaction message (XEP-0444). `referenced_id` is the id of the
/// original message being reacted to. `emojis` is the full set of emojis the
/// sender is expressing (empty slice clears all reactions from this sender).
pub fn chat_reaction(
    from: &str,
    to: &str,
    referenced_id: &str,
    emojis: &[&str],
) -> XmppStreamElement {
    let emojis_xml: String = emojis
        .iter()
        .map(|e| format!("<reaction>{e}</reaction>"))
        .collect();
    let reactions_elem: Element = format!(
        "<reactions xmlns='urn:xmpp:reactions:0' id='{referenced_id}'>{emojis_xml}</reactions>"
    )
    .parse()
    .expect("valid reactions element");
    let mut msg = Message::chat(Some(Jid::new(to).unwrap()));
    msg.from = Some(Jid::new(from).unwrap());
    msg.payloads.push(reactions_elem);
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg))
}

/// A groupchat reaction message (XEP-0444). In MUC context `referenced_id`
/// should be the stanza-id of the target message.
pub fn groupchat_reaction(
    from: &str,
    to: &str,
    referenced_id: &str,
    emojis: &[&str],
) -> XmppStreamElement {
    let emojis_xml: String = emojis
        .iter()
        .map(|e| format!("<reaction>{e}</reaction>"))
        .collect();
    let reactions_elem: Element = format!(
        "<reactions xmlns='urn:xmpp:reactions:0' id='{referenced_id}'>{emojis_xml}</reactions>"
    )
    .parse()
    .expect("valid reactions element");
    let mut msg = Message::new_with_type(MessageType::Groupchat, Some(Jid::new(to).unwrap()));
    msg.from = Some(Jid::new(from).unwrap());
    msg.payloads.push(reactions_elem);
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg))
}

/// XEP-0184 delivery receipt: the contact acknowledges receiving a message.
pub fn delivery_receipt(from: &str, to: &str, received_id: &str) -> XmppStreamElement {
    let receipt: Element = format!("<received xmlns='urn:xmpp:receipts' id='{received_id}'/>")
        .parse()
        .expect("valid XEP-0184 received element");
    let mut msg = Message::chat(Some(Jid::new(to).unwrap()));
    msg.from = Some(Jid::new(from).unwrap());
    msg.payloads.push(receipt);
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg))
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

pub struct XmppFixture {
    rt: Option<Runtime>,
    mock: MockServer,
    harness: Option<Harness>,
    mam_query_rx: mpsc::UnboundedReceiver<()>,
    outgoing_msg_rx: mpsc::UnboundedReceiver<Message>,
}

impl XmppFixture {
    pub fn new(roster: &[&str]) -> Self {
        Self::new_impl(roster, false, vec![], None, vec![], vec![], vec![]).0
    }

    pub fn new_with_disco(roster: &[&str]) -> Self {
        Self::new_impl(roster, true, vec![], None, vec![], vec![], vec![]).0
    }

    pub fn new_with_mam(roster: &[&str], archive: &[(&str, &str, &str, &str)]) -> Self {
        let mam_archive = archive
            .iter()
            .map(|(f, t, i, b)| (f.to_string(), t.to_string(), i.to_string(), b.to_string()))
            .collect();
        Self::new_impl(roster, true, mam_archive, None, vec![], vec![], vec![]).0
    }

    /// Create a fixture with a MAM archive that includes both a chat message and a
    /// reaction to it. `msg_archive` entries: (from, to, msg_id, body).
    /// `rxn_archive` entries: (from, to, referenced_msg_id, emoji).
    pub fn new_with_mam_and_reaction(
        roster: &[&str],
        msg_archive: &[(&str, &str, &str, &str)],
        rxn_archive: &[(&str, &str, &str, &str)],
    ) -> Self {
        let mam_archive = msg_archive
            .iter()
            .map(|(f, t, i, b)| (f.to_string(), t.to_string(), i.to_string(), b.to_string()))
            .collect();
        let mam_reaction_archive = rxn_archive
            .iter()
            .map(|(f, t, i, e)| (f.to_string(), t.to_string(), i.to_string(), e.to_string()))
            .collect();
        Self::new_impl(
            roster,
            true,
            mam_archive,
            None,
            vec![],
            vec![],
            mam_reaction_archive,
        )
        .0
    }

    /// Create fixture with OMEMO-aware mock server (empty contact devices).
    pub fn new_with_omemo(roster: &[&str]) -> (Self, OmemoCapture) {
        let (bundle_tx, bundle_rx) = mpsc::unbounded_channel();
        let (stanza_tx, stanza_rx) = mpsc::unbounded_channel();
        let cfg = OmemoMockConfig {
            contact_devices: HashMap::new(),
            bundle_tx,
            stanza_tx,
        };
        let (fixture, _) = Self::new_impl(roster, true, vec![], Some(cfg), vec![], vec![], vec![]);
        (
            fixture,
            OmemoCapture {
                bundle_rx,
                stanza_rx,
            },
        )
    }

    /// Create fixture with OMEMO support and one fake contact device+bundle.
    pub fn new_with_omemo_contact(
        contact_jid: &str,
        device_id: u32,
        bundle: legacy_omemo::Bundle,
    ) -> (Self, OmemoCapture) {
        let mut contact_devices = HashMap::new();
        contact_devices.insert(contact_jid.to_string(), vec![(device_id, bundle)]);
        let (bundle_tx, bundle_rx) = mpsc::unbounded_channel();
        let (stanza_tx, stanza_rx) = mpsc::unbounded_channel();
        let cfg = OmemoMockConfig {
            contact_devices,
            bundle_tx,
            stanza_tx,
        };
        let roster = [contact_jid];
        let (fixture, _) = Self::new_impl(&roster, true, vec![], Some(cfg), vec![], vec![], vec![]);
        (
            fixture,
            OmemoCapture {
                bundle_rx,
                stanza_rx,
            },
        )
    }

    /// Create a fixture whose mock server responds to XEP-0030 disco#info queries for
    /// the given MUC JIDs with XEP-0359 (Stanza IDs) support. Used to test
    /// `<displayed>` marker handling in MUC contexts.
    pub fn new_with_muc_sid(roster: &[&str], muc_jids: &[&str]) -> Self {
        let muc_with_sid = muc_jids.iter().map(|s| s.to_string()).collect();
        Self::new_impl(roster, true, vec![], None, muc_with_sid, vec![], vec![]).0
    }

    /// Create a fixture with XEP-0359 MUC support and a pre-populated groupchat MAM archive.
    /// `gc_archive` entries: (stanza_id, from_full_jid, msg_id, body).
    pub fn new_with_muc_sid_and_gc_mam(
        roster: &[&str],
        muc_jids: &[&str],
        gc_archive: &[(&str, &str, &str, &str)],
    ) -> Self {
        let muc_with_sid = muc_jids.iter().map(|s| s.to_string()).collect();
        let muc_mam_archive = gc_archive
            .iter()
            .map(|(sid, from, id, body)| {
                (
                    sid.to_string(),
                    from.to_string(),
                    id.to_string(),
                    body.to_string(),
                )
            })
            .collect();
        Self::new_impl(
            roster,
            true,
            vec![],
            None,
            muc_with_sid,
            muc_mam_archive,
            vec![],
        )
        .0
    }

    /// Create a fixture with a groupchat MAM archive but WITHOUT XEP-0359 support
    /// for any MUC. Disco responds (with no SID feature) to confirm the absence.
    /// `gc_archive` entries: (stanza_id, from_full_jid, msg_id, body).
    pub fn new_with_gc_mam_no_sid(
        roster: &[&str],
        gc_archive: &[(&str, &str, &str, &str)],
    ) -> Self {
        let muc_mam_archive = gc_archive
            .iter()
            .map(|(sid, from, id, body)| {
                (
                    sid.to_string(),
                    from.to_string(),
                    id.to_string(),
                    body.to_string(),
                )
            })
            .collect();
        // respond_to_disco=true so the mock sends an empty disco result (no SID),
        // which causes JidDisco to fire and confirm that XEP-0359 is unsupported.
        Self::new_impl(roster, true, vec![], None, vec![], muc_mam_archive, vec![]).0
    }

    fn new_impl(
        roster: &[&str],
        respond_to_disco: bool,
        mam_archive: Vec<(String, String, String, String)>,
        omemo_cfg: Option<OmemoMockConfig>,
        muc_with_sid: Vec<String>,
        muc_mam_archive: Vec<(String, String, String, String)>,
        mam_reaction_archive: Vec<(String, String, String, String)>,
    ) -> (Self, ()) {
        let rt = Runtime::new().unwrap();
        let (mam_query_tx, mam_query_rx) = mpsc::unbounded_channel::<()>();
        let (mock, port, outgoing_msg_rx) = start_mock_server(
            &rt,
            BOUND_JID,
            roster,
            respond_to_disco,
            mam_archive,
            omemo_cfg,
            Some(mam_query_tx),
            muc_with_sid,
            muc_mam_archive,
            mam_reaction_archive,
        );
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
        (
            Self {
                rt: Some(rt),
                mock,
                harness: Some(harness),
                mam_query_rx,
                outgoing_msg_rx,
            },
            (),
        )
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
        self.harness
            .as_ref()
            .unwrap()
            .send_command(&format!("/win {name}"));
    }

    pub fn snapshot(&self) -> vt100::Parser {
        self.harness.as_ref().unwrap().snapshot()
    }

    pub fn send_bytes(&self, bytes: &[u8]) {
        self.harness.as_ref().unwrap().send_bytes(bytes);
    }

    /// Drain all pending MAM query notifications, returning how many were queued.
    pub fn drain_mam_queries(&mut self) -> usize {
        let mut count = 0;
        while self.mam_query_rx.try_recv().is_ok() {
            count += 1;
        }
        count
    }

    /// Wait until aparte sends an outgoing message matching `predicate`.
    /// Returns the first matching message, or `None` on timeout.
    pub fn recv_outgoing_message_matching(
        &mut self,
        predicate: impl Fn(&Message) -> bool,
        timeout: Duration,
    ) -> Option<Message> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            while let Ok(msg) = self.outgoing_msg_rx.try_recv() {
                if predicate(&msg) {
                    return Some(msg);
                }
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Wait until aparte sends a groupchat `<displayed>` marker to `room_jid`.
    /// Returns `true` if the marker arrives within `timeout`, `false` otherwise.
    pub fn recv_outgoing_displayed_marker_for(
        &mut self,
        room_jid: &str,
        timeout: Duration,
    ) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            while let Ok(msg) = self.outgoing_msg_rx.try_recv() {
                let to_matches = msg
                    .to
                    .as_ref()
                    .map(|j| j.to_string() == room_jid)
                    .unwrap_or(false);
                let is_groupchat = msg.type_ == MessageType::Groupchat;
                let has_displayed = msg
                    .payloads
                    .iter()
                    .any(|p| p.is("displayed", "urn:xmpp:chat-markers:0"));
                if to_matches && is_groupchat && has_displayed {
                    return true;
                }
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(50));
        }
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
