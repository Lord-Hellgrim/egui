use std::{borrow::Cow, ops::Range, str::Utf8Error};

use epaint::{
    text::{
        cursor::{CCursor, PCursor},
        TAB_SIZE,
    },
    Galley,
};

use crate::text_selection::{
    text_cursor_state::{
        byte_index_from_char_index, ccursor_next_word, ccursor_previous_word, find_line_start,
        slice_char_range,
    },
    CursorRange,
};

/// Trait constraining what types [`crate::TextEdit`] may use as
/// an underlying buffer.
///
/// Most likely you will use a [`String`] which implements [`TextBuffer`].
pub trait TextBuffer {
    /// Can this text be edited?
    fn is_mutable(&self) -> bool;

    /// Returns this buffer as a `str`.
    fn as_str(&self) -> &str;

    /// Inserts text `text` into this buffer at character index `char_index`.
    ///
    /// # Notes
    /// `char_index` is a *character index*, not a byte index.
    ///
    /// # Return
    /// Returns how many *characters* were successfully inserted
    fn insert_text(&mut self, text: &str, char_index: usize) -> usize;

    /// Deletes a range of text `char_range` from this buffer.
    ///
    /// # Notes
    /// `char_range` is a *character range*, not a byte range.
    fn delete_char_range(&mut self, char_range: Range<usize>);

    /// Reads the given character range.
    fn char_range(&self, char_range: Range<usize>) -> &str {
        slice_char_range(self.as_str(), char_range)
    }

    fn byte_index_from_char_index(&self, char_index: usize) -> usize {
        byte_index_from_char_index(self.as_str(), char_index)
    }

    /// Clears all characters in this buffer
    fn clear(&mut self) {
        self.delete_char_range(0..self.as_str().len());
    }

    /// Replaces all contents of this string with `text`
    fn replace_with(&mut self, text: &str) {
        self.clear();
        self.insert_text(text, 0);
    }

    /// Clears all characters in this buffer and returns a string of the contents.
    fn take(&mut self) -> String {
        let s = self.as_str().to_owned();
        self.clear();
        s
    }

    fn insert_text_at(&mut self, ccursor: &mut CCursor, text_to_insert: &str, char_limit: usize) {
        if char_limit < usize::MAX {
            let mut new_string = text_to_insert;
            // Avoid subtract with overflow panic
            let cutoff = char_limit.saturating_sub(self.as_str().chars().count());

            new_string = match new_string.char_indices().nth(cutoff) {
                None => new_string,
                Some((idx, _)) => &new_string[..idx],
            };

            ccursor.index += self.insert_text(new_string, ccursor.index);
        } else {
            ccursor.index += self.insert_text(text_to_insert, ccursor.index);
        }
    }

    fn decrease_indentation(&mut self, ccursor: &mut CCursor) {
        let line_start = find_line_start(self.as_str(), *ccursor);

        let remove_len = if self.as_str().chars().nth(line_start.index) == Some('\t') {
            Some(1)
        } else if self
            .as_str()
            .chars()
            .skip(line_start.index)
            .take(TAB_SIZE)
            .all(|c| c == ' ')
        {
            Some(TAB_SIZE)
        } else {
            None
        };

        if let Some(len) = remove_len {
            self.delete_char_range(line_start.index..(line_start.index + len));
            if *ccursor != line_start {
                *ccursor -= len;
            }
        }
    }

    fn delete_selected(&mut self, cursor_range: &CursorRange) -> CCursor {
        let [min, max] = cursor_range.sorted_cursors();
        self.delete_selected_ccursor_range([min.ccursor, max.ccursor])
    }

    fn delete_selected_ccursor_range(&mut self, [min, max]: [CCursor; 2]) -> CCursor {
        self.delete_char_range(min.index..max.index);
        CCursor {
            index: min.index,
            prefer_next_row: true,
        }
    }

    fn delete_previous_char(&mut self, ccursor: CCursor) -> CCursor {
        if ccursor.index > 0 {
            let max_ccursor = ccursor;
            let min_ccursor = max_ccursor - 1;
            self.delete_selected_ccursor_range([min_ccursor, max_ccursor])
        } else {
            ccursor
        }
    }

    fn delete_next_char(&mut self, ccursor: CCursor) -> CCursor {
        self.delete_selected_ccursor_range([ccursor, ccursor + 1])
    }

