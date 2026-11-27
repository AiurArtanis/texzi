//! The built-in grammar with bounded work and sparse line-start checkpoints.

use std::{collections::BTreeMap, ops::Range, time::Duration};

use nsh_core::time::Instant;
use syntect::parsing::{SyntaxReference, SyntaxSet};

use super::syntect_lines::parse;
use super::{EditorHighlighter, EditorText, EditorTextEdit, EditorTokens, SLICE};
use crate::syntax::{Parse, find_syntax, syntaxes};

const CHECKPOINT_STRIDE: usize = 2_048;
const MAX_LINE_BYTES: usize = 16 * 1024;
const MAX_READ_BYTES: usize = 128 * 1024;
const MAX_LINES_PER_PASS: usize = 512;
const MAX_SCOPE_DEPTH: usize = 128;

#[derive(Clone)]
struct Checkpoint {
    offset: usize,
    state: Parse,
}

struct Cursor {
    line: usize,
    reader: LineReader,
    state: Parse,
}

impl Cursor {
    fn new(line: usize, checkpoint: Checkpoint) -> Self {
        Self {
            line,
            reader: LineReader::new(checkpoint.offset),
            state: checkpoint.state,
        }
    }

    fn from_start(line: usize, offset: usize, syntax: &'static SyntaxReference) -> Self {
        Self::new(
            line,
            Checkpoint {
                offset,
                state: Parse::new(syntax),
            },
        )
    }

    fn advance(
        &mut self,
        text: &EditorText,
        set: &SyntaxSet,
        budget: &mut usize,
        tokens: Option<&mut EditorTokens>,
    ) -> Option<bool> {
        let newline = match self.reader.read(text, budget)? {
            Line::Complete(newline) => newline,
            Line::Oversized => return Some(false),
        };
        let line = &mut self.reader.text;
        let content = line.len() - usize::from(newline);
        if !newline {
            line.push('\n');
        }
        let found = parse(&mut self.state, line, content, set, tokens.is_some());
        let exact = self.state.scopes.as_slice().len() <= MAX_SCOPE_DEPTH;
        if exact && let Some(tokens) = tokens {
            tokens.push(self.line, found);
        }
        self.reader.text.clear();
        self.line += 1;
        Some(exact)
    }
}

enum Line {
    Complete(bool),
    Oversized,
}

/// A logical LF-delimited line may span many Rope chunks. Stop reading
/// once it exceeds the grammar budget, even without finding its newline.
struct LineReader {
    offset: usize,
    text: String,
}

impl LineReader {
    fn new(offset: usize) -> Self {
        Self {
            offset,
            text: String::new(),
        }
    }

    fn read(&mut self, text: &EditorText, budget: &mut usize) -> Option<Line> {
        if self.offset >= text.len() {
            return Some(Line::Complete(false));
        }
        let (chunks, start, _, _) = text.rope().chunks_at_byte(self.offset);
        let mut skip = self.offset - start;
        for chunk in chunks {
            let chunk = &chunk[skip..];
            skip = 0;
            let newline = chunk.find('\n');
            let bytes = newline.map_or(chunk.len(), |at| at + 1);
            if self.text.len() + bytes > MAX_LINE_BYTES {
                return Some(Line::Oversized);
            }
            self.offset += bytes;
            *budget = budget.saturating_sub(bytes);
            self.text.push_str(&chunk[..bytes]);
            if newline.is_some() {
                return Some(Line::Complete(true));
            }
            if *budget == 0 {
                return None;
            }
        }
        Some(Line::Complete(false))
    }
}

pub(super) struct WindowedSyntect {
    hint: &'static str,
    syntax: Option<Option<&'static SyntaxReference>>,
    checkpoints: BTreeMap<usize, Checkpoint>,
    cursor: Option<Cursor>,
    window: Vec<Range<usize>>,
    next_range: usize,
    range_started: bool,
    unknown: Option<usize>,
}

impl WindowedSyntect {
    pub(super) fn new(hint: &'static str) -> Option<Self> {
        if crate::syntax::plain(hint.trim()) {
            return None;
        }
        Some(Self {
            hint,
            syntax: None,
            checkpoints: BTreeMap::new(),
            cursor: None,
            window: Vec::new(),
            next_range: 0,
            range_started: false,
            unknown: None,
        })
    }

    fn syntax(&mut self) -> Option<&'static SyntaxReference> {
        *self.syntax.get_or_insert_with(|| find_syntax(self.hint))
    }

    fn seek(&mut self, line: usize, syntax: &'static SyntaxReference) {
        let checkpoint = self.checkpoints.range(..=line).next_back();
        let lower = checkpoint.map_or(0, |(&line, _)| line);
        if self
            .cursor
            .as_ref()
            .is_some_and(|cursor| lower <= cursor.line && cursor.line <= line)
        {
            return;
        }
        self.cursor = Some(match checkpoint {
            Some((&line, checkpoint)) => Cursor::new(line, checkpoint.clone()),
            None => Cursor::from_start(0, 0, syntax),
        });
    }
}

