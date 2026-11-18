//! Limited Rust/JSON lexical decorations, not a full language parser.
//! No transactions, marks, document writes or grammar resources are produced.
use nsh_base::rich_editor::{
    model::Node,
    state::{Decoration, DecorationSet, EditorState, Plugin, PluginKey, PluginSpec},
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const PLUGIN_KEY: &str = "texzi.code-highlight.rust-json.v1";
// PERFORMANCE_PLAN §2's normal Markdown envelope has blocks <64 KiB and
// long-line protection starts at 20,000 Unicode scalars. The 2 ms slice is a
// conservative engineering share of its 20 ms scroll/33 ms input targets,
// not a measured performance claim. Other caps bound UI allocations/cache.
const MAX_BYTES: usize = 64 * 1024;
const MAX_LINE_SCALARS: usize = 20_000;
const MAX_NODES: usize = 50_000;
const MAX_DEPTH: usize = 64;
const MAX_SPANS: usize = 4096;
const CACHE_BLOCKS: usize = 16;
const SLICE: Duration = Duration::from_millis(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Language {
    Rust,
    Json,
}
impl Language {
    fn of(node: &Node) -> Option<Self> {
        if node.ty().name() != "codeBlock" {
            return None;
        }
        let language = node.attrs().get("language")?.as_str()?;
        if language.eq_ignore_ascii_case("rust") || language.eq_ignore_ascii_case("rs") {
            Some(Self::Rust)
        } else if language.eq_ignore_ascii_case("json") {
            Some(Self::Json)
        } else {
            None
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Json => "json",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Keyword,
    String,
    Number,
    Comment,
    Key,
    Literal,
}
impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Keyword => "keyword",
            Self::String => "string",
            Self::Number => "number",
            Self::Comment => "comment",
            Self::Key => "key",
            Self::Literal => "literal",
        }
    }
    // NSH tailwind::parse supports these stock palette utilities and dark:
    // variants. RichEditor consumes only the decoration's `class` attribute.
    fn class(self) -> &'static str {
        match self {
            Self::Keyword => "text-violet-700 dark:text-violet-300",
            Self::String => "text-emerald-700 dark:text-emerald-300",
            Self::Number | Self::Literal => "text-amber-800 dark:text-amber-300",
            Self::Comment => "text-slate-600 dark:text-slate-400",
            Self::Key => "text-sky-700 dark:text-sky-300",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Span {
    bytes: Range<usize>,
    units: Range<usize>,
    kind: Kind,
}
struct Cursor<'a> {
    text: &'a str,
    at: usize,
    units: usize,
    line: usize,
    checked: usize,
    deadline: Option<Instant>,
    exhausted: bool,
}
impl<'a> Cursor<'a> {
    fn new(text: &'a str, deadline: Option<Instant>) -> Self {
        Self {
            text,
            at: 0,
            units: 0,
            line: 0,
            checked: 0,
            deadline,
            exhausted: false,
        }
    }
    fn tail(&self) -> &'a str {
        &self.text[self.at..]
    }
    fn peek(&self) -> Option<char> {
        self.tail().chars().next()
    }
    fn advance(&mut self) {
        if let Some(ch) = self.peek() {
            self.at += ch.len_utf8();
            self.units += ch.len_utf16();
            self.line = if ch == '\n' { 0 } else { self.line + 1 };
            self.checked += 1;
            if self.line >= MAX_LINE_SCALARS
                || self.checked >= 256 && self.deadline.is_some_and(|d| Instant::now() >= d)
            {
                self.exhausted = true;
            }
            if self.checked >= 256 {
                self.checked = 0;
            }
        }
    }
    fn ascii(&mut self, n: usize) {
        for _ in 0..n {
            self.advance();
        }
    }
    fn quoted(&mut self) {
        self.advance();
        let mut escaped = false;
        while self.at < self.text.len() && !self.exhausted {
            let ch = self.peek().unwrap();
            self.advance();
            if ch == '"' && !escaped {
                break;
            }
            escaped = ch == '\\' && !escaped;
        }
    }
}
fn identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_alphabetic()
}
fn identifier_continue(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric() || !ch.is_ascii() && !ch.is_whitespace()
}
fn raw_prefix(text: &str) -> Option<(usize, usize)> {
    let mut at = if text.starts_with("br") || text.starts_with("cr") {
        2
    } else if text.starts_with('r') {
        1
    } else {
        return None;
    };
    let begin = at;
    while text.as_bytes().get(at) == Some(&b'#') {
        at += 1;
        if at - begin > 255 {
            return None;
        }
    }
    (text.as_bytes().get(at) == Some(&b'"')).then_some((at + 1, at - begin))
}
fn char_end(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    if chars.next()?.1 != '\'' {
        return None;
    }
    let (_, first) = chars.next()?;
    let end = if first == '\\' {
        let (at, escape) = chars.next()?;
        if escape == 'u' {
            let rest = &text[at + 1..];
            let body = rest.strip_prefix('{')?;
            let count = body.bytes().take_while(u8::is_ascii_hexdigit).count();
            if !(1..=6).contains(&count) || body.as_bytes().get(count) != Some(&b'}') {
                return None;
            }
            at + 1 + 1 + count + 1
        } else if escape == 'x' {
            let a = text.as_bytes().get(at + 1)?;
            let b = text.as_bytes().get(at + 2)?;
            if !a.is_ascii_hexdigit() || !b.is_ascii_hexdigit() {
                return None;
            }
            at + 3
        } else {
            at + escape.len_utf8()
        }
    } else {
        if matches!(first, '\n' | '\r' | '\'') {
            return None;
        }
        1 + first.len_utf8()
    };
    (text.as_bytes().get(end) == Some(&b'\'')).then_some(end + 1)
}
fn number(c: &mut Cursor<'_>, language: Language) {
    if c.peek() == Some('-') {
        c.advance();
    }
    let hex =
        language == Language::Rust && (c.tail().starts_with("0x") || c.tail().starts_with("0X"));
    while let Some(ch) = c.peek() {
        if c.exhausted {
            break;
        }
        let ascii = c.tail().as_bytes();
        let take = ch.is_ascii_digit()
            || language == Language::Rust && (ch.is_ascii_alphabetic() || ch == '_')
            || ch == '.' && ascii.get(1).is_some_and(u8::is_ascii_digit)
            || !hex && matches!(ch, 'e' | 'E')
            || !hex
                && matches!(ch, '+' | '-')
                && c.at > 0
                && matches!(c.text.as_bytes()[c.at - 1], b'e' | b'E')
                && ascii.get(1).is_some_and(u8::is_ascii_digit);
        if !take {
            break;
        }
        c.advance();
    }
}
fn tokenize(text: &str, language: Language, deadline: Option<Instant>) -> Option<Vec<Span>> {
    if text.len() >= MAX_BYTES {
        return None;
    }
    let mut c = Cursor::new(text, deadline);
    let mut spans = Vec::new();
    while c.at < text.len() && !c.exhausted {
        let (start, units) = (c.at, c.units);
        let tail = c.tail();
        let ch = c.peek().unwrap();
        let kind = if language == Language::Rust && tail.starts_with("//") {
            while c.at < text.len() && c.peek() != Some('\n') && !c.exhausted {
                c.advance();
            }
            Some(Kind::Comment)
        } else if language == Language::Rust && tail.starts_with("/*") {
            c.ascii(2);
            let mut depth = 1;
            while c.at < text.len() && depth > 0 && !c.exhausted {
                if c.tail().starts_with("/*") {
                    depth += 1;
                    c.ascii(2);
                } else if c.tail().starts_with("*/") {
                    depth -= 1;
                    c.ascii(2);
                } else {
                    c.advance();
                }
            }
            Some(Kind::Comment)
        } else if language == Language::Rust && raw_prefix(tail).is_some() {
            let (prefix, hashes) = raw_prefix(tail).unwrap();
            c.ascii(prefix);
            let end = format!("\"{}", "#".repeat(hashes));
            while c.at < text.len() && !c.exhausted {
                if c.tail().starts_with(&end) {
                    c.ascii(end.len());
                    break;
                }
                c.advance();
            }
            Some(Kind::String)
        } else if language == Language::Rust && (tail.starts_with("b\"") || tail.starts_with("c\""))
        {
            c.advance();
            c.quoted();
            Some(Kind::String)
        } else if ch == '"' {
            c.quoted();
            let following = c.tail().trim_start_matches([' ', '\t', '\r', '\n']);
            Some(
                if language == Language::Json && following.starts_with(':') {
                    Kind::Key
                } else {
                    Kind::String
                },
            )
        } else if language == Language::Rust
            && (ch == '\'' && char_end(tail).is_some()
                || tail.starts_with("b'") && char_end(&tail[1..]).is_some())
        {
            let end = if ch == 'b' {
                1 + char_end(&tail[1..]).unwrap()
            } else {
                char_end(tail).unwrap()
            };
            let target = c.at + end;
            while c.at < target {
                c.advance();
            }
            Some(Kind::String)
        } else if ch.is_ascii_digit()
            || language == Language::Json
                && ch == '-'
                && tail.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
        {
            number(&mut c, language);
            Some(Kind::Number)
        } else if language == Language::Rust
            && tail.starts_with("r#")
            && tail[2..].chars().next().is_some_and(identifier_start)
        {
            c.ascii(2);
            while c.peek().is_some_and(identifier_continue) && !c.exhausted {
                c.advance();
            }
            None
        } else if identifier_start(ch) {
            c.advance();
            while c.peek().is_some_and(identifier_continue) && !c.exhausted {
                c.advance();
            }
            let word = &text[start..c.at];
            if matches!(word, "true" | "false") || language == Language::Json && word == "null" {
                Some(Kind::Literal)
            } else if language == Language::Rust
                && matches!(
                    word,
                    "as" | "async"
                        | "await"
                        | "break"
                        | "const"
                        | "continue"
                        | "crate"
                        | "dyn"
                        | "else"
                        | "enum"
                        | "extern"
                        | "fn"
                        | "for"
                        | "if"
                        | "impl"
                        | "in"
                        | "let"
                        | "loop"
                        | "match"
                        | "mod"
                        | "move"
                        | "mut"
                        | "pub"
                        | "ref"
                        | "return"
                        | "self"
                        | "Self"
                        | "static"
                        | "struct"
                        | "super"
                        | "trait"
                        | "type"
                        | "unsafe"
                        | "use"
                        | "where"
                        | "while"
                        | "yield"
                        | "union"
                )
            {
                Some(Kind::Keyword)
            } else {
                None
            }
        } else {
            c.advance();
            None
        };
        if let Some(kind) = kind {
            if spans.len() >= MAX_SPANS {
                return None;
            }
            spans.push(Span {
                bytes: start..c.at,
                units: units..c.units,
                kind,
            });
        }
    }
    if c.exhausted || deadline.is_some_and(|d| Instant::now() >= d) {
        None
    } else {
        Some(spans)
    }
}

