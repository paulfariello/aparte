//! XMPP integration tests with a mock TCP server.
//!
//! Each test:
//!   1. Binds a random TCP port and runs a minimal XMPP handshake server
//!      (SASL PLAIN → bind → stanza loop) in a Tokio runtime.
//!   2. Launches the real `aparte` binary inside a PTY with a config that
//!      points at that port, `APARTE_INSECURE_XMPP=1`, and `autoconnect=true`.
//!   3. Polls the vt100 screen until the expected text appears (or times out).

use std::convert::TryFrom;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use xmpp_parsers::jid::Jid;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use tokio_xmpp::xmlstream::{accept_stream, StreamHeader, Timeouts, XmppStreamElement};
use xmpp_parsers::{
    bind::{BindFeature, BindResponse},
    carbons::Received,
    forwarding::Forwarded,
    iq::Iq,
    message::{Id, Message, MessageType},
    minidom::Element,
    ns,
    sasl::{Nonza as SaslNonza, Success},
    stream_features::{SaslMechanisms, StreamFeatures},
};

const ROWS: u16 = 24;
const COLS: u16 = 80;

// ---------------------------------------------------------------------------
// PTY harness (generalised from startup_ui.rs)
// ---------------------------------------------------------------------------

struct Harness {
    bytes: Arc<Mutex<Vec<u8>>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    _tmp: tempfile::TempDir,
}

impl Harness {
    fn spawn(config_toml: &str, extra_env: &[(&str, &str)]) -> Self {
        let tmp = tempfile::Builder::new()
            .prefix("aparte-xmpp-integ")
            .tempdir()
            .expect("tmpdir");

        let config_dir = tmp.path().join("config");
        let data_dir = match std::env::var_os("APARTE_TEST_DATA_DIR") {
            Some(p) => std::path::PathBuf::from(p),
            None => tmp.path().join("data"),
        };
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(config_dir.join("config.toml"), config_toml).unwrap();

        let pty = NativePtySystem::default();
        let pair = pty
            .openpty(PtySize { rows: ROWS, cols: COLS, pixel_width: 0, pixel_height: 0 })
            .expect("openpty");

        let exe = env!("CARGO_BIN_EXE_aparte");
        let mut cmd = CommandBuilder::new(exe);
        cmd.arg("--config");
        cmd.arg(config_dir.join("config.toml"));
        cmd.arg("--shared");
        cmd.arg(&data_dir);
        cmd.env(
            "RUST_LOG",
            std::env::var("APARTE_TEST_LOG").unwrap_or_else(|_| "error".into()),
        );
        cmd.env("TERM", "xterm-256color");
        cmd.env("HOME", tmp.path());
        for (k, v) in extra_env {
            cmd.env(k, v);
        }

        let child = pair.slave.spawn_command(cmd).expect("spawn aparte");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("clone reader");
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(pair.master.take_writer().expect("take writer")));

