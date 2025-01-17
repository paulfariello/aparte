/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use anyhow::{anyhow, Result};
use terminus::cursor::Cursor;
use unicode_segmentation::UnicodeSegmentation as _;

use crate::account::Account;
use crate::core::Aparte;

#[derive(Debug, Clone)]
pub struct Command {
    pub account: Option<Account>,
    pub context: String,
    pub args: Vec<String>,
    pub cursor: usize,
}

impl Command {
    pub fn new(account: Option<Account>, context: String, buf: String) -> Result<Self> {
        let cursor = Cursor::from_index(&buf, buf.graphemes(true).count() - 1)
            .map_err(|_| anyhow!("Invalid index"))?;
        Command::parse_with_cursor(account, context, buf, cursor)
    }

    pub fn parse_name(buf: &str) -> Result<&str> {
        if &buf[0..1] != "/" {
            anyhow::bail!("Missing starting /");
        }

        let buf = &buf[1..];
        match buf.find(|c: char| !c.is_alphanumeric()) {
            Some(end) => Ok(&buf[..end]),
            None => Ok(buf),
        }
    }

    pub fn parse_with_cursor(
        account: Option<Account>,
        context: String,
        buf: String,
        cursor: Cursor,
    ) -> Result<Self> {
        enum State {
            Initial,
            Delimiter,
            SimplyQuoted,
            DoublyQuoted,
            Unquoted,
            UnquotedEscaped,
            SimplyQuotedEscaped,
            DoublyQuotedEscaped,
        }

        use State::*;

        let mut string_cursor = cursor
            .try_index(&buf)
            .map_err(|_| anyhow!("Invalid index"))?;
        let mut tokens: Vec<String> = Vec::new();
        let mut token = String::new();
        let mut state = Initial;
        let mut chars = buf.chars();
        let mut token_cursor = None;

        loop {
            let c = chars.next();
            state = match state {
                Initial => match c {
                    Some('/') => Delimiter,
                    _ => anyhow::bail!("Missing starting /"),
                },
                Delimiter => match c {
                    Some(' ') => Delimiter,
                    Some('\'') => SimplyQuoted,
                    Some('\"') => DoublyQuoted,
                    Some('\\') => UnquotedEscaped,
                    Some(c) => {
                        token.push(c);
                        Unquoted
                    }
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
                    }
                    None => anyhow::bail!("Missing closing quote"),
                },
                DoublyQuoted => match c {
                    Some('\"') => Unquoted,
                    Some('\\') => DoublyQuotedEscaped,
                    Some(c) => {
                        token.push(c);
                        DoublyQuoted
                    }
                    None => anyhow::bail!("Missing closing quote"),
                },
                Unquoted => match c {
                    Some('\'') => SimplyQuoted,
                    Some('\"') => DoublyQuoted,
                    Some('\\') => UnquotedEscaped,
                    Some(' ') => {
                        tokens.push(token);
                        token = String::new();
                        Delimiter
                    }
                    Some(c) => {
                        token.push(c);
                        Unquoted
                    }
                    None => {
                        tokens.push(token);
                        break;
                    }
                },
                UnquotedEscaped => match c {
                    Some(c) => {
                        token.push(c);
                        Unquoted
                    }
                    None => anyhow::bail!("Missing escaped char"),
                },
                SimplyQuotedEscaped => match c {
                    Some(c) => {
                        token.push(c);
                        SimplyQuoted
                    }
                    None => anyhow::bail!("Missing escaped char"),
                },
                DoublyQuotedEscaped => match c {
                    Some(c) => {
                        token.push(c);
                        DoublyQuoted
                    }
                    None => anyhow::bail!("Missing escaped char"),
                },
            };

            if string_cursor == 0 {
                if token_cursor.is_none() {
                    token_cursor = c.map(|_| tokens.len())
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

        if !tokens.is_empty() {
            Ok(Command {
                account,
                context,
                args: tokens,
                cursor: token_cursor.unwrap(),
            })
        } else {
            Ok(Command {
                account,
                context,
                args: vec!["".to_string()],
                cursor: token_cursor.unwrap(),
            })
        }
    }

    fn escape(arg: &str) -> String {
        let mut quote = None;
        let mut escaped = String::with_capacity(arg.len());
        for c in arg.chars() {
            escaped.push_str(&match c {
                '\\' => "\\\\".to_string(),
                ' ' => {
                    if quote.is_none() {
                        quote = Some(' ');
                    }
                    " ".to_string()
                }
                '\'' => match quote {
                    Some('\'') => "\\'".to_string(),
                    Some('"') => "'".to_string(),
                    Some(' ') | None => {
                        quote = Some('"');
                        "'".to_string()
                    }
                    Some(_) => unreachable!(),
                },
                '"' => match quote {
                    Some('\'') => "\"".to_string(),
                    Some('"') => "\\\"".to_string(),
                    Some(' ') | None => {
                        quote = Some('\'');
                        "\"".to_string()
                    }
                    Some(_) => unreachable!(),
                },
                c => c.to_string(),
            })
        }

        if quote == Some(' ') {
            quote = Some('"');
        }

        if quote.is_none() {
            escaped
        } else {
            format!("{}{}{}", quote.unwrap(), escaped, quote.unwrap())
        }
    }

    pub fn assemble_args(args: &[String]) -> String {
        let mut command = String::new();
        let mut first = true;
        for arg in args {
            if !first {
                command.push(' ');
            } else {
                first = false;
            }
            command.push_str(&Command::escape(arg));
        }

        command
    }

    pub fn assemble(&self) -> String {
        let mut command = "/".to_string();

        let args = Command::assemble_args(&self.args);
        command.push_str(&args);

        command
    }
}

type AutoCompletion = Box<dyn Fn(&mut Aparte, Command) -> Vec<String>>;

pub struct CommandParser {
    pub name: &'static str,
    pub help: String,
    pub parse: fn(&Option<Account>, &str, &str) -> anyhow::Result<Command>,
    pub exec: fn(&mut Aparte, Command) -> anyhow::Result<()>,
    pub autocompletions: Vec<Option<AutoCompletion>>,
}

#[macro_export]
macro_rules! parse_subcommand_attrs(
    ($map:ident, {}) => ();
    ($map:ident, { children: $subcommands:tt $(, $($tail:tt)*)? }) => (
        build_subcommand_map!($map, $subcommands);
    );
    ($map:ident, { completion: |$aparte:ident, $command:ident| $completion:block $(, $($tail:tt)*)? }) => ();
);

#[macro_export]
macro_rules! build_subcommand_map(
    ($map:ident, {}) => ();
    ($map:ident, { $subname:tt: $subparser:ident $(, $($tail:tt)*)? }) => (
        $map.insert(String::from_str($subname).unwrap(), $subparser::new());
        build_subcommand_map!($map, { $($($tail)*)? });
    );
    ($map:ident, { completion: |$aparte:ident, $command:ident| $completion:block $(, $($tail:tt)*)? }) => ();
);

#[macro_export]
macro_rules! parse_lookup_arg(
    ($aparte:ident, $command:ident, $({})?) => (None);
    ($aparte:ident, $command:ident, $({ lookup: |$lookup_aparte:ident, $lookup_command:ident| $lookup:block $(, $($tail:tt)*)? })?) => (
        $({
            (|$lookup_aparte: &mut Aparte, $lookup_command: &mut Command| $lookup)($aparte, &mut $command)
        })?
    );
    ($aparte:ident, $command:ident, $({ completion: |$completion_aparte:ident, $completion_command:ident| $lookup:block $(, $($tail:tt)*)? })?) => (
        parse_lookup_arg!($aparte, $command, { $($($tail)*)? })
    );
);

#[macro_export]
macro_rules! parse_command_args(
    ($aparte:ident, $command:ident, $index:ident, {}) => ();
    ($aparte:ident, $command:ident, $index:ident, { $arg:ident: Password $(= $attrs:tt)? $(,)? }) => (
        let $arg: Password = if $command.args.len() <= $index {
            let $arg: Option<Password> = parse_lookup_arg!($aparte, $command, $($attrs)?);
            match $arg {
               None => {
                   $aparte.schedule(Event::ReadPassword($command.clone()));
                   return Ok(())
               },
               Some($arg) => $arg,
            }
        } else {
            Password::from_str(&$command.args[$index])?
        };

        $index += 1;
    );
    ($aparte:ident, $command:ident, $index:ident, { $arg:ident: Option<$type:ty> $(= $attrs:tt)? $(, $($tail:tt)*)? }) => (
        let $arg: Option<$type> = {
            if $command.args.len() > $index {
                Some(<$type>::from_str(&$command.args[$index])?)
            } else {
                parse_lookup_arg!($aparte, $command, $($attrs)?)
            }
        };

        $index += 1;

        parse_command_args!($aparte, $command, $index, { $($($tail)*)? });
    );
    ($aparte:ident, $command:ident, $index:ident, { $arg:ident: Named<$type:ty> $(= $attrs:tt)? $(, $($tail:tt)*)? }) => (
        let $arg: Option<$type> = {
            // Could use let matching = $command.args.drain_filter(|a| a.starts_with(stringify!($arg))).collect::<Vec<String>>();
            let mut matching = Vec::new();
            let mut i = 0;
            while i != $command.args.len() {
                if $command.args[i].starts_with(stringify!($arg)) {
                    matching.push($command.args.remove(i));
                } else {
                    i += 1;
                }
            }
            match matching.as_slice() {
                [] => None,
                [named] => {
                    let arg = named.splitn(2, "=").collect::<Vec<&str>>()[1];
                    Some(<$type>::from_str(&arg)?)
                }
                _ => ::anyhow::bail!("Multiple occurance of {} argument", stringify!($arg)),
            }
        };

        parse_command_args!($aparte, $command, $index, { $($($tail)*)? });
    );
    ($aparte:ident, $command:ident, $index:ident, { $arg:ident: Command = $attrs:tt $(, $($tail:tt)*)? }) => (
        if $command.args.len() <= $index {
            ::anyhow::bail!("Missing {} argument", stringify!($arg))
        }

        let mut sub_commands: HashMap<String, CommandParser> = HashMap::new();
        parse_subcommand_attrs!(sub_commands, $attrs);

        return match sub_commands.get(&$command.args[$index]) {
            Some(sub_parser) => {
                let sub_command = Command {
                    args: $command.args[$index..].to_vec(),
                    ..$command
                };
                (sub_parser.exec)($aparte, sub_command)
            },
            None => ::anyhow::bail!("Invalid subcommand {}", $command.args[$index]),
        };
    );
    ($aparte:ident, $command:ident, $index:ident, { $arg:ident: $type:ty $(= $attrs:tt)? $(, $($tail:tt)*)? }) => (
        if $command.args.len() <= $index {
            ::anyhow::bail!("Missing {} argument", stringify!($arg))
        }

        let $arg: $type = <$type>::from_str(&$command.args[$index])?;

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
        #[allow(clippy::vec_init_then_push)]
        $completion.push(String::from($subname));
        generate_sub_autocompletion!($completion, { $($($tail)*)? });
    );
);

#[macro_export]
macro_rules! generate_arg_autocompletion(
    ($autocompletions:ident, $type:ty, {}) => ();
    ($autocompletions:ident, $type:ty, { lookup: |$aparte:ident, $command:ident| $completion:block $(, $($tail:tt)*)? }) => ();
    ($autocompletions:ident, $type:ty, { children: $subs:tt $(, $($tail:tt)*)? }) => (
        #[allow(clippy::vec_init_then_push)]
        let sub = {
            let mut sub = vec![];
            generate_sub_autocompletion!(sub, $subs);
            sub
        };
        $autocompletions.push(Some(Box::new(move |_: &mut Aparte, _: Command| -> Vec<String> { sub.clone() })));
        generate_arg_autocompletion!($autocompletions, $type, { $($($tail)*)? });
    );
    ($autocompletions:ident, $type:ty, { completion: |$aparte:ident, $command:ident| $completion:block $(, $($tail:tt)*)? }) => (
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

            fn parse(account: &Option<Account>, context: &str, buf: &str) -> ::anyhow::Result<Command> {
                Command::new(account.clone(), context.to_string(), buf.to_string())
            }

            fn exec(aparte: &mut Aparte, command: Command) -> ::anyhow::Result<()> {
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
                    parse,
                    exec,
                    autocompletions,
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

            fn parse(account: &Option<Account>, context: &str, buf: &str) -> ::anyhow::Result<Command> {
                Command::new(account.clone(), context.to_string(), buf.to_string())
            }

            fn exec($aparte: &mut Aparte, mut $command: Command) -> ::anyhow::Result<()> {
                #[allow(unused_variables, unused_mut)]
                let mut index = 1;
                parse_command_args!($aparte, $command, index, $args);

                // Avoid unused_assignement warning
                // should use #[allow(unused_assignments)]
                let _ = index;
                $body
            }

            pub fn new() -> CommandParser {
                #[allow(unused_mut, clippy::vec_init_then_push)]
                let autocompletions = {
                    let mut autocompletions = Vec::<Option<Box<dyn Fn(&mut Aparte, Command) -> Vec<String>>>>::new();

                    generate_command_autocompletions!(autocompletions, $args);
                    autocompletions
                };

                CommandParser {
                    name: stringify!($name),
                    help: help(),
                    parse,
                    exec,
                    autocompletions,
                }
            }
        }
    );
);

#[cfg(test)]
mod tests_command_macro {
    use super::*;
    use std::str::FromStr;

    command_def!(no_args, "help", {}, |_aparte, _command| { Ok(()) });

    #[test]
    fn test_command_without_args() {
        let cmd = no_args::new();

        assert_eq!(cmd.name, "no_args");
        assert_eq!(cmd.help, "help");
    }

    command_def!(
        one_arg,
        "help",
        { _first_arg: String },
        |_aparte, _command| { Ok(()) }
    );

    #[test]
    fn test_command_with_one_arg() {
        let cmd = one_arg::new();

        assert_eq!(cmd.name, "one_arg");
        assert_eq!(cmd.help, "help");
    }

    command_def!(one_arg_completion, "help", {
                  _first_arg: String = {
                      completion: |_aparte, _command| {
                          Vec::new()
                      }
                  }
    }, |_aparte, _command| { Ok(()) });

    #[test]
    fn test_command_with_one_arg_with_completion() {
        let cmd = one_arg_completion::new();

        assert_eq!(cmd.name, "one_arg_completion");
        assert_eq!(cmd.help, "help");
        assert_eq!(cmd.autocompletions.len(), 1);
    }

    command_def!(two_args, "help", { _first_arg: String, _second_arg: String }, |_aparte, _command| { Ok(()) });

    #[test]
    fn test_command_with_two_args() {
        let cmd = two_args::new();

        assert_eq!(cmd.name, "two_args");
        assert_eq!(cmd.help, "help");
        assert_eq!(cmd.autocompletions.len(), 2);
    }

    command_def!(two_args_completion, "help", {
        _first_arg: String = {
            completion: |_aparte, _command| {
                Vec::new()
            }
        },
        _second_arg: String
    }, |_aparte, _command| { Ok(()) });

    #[test]
    fn test_command_with_two_args_with_completion() {
        let cmd = two_args_completion::new();

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
        let command = Command::new(None, "test".to_string(), "/test command".to_string());
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.args.len(), 2);
        assert_eq!(command.args[0], "test");
        assert_eq!(command.args[1], "command");
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_multiple_args_command_parsing() {
        let command = Command::new(
            None,
            "test".to_string(),
            "/test command with args".to_string(),
        );
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
        let command = Command::new(
            None,
            "test".to_string(),
            "/test \"command with arg\"".to_string(),
        );
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.args.len(), 2);
        assert_eq!(command.args[0], "test");
        assert_eq!(command.args[1], "command with arg");
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_simply_quoted_arg_command_parsing() {
        let command = Command::new(
            None,
            "test".to_string(),
            "/test 'command with arg'".to_string(),
        );
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.args.len(), 2);
        assert_eq!(command.args[0], "test");
        assert_eq!(command.args[1], "command with arg");
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_mixed_quote_arg_command_parsing() {
        let command = Command::new(
            None,
            "test".to_string(),
            "/test 'command with \" arg'".to_string(),
        );
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.args.len(), 2);
        assert_eq!(command.args[0], "test");
        assert_eq!(command.args[1], "command with \" arg");
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_missing_closing_quote() {
        let command = Command::new(
            None,
            "test".to_string(),
            "/test \"command with arg".to_string(),
        );
        assert!(command.is_err());
        assert_eq!(
            format!("{}", command.err().unwrap()),
            "Missing closing quote"
        );
    }

    #[test]
    fn test_command_args_parsing_with_cursor() {
        let command = Command::parse_with_cursor(
            None,
            "test".to_string(),
            "/test command with args".to_string(),
            Cursor::new(10),
        );
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
        let command =
            Command::parse_with_cursor(None, "test".to_string(), "/te".to_string(), Cursor::new(3));
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0], "te");
        assert_eq!(command.cursor, 0);
    }

