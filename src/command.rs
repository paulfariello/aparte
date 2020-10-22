/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::convert::TryFrom;
#[allow(unused_imports)]
use textwrap;

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
    pub help: String,
    pub parser: fn(&mut Aparte, Command) -> Result<(), String>,
    pub autocompletions: Vec<Option<Box<dyn Fn(&mut Aparte, Command) -> Vec<String>>>>,
}

#[macro_export]
macro_rules! parse_subcommand_attrs(
    ($map:ident, {}) => ();
    ($map:ident, { children: $subcommands:tt $(, $($tail:tt)*)? }) => (
        build_subcommand_map!($map, $subcommands);
    );
    ($map:ident, { completion: (|$aparte:ident, $command:ident| $completion:block) $(, $($tail:tt)*)? }) => ();
);

#[macro_export]
macro_rules! build_subcommand_map(
    ($map:ident, {}) => ();
    ($map:ident, { $subname:tt: $subparser:ident $(, $($tail:tt)*)? }) => (
        $map.insert(String::from_str($subname).unwrap(), $subparser::new());
        build_subcommand_map!($map, { $($($tail)*)? });
    );
    ($map:ident, { completion: (|$aparte:ident, $command:ident| $completion:block) $(, $($tail:tt)*)? }) => ();
);

#[macro_export]
macro_rules! parse_command_args(
    ($aparte:ident, $command:ident, $index:ident, {}) => ();
    ($aparte:ident, $command:ident, $index:ident, { $arg:ident: Password<$type:ty> }) => (
        if $command.args.len() <= $index {
            $aparte.schedule(Event::ReadPassword($command.clone()));
            return Ok(())
        }

        let $arg: Password<$type> = match Password::from_str(&$command.args[$index]) {
            Ok(arg) => arg,
            Err(e) => return Err(format!("Invalid format for {} argument: {}", stringify!($arg), e)),
        };

        #[allow(unused_assignments)]
        $index += 1;
    );
    ($aparte:ident, $command:ident, $index:ident, { $arg:ident: Option<$type:ty> $(= $attr:tt)? $(, $($tail:tt)*)? }) => (
        let $arg: Option<$type> = {
            if $command.args.len() > $index {
                match <$type>::from_str(&$command.args[$index]) {
                    Ok(arg) => Some(arg),
                    Err(e) => return Err(format!("Invalid format for {} argument: {}", stringify!($arg), e)),
                }
            } else {
                None
            }
        };

        #[allow(unused_assignments)]
        $index += 1;

        parse_command_args!($aparte, $command, $index, { $($($tail)*)? });
    );
    ($aparte:ident, $command:ident, $index:ident, { $arg:ident: Named<$type:ty> $(= $attr:tt)? $(, $($tail:tt)*)? }) => (
        let $arg: Option<$type> = {
            let matching = $command.args.drain_filter(|a| a.starts_with(stringify!($arg))).collect::<Vec<String>>();
            match matching.as_slice() {
                [] => None,
                [named] => {
                    let arg = named.splitn(2, "=").collect::<Vec<&str>>()[1];
                    match <$type>::from_str(&arg) {
                        Ok(arg) => Some(arg),
                        Err(e) => return Err(format!("Invalid format for {} argument: {}", stringify!($arg), e)),
                    }
                }
                _ => return Err(format!("Multiple occurance of {} argument", stringify!($arg))),
            }
        };

        parse_command_args!($aparte, $command, $index, { $($($tail)*)? });
    );
    ($aparte:ident, $command:ident, $index:ident, { $arg:ident: Command = $attr:tt $(, $($tail:tt)*)? }) => (
        if $command.args.len() <= $index {
            return Err(format!("Missing {} argument", stringify!($arg)))
        }

        let mut sub_commands: HashMap<String, CommandParser> = HashMap::new();
        parse_subcommand_attrs!(sub_commands, $attr);

        return match sub_commands.get(&$command.args[$index]) {
            Some(sub_parser) => (sub_parser.parser)($aparte, Command { args: $command.args[$index..].to_vec(), cursor: 0 }),
            None => Err(format!("Invalid subcommand {}", $command.args[$index])),
        };
    );
    ($aparte:ident, $command:ident, $index:ident, { $arg:ident: $type:ty $(= $attr:tt)? $(, $($tail:tt)*)? }) => (
        if $command.args.len() <= $index {
            return Err(format!("Missing {} argument", stringify!($arg)))
        }

        let $arg: $type = match <$type>::from_str(&$command.args[$index]) {
            Ok(arg) => arg,
            Err(e) => return Err(format!("Invalid format for {} argument: {}", stringify!($arg), e)),
        };

        #[allow(unused_assignments)]
        $index += 1;

        parse_command_args!($aparte, $command, $index, { $($($tail)*)? });
    );
);

