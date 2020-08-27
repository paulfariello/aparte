/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::convert::TryFrom;
use std::rc::Rc;

use crate::core::Aparte;

#[derive(Debug, Clone)]
pub struct Command {
    pub args: Vec<String>,
    pub cursor: usize,
}

impl Command {
    pub fn parse_with_cursor(string: &str, cursor: usize) -> Result<Self, &'static str> {
        enum State {
            Initial,
            Delimiter,
            SimplyQuoted,
            DoublyQuoted,
            Unquoted,
            UnquotedEscaped,
            SimplyQuotedEscaped,
            DoublyQuotedEscaped,
        };

        use State::*;

        let mut string_cursor = cursor;
        let mut tokens: Vec<String> = Vec::new();
        let mut token = String::new();
        let mut state = Initial;
        let mut chars = string.chars();
        let mut token_cursor = None;

        loop {
            let c = chars.next();
            state = match state {
                Initial => match c {
                    Some('/') => Delimiter,
                    _ => return Err("Missing starting /"),
                },
                Delimiter => match c {
                    Some(' ') => Delimiter,
                    Some('\'') => SimplyQuoted,
                    Some('\"') => DoublyQuoted,
                    Some('\\') => UnquotedEscaped,
                    Some(c) => {
                        token.push(c);
                        Unquoted
                    },
                    None => {
                        break;
                    }
                },
                SimplyQuoted => match c {
                    Some('\'') => Unquoted,
                    Some('\\') => SimplyQuotedEscaped,
                    Some(c) => {
                        token.push(c);
                        SimplyQuoted
                    },
                    None => return Err("Missing closing quote"),
                },
                DoublyQuoted => match c {
                    Some('\"') => Unquoted,
                    Some('\\') => DoublyQuotedEscaped,
                    Some(c) => {
                        token.push(c);
                        DoublyQuoted
                    },
                    None => return Err("Missing closing quote"),
                },
                Unquoted => match c {
                    Some('\'') => SimplyQuoted,
                    Some('\"') => DoublyQuoted,
                    Some('\\') => UnquotedEscaped,
                    Some(' ') => {
                        tokens.push(token);
                        token = String::new();
                        Delimiter
                    },
                    Some(c) => {
                        token.push(c);
                        Unquoted
                    },
                    None => {
                        tokens.push(token);
                        break;
                    }
                },
                UnquotedEscaped => match c {
                    Some(c) => {
                        token.push(c);
                        Unquoted
                    },
                    None => return Err("Missing escaped char"),
                },
                SimplyQuotedEscaped => match c {
                    Some(c) => {
                        token.push(c);
                        SimplyQuoted
                    },
                    None => return Err("Missing escaped char"),
                },
                DoublyQuotedEscaped => match c {
                    Some(c) => {
                        token.push(c);
                        DoublyQuoted
                    },
                    None => return Err("Missing escaped char"),
                }
            };

            if string_cursor == 0 {
                if token_cursor.is_none() {
                    token_cursor = match c {
                        Some(_) => Some(tokens.len()),
                        None => None,
                    }
                }
            } else {
                string_cursor -= 1;
            }
        }

        if token_cursor.is_none() {
            token_cursor = match state {
                Delimiter => Some(tokens.len()),
                _ => Some(tokens.len() - 1),
            };
        }

        if tokens.len() > 0 {
            Ok(Command {
                args: tokens,
                cursor: token_cursor.unwrap(),
            })
        } else {
            Ok(Command {
                args: vec!["".to_string()],
                cursor: token_cursor.unwrap(),
            })
        }
    }

    fn escape(arg: &str) -> String {
        let mut quote = None;
        let mut escaped = String::with_capacity(arg.len());
        for c in arg.chars() {
            escaped.extend(match c {
                '\\' => "\\\\".to_string(),
                ' ' => {
                    if quote.is_none() {
                        quote = Some(' ');
                    }
                    " ".to_string()
                },
                '\'' => {
                    match quote {
                        Some('\'') => "\\'".to_string(),
                        Some('"') => "'".to_string(),
                        Some(' ') | None => {
                            quote = Some('"');
                            "'".to_string()
                        },
                        Some(_) => unreachable!(),
                    }
                }
                '"' => {
                    match quote {
                        Some('\'') => "\"".to_string(),
                        Some('"') => "\\\"".to_string(),
                        Some(' ') | None => {
                            quote = Some('\'');
                            "\"".to_string()
                        },
                        Some(_) => unreachable!(),
                    }
                }
                c => c.to_string(),
            }.chars())
        }

        if quote == Some(' ') {
            quote = Some('"');
        }

        if quote.is_none() {
            return escaped;
        } else {
            return format!("{}{}{}", quote.unwrap(), escaped, quote.unwrap());
        }

    }

    pub fn assemble(&self) -> String {
        let mut command = "/".to_string();

        let mut first = true;
        for arg in &self.args {
            if ! first {
                command.push(' ');
            } else {
                first = false;
            }
            command.extend(Command::escape(arg).chars());
        }

        command
    }
}

impl TryFrom<&str> for Command {
    type Error = &'static str;

