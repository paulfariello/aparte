/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::fmt;
use std::rc::Rc;

use crate::core::{Plugin, Aparte, Event};
use crate::command::Command;

pub struct CompletionPlugin {
    completions: Option<Vec<String>>,
    current_completion: usize,
}

impl CompletionPlugin {
    pub fn autocomplete(&mut self, aparte: Rc<Aparte>, raw_buf: String, cursor: usize) {
        if self.completions.is_none() {
            self.build_completions(Rc::clone(&aparte), raw_buf.clone(), cursor);
        }

        if let Some(completions) = &self.completions {
            if completions.len() > 0 {
                let mut completed_buf = String::new();
                let completion = completions[self.current_completion].clone();
                if raw_buf.starts_with("/") {
                    if let Ok(mut command) = Command::parse_with_cursor(&raw_buf, cursor) {
                        if command.cursor < command.args.len() {
                            command.args[command.cursor] = completion;
                        } else {
                            command.args.push(completion);
                        }
                        completed_buf = command.assemble();
                    }
                }

                self.current_completion += 1;
                self.current_completion %= completions.len();

                Rc::clone(&aparte).event(Event::Completed(completed_buf.clone(), completed_buf.len()));
            }
        }
    }

    pub fn build_completions(&mut self, aparte: Rc<Aparte>, raw_buf: String, cursor: usize) {
        if raw_buf.starts_with("/") {
            let mut completions = Vec::new();
            if let Ok(command) = Command::parse_with_cursor(&raw_buf, cursor) {
                if command.cursor == 0 {
                    completions = Rc::clone(&aparte).commands.iter().map(|c| c.0.to_string()).collect()
                } else {
                    if let Some(parser) = Rc::clone(&aparte).commands.get(&command.args[0]) {
                        if command.cursor - 1 < parser.autocompletions.len() {
                            if let Some(completion) = &parser.autocompletions[command.cursor - 1] {
                                completions = completion(Rc::clone(&aparte), command.clone())
                            }
                        }
                    }
                }

                self.completions = Some(completions.iter().filter_map(|c| {
                    if command.args.len() == command.cursor {
                        Some(c.to_string())
                    } else if c.starts_with(&command.args[command.cursor]) {
                        Some(c.to_string())
                    } else {
                        None
                    }
                }).collect());
                self.current_completion = 0;
            }
        } else {
            // TODO autocomplete with list of participant
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

    fn init(&mut self, _aparte: &Aparte) -> Result<(), ()> {
        Ok(())
    }

    fn on_event(&mut self, aparte: Rc<Aparte>, event: &Event) {
        match event {
            Event::AutoComplete(raw_buf, cursor) => self.autocomplete(aparte, raw_buf.clone(), cursor.clone()),
            Event::ResetCompletion => self.reset_completion(),
            _ => {},
        }
    }
}

impl fmt::Display for CompletionPlugin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Autocompletion")
    }
}