impl EditorHighlighter for WindowedSyntect {
    fn edited(&mut self, edit: &EditorTextEdit) {
        // A checkpoint at the start of the first changed line is still valid.
        self.checkpoints.split_off(&(edit.first_line + 1));
        if self.unknown.is_some_and(|line| edit.first_line <= line) {
            self.unknown = None;
        }
        if self
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.line >= edit.first_line)
        {
            self.cursor = None;
        }
        self.next_range = 0;
        self.range_started = false;
    }

    fn set_window(&mut self, window: &[Range<usize>]) {
        if self.window != window {
            self.window = window.to_vec();
            self.checkpoints.retain(|line, _| {
                line.is_multiple_of(CHECKPOINT_STRIDE)
                    || self.window.iter().any(|range| range.start == *line)
            });
            self.next_range = 0;
            self.range_started = false;
        }
    }

    fn unknown_from(&self) -> Option<usize> {
        self.unknown
    }

    fn highlight(
        &mut self,
        text: &EditorText,
        _: usize,
        budget: Duration,
        tokens: &mut EditorTokens,
    ) -> bool {
        let started = Instant::now();
        let Some(syntax) = self.syntax() else {
            return true;
        };
        let set = syntaxes();
        let mut remaining = MAX_READ_BYTES;
        let mut lines = 0;
        while let Some(range) = self.window.get(self.next_range).cloned() {
            let end = range.end.min(self.unknown.unwrap_or(usize::MAX));
            if range.start >= end {
                self.next_range += 1;
                self.range_started = false;
                continue;
            }
            if !self.range_started {
                self.seek(range.start, syntax);
                self.range_started = true;
            }
            let cursor = self.cursor.as_mut().expect("requested region has a cursor");
            if cursor.line >= end {
                self.next_range += 1;
                self.range_started = false;
                continue;
            }
            if (cursor.line.is_multiple_of(CHECKPOINT_STRIDE) || cursor.line == range.start)
                && cursor.reader.text.is_empty()
            {
                self.checkpoints
                    .entry(cursor.line)
                    .or_insert_with(|| Checkpoint {
                        offset: cursor.reader.offset,
                        state: cursor.state.clone(),
                    });
            }
            let line = cursor.line;
            let capture = range.contains(&line);
            match cursor.advance(text, set, &mut remaining, capture.then_some(&mut *tokens)) {
                Some(true) => {}
                Some(false) => {
                    self.unknown = Some(line);
                    self.cursor = None;
                    self.next_range += 1;
                    self.range_started = false;
                }
                None => return false,
            }
            lines += 1;
            if remaining == 0 || lines >= MAX_LINES_PER_PASS || started.elapsed() >= budget {
                break;
            }
        }
        self.next_range >= self.window.len()
    }

    fn frontier(&self) -> usize {
        self.cursor.as_ref().map_or(0, |cursor| cursor.line)
    }

    fn guess(
        &mut self,
        text: &EditorText,
        first_line: usize,
        offset: usize,
        last_line: usize,
        tokens: &mut EditorTokens,
    ) -> bool {
        let started = Instant::now();
        let Some(syntax) = self.syntax() else {
            return false;
        };
        let set = syntaxes();
        let mut cursor = Cursor::from_start(first_line, offset, syntax);
        let mut remaining = MAX_READ_BYTES;
        while cursor.line <= last_line && cursor.line - first_line < MAX_LINES_PER_PASS {
            if cursor.advance(text, set, &mut remaining, Some(tokens)) != Some(true) {
                break;
            }
            if remaining == 0 || started.elapsed() >= SLICE {
                break;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use ropey::Rope;

    use super::*;
    use crate::syntax::Token;

    fn finish(parser: &mut WindowedSyntect, source: &str) -> EditorTokens {
        let rope = EditorText::new(Rope::from_str(source));
        let mut output = EditorTokens::default();
        while !parser.highlight(&rope, usize::MAX, Duration::MAX, &mut output) {}
        output
    }

    fn contains_token(output: &EditorTokens, line: usize, token: Token) -> bool {
        output
            .lines
            .iter()
            .any(|(at, tokens)| *at == line && tokens.iter().any(|(_, kind)| *kind == token))
    }

    #[test]
    fn long_comment_and_raw_string_do_not_create_false_exact_context() {
        for (opening, close, inside) in [
            ("/*", "*/", Token::Comment),
            ("let raw = r###\"", "\"###;", Token::String),
        ] {
            let mut parser = WindowedSyntect::new("rs").unwrap();
            parser.set_window(&[0..6]);
            let long = "x".repeat(MAX_READ_BYTES * 2);
            let unknown =
                format!("{opening}\n{long}\nlet in_scope = 7;\n{close}\nlet outside = 9;\n");
            let truncated = finish(&mut parser, &unknown);
            assert_eq!(parser.unknown_from(), Some(1));
            assert!(truncated.lines.iter().all(|(line, _)| *line == 0));
            assert!(parser.checkpoints.keys().all(|line| *line <= 1));

            parser.edited(&EditorTextEdit::lines(1, 1, 1));
            let known = format!("{opening}\nshort\nlet in_scope = 7;\n{close}\nlet outside = 9;\n");
            let recolored = finish(&mut parser, &known);
            assert_eq!(parser.unknown_from(), None);
            assert!(contains_token(&recolored, 2, inside));
            assert!(!contains_token(&recolored, 2, Token::Keyword));
            assert!(contains_token(&recolored, 4, Token::Keyword));
            assert!(contains_token(&recolored, 4, Token::Number));
        }
    }

    #[test]
    fn a_far_exact_request_retains_only_requested_tokens_and_sparse_context() {
        let mut parser = WindowedSyntect::new("rs").unwrap();
        let mut source = String::from("/*\n");
        source.push_str(&"let name = 5;\n".repeat(4_100));
        source.push_str("*/\nprintln!(\"out\");\n");
        parser.set_window(&[0..2, 4_100..4_104]);
        let output = finish(&mut parser, &source);
        assert_eq!(output.len(), 6);
        assert!(contains_token(&output, 4_100, Token::Comment));
        assert!(contains_token(&output, 4_102, Token::String));
        assert!(contains_token(&output, 4_102, Token::Macro));
        assert!(parser.checkpoints.len() <= 4);

        parser.set_window(&[200..202]);
        let replay = finish(&mut parser, &source);
        assert_eq!(replay.len(), 2);
        assert!(contains_token(&replay, 200, Token::Comment));
    }
}