#[macro_export]
macro_rules! generate_command_autocompletions(
    ($autocompletions:ident, {}) => ();
    ($autocompletions:ident, { $argname:ident: $type:ty = $attrs:tt $(, $($tail:tt)*)? }) => (
        let count = $autocompletions.len();
        generate_arg_autocompletion!($autocompletions, $type, $attrs);
        if count == $autocompletions.len() {
            $autocompletions.push(None);
        }
        assert!($autocompletions.len() == count + 1, "Two completion pushed for the argument {}", stringify!($argname));
        generate_command_autocompletions!($autocompletions, { $($($tail)*)? });
    );
    ($autocompletions:ident, { $argname:ident: $type:ty $(, $($tail:tt)*)? }) => (
        $autocompletions.push(None);
        generate_command_autocompletions!($autocompletions, { $($($tail)*)? });
    );
);


#[macro_export]
macro_rules! generate_sub_autocompletion(
    ($completion:ident, {}) => ();
    ($completion:ident, { $subname:tt: $sub:ident $(, $($tail:tt)*)? }) => (
        $completion.push(String::from($subname));
        generate_sub_autocompletion!($completion, { $($($tail)*)? });
    );
);

#[macro_export]
macro_rules! generate_arg_autocompletion(
    ($autocompletions:ident, $type:ty, {}) => ();
    ($autocompletions:ident, $type:ty, { children: $subs:tt $(, $($tail:tt)*)? }) => (
        let mut sub = vec![];
        generate_sub_autocompletion!(sub, $subs);
        $autocompletions.push(Some(Box::new(move |_: &mut Aparte, _: Command| -> Vec<String> { sub.clone() })));
        generate_arg_autocompletion!($autocompletions, $type, { $($($tail)*)? });
    );
    ($autocompletions:ident, $type:ty, { completion: (|$aparte:ident, $command:ident| $completion:block) $(, $($tail:tt)*)? }) => (
        $autocompletions.push(Some(Box::new(|$aparte: &mut Aparte, $command: Command| -> Vec<String> { $completion })));
        generate_arg_autocompletion!($autocompletions, $type, { $($($tail)*)? });
    );
);

#[macro_export]
macro_rules! generate_sub_help(
    ($help:ident, {}) => ();
    ($help:ident, { $subname:tt: $sub:ident $(, $($tail:tt)*)? }) => (
        $help.push(String::from("\n"));
        let sub_help = $sub::help();
        $help.push(textwrap::indent(&sub_help, "\t"));
        generate_sub_help!($help, { $($($tail)*)? });
    );
);

#[macro_export]
macro_rules! generate_subs_help(
    ($help:ident, { children: $subs:tt $(, $($tail:tt)*)? }) => (
        generate_sub_help!($help, $subs);
    );
);

#[macro_export]
macro_rules! generate_help(
    ($help:ident, {}) => ();
    ($help:ident, { $arg:ident: Command = $attr:tt $(, $($tail:tt)*)? }) => (
        generate_subs_help!($help, $attr);
        generate_help!($help, { $($($tail)*)? });
    );
    ($help:ident, { $arg:ident: $type:ty $(= $attr:tt)? $(, $($tail:tt)*)? }) => (
        generate_help!($help, { $($($tail)*)? });
    );
);

