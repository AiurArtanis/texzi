//! Count Han, kana, Hangul and Bopomofo in writing units, and other letters
//! and numbers in runs. Combining marks attach to the preceding unit. Within
//! a run, one apostrophe (' ’ ʼ) or a sequence of connector punctuation (e.g.
//! underscores) joins two alphanumeric parts. Dashes and decimal points split
//! runs. Punctuation, whitespace and emoji add no units.
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};
use unicode_script::{Script, UnicodeScript};

const POLL_BYTES: usize = 4096;

#[derive(Clone, Copy, Default)]
enum Tail {
    #[default]
    Gap,
    Word,
    Apostrophe,
    Connectors,
    Hangul(Hangul),
}

#[derive(Clone, Copy)]
enum Hangul {
    Leading,
    Vowel,
    Trailing,
}

// In the pinned Unicode 18 data the only letters/numbers with Emoji=Yes
// are ASCII digits and ℹ. Recognize their emoji presentation across pushes.
#[derive(Clone, Copy)]
struct EmojiBase {
    ch: char,
    started_word: bool,
}

#[derive(Default)]
pub(super) struct Counter {
    units: u64,
    tail: Tail,
    emoji_base: Option<EmojiBase>,
}

impl Counter {
    /// A false return leaves only the prefix processed before cancellation.
    pub(super) fn push(&mut self, mut text: &str, mut cancelled: impl FnMut() -> bool) -> bool {
        loop {
            if cancelled() {
                return false;
            }
            let limit = text.floor_char_boundary(text.len().min(POLL_BYTES));
            let (piece, rest) = text.split_at(limit);
            for ch in piece.chars() {
                self.accept(ch);
            }
            if rest.is_empty() {
                return true;
            }
            text = rest;
        }
    }

    /// End a paragraph or another structural unit. Inline styles and Rope
    /// leaves pass straight through so they cannot introduce a word break.
    pub(super) fn boundary(&mut self) {
        self.tail = Tail::Gap;
        self.emoji_base = None;
    }

    pub(super) fn total(&self) -> u64 {
        self.units
    }

    fn accept(&mut self, ch: char) {
        if let Some(base) = self.emoji_base.take() {
            match (base.ch, ch) {
                ('0'..='9', '\u{20e3}') | ('ℹ', '\u{fe0f}') => {
                    if base.started_word {
                        self.units -= 1;
                    }
                    self.tail = Tail::Gap;
                    return;
                }
                ('0'..='9', '\u{fe0f}') => {
                    self.emoji_base = Some(base);
                    return;
                }
                _ => {}
            }
        }
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' => self.word(ch),
            '\'' | '’' | 'ʼ' => {
                self.tail = if matches!(self.tail, Tail::Word) {
                    Tail::Apostrophe
                } else {
                    Tail::Gap
                };
            }
            '_' => self.connector(),
            ch if ch.is_ascii() => self.tail = Tail::Gap,
            '\u{4e00}'..='\u{9fff}' => self.single(),
            // Halfwidth voiced/semi-voiced kana marks belong to the preceding kana.
            '\u{ff9e}' | '\u{ff9f}' => self.mark(),
            _ => {
                use GeneralCategory::*;
                match ch.general_category() {
                    UppercaseLetter | LowercaseLetter | TitlecaseLetter | ModifierLetter
                    | OtherLetter | DecimalNumber | LetterNumber | OtherNumber => match ch.script()
                    {
                        Script::Hangul => self.hangul(ch),
                        Script::Han | Script::Hiragana | Script::Katakana | Script::Bopomofo => {
                            self.single()
                        }
                        Script::Common if shared_kana_letter(ch) => self.single(),
                        _ => self.word(ch),
                    },
                    NonspacingMark | SpacingMark | EnclosingMark => self.mark(),
                    ConnectorPunctuation => self.connector(),
                    _ => self.tail = Tail::Gap,
                }
            }
        }
    }

    fn word(&mut self, ch: char) {
        let started_word = !matches!(self.tail, Tail::Word | Tail::Apostrophe | Tail::Connectors);
        self.units += u64::from(started_word);
        self.tail = Tail::Word;
        if ch.is_ascii_digit() || ch == 'ℹ' {
            self.emoji_base = Some(EmojiBase { ch, started_word });
        }
    }

    fn single(&mut self) {
        self.units += 1;
        self.tail = Tail::Gap;
    }

    fn connector(&mut self) {
        self.tail = if matches!(self.tail, Tail::Word | Tail::Connectors) {
            Tail::Connectors
        } else {
            Tail::Gap
        };
    }

    fn mark(&mut self) {
        // An intervening mark terminates the adjacency required by Hangul GB6–GB8.
        if matches!(self.tail, Tail::Hangul(_)) {
            self.tail = Tail::Gap;
        }
    }

    fn hangul(&mut self, ch: char) {
        use Hangul::*;
        let last = self.tail;
        // Unicode extended grapheme rules GB6–GB8. A decomposed syllable and
        // its precomposed spelling count alike, even on different Rope leaves.
        let (next, joins) = match ch as u32 {
            0x1100..=0x115f | 0xa960..=0xa97c => (Leading, matches!(last, Tail::Hangul(Leading))),
            0x1160..=0x11a7 | 0xd7b0..=0xd7c6 => {
                (Vowel, matches!(last, Tail::Hangul(Leading | Vowel)))
            }
            0x11a8..=0x11ff | 0xd7cb..=0xd7fb => {
                (Trailing, matches!(last, Tail::Hangul(Vowel | Trailing)))
            }
            code @ 0xac00..=0xd7a3 => {
                let ending = if (code - 0xac00) % 28 == 0 {
                    Vowel
                } else {
                    Trailing
                };
                (ending, matches!(last, Tail::Hangul(Leading)))
            }
            _ => return self.single(),
        };
        self.units += u64::from(!joins);
        self.tail = Tail::Hangul(next);
    }
}

