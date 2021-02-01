/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use std::fmt;
use std::rc::Rc;
use xmpp_parsers::BareJid;

use crate::account::Account;
use crate::command::Command;
use crate::conversation::Conversation;
use crate::core::{Aparte, Event, Plugin};
use crate::cursor::Cursor;
use crate::plugins::conversation::ConversationPlugin;
use crate::word::Words;

pub struct CompletionPlugin {
    /// List of possible completions for current raw_buf
    completions: Option<Vec<String>>,
    /// Index of currently displayed completion
    current_completion: usize,
}

impl CompletionPlugin {
    pub fn autocomplete(
        &mut self,
        aparte: &mut Aparte,
        account: &Option<Account>,
        conversation: &Option<BareJid>,
        raw_buf: String,
        cursor: Cursor,
    ) {
        if self.completions.is_none() {
            self.build_completions(aparte, account, conversation, raw_buf.clone(), &cursor);
        }

        if let Some(completions) = &self.completions {
            if completions.len() > 0 {
                let mut completed_buf = String::new();
                let mut new_index = 0;
                let completion = completions[self.current_completion].clone();
                if raw_buf.starts_with("/") {
                    if let Ok(mut command) = Command::parse_with_cursor(&raw_buf, &cursor) {
                        if command.cursor < command.args.len() {
                            command.args[command.cursor] = completion;
                        } else {
                            command.args.push(completion);
                        }
                        completed_buf = command.assemble();
                        // TODO handle in place completion, cursor shouldn't move to end of input
                        new_index = completed_buf.len();
                    }
                } else {
                    let words = Words::new(&raw_buf);
                    let old_index = cursor.index(&raw_buf);
                    let mut iter_index = 0;
                    completed_buf = words
                        .map(|word| {
                            if iter_index < old_index && old_index <= iter_index + word.len() {
                                iter_index += completion.len();
                                new_index = iter_index;
                                &completion
                            } else {
                                iter_index += word.len();
                                word
                            }
                        })
                        .collect::<Vec<&str>>()
                        .join("");
                }

                self.current_completion += 1;
                self.current_completion %= completions.len();

                aparte.schedule(Event::Completed(
                    completed_buf.clone(),
                    Cursor::from_index(&completed_buf, new_index).unwrap(),
                ));
            }
        }
    }

    pub fn build_completions(
        &mut self,
        aparte: &mut Aparte,
        account: &Option<Account>,
        conversation: &Option<BareJid>,
        raw_buf: String,
        cursor: &Cursor,
    ) {
        if raw_buf.starts_with("/") {
            let mut completions = Vec::new();
            if let Ok(command) = Command::parse_with_cursor(&raw_buf, cursor) {
                if command.cursor == 0 {
                    completions = aparte
                        .command_parsers
                        .iter()
                        .map(|c| c.0.to_string())
                        .collect()
                } else {
                    let command_parsers = Rc::clone(&aparte.command_parsers);
                    if let Some(parser) = command_parsers.get(&command.args[0]) {
                        if command.cursor - 1 < parser.autocompletions.len() {
                            if let Some(completion) = &parser.autocompletions[command.cursor - 1] {
                                completions = completion(aparte, command.clone())
                            }
                        }
                    }
                }

                let matcher = SkimMatcherV2::default();
                let mut scored = completions
                    .iter()
                    .map(|c| {
                        (
                            matcher.fuzzy_match(c, &command.args[command.cursor]),
                            c.clone(),
                        )
                    })
                    .filter(|sc| sc.0.is_some())
                    .collect::<Vec<_>>();
                scored.sort_by(|a, b| a.0.cmp(&b.0));

                self.completions = Some(scored.iter().map(|(_, c)| c).cloned().collect());
                self.current_completion = 0;
            }
        } else {
            match (account, conversation) {
                (Some(account), Some(conversation)) => {
                    if let Some(conversation_plugin) = aparte.get_plugin::<ConversationPlugin>() {
                        if let Some(Conversation::Channel(channel)) =
                            conversation_plugin.get(account, conversation)
                        {
                            let words =
                                Words::new(&raw_buf[..cursor.index(&raw_buf)]).collect::<Vec<_>>();
                            let current_word = *words.last().unwrap_or(&"");

                            let append = if words.len() <= 1 { ": " } else { " " };

                            // Collect completion candidates
                            self.completions = Some(
                                channel
                                    .occupants
                                    .iter()
                                    .filter_map(|(_, occupant)| {
                                        if occupant.nick.starts_with(current_word) {
                                            Some(occupant.nick.clone() + append)
                                        } else {
                                            None
                                        }
                                    })
                                    .collect(),
                            );
                            self.current_completion = 0;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub fn reset_completion(&mut self) {
        self.completions = None;
        self.current_completion = 0;
    }
}

impl Plugin for CompletionPlugin {
    fn new() -> CompletionPlugin {
        CompletionPlugin {
            completions: None,
            current_completion: 0,
        }
    }

    fn init(&mut self, _aparte: &mut Aparte) -> Result<(), ()> {
        Ok(())
    }

    fn on_event(&mut self, aparte: &mut Aparte, event: &Event) {
        match event {
            Event::AutoComplete {
                account,
                conversation,
                raw_buf,
                cursor,
            } => self.autocomplete(
                aparte,
                account,
                conversation,
                raw_buf.clone(),
                cursor.clone(),
            ),
            Event::ResetCompletion => self.reset_completion(),
            _ => {}
        }
    }
}

impl fmt::Display for CompletionPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Autocompletion")
    }
}
