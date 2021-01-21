use std::ops::Range;
use std::str::Chars;

#[derive(Debug, Clone)]
enum ParserState {
    Init,
    Space,
    Separator,
    Word,
}

#[derive(Clone)]
pub struct Words<'a> {
    /// String buffer to be splited into words
    buf: &'a str,
    /// Char iterator (utf codepoint)
    chars: Chars<'a>,
    /// Current parsing state
    state: ParserState,
    /// Current codepoint index in self.buf
    /// must be converted to byte index with byte_index()
    index: usize,
    /// Current word start
    /// This is required because chars iterator always have one char in advance
    word_start: usize,
}

impl<'a> Words<'a> {
    pub fn new(buf: &'a str) -> Words<'a> {
        Self {
            buf,
            chars: buf.chars(),
            state: ParserState::Init,
            index: 0,
            word_start: 0,
        }
    }

    fn word_at(&self, range: Range<usize>) -> &'a str {
        let start = byte_index(self.buf, range.start);
        let end = byte_index(self.buf, range.end);
        &self.buf[start..end]
    }
}

impl<'a> Iterator for Words<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        let mut word = None;

        while let Some(c) = self.chars.next() {
            self.state = match self.state {
                ParserState::Init => match c {
                    ' ' => ParserState::Space,
                    '/' | '\\' | '\'' | '"' | '&' | '(' | ')' | '*' | ',' | ';' | '<' | '='
                    | '>' | '?' | '@' | '[' | ']' | '^' | '{' | '|' | '}' => ParserState::Separator,
                    _ => ParserState::Word,
                },
                ParserState::Space => match c {
                    ' ' => ParserState::Space,
                    '/' | '\\' | '\'' | '"' | '&' | '(' | ')' | '*' | ',' | ';' | '<' | '='
                    | '>' | '?' | '@' | '[' | ']' | '^' | '{' | '|' | '}' => {
                        word = Some(self.word_at(self.word_start..self.index));
                        ParserState::Separator
                    }
                    _ => {
                        word = Some(self.word_at(self.word_start..self.index));
                        ParserState::Word
                    }
                },
                ParserState::Separator => match c {
                    '/' | '\\' | '\'' | '"' | '&' | '(' | ')' | '*' | ',' | ';' | '<' | '='
                    | '>' | '?' | '@' | '[' | ']' | '^' | '{' | '|' | '}' => ParserState::Separator,
                    ' ' => ParserState::Space,
                    _ => {
                        word = Some(self.word_at(self.word_start..self.index));
                        ParserState::Word
                    }
                },
                ParserState::Word => match c {
                    ' ' => ParserState::Space,
                    '/' | '\\' | '\'' | '"' | '&' | '(' | ')' | '*' | ',' | ';' | '<' | '='
                    | '>' | '?' | '@' | '[' | ']' | '^' | '{' | '|' | '}' => {
                        word = Some(self.word_at(self.word_start..self.index));
                        ParserState::Separator
                    }
                    _ => ParserState::Word,
                },
            };

            if word.is_some() {
                self.word_start = self.index;
                self.index += 1;
                return word;
            } else {
                self.index += 1;
            }
        }

        if self.word_start != self.index {
            word = Some(self.word_at(self.word_start..self.index));
            self.word_start = self.index;
            word
        } else {
            None
        }
    }
}

pub fn byte_index(buf: &str, mut cursor: usize) -> usize {
    let mut byte_index = 0;
    while cursor > 0 && byte_index < buf.len() {
        byte_index += 1;
        if buf.is_char_boundary(byte_index) {
            cursor -= 1;
        }
    }
    byte_index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_words() {
        // Given
        let input = "three simple words";

        // When
        let words = Words::new(input);

        // Then
        assert_eq!(
            words.collect::<Vec<&str>>(),
            vec!["three ", "simple ", "words"]
        );
    }

    #[test]
    fn test_split_multiple_spaces() {
        // Given
        let input = "   ";

        // When
        let words = Words::new(input);

        // Then
        assert_eq!(words.collect::<Vec<&str>>(), vec!["   "]);
    }

    #[test]
    fn test_split_spaces_and_separator() {
        // Given
        let input = "a && b";

        // When
        let words = Words::new(input);

        // Then
        assert_eq!(words.collect::<Vec<&str>>(), vec!["a ", "&& ", "b"]);
    }

    #[test]
    fn test_split_multibyte_codepoint() {
        // Given
        let input = "I love 🍺 more than 🐈";

        // When
        let words = Words::new(input);

        // Then
        assert_eq!(
            words.collect::<Vec<&str>>(),
            vec!["I ", "love ", "🍺 ", "more ", "than ", "🐈"]
        );
    }

    #[test]
    fn test_empty_string() {
        // Given
        let input = "";

        // When
        let words = Words::new(input);

        // Then
        assert_eq!(words.collect::<Vec<&str>>(), Vec::<&str>::new());
    }
}
