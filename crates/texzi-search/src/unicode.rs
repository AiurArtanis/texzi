use regex_syntax::hir::{ClassUnicode, ClassUnicodeRange};
use std::collections::HashMap;

/// Only query characters and their simple-fold equivalents are needed during
/// a scan. Simple folding represents exactly one scalar, even when equivalent
/// scalars have different source UTF-8 lengths (e.g. K, k and Kelvin sign).
#[derive(Debug, Default)]
pub(crate) struct QueryFold {
    known: HashMap<char, char>,
}

impl QueryFold {
    pub(crate) fn add(&mut self, ch: char) -> char {
        if let Some(&key) = self.known.get(&ch) {
            return key;
        }
        let mut class = ClassUnicode::new([ClassUnicodeRange::new(ch, ch)]);
        class.case_fold_simple();
        let key = class.ranges()[0].start().to_ascii_lowercase();
        for range in class.iter() {
            for member in range.start()..=range.end() {
                self.known.insert(member, key);
            }
        }
        key
    }

    #[inline]
    pub(crate) fn key(&self, ch: char) -> char {
        if ch.is_ascii() {
            ch.to_ascii_lowercase()
        } else {
            self.known.get(&ch).copied().unwrap_or(ch)
        }
    }
}