fn code_text(node: &Node) -> Option<&str> {
    match node.child_count() {
        0 => Some(""),
        1 => node.child(0).text(),
        _ => None,
    }
}
struct Entry {
    node: Node,
    language: Language,
    spans: Arc<Vec<Span>>,
}
#[derive(Default)]
struct Cache {
    entries: Vec<Entry>,
    last: Option<(Node, Option<usize>, DecorationSet)>,
}
struct Highlight {
    cache: Mutex<Cache>,
}
struct Pass {
    deadline: Instant,
    bytes: usize,
    nodes: usize,
    decorations: Vec<Decoration>,
    active: Option<usize>,
}
impl Cache {
    fn block(&mut self, node: &Node, pos: usize, pass: &mut Pass) {
        let Some(language) = Language::of(node) else {
            return;
        };
        let Some(text) = code_text(node) else { return };
        if text.len() >= MAX_BYTES
            || pass.bytes + text.len() > MAX_BYTES
            || pass.decorations.len() >= MAX_SPANS
            || Instant::now() >= pass.deadline
        {
            return;
        }
        pass.bytes += text.len();
        let spans = if let Some(index) = self.entries.iter().position(|entry| {
            entry.language == language
                && (entry.node.ptr_eq(node) || code_text(&entry.node) == Some(text))
        }) {
            let entry = self.entries.remove(index);
            let spans = entry.spans.clone();
            self.entries.push(entry);
            spans
        } else {
            let Some(spans) = tokenize(text, language, Some(pass.deadline)) else {
                return;
            };
            let spans = Arc::new(spans);
            self.entries.push(Entry {
                node: node.clone(),
                language,
                spans: spans.clone(),
            });
            if self.entries.len() > CACHE_BLOCKS {
                self.entries.remove(0);
            }
            spans
        };
        if pass.decorations.len() + spans.len() > MAX_SPANS {
            return;
        }
        let previous = pass.decorations.len();
        for (index, span) in spans.iter().enumerate() {
            if index % 64 == 0 && Instant::now() >= pass.deadline {
                pass.decorations.truncate(previous);
                return;
            }
            let attrs = BTreeMap::from([("class".into(), json!(span.kind.class()))]);
            let spec = BTreeMap::from([
                ("kind".into(), json!(span.kind.name())),
                ("language".into(), json!(language.name())),
            ]);
            pass.decorations.push(
                Decoration::inline(pos + 1 + span.units.start, pos + 1 + span.units.end, attrs)
                    .with_spec(spec),
            );
        }
    }
    fn walk(&mut self, node: &Node, pos: usize, depth: usize, pass: &mut Pass) {
        if pass.nodes >= MAX_NODES || depth >= MAX_DEPTH || Instant::now() >= pass.deadline {
            return;
        }
        pass.nodes += 1;
        if node.ty().name() == "codeBlock" {
            if pass.active != Some(pos) {
                self.block(node, pos, pass);
            }
            return;
        }
        let mut at = if depth == 0 { pos } else { pos + 1 };
        for i in 0..node.child_count() {
            if pass.nodes >= MAX_NODES || Instant::now() >= pass.deadline {
                break;
            }
            let child = node.child(i);
            self.walk(child, at, depth + 1, pass);
            at += child.node_size();
        }
    }
}
impl PluginSpec for Highlight {
    fn key(&self) -> PluginKey {
        PluginKey::new(PLUGIN_KEY)
    }
    fn decorations(&self, state: &EditorState) -> Option<DecorationSet> {
        let Ok(mut cache) = self.cache.lock() else {
            return Some(DecorationSet::empty());
        };
        let head = state.selection().resolved_head();
        let active = (head.parent().ty().name() == "codeBlock").then(|| head.before(head.depth));
        if let Some((doc, at, set)) = &cache.last {
            if doc.ptr_eq(state.doc()) && *at == active {
                return Some(set.clone());
            }
        }
        let mut pass = Pass {
            deadline: Instant::now() + SLICE,
            bytes: 0,
            nodes: 0,
            decorations: Vec::new(),
            active,
        };
        if let Some(pos) = active {
            cache.block(head.parent(), pos, &mut pass);
        }
        cache.walk(state.doc(), 0, 0, &mut pass);
        let set = DecorationSet::create(state.doc(), pass.decorations);
        // Scheduling delay must not permanently memoize a timed-out plain
        // result for a small unchanged block. A later view query may retry.
        cache.last = if Instant::now() < pass.deadline {
            Some((state.doc().clone(), active, set.clone()))
        } else {
            None
        };
        Some(set)
    }
}

