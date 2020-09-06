extern crate futures;
#[macro_use]
extern crate log;
extern crate tokio;
extern crate tokio_xmpp;
extern crate xmpp_parsers;

use bytes::BytesMut;
use futures::{future, Future, Sink, Stream};
use signal_hook::iterator::Signals;
use std::collections::{VecDeque, HashMap};
use std::io::{Error as IoError, ErrorKind};
use std::sync::{Arc, Mutex};
use termion::event::Key;
use termion::input::TermRead;
use tokio::codec::FramedRead;
use tokio::runtime::current_thread::Runtime;
use tokio_codec::{Decoder};
use tokio_xmpp::{Client, Error as XmppError, Packet};
use xmpp_parsers::presence::{Presence, Show as PresenceShow, Type as PresenceType};
use xmpp_parsers::Element;
use futures::unsync::mpsc::UnboundedSender;

#[derive(Debug)]
pub enum Event {
    Key(Key),
}

pub type EventStream = FramedRead<tokio::reactor::PollEvented2<tokio_file_unix::File<std::fs::File>>, KeyCodec>;

struct Context {
    connections: HashMap<String, UnboundedSender<Packet>>,
}

impl Context {
    fn event_stream(&self) -> EventStream {
        let file = tokio_file_unix::raw_stdin().unwrap();
        let file = tokio_file_unix::File::new_nb(file).unwrap();
        let file = file.into_io(&tokio::reactor::Handle::default()).unwrap();

        FramedRead::new(file, KeyCodec::new())
    }

    fn add_connection(&mut self, jid: String, tx: UnboundedSender<Packet>) {
        self.connections.insert(jid, tx);
    }

    fn log(&mut self, log: String) {
        println!("{}", log);
    }

    pub fn send(&mut self, element: Element) {
        let mut raw = Vec::<u8>::new();
        element.write_to(&mut raw);
        debug!("SEND: {}", String::from_utf8(raw).unwrap());
        let packet = Packet::Stanza(element);
        // TODO use correct connection
        let current_connection = self.connections.iter_mut().next().unwrap().1;
        if let Err(e) = current_connection.start_send(packet) {
            warn!("Cannot send packet: {}", e);
        }
    }

    fn connect(this: Arc<Mutex<Context>>) {
        println!("Connection");
        let jid = String::from("paul@fariello.eu");
        let client = Client::new(&jid, "{ji(N)4EpsXvAn,ul-R-p@f}").unwrap();

        let (sink, stream) = client.split();
        let (tx, rx) = futures::unsync::mpsc::unbounded();

        this.lock().unwrap().add_connection(jid.clone(), tx);

        tokio::runtime::current_thread::spawn(
            rx.forward(
                sink.sink_map_err(|_| panic!("Pipe"))
                ).map(|(rx, mut sink)| {
                drop(rx);
                let _ = sink.close();
            }).map_err(|e| {
                    panic!("Send error: {:?}", e);
                })
            );

        let context_for_client = this.clone();
        let client = stream.for_each(move |event| {
            let mut context = context_for_client.lock().unwrap();
            if event.is_online() {
                context.log(format!("Connected as {}", jid));

                let mut presence = Presence::new(PresenceType::None);
                presence.show = Some(PresenceShow::Chat);

                context.send(presence.into());
            } else if let Some(stanza) = event.into_stanza() {
                context.log(format!("RECV: {}", String::from(&stanza)));
            }

            future::ok(())
        });

        let context_for_client_err = this.clone();
        let client = client.map_err(move |error| {
            let mut context = context_for_client_err.lock().unwrap();
            match error {
                XmppError::Auth(auth) => {
                    context.log(format!("Authentication failed {}", auth));
                },
                error => {
                    context.log(format!("Connection error {:?}", error));
                },
            }
        });

        tokio::runtime::current_thread::spawn(client);
    }

    fn event(this: Arc<Mutex<Context>>, event: Event) -> Result<(), IoError> {
        println!("event: {:?}", event);

        match event {
            Event::Key(Key::Char('c')) => Context::connect(this.clone()),
            _ => {}
        };

        Ok(())
    }

    fn run(this: Context) {
        let event_stream = this.event_stream();
        let context = Arc::new(Mutex::new(this));

        let mut rt = Runtime::new().unwrap();

        let signals = Signals::new(&[signal_hook::SIGWINCH]).unwrap().into_async().unwrap().for_each(move |sig| {
            println!("signal: {:?}", sig);
            Ok(())
        }).map_err(|e| panic!("{}", e));

        rt.spawn(signals);

        let context_for_event = context.clone();
        if let Err(e) = rt.block_on(event_stream.for_each(move |event| {
            Context::event(context_for_event.clone(), event)
        })) {
          info!("Error in event stream: {}", e);
        }
    }
}

pub struct KeyCodec {
    queue: VecDeque<Result<Event, IoError>>,
}

impl KeyCodec {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }
}

impl Decoder for KeyCodec {
    type Item = Event;
    type Error = IoError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let mut keys = buf.keys();
        while let Some(key) = keys.next() {
            match key {
                Ok(Key::Alt('\x1b')) => {
                    match keys.next() {
                        Some(Ok(Key::Char('['))) => {
                            match keys.next() {
                                Some(Ok(_)) => {},
                                Some(Err(_)) => {},
                                None => {},
                            };
                        },
                        Some(Ok(_)) => {},
                        Some(Err(_)) => {},
                        None => {},
                    };
                },
                Ok(Key::Char(c)) => self.queue.push_back(Ok(Event::Key(Key::Char(c)))),
                Ok(Key::Backspace) => self.queue.push_back(Ok(Event::Key(Key::Backspace))),
                Ok(Key::Delete) => self.queue.push_back(Ok(Event::Key(Key::Delete))),
                Ok(Key::Home) => self.queue.push_back(Ok(Event::Key(Key::Home))),
                Ok(Key::End) => self.queue.push_back(Ok(Event::Key(Key::End))),
                Ok(Key::Up) => self.queue.push_back(Ok(Event::Key(Key::Up))),
                Ok(Key::Down) => self.queue.push_back(Ok(Event::Key(Key::Down))),
                Ok(Key::Left) => self.queue.push_back(Ok(Event::Key(Key::Left))),
                Ok(Key::Right) => self.queue.push_back(Ok(Event::Key(Key::Right))),
                Ok(Key::Ctrl(c)) => self.queue.push_back(Ok(Event::Key(Key::Ctrl(c)))),
                Ok(Key::Alt(c)) => self.queue.push_back(Ok(Event::Key(Key::Alt(c)))),
                Ok(_) => {},
                Err(_) => {},
            };
        }

        buf.clear();

        match self.queue.pop_front() {
            Some(Ok(command)) => Ok(Some(command)),
            Some(Err(err)) => Err(err),
            None => Ok(None),
        }
    }
}

fn main() {
    println!("Hello, world!");
    let context = Context {
        connections: HashMap::new(),
    };
    Context::run(context);
}
