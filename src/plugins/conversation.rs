use std::fmt;
use std::rc::Rc;
use std::collections::HashMap;
use uuid::Uuid;
use xmpp_parsers::{Element, roster, ns, Jid, BareJid, presence};
use xmpp_parsers::iq::{Iq, IqType};
use std::convert::TryFrom;

use crate::core::{Plugin, Aparte, Event, conversation,};

pub struct ConversationPlugin {
    conversations: HashMap<BareJid, conversation::Conversation>,
}

impl ConversationPlugin {
}

impl Plugin for ConversationPlugin {
    fn new() -> ConversationPlugin {
        Self {
            conversations: HashMap::new(),
        }
    }

    fn init(&mut self, _aparte: &Aparte) -> Result<(), ()> {
        Ok(())
    }

    fn on_event(&mut self, aparte: Rc<Aparte>, event: &Event) {
        match event {
            _ => {},
        }
    }
}

impl fmt::Display for ConversationPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Conversations management")
    }
}