    fn delete_previous_word(&mut self, max_ccursor: CCursor) -> CCursor {
        let min_ccursor = ccursor_previous_word(self.as_str(), max_ccursor);
        self.delete_selected_ccursor_range([min_ccursor, max_ccursor])
    }

    fn delete_next_word(&mut self, min_ccursor: CCursor) -> CCursor {
        let max_ccursor = ccursor_next_word(self.as_str(), min_ccursor);
        self.delete_selected_ccursor_range([min_ccursor, max_ccursor])
    }

    fn delete_paragraph_before_cursor(
        &mut self,
        galley: &Galley,
        cursor_range: &CursorRange,
    ) -> CCursor {
        let [min, max] = cursor_range.sorted_cursors();
        let min = galley.from_pcursor(PCursor {
            paragraph: min.pcursor.paragraph,
            offset: 0,
            prefer_next_row: true,
        });
        if min.ccursor == max.ccursor {
            self.delete_previous_char(min.ccursor)
        } else {
            self.delete_selected(&CursorRange::two(min, max))
        }
    }

    fn delete_paragraph_after_cursor(
        &mut self,
        galley: &Galley,
        cursor_range: &CursorRange,
    ) -> CCursor {
        let [min, max] = cursor_range.sorted_cursors();
        let max = galley.from_pcursor(PCursor {
            paragraph: max.pcursor.paragraph,
            offset: usize::MAX, // end of paragraph
            prefer_next_row: false,
        });
        if min.ccursor == max.ccursor {
            self.delete_next_char(min.ccursor)
        } else {
            self.delete_selected(&CursorRange::two(min, max))
        }
    }
}

impl TextBuffer for String {
    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        self.as_ref()
    }

    fn insert_text(&mut self, text: &str, char_index: usize) -> usize {
        // Get the byte index from the character index
        let byte_idx = byte_index_from_char_index(self.as_str(), char_index);

        // Then insert the string
        self.insert_str(byte_idx, text);

        text.chars().count()
    }

    fn delete_char_range(&mut self, char_range: Range<usize>) {
        assert!(char_range.start <= char_range.end);

        // Get both byte indices
        let byte_start = byte_index_from_char_index(self.as_str(), char_range.start);
        let byte_end = byte_index_from_char_index(self.as_str(), char_range.end);

        // Then drain all characters within this range
        self.drain(byte_start..byte_end);
    }

    fn clear(&mut self) {
        self.clear();
    }

    fn replace_with(&mut self, text: &str) {
        text.clone_into(self);
    }

    fn take(&mut self) -> String {
        std::mem::take(self)
    }
}

impl<'a> TextBuffer for Cow<'a, str> {
    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        self.as_ref()
    }

    fn insert_text(&mut self, text: &str, char_index: usize) -> usize {
        <String as TextBuffer>::insert_text(self.to_mut(), text, char_index)
    }

    fn delete_char_range(&mut self, char_range: Range<usize>) {
        <String as TextBuffer>::delete_char_range(self.to_mut(), char_range);
    }

    fn clear(&mut self) {
        <String as TextBuffer>::clear(self.to_mut());
    }

    fn replace_with(&mut self, text: &str) {
        *self = Cow::Owned(text.to_owned());
    }

    fn take(&mut self) -> String {
        std::mem::take(self).into_owned()
    }
}

/// Immutable view of a `&str`!
impl<'a> TextBuffer for &'a str {
    fn is_mutable(&self) -> bool {
        false
    }

    fn as_str(&self) -> &str {
        self
    }

    fn insert_text(&mut self, _text: &str, _ch_idx: usize) -> usize {
        0
    }

    fn delete_char_range(&mut self, _ch_range: Range<usize>) {}
}


#[derive(Clone, Copy, Hash, PartialEq)]
pub struct String64 {
    inner: [u8;64],
}

impl std::fmt::Debug for String64 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("String64").field("inner", &self.as_str()).finish()
    }
}

impl std::fmt::Display for String64 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = bytes_to_str(&self.inner).expect(&format!("A String64 should always be valid utf8.\nThe String64 that was just attempted to Display was:\n{:x?}", self.inner));
        write!(f, "{}", text)
    }
}

impl Default for String64 {
    fn default() -> Self {
        Self { inner: [0;64] }
    }
}

