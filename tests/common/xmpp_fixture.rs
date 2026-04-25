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
    carbons::Received,
    forwarding::Forwarded,
    iq::Iq,
    jid::BareJid,
    message::{Id, Message, MessageType},
    minidom::Element,
    ns,
    roster::{Ask, Item as RosterItem, Roster, Subscription},
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
                        if let Iq::Get { ref id, ref payload, .. } = iq {
                            if payload.is("query", ns::ROSTER) {
                                let refs: Vec<&str> =
                                    roster_contacts.iter().map(|s| s.as_str()).collect();
                                let resp = roster_result_iq(id, &user_bare_jid, &refs);
                                let _ = stream.send(&resp).await;
                            }
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
        let rt = Runtime::new().unwrap();
        let (mock, port) = start_mock_server(&rt, BOUND_JID, roster);
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