    fn try_from(string: &str) -> Result<Self, Self::Error> {
        Command::parse_with_cursor(string, string.len())
    }
}

pub struct CommandParser {
    pub name: &'static str,
    pub help: &'static str,
    pub parser: Box<dyn Fn(Rc<Aparte>, Command) -> Result<(), String>>,
    pub autocompletions: Vec<Option<Box<dyn Fn(Rc<Aparte>, Command) -> Vec<String>>>>,
}

#[macro_export]
macro_rules! parse_command_args {
    ($aparte:ident, $command:ident, $index:ident) => ();
    ($aparte:ident, $command:ident, $index:ident, (password) $arg:ident) => (
        if $command.args.len() <= $index {
            Rc::clone(&$aparte).event(Event::ReadPassword($command.clone()));
            return Ok(())
        }

        let $arg = $command.args[$index].clone();

        $index += 1;
    );
    ($aparte:ident, $command:ident, $index:ident, (optional) $arg:ident) => (
        let $arg = {
            if $command.args.len() > $index {
                Some($command.args[$index].clone())
            } else {
                None
            }
        };

        $index += 1;
    );
    ($aparte:ident, $command:ident, $index:ident, $arg:ident) => (
        if $command.args.len() <= $index {
            return Err(format!("Missing {} argument", stringify!($arg)))
        }
        let $arg = $command.args[$index].clone();
    );
    ($aparte:ident, $command:ident, $index:ident, (optional) $arg:ident, $($(($attr:ident))? $args:ident),+) => (
        let $arg = {
            if $command.args.len() > $index {
                Some($command.args[$index].clone())
            } else {
                None
            }
        };

        $index += 1;

        parse_command_args!($aparte, $command, $index, $($(($attr))? $args),*);
    );
    ($aparte:ident, $command:ident, $index:ident, $arg:ident, $($(($attr:ident))? $args:ident),+) => (
        if $command.args.len() <= $index {
            return Err(format!("Missing {} argument", stringify!($arg)))
        }

        let $arg = $command.args[$index].clone();

        $index += 1;

        parse_command_args!($aparte, $command, $index, $($(($attr))? $args),*);
    );
}

#[macro_export]
macro_rules! generate_command_autocompletions {
    ($autocompletions:ident) => ();
    ($autocompletions:ident, $argname:ident) => (
        $autocompletions.push(None);
    );
    ($autocompletions:ident, $argname:ident: { completion: |$aparte:ident, $command:ident| $completion:block }) => (
        $autocompletions.push(Some(Box::new(|$aparte: Rc<Aparte>, $command: Command| -> Vec<String> { $completion })));
    );
    ($autocompletions:ident, $argname:ident, $($argnames:ident$(: $args:tt)?),+) => (
        $autocompletions.push(None);
        generate_command_autocompletions!($autocompletions, $($argnames$(: $args)?),*);
    );
    ($autocompletions:ident, $argname:ident: { completion: |$aparte:ident, $command:ident| $completion:block }, $($argnames:ident$(: $args:tt)?),+) => (
        $autocompletions.push(Some(Box::new(|$aparte: Rc<Aparte>, $command: Command| -> Vec<String> { $completion })));
        generate_command_autocompletions!($autocompletions, $($argnames$(: $args)?),*);
    );
}