/// Add once to RichOwner's EditorStateConfig plugins. Existing Source refresh
/// and history reconstruction keep that plugin list and trigger fresh queries.
pub fn plugin() -> Plugin {
    Arc::new(Highlight {
        cache: Mutex::new(Cache::default()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{editor_owner::NativeView, rich_owner::ModeGate};
    use nsh_base::editor::{ExternalDocument, ExternalEditKind, ExternalSelections};
    use nsh_core::prelude::*;

    fn valid_boundaries(text: &str, spans: &[Span]) {
        let mut previous = 0;
        for span in spans {
            assert!(span.bytes.start >= previous && span.bytes.start < span.bytes.end);
            assert!(
                text.is_char_boundary(span.bytes.start) && text.is_char_boundary(span.bytes.end)
            );
            assert_eq!(
                span.units.start,
                text[..span.bytes.start].encode_utf16().count()
            );
            assert_eq!(
                span.units.end,
                text[..span.bytes.end].encode_utf16().count()
            );
            previous = span.bytes.end;
        }
    }
    #[test]
    fn json_fixed_escaped_strings_keys_numbers_literals_and_unicode_utf16_ranges() {
        let text = r#"{"键🚀":"a\"b\\c\u0041","n":-1.25e+2,"ok":true,"nil":null}"#;
        let spans = tokenize(text, Language::Json, None).unwrap();
        let actual: Vec<_> = spans
            .iter()
            .map(|span| (span.kind, &text[span.bytes.clone()]))
            .collect();
        assert_eq!(
            actual,
            vec![
                (Kind::Key, r#""键🚀""#),
                (Kind::String, r#""a\"b\\c\u0041""#),
                (Kind::Key, "\"n\""),
                (Kind::Number, "-1.25e+2"),
                (Kind::Key, "\"ok\""),
                (Kind::Literal, "true"),
                (Kind::Key, "\"nil\""),
                (Kind::Literal, "null"),
            ]
        );
        assert_eq!(spans[0].bytes, 1..10);
        assert_eq!(spans[0].units, 1..6);
        valid_boundaries(text, &spans);
    }
    #[test]
    fn rust_nested_comments_quoted_raw_byte_strings_lifetimes_and_unicode_stay_in_their_tokens() {
        let text = r####"pub fn main<'a>() {
    let 中文 = r##"// 甲🚀 \" not-comment"##;
    let b = br#"/* not */"#;
    let c = '\u{1F680}';
    let 字 = '🚀';
    let r#type = 1;
    /* 外 /* 内 */ end */
    // 行🚀
}"####;
        let spans = tokenize(text, Language::Rust, None).unwrap();
        valid_boundaries(text, &spans);
        let comments: Vec<_> = spans
            .iter()
            .filter(|span| span.kind == Kind::Comment)
            .map(|span| &text[span.bytes.clone()])
            .collect();
        assert_eq!(comments, vec!["/* 外 /* 内 */ end */", "// 行🚀"]);
        let strings: Vec<_> = spans
            .iter()
            .filter(|span| span.kind == Kind::String)
            .map(|span| &text[span.bytes.clone()])
            .collect();
        assert_eq!(
            strings,
            vec![
                r####"r##"// 甲🚀 \" not-comment"##"####,
                r###"br#"/* not */"#"###,
                "'\\u{1F680}'",
                "'🚀'"
            ]
        );
        assert!(!spans.iter().any(|span| &text[span.bytes.clone()] == "type"));
        assert_eq!(&text[spans[0].bytes.clone()], "pub");
        assert_eq!(spans[0].kind, Kind::Keyword);
    }
    #[test]
    fn configured_limits_fall_back_to_plain_instead_of_partial_or_stale_spans() {
        assert!(tokenize(&"x".repeat(MAX_BYTES), Language::Rust, None).is_none());
        assert!(
            tokenize(
                &format!("//{}", "中".repeat(MAX_LINE_SCALARS - 2)),
                Language::Rust,
                None
            )
            .is_none()
        );
        assert!(tokenize(&"let ".repeat(MAX_SPANS + 1), Language::Rust, None).is_none());
        assert!(
            tokenize(
                "let x=1;",
                Language::Rust,
                Some(Instant::now() - Duration::from_millis(1))
            )
            .is_none()
        );
        assert!(
            !tokenize("let x=1;", Language::Rust, None)
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn decoration_color_classes_are_understood_and_change_colors_in_both_native_appearances() {
        use nsh_core::tailwind::{self, ClassEnvironment, ClassProps, Context, Theme};
        let theme = Theme::default();
        for kind in [
            Kind::Keyword,
            Kind::String,
            Kind::Number,
            Kind::Comment,
            Kind::Key,
            Kind::Literal,
        ] {
            let list = tailwind::parse(&theme, kind.class());
            assert!(
                list.unknown().is_empty(),
                "{:?}: {:?}",
                kind,
                list.unknown()
            );
            let mut colors = Vec::new();
            for dark in [false, true] {
                let mut style = nsh_core::Style::default();
                let mut props = ClassProps::default();
                let group = |_: &str| 0;
                let cx = Context {
                    env: ClassEnvironment {
                        dark,
                        ..Default::default()
                    },
                    window: Size::new(px(800.0), px(600.0)),
                    rtl: false,
                    states: 0,
                    position: 0,
                    group: &group,
                };
                list.apply(&cx, &theme, &mut style, &mut props, |_| {});
                assert!(style.text.color.is_some());
                colors.push(style.text.color);
            }
            assert_ne!(colors[0], colors[1]);
        }
    }

    fn run(f: impl FnOnce()) {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        f();
    }
    fn audit(source: &NativeView) -> String {
        source.inspect(|store, id| {
            let view = store.view(id).unwrap();
            let snapshot = store.snapshot(view.doc).unwrap();
            format!(
                "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
                view,
                snapshot.revision,
                snapshot.token,
                snapshot.body.to_utf8(),
                snapshot.body.origins(),
                store.history_depth(view.doc),
                store.dirty(view.doc)
            )
        })
    }
    fn query(gate: &ModeGate) -> Vec<(String, String, Range<usize>)> {
        let state = gate.session().state();
        let installed = state
            .plugins()
            .iter()
            .find(|plugin| plugin.key().0.as_ref() == PLUGIN_KEY)
            .expect("actual RichOwner must register the highlight plugin");
        let set = installed
            .decorations(&state)
            .expect("read-only decorations");
        set.iter().map(|decoration|{
            assert!(decoration.from<decoration.to&&decoration.to<=state.doc().content().size());
            assert!(matches!(&decoration.kind,nsh_base::rich_editor::state::DecorationKind::Inline{attrs,..} if attrs.get("class").and_then(serde_json::Value::as_str).is_some_and(|class|!class.is_empty())&&!attrs.contains_key("style")));
            (decoration.spec["kind"].as_str().unwrap().to_owned(),state.doc().text_between(decoration.from,decoration.to,None,None),decoration.from..decoration.to)
        }).collect()
    }
    fn source_edit(source: &NativeView, range: Range<usize>, text: &str) {
        let old = source.snapshot().unwrap();
        let caret = range.start + text.len();
        batch(|| {
            source
                .edit(
                    old.revision,
                    old.view_epoch,
                    vec![(range, text.into())],
                    ExternalSelections {
                        primary: 0,
                        ranges: vec![(caret, caret)],
                    },
                    ExternalEditKind::Other,
                )
                .unwrap();
            source.notify();
        });
    }
    #[test]
    fn actual_owner_source_edit_undo_and_shifted_blocks_update_decorations_without_extra_body_history_or_dirty_changes()
     {
        run(|| {
            let raw =
                "```json\r\n{\"键🚀\":true,\"s\":\"x\\\"y\"}\n```\r\n\r\n<custom>stay</custom>\n";
            let source = NativeView::fixture(raw);
            let rich = source.sibling().unwrap();
            let gate = ModeGate::new(source.clone(), rich.clone());
            gate.to_rich();
            assert!(gate.rich_mode.get(), "{}", gate.status.get());
            let original = (audit(&source), audit(&rich));
            let original_doc = gate.session().state().doc().clone();
            let initial = query(&gate);
            assert!(
                initial
                    .iter()
                    .any(|(kind, text, _)| kind == "key" && text == "\"键🚀\"")
            );
            assert!(
                initial
                    .iter()
                    .any(|(kind, text, _)| kind == "literal" && text == "true")
            );
            for _ in 0..30 {
                assert_eq!(query(&gate), initial);
            }
            assert_eq!((audit(&source), audit(&rich)), original);
            assert_eq!(gate.session().state().doc(), &original_doc);
            let canonical = source.snapshot().unwrap().rope.to_string();
            let at = canonical.find("true").unwrap();
            source_edit(&source, at..at + 4, "false");
            let after_edit = (audit(&source), audit(&rich));
            let changed = query(&gate);
            let before_shift = changed
                .iter()
                .find(|(kind, text, _)| kind == "literal" && text == "false")
                .unwrap()
                .2
                .clone();
            assert!(
                !changed
                    .iter()
                    .any(|(kind, text, _)| kind == "literal" && text == "true")
            );
            assert_eq!((audit(&source), audit(&rich)), after_edit);
            source_edit(&source, 0..0, "额🚀\n\n");
            let before_query = (audit(&source), audit(&rich));
            let shifted = query(&gate);
            assert_eq!(
                shifted
                    .iter()
                    .find(|(kind, text, _)| kind == "literal" && text == "false")
                    .unwrap()
                    .2,
                before_shift.start + 5..before_shift.end + 5
            );
            assert_eq!((audit(&source), audit(&rich)), before_query);
            // These edits were authored by the Source view. Undo through its
            // real command so its source-only fence positions need no exact
            // Rich selection round trip; the active RichOwner still refreshes.
            source.undo_redo(true).unwrap();
            let before_query = (audit(&source), audit(&rich));
            assert_eq!(query(&gate), changed);
            assert_eq!((audit(&source), audit(&rich)), before_query);
            source.undo_redo(true).unwrap();
            let before_query = (audit(&source), audit(&rich));
            assert_eq!(query(&gate), initial);
            assert_eq!((audit(&source), audit(&rich)), before_query);
            source.inspect(|store, id| {
                let doc = store.view(id).unwrap().doc;
                assert_eq!(store.snapshot(doc).unwrap().body.to_utf8(), raw.as_bytes());
                assert_eq!(store.history_depth(doc).unwrap(), (0, 2));
                assert!(!store.dirty(doc).unwrap());
            });
            let canonical = source.snapshot().unwrap().rope.to_string();
            let language = canonical.find("json").unwrap();
            source_edit(&source, language..language + 4, "rust");
            let before_query = (audit(&source), audit(&rich));
            let rust = query(&gate);
            assert!(rust.iter().any(|(kind, _, _)| kind == "string"));
            assert!(!rust.iter().any(|(kind, _, _)| kind == "key"));
            assert_eq!((audit(&source), audit(&rich)), before_query);
            source.undo_redo(true).unwrap();
            let before_query = (audit(&source), audit(&rich));
            assert_eq!(query(&gate), initial);
            assert_eq!((audit(&source), audit(&rich)), before_query);
        });
    }
    #[test]
    fn actual_owner_unknown_no_language_literal_and_oversize_code_stay_plain_without_store_changes()
    {
        run(|| {
            for raw in ["```wat\nfn unknown() {}\n```\n\n```\nlet no_language=1;\n```\n\n<div>fn hidden(){}</div>\n".to_owned(),format!("```rust\n{}\n```\n","x".repeat(MAX_BYTES))]{
            let source=NativeView::fixture(&raw);let rich=source.sibling().unwrap();let gate=ModeGate::new(source.clone(),rich.clone());gate.to_rich();assert!(gate.rich_mode.get(),"{}",gate.status.get());
            let before=(audit(&source),audit(&rich));assert!(query(&gate).is_empty());assert_eq!((audit(&source),audit(&rich)),before);
        }
        });
    }
}
