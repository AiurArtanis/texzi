use std::{collections::HashMap, ops::Range};

use super::{RegexError, TemplateIssue};

/// Unchanged captured text can be a slice of the frozen Rope, even in a lookahead.
#[derive(Debug, PartialEq, Eq)]
pub enum Piece<'a> {
    Text(&'a str),
    Source(Range<usize>),
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(super) enum Group<'a> {
    Number(u8),
    Name(&'a str),
}

enum Token<'a> {
    Text(&'a str),
    Capture(usize),
}

pub(super) struct Parsed<'a> {
    tokens: Vec<Token<'a>>,
    // Each referenced group is sent only once per match, even for "$1$1...".
    pub groups: Vec<(Group<'a>, usize)>,
}

impl<'a> Parsed<'a> {
    pub fn new(text: &'a str) -> Result<Self, RegexError> {
        if text.len() > super::MAX_EXPRESSION_BYTES {
            return Err(RegexError::Resource);
        }
        let mut tokens = Vec::new();
        let mut groups = Vec::new();
        let mut slots = HashMap::new();
        let bytes = text.as_bytes();
        let (mut literal, mut cursor) = (0, 0);
        while let Some(delta) = text[cursor..].find('$') {
            let start = cursor + delta;
            let rest = &bytes[start + 1..];
            let (token, end) = match rest {
                [b'$', ..] => (None, start + 2),
                [b'&' | b'0', ..] => (Some(Group::Number(0)), start + 2),
                [d @ b'1'..=b'9', tail @ ..] => {
                    let mut number = d - b'0';
                    let mut end = start + 2;
                    if let Some(e @ b'0'..=b'9') = tail.first() {
                        number = number * 10 + e - b'0';
                        end += 1;
                    }
                    (Some(Group::Number(number)), end)
                }
                [b'{', ..] => {
                    let name = &text[start + 2..];
                    let close =
                        name.find('}')
                            .filter(|end| *end != 0)
                            .ok_or(RegexError::Template {
                                byte: start,
                                issue: TemplateIssue::InvalidName,
                            })?;
                    (Some(Group::Name(&name[..close])), start + 3 + close)
                }
                _ => {
                    cursor = start + 1;
                    continue;
                }
            };
            if literal < start {
                tokens.push(Token::Text(&text[literal..start]));
            }
            if let Some(group) = token {
                let slot = *slots.entry(group).or_insert_with(|| {
                    groups.push((group, start));
                    groups.len() - 1
                });
                tokens.push(Token::Capture(slot));
            } else {
                tokens.push(Token::Text("$"));
            }
            cursor = end;
            literal = end;
        }
        if literal < text.len() {
            tokens.push(Token::Text(&text[literal..]));
        }
        if super::wire::record_bytes(groups.len())? > super::wire::BATCH_BYTES {
            return Err(RegexError::Resource);
        }
        Ok(Self { tokens, groups })
    }
}

/// Valid only during its visitor call. Pieces refer to the original template or subject.
pub struct Replacement<'a> {
    pub(super) template: &'a Parsed<'a>,
    pub(super) captures: &'a [Option<Range<usize>>],
}

impl<'a> Replacement<'a> {
    pub fn pieces(&self) -> impl Iterator<Item = Piece<'a>> + '_ {
        self.template.tokens.iter().filter_map(|token| match token {
            Token::Text(text) => Some(Piece::Text(text)),
            Token::Capture(slot) => self.captures[*slot].clone().map(Piece::Source),
        })
    }
}