        let bytes = Arc::new(Mutex::new(Vec::<u8>::new()));
        {
            let bytes = Arc::clone(&bytes);
            let writer = Arc::clone(&writer);
            thread::spawn(move || {
                let mut shadow = vt100::Parser::new(ROWS, COLS, 0);
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let chunk = &buf[..n];
                            bytes.lock().unwrap().extend_from_slice(chunk);
                            shadow.process(chunk);
                            let queries = count_dsr_queries(chunk);
                            if queries > 0 {
                                let (row, col) = shadow.screen().cursor_position();
                                let reply = format!("\x1b[{};{}R", row + 1, col + 1);
                                let mut w = writer.lock().unwrap();
                                for _ in 0..queries {
                                    let _ = w.write_all(reply.as_bytes());
                                }
                                let _ = w.flush();
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        Harness { bytes, child, writer, _tmp: tmp }
    }

    fn snapshot(&self) -> vt100::Parser {
        let data = self.bytes.lock().unwrap().clone();
        let mut parser = vt100::Parser::new(ROWS, COLS, 0);
        parser.process(&data);
        parser
    }

    fn send_command(&self, cmd: &str) {
        let mut w = self.writer.lock().unwrap();
        let _ = w.write_all(cmd.as_bytes());
        let _ = w.write_all(b"\n");
        let _ = w.flush();
    }

    fn shutdown(mut self) {
        {
            let mut w = self.writer.lock().unwrap();
            let _ = w.write_all(b"/quit\n");
            let _ = w.flush();
        }
        let deadline = Instant::now() + Duration::from_millis(800);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn count_dsr_queries(bytes: &[u8]) -> usize {
    const NEEDLE: &[u8] = b"\x1b[6n";
    let mut count = 0;
    let mut i = 0;
    while i + NEEDLE.len() <= bytes.len() {
        if &bytes[i..i + NEEDLE.len()] == NEEDLE {
            count += 1;
            i += NEEDLE.len();
        } else {
            i += 1;
        }
    }
    count
}

fn row_text(screen: &vt100::Screen, r: u16) -> String {
    let mut s = String::new();
    for c in 0..COLS {
        if let Some(cell) = screen.cell(r, c) {
            let contents = cell.contents();
            if !contents.is_empty() {
                s.push_str(&contents);
            }
        }
    }
    s
}

fn grid_contains(screen: &vt100::Screen, needle: &str) -> bool {
    (0..ROWS).any(|r| row_text(screen, r).contains(needle))
}

fn describe(screen: &vt100::Screen) -> String {
    let mut s = String::new();
    for r in 0..ROWS {
        s.push_str(&format!("{:02}: {:?}\n", r, row_text(screen, r)));
    }
    s
}

fn wait_for_screen(h: &Harness, needle: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let parser = h.snapshot();
        if grid_contains(parser.screen(), needle) {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
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
) {
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
    let accepted = stream.accept_reset(&success).await.expect("mock: accept_reset");

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
        .send(&XmppStreamElement::Stanza(tokio_xmpp::Stanza::Iq(bind_result)))
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
                    Some(Ok(_)) => {}
                    None | Some(Err(_)) => break,
                }
            }
        }
    }
}

fn start_mock_server(rt: &Runtime, bound_jid: &str) -> (MockServer, u16) {
    let listener = rt
        .block_on(async { TcpListener::bind("127.0.0.1:0").await.expect("bind mock port") });
    let port = listener.local_addr().unwrap().port();
    let (inject_tx, inject_rx) = mpsc::unbounded_channel::<XmppStreamElement>();
    rt.spawn(run_mock_server(listener, bound_jid.to_string(), inject_rx));
    (MockServer { inject_tx }, port)
}

// ---------------------------------------------------------------------------
// Stanza builders
// ---------------------------------------------------------------------------

fn chat_message(from: &str, to: &str, id: &str, body: &str) -> XmppStreamElement {
    let mut msg = Message::chat(Some(Jid::new(to).unwrap()));
    msg.from = Some(Jid::new(from).unwrap());
    msg.id = Some(Id(id.to_string()));
    msg.bodies.insert(Default::default(), body.to_string());
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg))
}

fn carbon_received(
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
        forwarded: Forwarded { delay: None, message: inner },
    };

    let mut outer = Message::new_with_type(MessageType::Normal, Some(Jid::new(outer_to).unwrap()));
    outer.from = Some(Jid::new(outer_from).unwrap());
    outer.payloads.push(received.into());
    XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(outer))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Verify that carbon stanzas can be built and round-trip through xmpp-parsers.
#[test]
fn carbon_type_construction() {
    let stanza = carbon_received(
        "user@localhost",
        "user@localhost/aparte_test",
        "contact@localhost",
        "user@localhost",
        "c1",
        "Carbon message!",
    );
    match stanza {
        XmppStreamElement::Stanza(tokio_xmpp::Stanza::Message(msg)) => {
            assert!(!msg.payloads.is_empty(), "payloads should contain <received>");
            assert_eq!(msg.from, Some(Jid::new("user@localhost").unwrap()));
        }
        _ => panic!("expected a Message stanza"),
    }
}

