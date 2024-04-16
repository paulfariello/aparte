/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use anyhow::Context;
use anyhow::Result;
use xmpp_parsers::ns;
use xmpp_parsers::pubsub::PubSubEvent;
use xmpp_parsers::{BareJid, Jid};

use crate::account::Account;
use crate::command::{Command, CommandParser};
use crate::contact;
use crate::contact::Bookmark;
use crate::core::AparteAsync;
use crate::core::{Aparte, Event, ModTrait};
use crate::mods::disco;

command_def!(bookmark_add,
r#"/bookmark add <bookmark> <conference> [autojoin=on|off]

    bookmark    The bookmark friendly name
    conference  The conference room jid
    nick        Your nick in the conference
    autojoin    Wether the conference room should be automatically joined on startup

Description:
    Add a bookmark

Examples:
    /bookmark add aparte aparte@conference.fariello.eu
    /bookmark add aparte aparte@conference.fariello.eu nick=needle
    /bookmark add aparte aparte@conference.fariello.eu autojoin=on
"#,
{
    name: String,
    conference: BareJid,
    nick: Named<String>,
    autojoin: Named<bool>
},
|aparte, _command| {
    let account = aparte.current_account().context("No connection found")?;
    let autojoin = autojoin.unwrap_or(false);
    let bookmark = contact::Bookmark {
        jid: conference,
        name: Some(name),
        nick,
        password: None,
        autojoin,
        extensions: None,
    };
    let mut bookmarks = aparte.get_mod_mut::<BookmarksMod>();
    bookmarks.add(aparte, &account, bookmark.clone());
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
{ conference: BareJid },
|aparte, _command| {
    let account = aparte.current_account().context("No connection found")?;
    let mut bookmarks = aparte.get_mod_mut::<BookmarksMod>();
    bookmarks.delete(aparte, &account, conference)
}
);

command_def!(bookmark_edit,
r#"/bookmark edit <bookmark> [<conference>] [autojoin=on|off]

    bookmark    The bookmark friendly name
    conference  The conference room jid
    autojoin    Wether the conference room should be automatically joined on startup

Description:
    Edit a bookmark

Examples:
    /bookmark edit aparte autojoin=true
    /bookmark edit aparte aparte@conference.fariello.eu
    /bookmark edit aparte nick=needle
    /bookmark edit aparte aparte@conference.fariello.eu autojoin=false
"#,
{
    name: String,
    nick: Named<String>,
    autojoin: Named<bool>,
    conference: Option<BareJid>,
},
|aparte, _command| {
    let account = aparte.current_account().context("No connection found")?;
    let mut bookmarks = aparte.get_mod_mut::<BookmarksMod>();
    bookmarks.edit(aparte, &account, name.clone(), conference, nick, autojoin).with_context(|| format!("Unknown bookmark {name}"))?;

    Ok(())
});

command_def!(bookmark,
r#"/bookmark add|del|edit"#,
{
    action: Command = {
        children: {
            "add": bookmark_add,
            "del": bookmark_del,
            "edit": bookmark_edit,
        }
    },
});

#[derive(Clone)]
enum Backend {
    BookmarksV1,
    BookmarksV2,
}

mod bookmarks_v1 {
    use std::convert::TryFrom;

    use anyhow::{anyhow, Result};
    use uuid::Uuid;
    use xmpp_parsers::{
        bookmarks,
        data_forms::{DataForm, DataFormType, Field, FieldType},
        iq::{Iq, IqType},
        ns,
        pubsub::{
            pubsub::{self, Items, Publish, PublishOptions},
            Item, ItemId, NodeName, PubSub,
        },
        Jid,
    };

    use crate::{
        account::Account,
        contact::{self, Bookmark},
        core::AparteAsync,
        i18n,
    };

    pub async fn get_bookmarks(
        aparte: &mut AparteAsync,
        account: &Account,
    ) -> Result<Vec<Bookmark>> {
        match aparte.iq(account, get_bookmarks_iq()).await?.payload {
            IqType::Result(Some(el)) => {
                if let PubSub::Items(items) = PubSub::try_from(el)? {
                    match &items.node.0 as &str {
                        ns::BOOKMARKS | ns::BOOKMARKS2 => Ok(handle(
                            items.items.iter().cloned().map(|item| item.0).collect(),
                        )),
                        _ => Err(anyhow!("Can't get bookmarks: invalid result")),
                    }
                } else {
                    Err(anyhow!("Can't get bookmarks: invalid result"))
                }
            }
            IqType::Error(err) => Err(anyhow!(
                "Can't get bookmarks: {}",
                i18n::xmpp_err_to_string(&err, vec![]).1
            )),
            _ => Err(anyhow!("Can't get bookmarks: invalid result")),
        }
    }

    fn get_bookmarks_iq() -> Iq {
        let id = Uuid::new_v4().hyphenated().to_string();
        let items = Items {
            max_items: None,
            node: NodeName(String::from(ns::BOOKMARKS)),
            subid: None,
            items: vec![],
        };
        let pubsub = PubSub::Items(items);
        Iq::from_get(id, pubsub)
    }

    pub async fn update(
        aparte: &mut AparteAsync,
        account: &Account,
        bookmarks: &Vec<contact::Bookmark>,
    ) -> Result<()> {
        match aparte.iq(account, update_iq(bookmarks)).await?.payload {
            IqType::Result(_) => Ok(()),
            IqType::Error(err) => Err(anyhow!(
                "Can't update bookmarks: {}",
                i18n::xmpp_err_to_string(&err, vec![]).1
            )),
            _ => Err(anyhow!("Can't update bookmarks: invalid result")),
        }
    }

    fn update_iq(bookmarks: &Vec<contact::Bookmark>) -> Iq {
        let id = Uuid::new_v4().hyphenated().to_string();
        let confs = bookmarks
            .iter()
            .map(|bookmark| bookmarks::Conference {
                autojoin: match bookmark.autojoin {
                    true => bookmarks::Autojoin::True,
                    false => bookmarks::Autojoin::False,
                },
                jid: bookmark.jid.clone(),
                name: Some(bookmark.name.clone().unwrap_or(bookmark.jid.to_string())),
                nick: bookmark.nick.clone(),
                password: None,
            })
            .collect();
        let storage = bookmarks::Storage {
            conferences: confs,
            urls: vec![],
        };
        let item = Item {
            id: Some(ItemId(String::from("current"))),
            payload: Some(storage.into()),
            publisher: None,
        };
        let publish = Publish {
            node: NodeName(String::from(ns::BOOKMARKS)),
            items: vec![pubsub::Item(item)],
        };
        let options = PublishOptions {
            form: Some(DataForm {
                type_: DataFormType::Submit,
                form_type: Some(String::from(
                    "http://jabber.org/protocol/pubsub#publish-options",
                )),
                title: None,
                instructions: None,
                fields: vec![
                    Field {
                        var: String::from("pubsub#persist_items"),
                        type_: FieldType::Boolean,
                        label: None,
                        required: false,
                        media: vec![],
                        options: vec![],
                        values: vec![String::from("true")],
                    },
                    Field {
                        var: String::from("pubsub#access_model"),
                        type_: FieldType::TextSingle,
                        label: None,
                        required: false,
                        media: vec![],
                        options: vec![],
                        values: vec![String::from("whitelist")],
                    },
                ],
            }),
        };
        let pubsub = PubSub::Publish {
            publish,
            publish_options: Some(options),
        };
        Iq::from_set(id, pubsub)
    }

    pub fn handle(items: Vec<Item>) -> Vec<contact::Bookmark> {
        let mut bookmarks = vec![];
        for item in items {
            if let Some(el) = item.payload.clone() {
                if let Ok(storage) = bookmarks::Storage::try_from(el) {
                    for conf in storage.conferences {
                        let bookmark = contact::Bookmark {
                            jid: conf.jid.clone(),
                            name: conf.name.clone(),
                            nick: conf.nick.clone(),
                            password: conf.password.clone(),
                            autojoin: conf.autojoin == bookmarks::Autojoin::True,
                            extensions: None,
                        };

                        bookmarks.push(bookmark);
                    }
                }
            } else {
                log::warn!("Missing storage element");
            }
        }

        bookmarks
    }

    fn subscribe_iq(account: &Account) -> Iq {
        let id = Uuid::new_v4().hyphenated().to_string();
        let pubsub = PubSub::Subscribe {
            subscribe: Some(pubsub::Subscribe {
                node: Some(NodeName(String::from(ns::BOOKMARKS))),
                jid: Jid::from(account.clone()),
            }),
            options: None,
        };
        Iq::from_set(id, pubsub)
    }

    pub async fn init(aparte: &mut AparteAsync, account: &Account) -> Result<()> {
        aparte.iq(account, subscribe_iq(account));

        Ok(())
    }
}

mod bookmarks_v2 {
    use std::{convert::TryFrom, str::FromStr};

    use anyhow::{anyhow, Result};
    use uuid::Uuid;
    use xmpp_parsers::{
        bookmarks2,
        data_forms::{DataForm, DataFormType, Field, FieldType},
        iq::{Iq, IqType},
        ns,
        pubsub::{
            owner,
            pubsub::{self, Items, Publish, PublishOptions, Retract},
            Item, ItemId, NodeName, PubSub, PubSubOwner,
        },
        BareJid, Jid,
    };

    use crate::{
        account::Account,
        contact::{self, Bookmark},
        core::AparteAsync,
        i18n,
    };

    pub async fn get_bookmarks(
        aparte: &mut AparteAsync,
        account: &Account,
    ) -> Result<Vec<Bookmark>> {
        match aparte.iq(account, get_bookmarks_iq()).await?.payload {
            IqType::Result(Some(el)) => {
                if let PubSub::Items(items) = PubSub::try_from(el)? {
                    match &items.node.0 as &str {
                        ns::BOOKMARKS | ns::BOOKMARKS2 => Ok(handle(
                            items.items.iter().cloned().map(|item| item.0).collect(),
                        )),
                        _ => Err(anyhow!("Can't get bookmarks: invalid result")),
                    }
                } else {
                    Err(anyhow!("Can't get bookmarks: invalid result"))
                }
            }
            IqType::Error(err) => Err(anyhow!(
                "Can't get bookmarks: {}",
                i18n::xmpp_err_to_string(&err, vec![]).1
            )),
            _ => Err(anyhow!("Can't get bookmarks: invalid result")),
        }
    }

    fn get_bookmarks_iq() -> Iq {
        let id = Uuid::new_v4().hyphenated().to_string();
        let items = Items {
            max_items: None,
            node: NodeName(String::from(ns::BOOKMARKS2)),
            subid: None,
            items: vec![],
        };
        let pubsub = PubSub::Items(items);
        Iq::from_get(id, pubsub)
    }

    fn config_node_form() -> DataForm {
        DataForm {
            type_: DataFormType::Submit,
            form_type: Some(String::from(
                "http://jabber.org/protocol/pubsub#node_config",
            )),
            title: None,
            instructions: None,
            fields: vec![
                Field {
                    var: String::from("pubsub#persist_items"),
                    type_: FieldType::Boolean,
                    label: None,
                    required: false,
                    media: vec![],
                    options: vec![],
                    values: vec![String::from("true")],
                },
                Field {
                    var: String::from("pubsub#send_last_published_item"),
                    type_: FieldType::TextSingle,
                    label: None,
                    required: false,
                    media: vec![],
                    options: vec![],
                    values: vec![String::from("never")],
                },
                Field {
                    var: String::from("pubsub#access_model"),
                    type_: FieldType::TextSingle,
                    label: None,
                    required: false,
                    media: vec![],
                    options: vec![],
                    values: vec![String::from("whitelist")],
                },
                Field {
                    var: String::from("pubsub#max_items"),
                    type_: FieldType::TextSingle,
                    label: None,
                    required: false,
                    media: vec![],
                    options: vec![],
                    values: vec![String::from("10")],
                },
            ],
        }
    }

    fn create_node_iq() -> Iq {
        let id = Uuid::new_v4().hyphenated().to_string();
        let create = pubsub::Create {
            node: Some(NodeName(String::from(ns::BOOKMARKS2))),
        };
        let pubsub = PubSub::Create {
            create,
            configure: None,
        };
        Iq::from_set(id, pubsub)
    }

    fn config_node_iq() -> Iq {
        let id = Uuid::new_v4().hyphenated().to_string();
        let config = owner::Configure {
            node: Some(NodeName(String::from(ns::BOOKMARKS2))),
            form: Some(config_node_form()),
        };
        let pubsub = PubSubOwner::Configure(config);
        Iq::from_set(id, pubsub)
    }

    pub async fn add(
        aparte: &mut AparteAsync,
        account: &Account,
        bookmark: &contact::Bookmark,
    ) -> Result<()> {
        match aparte.iq(account, add_iq(bookmark)).await?.payload {
            IqType::Result(_) => Ok(()),
            IqType::Error(err) => Err(anyhow!(
                "Can't add bookmarks: {}",
                i18n::xmpp_err_to_string(&err, vec![]).1
            )),
            _ => Err(anyhow!("Can't add bookmarks: invalid result")),
        }
    }

    fn add_iq(bookmark: &contact::Bookmark) -> Iq {
        let id = Uuid::new_v4().hyphenated().to_string();
        let item = Item {
            id: Some(ItemId(bookmark.jid.to_string())),
            payload: Some(
                bookmarks2::Conference {
                    autojoin: match bookmark.autojoin {
                        true => bookmarks2::Autojoin::True,
                        false => bookmarks2::Autojoin::False,
                    },
                    name: bookmark.name.clone(),
                    nick: bookmark.nick.clone(),
                    password: None,
                    extensions: Vec::new(),
                }
                .into(),
            ),
            publisher: None,
        };
        let publish = Publish {
            node: NodeName(String::from(ns::BOOKMARKS2)),
            items: vec![pubsub::Item(item)],
        };
        let options = PublishOptions {
            form: Some(DataForm {
                type_: DataFormType::Submit,
                form_type: Some(String::from(
                    "http://jabber.org/protocol/pubsub#publish-options",
                )),
                title: None,
                instructions: None,
                fields: vec![
                    Field {
                        var: String::from("pubsub#persist_items"),
                        type_: FieldType::Boolean,
                        label: None,
                        required: false,
                        media: vec![],
                        options: vec![],
                        values: vec![String::from("true")],
                    },
                    Field {
                        var: String::from("pubsub#access_model"),
                        type_: FieldType::TextSingle,
                        label: None,
                        required: false,
                        media: vec![],
                        options: vec![],
                        values: vec![String::from("whitelist")],
                    },
                ],
            }),
        };
        let pubsub = PubSub::Publish {
            publish,
            publish_options: Some(options),
        };
        Iq::from_set(id, pubsub)
    }

    pub async fn delete(
        aparte: &mut AparteAsync,
        account: &Account,
        bookmark: BareJid,
    ) -> Result<()> {
        match aparte.iq(account, delete_iq(bookmark)).await?.payload {
            IqType::Result(_) => Ok(()),
            IqType::Error(err) => Err(anyhow!(
                "Can't delete bookmarks: {}",
                i18n::xmpp_err_to_string(&err, vec![]).1
            )),
            _ => Err(anyhow!("Can't delete bookmarks: invalid result")),
        }
    }

    fn delete_iq(conference: BareJid) -> Iq {
        let id = Uuid::new_v4().hyphenated().to_string();
        let item = Item {
            id: Some(ItemId(conference.to_string())),
            payload: None,
            publisher: None,
        };
        let retract = Retract {
            node: NodeName(String::from(ns::BOOKMARKS2)),
            items: vec![pubsub::Item(item)],
            notify: pubsub::Notify::False,
        };
        let pubsub = PubSub::Retract(retract);
        Iq::from_set(id, pubsub)
    }

    pub fn handle(items: Vec<Item>) -> Vec<contact::Bookmark> {
        let mut bookmarks = vec![];
        for item in items {
            if let Some(id) = item.id.clone() {
                if let Ok(bare_jid) = BareJid::from_str(&id.0) {
                    if let Some(el) = item.payload.clone() {
                        if let Ok(conf) = bookmarks2::Conference::try_from(el) {
                            let bookmark = contact::Bookmark {
                                jid: bare_jid.clone(),
                                name: conf.name.clone(),
                                nick: conf.nick.clone(),
                                password: conf.password.clone(),
                                autojoin: conf.autojoin == bookmarks2::Autojoin::True,
                                extensions: None,
                            };

                            bookmarks.push(bookmark);
                        }
                    } else {
                        log::warn!("Empty bookmark element {}", id.0);
                    }
                } else {
                    log::warn!("Invalid bookmark jid {}", id.0);
                }
            } else {
                log::warn!("Missing bookmark id");
            }
        }

        bookmarks
    }

    fn subscribe_iq(account: &Account) -> Iq {
        let id = Uuid::new_v4().hyphenated().to_string();
        let pubsub = PubSub::Subscribe {
            subscribe: Some(pubsub::Subscribe {
                node: Some(NodeName(String::from(ns::BOOKMARKS2))),
                jid: Jid::from(account.clone()),
            }),
            options: None,
        };
        Iq::from_set(id, pubsub)
    }

    pub async fn init(aparte: &mut AparteAsync, account: &Account) -> Result<()> {
        aparte.iq(account, create_node_iq()).await?;
        aparte.iq(account, config_node_iq()).await?;
        aparte.iq(account, subscribe_iq(account)).await?;

        Ok(())
    }
}

pub struct BookmarksMod {
    backend: Backend,
    pub bookmarks: Vec<contact::Bookmark>,
    pub bookmarks_by_name: HashMap<String, usize>,
    pub bookmarks_by_jid: HashMap<Jid, usize>,
}

impl BookmarksMod {
    pub fn new() -> Self {
        Self {
            backend: Backend::BookmarksV1,
            bookmarks: vec![],
            bookmarks_by_name: HashMap::new(),
            bookmarks_by_jid: HashMap::new(),
        }
    }

    async fn init_backend(
        aparte: &mut AparteAsync,
        account: &Account,
        backend: &Backend,
    ) -> Result<()> {
        log::info!("Init bookmarks");
        match backend {
            Backend::BookmarksV1 => bookmarks_v1::init(aparte, &account).await,
            Backend::BookmarksV2 => bookmarks_v2::init(aparte, &account).await,
        }
    }

    async fn get_bookmarks(
        aparte: &mut AparteAsync,
        account: &Account,
        backend: &Backend,
    ) -> Result<()> {
        log::info!("Fetch bookmarks");
        let bookmarks = match backend {
            Backend::BookmarksV1 => bookmarks_v1::get_bookmarks(aparte, &account).await?,
            Backend::BookmarksV2 => bookmarks_v2::get_bookmarks(aparte, &account).await?,
        };
        aparte.schedule(Event::BookmarksUpdate(account.clone(), bookmarks));

        Ok(())
    }

    fn add(&mut self, aparte: &Aparte, account: &Account, bookmark: contact::Bookmark) {
        self.bookmarks.push(bookmark.clone());

        Aparte::spawn({
            let backend = self.backend.clone();
            let mut aparte = aparte.proxy();
            let account = account.clone();
            let bookmarks = self.bookmarks.clone();
            let bookmark = bookmark.clone();
            async move {
                let ret = match backend {
                    Backend::BookmarksV1 => {
                        bookmarks_v1::update(&mut aparte, &account, &bookmarks).await
                    }
                    Backend::BookmarksV2 => {
                        bookmarks_v2::add(&mut aparte, &account, &bookmark).await
                    }
                };

                match ret {
                    Err(err) => crate::error!(aparte, err, "Can't add bookmark"),
                    Ok(()) => aparte.schedule(Event::Bookmark(account.clone(), bookmark)),
                }
            }
        });
    }

    pub fn edit(
        &mut self,
        aparte: &Aparte,
        account: &Account,
        name: String,
        jid: Option<BareJid>,
        nick: Option<String>,
        autojoin: Option<bool>,
    ) -> Result<()> {
        let index = self
            .bookmarks_by_name
            .get(&name)
            .context("Unknown bookmark")?;
        let bookmark = self.bookmarks.get_mut(*index).unwrap();
        match jid {
            Some(jid) => bookmark.jid = jid,
            None => {}
        }
        match nick {
            Some(nick) if nick.is_empty() => bookmark.nick = None,
            Some(nick) => bookmark.nick = Some(nick),
            None => {}
        }
        match autojoin {
            Some(autojoin) => bookmark.autojoin = autojoin,
            None => {}
        }

        Aparte::spawn({
            let backend = self.backend.clone();
            let mut aparte = aparte.proxy();
            let account = account.clone();
            let bookmark = bookmark.clone();
            let bookmarks = self.bookmarks.clone();
            async move {
                let ret = match backend {
                    Backend::BookmarksV1 => {
                        bookmarks_v1::update(&mut aparte, &account, &bookmarks).await
                    }
                    Backend::BookmarksV2 => {
                        bookmarks_v2::add(&mut aparte, &account, &bookmark).await
                    }
                };

                match ret {
                    Err(err) => crate::error!(aparte, err, "Can't edit bookmark"),
                    Ok(()) => {}
                }
            }
        });

        Ok(())
    }

    fn delete(&mut self, aparte: &Aparte, account: &Account, conference: BareJid) -> Result<()> {
        let index = self
            .bookmarks
            .iter()
            .position(|b| {
                (conference.node().is_none() && b.name == Some(conference.to_string()))
                    || (conference.node().is_some() && b.jid == conference)
            })
            .context("Unknown bookmark")?;
        let bookmark = self.bookmarks.remove(index);

        Aparte::spawn({
            let backend = self.backend.clone();
            let mut aparte = aparte.proxy();
            let account = account.clone();
            let bookmarks = self.bookmarks.clone();
            async move {
                let ret = match backend {
                    Backend::BookmarksV1 => {
                        bookmarks_v1::update(&mut aparte, &account, &bookmarks).await
                    }
                    Backend::BookmarksV2 => {
                        bookmarks_v2::delete(&mut aparte, &account, conference).await
                    }
                };

                match ret {
                    Err(err) => crate::error!(aparte, err, "Can't delete bookmark"),
                    Ok(()) => aparte.schedule(Event::DeletedBookmark(bookmark.jid)),
                };
            }
        });

        Ok(())
    }

    fn update_indexes(&mut self) {
        self.bookmarks_by_name = self
            .bookmarks
            .iter()
            .enumerate()
            .filter(|(_, bookmark)| bookmark.name.is_some())
            .map(|(index, bookmark)| (bookmark.name.clone().unwrap(), index))
            .collect();
        self.bookmarks_by_jid = self
            .bookmarks
            .iter()
            .enumerate()
            .map(|(index, bookmark)| (bookmark.jid.clone().into(), index))
            .collect();
    }

    fn handle_bookmarks(
        &mut self,
        aparte: &mut Aparte,
        account: &Account,
        bookmarks: &Vec<Bookmark>,
    ) -> Result<()> {
        let added: Vec<contact::Bookmark> = bookmarks
            .iter()
            .filter(|bookmark| !self.bookmarks.contains(bookmark))
            .cloned()
            .collect();
        let removed: Vec<contact::Bookmark> = self
            .bookmarks
            .iter()
            .filter(|bookmark| !bookmarks.contains(bookmark))
            .cloned()
            .collect();

        self.bookmarks = bookmarks.clone();
        self.update_indexes();

        for bookmark in added.iter() {
            aparte.schedule(Event::Bookmark(account.clone(), bookmark.clone()));
            if bookmark.autojoin {
                let jid = match &bookmark.nick {
                    Some(nick) => Jid::from(bookmark.jid.clone().with_resource_str(&nick).unwrap()), // TODO avoid unwrap
                    None => Jid::from(bookmark.jid.clone()),
                };
                log::info!("Autojoin {}", jid.to_string());
                aparte.schedule(Event::Join {
                    account: account.clone(),
                    channel: jid,
                    user_request: false,
                });
            }
        }

        for bookmark in removed.iter() {
            aparte.schedule(Event::DeletedBookmark(bookmark.jid.clone()));
            // TODO leave channel?
        }

        Ok(())
    }

    pub fn get_by_name(&self, name: &str) -> Option<contact::Bookmark> {
        match self.bookmarks_by_name.get(name) {
            Some(index) => self.bookmarks.get(*index).cloned(),
            None => None,
        }
    }
}

impl ModTrait for BookmarksMod {
    fn init(&mut self, aparte: &mut Aparte) -> Result<(), ()> {
        aparte.add_command(bookmark::new());
        let mut disco = aparte.get_mod_mut::<disco::DiscoMod>();
        disco.add_feature(ns::BOOKMARKS2);

        Ok(())
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::Disco(account, features) => {
                if features.iter().any(|feature| feature == ns::BOOKMARKS2) {
                    self.backend = Backend::BookmarksV2;
                }

                Aparte::spawn({
                    let mut aparte = aparte.proxy();
                    let account = account.clone();
                    let backend = self.backend.clone();
                    async move {
                        if let Err(err) = Self::init_backend(&mut aparte, &account, &backend).await
                        {
                            crate::error!(aparte, err, "Can't init bookmarks");
                            return;
                        }

                        if let Err(err) = Self::get_bookmarks(&mut aparte, &account, &backend).await
                        {
                            crate::error!(aparte, err, "Can't get bookmarks");
                            return;
                        }
                    }
                });
            }
            Event::BookmarksUpdate(account, bookmarks) => {
                if let Err(err) = self.handle_bookmarks(aparte, account, &bookmarks) {
                    crate::error!(aparte, err, "Cannot update bookmarks");
                }
            }
            Event::PubSub {
                account,
                from: _,
                event,
            } => match event {
                PubSubEvent::PublishedItems { node, items } => match &node.0 as &str {
                    ns::BOOKMARKS | ns::BOOKMARKS2 => {
                        let items = items.iter().cloned().map(|item| item.0).collect();

                        let bookmarks = match self.backend {
                            Backend::BookmarksV1 => bookmarks_v1::handle(items),
                            Backend::BookmarksV2 => bookmarks_v2::handle(items),
                        };

                        if let Err(err) = self.handle_bookmarks(aparte, account, &bookmarks) {
                            crate::error!(aparte, err, "Cannot update bookmarks");
                        }
                    }
                    _ => {}
                },
                _ => {}
            },
            _ => {}
        }
    }
}

impl fmt::Display for BookmarksMod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "XEP-0402: PEP Native Bookmarks")
    }
}