#[macro_export]
macro_rules! command_def (
    ($name:ident, $help:tt, $args:tt) => (
        mod $name {
            use super::*;

            pub fn help() -> String {
                #[allow(unused_mut)]
                let mut help = vec![String::from($help)];
                generate_help!(help, $args);
                return help.join("\n");
            }

            fn parser(aparte: &mut Aparte, command: Command) -> Result<(), String> {
                #[allow(unused_variables, unused_mut)]
                let mut index = 1;
                parse_command_args!(aparte, command, index, $args);
            }

            pub fn new() -> CommandParser {
                let mut autocompletions = Vec::<Option<Box<dyn Fn(&mut Aparte, Command) -> Vec<String>>>>::new();
                generate_command_autocompletions!(autocompletions, $args);

                CommandParser {
                    name: stringify!($name),
                    help: help(),
                    parser: parser,
                    autocompletions: autocompletions,
                }
            }
        }
    );
    ($name:ident, $help:tt, $args:tt, |$aparte:ident, $command:ident| $body:block) => (
        mod $name {
            use super::*;

            pub fn help() -> String {
                #[allow(unused_mut)]
                let mut help = vec![String::from($help)];
                generate_help!(help, $args);
                return help.join("\n");
            }

            fn parser($aparte: &mut Aparte, mut $command: Command) -> Result<(), String> {
                #[allow(unused_variables, unused_mut)]
                let mut index = 1;
                parse_command_args!($aparte, $command, index, $args);
                $body
            }

            pub fn new() -> CommandParser {
                #[allow(unused_mut)]
                let mut autocompletions = Vec::<Option<Box<dyn Fn(&mut Aparte, Command) -> Vec<String>>>>::new();

                generate_command_autocompletions!(autocompletions, $args);

                CommandParser {
                    name: stringify!($name),
                    help: help(),
                    parser: parser,
                    autocompletions: autocompletions,
                }
            }
        }
    );
);

#[cfg(test)]
mod tests_command_macro {
    use super::*;

    command_def!(no_args, "help", {}, |_aparte, _command| { Ok(()) });

    #[test]
    fn test_command_without_args() {
        let cmd = no_args();

        assert_eq!(cmd.name, "no_args");
        assert_eq!(cmd.help, "help");
    }

    command_def!(one_arg, "help", { _first_arg: String }, |_aparte, _command| { Ok(()) });

    #[test]
    fn test_command_with_one_arg() {
        let cmd = one_arg();

        assert_eq!(cmd.name, "one_arg");
        assert_eq!(cmd.help, "help");
    }

    command_def!(one_arg_completion, "help", {
                  _first_arg: String = {
                      completion: (|_aparte, _command| {
                          Vec::new()
                      })
                  }
    }, |_aparte, _command| { Ok(()) });

    #[test]
    fn test_command_with_one_arg_with_completion() {
        let cmd = one_arg_completion();

        assert_eq!(cmd.name, "one_arg_completion");
        assert_eq!(cmd.help, "help");
        assert_eq!(cmd.autocompletions.len(), 1);
    }

    command_def!(two_args, "help", { _first_arg: String, _second_arg: String }, |_aparte, _command| { Ok(()) });

    #[test]
    fn test_command_with_two_args() {
        let cmd = two_args();

        assert_eq!(cmd.name, "two_args");
        assert_eq!(cmd.help, "help");
        assert_eq!(cmd.autocompletions.len(), 2);
    }

    command_def!(two_args_completion, "help", {
        _first_arg: String = {
            completion: (|_aparte, _command| {
                Vec::new()
            })
        },
        _second_arg: String
    }, |_aparte, _command| { Ok(()) });

    #[test]
    fn test_command_with_two_args_with_completion() {
        let cmd = two_args_completion();

        assert_eq!(cmd.name, "two_args_completion");
        assert_eq!(cmd.help, "help");
        assert_eq!(cmd.autocompletions.len(), 2);
    }
}

#[cfg(test)]
mod tests_command_parser {
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