/// Turns a &str into a String64. If the &str has more than 64 bytes, the last bytes will be cut.
impl From<&str> for String64 {
    fn from(s: &str) -> Self {

        let mut inner = [0u8;64];

        let mut min = std::cmp::min(s.len(), 64);
        inner[0..min].copy_from_slice(&s.as_bytes()[0..min]);

        loop {
            if min == 0 {break}
            match std::str::from_utf8(&inner[0..min]) {
                Ok(_) => break,
                Err(_) => min -= 1,
            }
        }

        String64 {
            inner
        }

    }
}


impl TryFrom<&[u8]> for String64 {
    type Error = Utf8Error;

    fn try_from(s: &[u8]) -> Result<Self, Self::Error> {
        let mut inner = [0u8;64];

        let min = std::cmp::min(s.len(), 64);
        inner[0..min].copy_from_slice(&s[0..min]);

        match std::str::from_utf8(&inner) {
            Ok(_) => {
                Ok(String64 {inner})
            },
            Err(e) => Err(e)
        }
    }
}

impl Eq for String64 {}

impl Ord for String64 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for String64 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.as_str().cmp(other.as_str()))
    }
}

impl TextBuffer for String64 {
    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        self.as_str()
    }

    fn insert_text(&mut self, text: &str, char_index: usize) -> usize {
        // Get the byte index from the character index
        let byte_idx = byte_index_from_char_index(self.as_str(), char_index);

        // Then insert the string64
        let mut temp = self.to_string();
        temp.insert_str(byte_idx, text);
        *self = String64::from(temp.as_str());

        text.chars().count()
    }

    fn delete_char_range(&mut self, char_range: Range<usize>) {
        assert!(char_range.start <= char_range.end);

        // Get both byte indices
        let byte_start = byte_index_from_char_index(self.as_str(), char_range.start);
        let byte_end = byte_index_from_char_index(self.as_str(), char_range.end);

        // Then drain all characters within this range
        let mut temp = self.to_string();
        temp.drain(byte_start..byte_end);
        *self = String64::from(temp.as_str());
    }

    fn clear(&mut self) {
        *self = String64::new();
    }

    fn replace_with(&mut self, text: &str) {
        *self = String64::from(text);
    }
}

impl String64 {

    pub fn new() -> Self {
        String64 {
            inner: [0u8; 64]
        }
    }

    pub fn len(&self) -> usize {
        let mut output = 0;
        for byte in self.inner {
            match byte {
                0 => break,
                _ => output += 1,
            }
        }
        output
    }

    pub fn push(&mut self, s: &str) {

        if self.len() + s.len() > 64 {
            return
        }

        let mut end_index = 0;
        for (index, byte) in self.inner.iter().enumerate() {
            if byte == &0 {
                end_index = index+1;
            }
        }

        for (index, byte) in s.as_bytes().iter().enumerate() {
            self.inner[index+end_index] = *byte;
        }

    }

    pub fn as_str(&self) -> &str {
        // This is safe since an enforced invariant of String64 is that it is utf8
        std::str::from_utf8(&self.inner[0..self.len()]).unwrap()
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.inner[0..self.len()]
    }

    pub fn raw(&self) -> &[u8] {
        &self.inner
    }

    /// These functions may panic and should only be called if you are certain that the String64 contains a valid number
    pub fn to_i32(&self) -> i32 {
        self.as_str().parse::<i32>().unwrap()
    }

    /// These functions may panic and should only be called if you are certain that the String64 contains a valid number
    pub fn to_f32(&self) -> f32 {
        self.as_str().parse::<f32>().unwrap()
    }

    pub fn to_i32_checked(&self) -> Result<i32, std::num::ParseIntError> {
        self.as_str().parse::<i32>()
    }

    pub fn to_f32_checked(&self) -> Result<f32, std::num::ParseFloatError> {
        self.as_str().parse::<f32>()
    }

}


/// Removes the trailing 0 bytes from a str created from a byte buffer
pub fn bytes_to_str(bytes: &[u8]) -> Result<&str, Utf8Error> {
    let mut index: usize = 0;
    let len = bytes.len();
    let mut start: usize = 0;

    while index < len {
        if bytes[index] != 0 {
            break
        }
        index += 1;
        start += 1;
    }

    if bytes.is_empty() {
        return Ok("")
    }

    if start >= bytes.len()-1 {
        return Ok("")
    }

    let mut stop: usize = start;
    while index < len {
        if bytes[index] == 0 {
            break
        }
        index += 1;
        stop += 1;
    }

    std::str::from_utf8(&bytes[start..stop])
}
