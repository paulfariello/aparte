/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::fmt;
use std::str::FromStr;
use std::collections::HashMap;
use std::rc::Rc;
use uuid::Uuid;
use xmpp_parsers::Element;
use xmpp_parsers::pubsub::{PubSub, pubsub::Items, NodeName};
use xmpp_parsers::iq::Iq;
use xmpp_parsers::ns;

use crate::core::{Plugin, Aparte, Event};
use crate::command::{Command, CommandParser};
use crate::plugins::disco;

command_def!(bookmark_add,
r#"/bookmark add <bookmark> <conference> [autojoin=on|off]

    bookmark    The bookmark friendly name
    conference  The conference room jid
    autojoin    Wether the conference room should be automatically joined on startup

Description:
    Add a bookmark

Examples:
    /bookmark add aparte aparte@conference.fariello.eu
    /bookmark add aparte aparte@conference.fariello.eu/mynick
    /bookmark add aparte aparte@conference.fariello.eu/mynick autojoin=on
"#,
{
    name: String,
    conference: String,
    autojoin: Option<String>
},
|aparte, _command| {
    // TODO
    Ok(())
});

command_def!(bookmark_del,
r#"/bookmark del <bookmark>

    bookmark    The bookmark friendly name

Description:
    Delete a bookmark

Examples:
    /bookmark del aparte
"#,
{
    name: String
},
|aparte, _command| {
    Ok(())
});

command_def!(bookmark_edit,
r#"/bookmark edit <bookmark> [<conference>] [autojoin=on|off]

    bookmark    The bookmark friendly name
    conference  The conference room jid
    autojoin    Wether the conference room should be automatically joined on startup

Description:
    Edit a bookmark

Examples:
    /bookmark edit aparte autojoin=on
    /bookmark edit aparte aparte@conference.fariello.eu
    /bookmark edit aparte aparte@conference.fariello.eu autojoin=off
"#,
{
    name: String
},
|aparte, _command| {
    Ok(())
});

command_def!(bookmark,
r#"/bookmark add|del|edit"#,
{
    action: Command = {
        children: {
            "add": bookmark_add
        }
    },
});

pub struct BookmarksPlugin {
}

impl BookmarksPlugin {
    fn retreive(&self) -> Element {
        let id = Uuid::new_v4().to_hyphenated().to_string();
        let items = Items {
            max_items: None,
            node: NodeName(String::from(ns::BOOKMARKS2)),
            subid: None,
            items: vec![],
        };
        let pubsub = PubSub::Items(items);
        let iq = Iq::from_get(id, pubsub);
        iq.into()
    }
}

impl Plugin for BookmarksPlugin {
    fn new() -> BookmarksPlugin {
        BookmarksPlugin { }
    }

    fn init(&mut self, aparte: &Aparte) -> Result<(), ()> {
        aparte.add_command(bookmark());
        let mut disco = aparte.get_plugin_mut::<disco::Disco>().unwrap();
        disco.add_feature(ns::BOOKMARKS2)
    }

    fn on_event(&mut self, aparte: Rc<Aparte>, event: &Event) {
        match event {
            Event::Connected(_jid) => {
                aparte.send(self.retreive())
            },
            _ => {},
        }
    }
}

impl fmt::Display for BookmarksPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0402: PEP Native Bookmarks")
    }
}
