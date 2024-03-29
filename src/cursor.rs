use std::cell::Cell;

use unicode_segmentation::UnicodeSegmentation;

/// Represent a position in a rendered string (starting at 0)
///
/// Cursor(1) on "être" points on 't'
/// corresponding byte index is 2
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Cursor(Cell<usize>);

impl Cursor {
    pub fn new(value: usize) -> Self {
        Self(Cell::new(value))
    }

    pub fn from_index(input: &str, index: usize) -> Result<Self, ()> {
        let mut value = 0;
        for (indice, grapheme) in input.grapheme_indices(true) {
            if indice <= index && index < indice + grapheme.len() {
                return Ok(Self(Cell::new(value)));
            }
            value += 1;
        }

        if index == input.len() {
            Ok(Self(Cell::new(value)))
        } else {
            Err(())
        }
    }

    pub fn index(&self, input: &str) -> usize {
        match input.grapheme_indices(true).nth(self.0.get()) {
            Some((indice, _)) => indice,
            None => input.len(),
        }
    }

    pub fn try_index(&self, input: &str) -> Result<usize, ()> {
        match input.grapheme_indices(true).nth(self.0.get()) {
            Some((indice, _)) => Ok(indice),
            None => {
                if input.grapheme_indices(true).count() == self.0.get() {
                    Ok(input.len())
                } else {
                    Err(())
                }
            }
        }
    }

    pub fn get(&self) -> usize {
        self.0.get()
    }

    pub fn set(&self, value: usize) {
        self.0.set(value)
    }

    pub fn update(&self, other: Self) {
        self.0.set(other.get())
    }
}

impl std::ops::Add<usize> for &Cursor {
    type Output = Cursor;

    fn add(self, other: usize) -> <Self as std::ops::Add<usize>>::Output {
        Cursor::new(self.get() + other)
    }
}

impl std::ops::AddAssign<usize> for Cursor {
    fn add_assign(&mut self, other: usize) {
        self.set(self.get() + other)
    }
}

impl std::ops::Sub<usize> for &Cursor {
    type Output = Cursor;

    fn sub(self, other: usize) -> <Self as std::ops::Sub<usize>>::Output {
        Cursor::new(self.get() - other)
    }
}

impl std::ops::SubAssign<usize> for Cursor {
    fn sub_assign(&mut self, other: usize) {
        self.set(self.get() - other)
    }
}

impl std::ops::Add for &Cursor {
    type Output = Cursor;

    fn add(self, other: Self) -> <Self as std::ops::Add<Self>>::Output {
        Cursor::new(self.get() + other.get())
    }
}

impl std::ops::AddAssign for Cursor {
    fn add_assign(&mut self, other: Self) {
        self.set(self.get() + other.get())
    }
}

impl std::ops::Sub for &Cursor {
    type Output = Cursor;

    fn sub(self, other: Self) -> <Self as std::ops::Sub<Self>>::Output {
        Cursor::new(self.get() - other.get())
    }
}

impl std::ops::SubAssign for Cursor {
    fn sub_assign(&mut self, other: Self) {
        self.set(self.get() - other.get())
    }
}

impl std::cmp::PartialOrd<usize> for Cursor {
    fn partial_cmp(&self, other: &usize) -> Option<std::cmp::Ordering> {
        Some(self.get().cmp(other))
    }
}

impl std::cmp::PartialEq<usize> for Cursor {
    fn eq(&self, other: &usize) -> bool {
        self.get().eq(other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_on_simple_string() {
        // Given
        let input = "abcd";
        let cursor = Cursor::new(2); // point to 'c'

        // When
        let index = cursor.index(input);

        // Then
        assert_eq!(index, 2);
    }

    #[test]
    fn test_cursor_on_multibyte_codepoint() {
        // Given
        let input = "🍺a";
        let cursor = Cursor::new(1); // points to 'a'

        // When
        let index = cursor.index(input);

        // Then
        assert_eq!(index, 4);
        assert_eq!(input[index..], "a"[..]);
    }

    #[test]
    fn test_cursor_on_multicodepoint_grapheme() {
        // Given
        let input = "êa";
        let cursor = Cursor::new(1); // points to 'a'

        // When
        let index = cursor.index(input);

        // Then
        assert_eq!(index, 2);
        assert_eq!(input[index..], "a"[..]);
    }

    #[test]
    fn test_cursor_from_index() {
        // Given
        let input = "🍺a";

        // When
        let cursor = Cursor::from_index(input, 1); // points to '🍺'

        // Then
        assert_eq!(cursor, Ok(Cursor::new(0)));
    }

    #[test]
    fn test_cursor_from_index_at_end() {
        // Given
        let input = "abc";

        // When
        let cursor = Cursor::from_index(input, 3);

        // Then
        assert_eq!(cursor, Ok(Cursor::new(3)));
    }

    #[test]
    fn test_cursor_index_at_end() {
        // Given
        let input = "abc";
        let cursor = Cursor::new(3);

        // When
        let index = cursor.index(input);

        // Then
        assert_eq!(index, 3);
    }
}