    #[test]
    fn test_command_end_with_space_parsing_with_cursor() {
        let command = Command::parse_with_cursor(
            None,
            "test".to_string(),
            "/test ".to_string(),
            Cursor::new(6),
        );
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0], "test");
        assert_eq!(command.cursor, 1);
    }

    #[test]
    fn test_no_command_parsing_with_cursor() {
        let command =
            Command::parse_with_cursor(None, "test".to_string(), "/".to_string(), Cursor::new(1));
        assert!(command.is_ok());
        let command = command.unwrap();
        assert_eq!(command.args.len(), 1);
        assert_eq!(command.args[0], "");
        assert_eq!(command.cursor, 0);
    }

    #[test]
    fn test_command_assemble() {
        let command = Command {
            account: None,
            context: "test".to_string(),
            args: vec!["foo".to_string(), "bar".to_string()],
            cursor: 0,
        };

        assert_eq!(command.assemble(), "/foo bar");
    }

    #[test]
    fn test_command_with_double_quote_assemble() {
        let command = Command {
            account: None,
            context: "test".to_string(),
            args: vec!["test".to_string(), "fo\"o".to_string(), "bar".to_string()],
            cursor: 0,
        };

        assert_eq!(command.assemble(), "/test 'fo\"o' bar");
    }

    #[test]
    fn test_command_with_simple_quote_assemble() {
        let command = Command {
            account: None,
            context: "test".to_string(),
            args: vec!["test".to_string(), "fo'o".to_string(), "bar".to_string()],
            cursor: 0,
        };

        assert_eq!(command.assemble(), "/test \"fo'o\" bar");
    }

    #[test]
    fn test_command_with_space_assemble() {
        let command = Command {
            account: None,
            context: "test".to_string(),
            args: vec!["test".to_string(), "foo bar".to_string()],
            cursor: 0,
        };

        assert_eq!(command.assemble(), "/test \"foo bar\"");
    }

    #[test]
    fn test_command_with_space_and_quote_assemble() {
        let command = Command {
            account: None,
            context: "test".to_string(),
            args: vec!["test".to_string(), "foo bar\"".to_string()],
            cursor: 0,
        };

        assert_eq!(command.assemble(), "/test 'foo bar\"'");
    }

    #[test]
    fn test_command_parse_name() {
        let name = Command::parse_name("/me's best client is Aparté");
        assert!(name.is_ok());
        assert_eq!("me", name.unwrap());
    }

    #[test]
    fn test_command_parse_name_without_args() {
        let name = Command::parse_name("/close");
        assert!(name.is_ok());
        assert_eq!("close", name.unwrap());
    }
}