/// After a successful connection the bound JID appears in the console log.
#[test]
fn connection_shows_jid_in_console() {
    let rt = Runtime::new().unwrap();
    let bound_jid = "user@localhost/aparte_test";
    let (_mock, port) = start_mock_server(&rt, bound_jid);

    let config = format!(
        "[accounts.test]\n\
         jid = \"user@localhost\"\n\
         server = \"127.0.0.1\"\n\
         port = {port}\n\
         autoconnect = true\n\
         password = \"test\"\n"
    );

    let h = Harness::spawn(&config, &[("APARTE_INSECURE_XMPP", "1")]);

    let found = wait_for_screen(&h, "Connected as", Duration::from_secs(15));
    let parser = h.snapshot();
    h.shutdown();
    rt.shutdown_background();

    assert!(
        found,
        "expected 'Connected as' in screen within 15 s\n{}",
        describe(parser.screen()),
    );
}

/// An incoming chat message body appears in the UI.
#[test]
fn incoming_chat_message_appears_in_ui() {
    let rt = Runtime::new().unwrap();
    let bound_jid = "user@localhost/aparte_test";
    let (mock, port) = start_mock_server(&rt, bound_jid);

    let config = format!(
        "[accounts.test]\n\
         jid = \"user@localhost\"\n\
         server = \"127.0.0.1\"\n\
         port = {port}\n\
         autoconnect = true\n\
         password = \"test\"\n"
    );

    let h = Harness::spawn(&config, &[("APARTE_INSECURE_XMPP", "1")]);

    assert!(
        wait_for_screen(&h, "Connected as", Duration::from_secs(15)),
        "aparte did not connect within 15 s",
    );
    thread::sleep(Duration::from_millis(300));

    mock.inject(chat_message(
        "contact@localhost",
        "user@localhost/aparte_test",
        "m1",
        "Hello from mock!",
    ));

    // Wait for the window to be created, then switch to it.
    thread::sleep(Duration::from_millis(400));
    h.send_command("/win contact@localhost");

    let found = wait_for_screen(&h, "Hello from mock!", Duration::from_secs(5));
    let parser = h.snapshot();
    h.shutdown();
    rt.shutdown_background();

    assert!(
        found,
        "expected 'Hello from mock!' in screen within 5 s\n{}",
        describe(parser.screen()),
    );
}

/// An incoming carbon copy (XEP-0280 received) shows the forwarded body.
#[test]
fn incoming_carbon_appears_in_ui() {
    let rt = Runtime::new().unwrap();
    let bound_jid = "user@localhost/aparte_test";
    let (mock, port) = start_mock_server(&rt, bound_jid);

    let config = format!(
        "[accounts.test]\n\
         jid = \"user@localhost\"\n\
         server = \"127.0.0.1\"\n\
         port = {port}\n\
         autoconnect = true\n\
         password = \"test\"\n"
    );

    let h = Harness::spawn(&config, &[("APARTE_INSECURE_XMPP", "1")]);

    assert!(
        wait_for_screen(&h, "Connected as", Duration::from_secs(15)),
        "aparte did not connect within 15 s",
    );
    thread::sleep(Duration::from_millis(300));

    mock.inject(carbon_received(
        "user@localhost",
        "user@localhost/aparte_test",
        "contact@localhost",
        "user@localhost",
        "c1",
        "Carbon message!",
    ));

    // Wait for the window to be created, then switch to it.
    thread::sleep(Duration::from_millis(400));
    h.send_command("/win contact@localhost");

    let found = wait_for_screen(&h, "Carbon message!", Duration::from_secs(5));
    let parser = h.snapshot();
    h.shutdown();
    rt.shutdown_background();

    assert!(
        found,
        "expected 'Carbon message!' in screen within 5 s\n{}",
        describe(parser.screen()),
    );
}