use regex_syntax::hir::{Class, ClassUnicode, ClassUnicodeRange, HirKind};
use std::{collections::HashMap, sync::LazyLock};

// regex-syntax 0.8.11 and the bundled PCRE2 10.46 both use Unicode 16.0.0.
static WORD: LazyLock<ClassUnicode> = LazyLock::new(|| {
    let union = regex_syntax::parse(r"[\p{L}\p{N}\p{M}\p{Pc}]").expect("valid Unicode categories");
    match union.into_kind() {
        HirKind::Class(Class::Unicode(class)) => class,
        _ => unreachable!("the word categories contain multiple Unicode ranges"),
    }
});

/// Product whole-word searches use exactly General_Category L, N, M or Pc.
/// A regex's own `\b` remains a PCRE2 assertion.
pub fn is_word(ch: char) -> bool {
    if ch.is_ascii() {
        return ch.is_ascii_alphanumeric() || ch == '_';
    }
    let ranges = WORD.ranges();
    let i = ranges.partition_point(|r| r.end() < ch);
    ranges.get(i).is_some_and(|r| r.start() <= ch)
}

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
