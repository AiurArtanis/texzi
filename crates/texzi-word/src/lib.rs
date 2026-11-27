//! Word runs used by source-editor movement, selection and whole-word search.
use regex_syntax::hir::{Class as RegexClass, ClassUnicode, HirKind};
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Space,
    Word,
    Ideograph,
    Punct,
}

impl Class {
    pub fn is_word(self) -> bool {
        matches!(self, Self::Word | Self::Ideograph)
    }
}

// Pinned to Unicode 16.0.0, the same character data used by the search engines.
static WORD: LazyLock<ClassUnicode> = LazyLock::new(|| {
    let union = regex_syntax::parse(r"[\p{L}\p{N}\p{M}\p{Pc}]").expect("valid Unicode categories");
    match union.into_kind() {
        HirKind::Class(RegexClass::Unicode(class)) => class,
        _ => unreachable!("the word categories contain multiple Unicode ranges"),
    }
});

pub fn is_word(ch: char) -> bool {
    if ch.is_ascii() {
        return ch.is_ascii_alphanumeric() || ch == '_';
    }
    let ranges = WORD.ranges();
    let i = ranges.partition_point(|r| r.end() < ch);
    ranges.get(i).is_some_and(|r| r.start() <= ch)
}

/// Classify a grapheme by its first scalar. Kana, Han and Hangul share the
/// editor's dictionary-free run. Language-specific constituents are additive.
pub fn class_of(ch: char, extra: &str) -> Class {
    if ch.is_whitespace() {
        Class::Space
    } else if matches!(ch as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF
        | 0xF900..=0xFAFF | 0x20000..=0x3134F)
    {
        Class::Ideograph
    } else if is_word(ch) || extra.contains(ch) {
        Class::Word
    } else {
        Class::Punct
    }
}