fn shared_kana_letter(ch: char) -> bool {
    let scripts = ch.script_extension();
    !scripts.is_common()
        && !scripts.is_inherited()
        && scripts.contains_script(Script::Hiragana)
        && scripts.contains_script(Script::Katakana)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(parts: &[&str]) -> u64 {
        let mut c = Counter::default();
        for part in parts {
            assert!(c.push(part, || false));
        }
        c.total()
    }

    #[test]
    fn committed_cjk_units_words_and_internal_punctuation() {
        let cases = [
            ("", 0),
            ("山河", 2),
            ("𠮷野かなカナー𛀁", 8),
            ("ｶﾞ ㄅㄆ", 3),
            ("한글 한글", 4),
            ("猫alpha42犬", 3),
            ("cafe\u{301} élan", 2),
            ("_x__y 甲_c3 x‿y", 4),
            ("don't l’été ʼx ‘ab’ x''y", 6),
            ("12.34 foo-bar", 4),
            ("，。!? — ___ '' \u{301}", 0),
            ("👩🏽‍💻 🇨🇳 #️⃣ *️⃣ 1️⃣ ℹ️", 0),
            ("a1️⃣b 2 31️⃣x ℹ", 6),
        ];
        for (text, expected) in cases {
            assert_eq!(count(&[text]), expected, "{text:?}");
            for (cut, _) in text.char_indices() {
                assert_eq!(
                    count(&[&text[..cut], &text[cut..]]),
                    expected,
                    "{text:?} at {cut}"
                );
            }
        }
    }

    #[test]
    fn inline_styles_join_and_structural_boundaries_separate_words() {
        let mut c = Counter::default();
        assert!(c.push("pre", || false));
        assert!(c.push("fix", || false));
        c.boundary();
        assert!(c.push("word", || false));
        assert_eq!(c.total(), 2);
        assert_eq!(
            count(&["a", "’", "b", "\u{301}", "一", "ᄒ", "ᅡ", "ᆫ", ""]),
            3
        );
    }

    #[test]
    fn one_long_run_can_be_cancelled_within_one_utf8_block() {
        let text = "é".repeat(POLL_BYTES * 3);
        let mut c = Counter::default();
        let mut polls = 0;
        assert!(!c.push(&text, || {
            polls += 1;
            polls == 2
        }));
        assert_eq!(polls, 2);
        assert_eq!(c.total(), 1);
        let mut han = Counter::default();
        assert!(!han.push(&"山".repeat(POLL_BYTES), {
            let mut second = false;
            move || std::mem::replace(&mut second, true)
        }));
        assert_eq!(han.total(), (POLL_BYTES / '山'.len_utf8()) as u64);
    }
}
