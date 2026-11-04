/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MotionType {
    #[default]
    Char,
    Line,
    Block,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterValue {
    pub text: String,
    pub kind: MotionType,
}

impl RegisterValue {
    pub fn new(text: impl Into<String>, kind: MotionType) -> Self {
        Self {
            text: text.into(),
            kind,
        }
    }
}

/// Vim-style named register store.
///
/// Each register holds a `RegisterValue` (text + kind). Special rules:
/// - Uppercase names (A–Z) are aliases: `set` appends to the lowercase
///   counterpart; `get` reads from it.
/// - `UNNAMED` (`"`) is the default register; `yank` always writes to it
///   in addition to any explicitly named register.
#[derive(Debug, Default)]
pub struct Registers {
    store: HashMap<char, RegisterValue>,
}

impl Registers {
    pub const UNNAMED: char = '"';

    pub fn new() -> Self {
        Self::default()
    }

    /// Write `value` into register `name`.
    ///
    /// If `name` is an uppercase ASCII letter, the text is **appended** to the
    /// lowercase counterpart's content (creating it if absent). The kind of the
    /// existing entry is preserved; if the entry is new, `value.kind` is used.
    pub fn set(&mut self, name: char, value: RegisterValue) {
        let key = to_lower(name);
        if name.is_ascii_uppercase() {
            if let Some(existing) = self.store.get_mut(&key) {
                existing.text.push_str(&value.text);
            } else {
                self.store.insert(key, value);
            }
        } else {
            self.store.insert(key, value);
        }
    }

    /// Read from register `name`.
    ///
    /// Uppercase names are resolved to their lowercase counterpart.
    pub fn get(&self, name: char) -> Option<&RegisterValue> {
        self.store.get(&to_lower(name))
    }

    /// Yank `value`: write to `name` (if `Some`) **and** to `UNNAMED`.
    ///
    /// Uppercase names follow the same append rule as `set`. After the named
    /// write, the current content of the lowercase register (post-append) is
    /// cloned into `UNNAMED`, so both always hold the same final text.
    pub fn yank(&mut self, name: Option<char>, value: RegisterValue) {
        if let Some(n) = name {
            self.set(n, value);
            let shadowed = self.store[&to_lower(n)].clone();
            self.store.insert(Self::UNNAMED, shadowed);
        } else {
            self.store.insert(Self::UNNAMED, value);
        }
    }
}

fn to_lower(name: char) -> char {
    if name.is_ascii_uppercase() {
        name.to_ascii_lowercase()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(text: &str) -> RegisterValue {
        RegisterValue {
            text: text.to_string(),
            kind: MotionType::Char,
        }
    }

    #[test]
    fn set_get_roundtrip() {
        let mut r = Registers::new();
        r.set('a', val("hello"));
        assert_eq!(r.get('a').unwrap().text, "hello");
    }

    #[test]
    fn get_empty_returns_none() {
        let r = Registers::new();
        assert!(r.get('z').is_none());
    }

    #[test]
    fn overwrite_replaces_value() {
        let mut r = Registers::new();
        r.set('a', val("first"));
        r.set('a', val("second"));
        assert_eq!(r.get('a').unwrap().text, "second");
    }

    #[test]
    fn registers_are_independent() {
        let mut r = Registers::new();
        r.set('a', val("alpha"));
        assert!(r.get('b').is_none());
    }

    #[test]
    fn uppercase_set_appends_to_lowercase() {
        let mut r = Registers::new();
        r.set('a', val("hello "));
        r.set('A', val("world"));
        assert_eq!(r.get('a').unwrap().text, "hello world");
    }

    #[test]
    fn uppercase_set_on_empty_creates_entry() {
        let mut r = Registers::new();
        r.set('A', val("new"));
        assert_eq!(r.get('a').unwrap().text, "new");
    }

    #[test]
    fn uppercase_get_reads_lowercase() {
        let mut r = Registers::new();
        r.set('a', val("data"));
        assert_eq!(r.get('A').unwrap().text, "data");
    }

    #[test]
    fn yank_named_also_sets_unnamed() {
        let mut r = Registers::new();
        r.yank(Some('a'), val("yanked"));
        assert_eq!(r.get('a').unwrap().text, "yanked");
        assert_eq!(r.get(Registers::UNNAMED).unwrap().text, "yanked");
    }

    #[test]
    fn yank_none_sets_only_unnamed() {
        let mut r = Registers::new();
        r.yank(None, val("anon"));
        assert_eq!(r.get(Registers::UNNAMED).unwrap().text, "anon");
        assert!(r.get('a').is_none());
    }

    #[test]
    fn yank_uppercase_appends_and_updates_unnamed() {
        let mut r = Registers::new();
        r.set('a', val("line1\n"));
        r.yank(Some('A'), val("line2"));
        assert_eq!(r.get('a').unwrap().text, "line1\nline2");
        assert_eq!(r.get(Registers::UNNAMED).unwrap().text, "line1\nline2");
    }

    #[test]
    fn motion_type_default_is_char() {
        assert_eq!(MotionType::default(), MotionType::Char);
    }

    #[test]
    fn unnamed_constant_is_double_quote() {
        assert_eq!(Registers::UNNAMED, '"');
    }
}