#[macro_export]
macro_rules! command_def {
    ($name:ident, $help: tt, |$aparte:ident, $command:ident| $body:block) => (
        fn $name() -> CommandParser {
            let autocompletions = Vec::<Option<Box<dyn Fn(Rc<Aparte>, Command) -> Vec<String>>>>::new();

            CommandParser {
                name: stringify!($name),
                help: $help,
                parser: Box::new(|$aparte: Rc<Aparte>, $command: Command| -> Result<(), String> {
                    #[allow(unused_mut)]
                    $body
                }),
                autocompletions: autocompletions,
            }
        }
    );
    ($name:ident, $help: tt, $($(($attr:ident))? $argnames:ident$(: $args:tt)?),*, |$aparte:ident, $command:ident| $body:block) => (
        fn $name() -> CommandParser {
            let mut autocompletions = Vec::<Option<Box<dyn Fn(Rc<Aparte>, Command) -> Vec<String>>>>::new();

            generate_command_autocompletions!(autocompletions, $($argnames$(: $args)?),*);

            CommandParser {
                name: stringify!($name),
                help: $help,
                parser: Box::new(|$aparte: Rc<Aparte>, $command: Command| -> Result<(), String> {
                    #[allow(unused_mut)]
                    let mut index = 1;
                    parse_command_args!($aparte, $command, index, $($(($attr))? $argnames),*);
                    $body
                }),
                autocompletions: autocompletions,
            }
        }
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    command_def!{
        no_args,
        "help",
        |_aparte, _command| {
            Ok(())
        }
    }

    #[test]
    fn test_command_without_args() {
        let cmd = no_args();

        assert_eq!(cmd.name, "no_args");
        assert_eq!(cmd.help, "help");
    }

    command_def!{
        one_arg,
        "help",
        _first_arg,
        |_aparte, _command| {
            Ok(())
        }
    }

    #[test]
    fn test_command_with_one_arg() {
        let cmd = one_arg();

        assert_eq!(cmd.name, "one_arg");
        assert_eq!(cmd.help, "help");
    }

    command_def!{
        one_arg_completion,
        "help",
        _first_arg: {
            completion: |_aparte, _command| {
                Vec::new()
            }
        },
        |_aparte, _command| {
            Ok(())
        }
    }

    #[test]
    fn test_command_with_one_arg_with_completion() {
        let cmd = one_arg_completion();

        assert_eq!(cmd.name, "one_arg_completion");
        assert_eq!(cmd.help, "help");
        assert_eq!(cmd.autocompletions.len(), 1);
    }

    command_def!{
        two_args,
        "help",
        _first_arg,
        _second_arg,
        |_aparte, _command| {
            Ok(())
        }
    }

    #[test]
    fn test_command_with_two_args() {
        let cmd = two_args();

        assert_eq!(cmd.name, "two_args");
        assert_eq!(cmd.help, "help");
        assert_eq!(cmd.autocompletions.len(), 2);
    }

    command_def!{
        two_args_completion,
        "help",
        _first_arg: {
            completion: |_aparte, _command| {
                Vec::new()
            }
        },
        _second_arg,
        |_aparte, _command| {
            Ok(())
        }
    }

    #[test]
    fn test_command_with_two_args_with_completion() {
        let cmd = two_args_completion();

        assert_eq!(cmd.name, "two_args_completion");
        assert_eq!(cmd.help, "help");
        assert_eq!(cmd.autocompletions.len(), 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_command_parsing() {
        let command = Command::try_from("/test command");
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.args.len(), 2);
        assert_eq!(command.args[0], "test");
        assert_eq!(command.args[1], "command");
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_multiple_args_command_parsing() {
        let command = Command::try_from("/test command with args");
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.args.len(), 4);
        assert_eq!(command.args[0], "test");
        assert_eq!(command.args[1], "command");
        assert_eq!(command.args[2], "with");
        assert_eq!(command.args[3], "args");
        assert_eq!(command.cursor, 3);
    }

    #[test]
    fn test_doubly_quoted_arg_command_parsing() {
        let command = Command::try_from("/test \"command with arg\"");
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.args.len(), 2);
        assert_eq!(command.args[0], "test");
        assert_eq!(command.args[1], "command with arg");
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_simply_quoted_arg_command_parsing() {
        let command = Command::try_from("/test 'command with arg'");
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.args.len(), 2);
        assert_eq!(command.args[0], "test");
        assert_eq!(command.args[1], "command with arg");
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_mixed_quote_arg_command_parsing() {
        let command = Command::try_from("/test 'command with \" arg'");
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.args.len(), 2);
        assert_eq!(command.args[0], "test");
        assert_eq!(command.args[1], "command with \" arg");
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_missing_closing_quote() {
        let command = Command::try_from("/test \"command with arg");
        assert!(command.is_err());
        assert_eq!(command.err(), Some("Missing closing quote"));
    }

    #[test]
    fn test_command_args_parsing_with_cursor() {
        let command = Command::parse_with_cursor("/test command with args", 10);
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.args.len(), 4);
        assert_eq!(command.args[0], "test");
        assert_eq!(command.args[1], "command");
        assert_eq!(command.args[2], "with");
        assert_eq!(command.args[3], "args");
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_command_parsing_with_cursor() {
        let command = Command::parse_with_cursor("/te", 3);
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0], "te");
        assert_eq!(command.cursor, 0);
    }

    #[test]
    fn test_command_end_with_space_parsing_with_cursor() {
        let command = Command::parse_with_cursor("/test ", 6);
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0], "test");
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_no_command_parsing_with_cursor() {
        let command = Command::parse_with_cursor("/", 1);
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0], "");
        assert_eq!(command.cursor, 0);
    }

    #[test]
    fn test_command_assemble() {
        let command = Command {
            args: vec!["foo".to_string(), "bar".to_string()],
            cursor: 0,
        };

        assert_eq!(command.assemble(), "/foo bar");
    }

    #[test]
    fn test_command_with_double_quote_assemble() {
        let command = Command {
            args: vec!["test".to_string(), "fo\"o".to_string(), "bar".to_string()],
            cursor: 0,
        };

        assert_eq!(command.assemble(), "/test 'fo\"o' bar");
    }

    #[test]
    fn test_command_with_simple_quote_assemble() {
        let command = Command {
            args: vec!["test".to_string(), "fo'o".to_string(), "bar".to_string()],
            cursor: 0,
        };

        assert_eq!(command.assemble(), "/test \"fo'o\" bar");
    }

    #[test]
    fn test_command_with_space_assemble() {
        let command = Command {
            args: vec!["test".to_string(), "foo bar".to_string()],
            cursor: 0,
        };

        assert_eq!(command.assemble(), "/test \"foo bar\"");
    }

    #[test]
    fn test_command_with_space_and_quote_assemble() {
        let command = Command {
            args: vec!["test".to_string(), "foo bar\"".to_string()],
            cursor: 0,
        };

        assert_eq!(command.assemble(), "/test 'foo bar\"'");
    }
}
