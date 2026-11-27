use crate::rich_owner::RenderedText;
use nsh_base::rich_editor::model::Node;
use std::{
    ops::{ControlFlow, Range},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use texzi_document::{BulkPart, PreparedBulk, PreparedBulkBuilder, Snapshot};
use texzi_search::{
    Collector, Hit, Literal, MAX_SELECTIONS, Navigation, Options, Rope, ScanEvent, Scope,
    SearchResults,
    pcre::{Budget, Piece, RegexQuery, RegexSubject, VisitError},
};

/// Positions supplied by a view are canonical source bytes or rendered document
/// positions. Only this immutable projection crosses into search byte space.
#[derive(Clone)]
pub(super) enum Text {
    Source(Rope),
    Rich(Arc<RenderedText>),
}
impl Text {
    fn rope(&self) -> &Rope {
        match self {
            Self::Source(rope) => rope,
            Self::Rich(text) => &text.rope,
        }
    }
    fn byte(&self, position: usize, end: bool) -> usize {
        match self {
            Self::Source(rope) => position.min(rope.len_bytes()),
            Self::Rich(text) => text.byte_at(position, end),
        }
    }
    pub(super) fn view(&self, range: &Range<usize>) -> Option<Range<usize>> {
        match self {
            Self::Source(_) => Some(range.clone()),
            Self::Rich(text) => {
                Some(text.rich_at(range.start, false)?..text.rich_at(range.end, true)?)
            }
        }
    }
    fn band(&self, r: Range<usize>) -> Range<usize> {
        self.byte(r.start, false)..self.byte(r.end, true)
    }
}

#[derive(Clone)]
pub(super) struct Subject {
    pub text: Text,
    regex: Option<Arc<RegexSubject>>,
}
impl Subject {
    pub(super) fn retire(&self) {
        if let Some(regex) = &self.regex {
            regex.retire();
        }
    }
}
#[derive(Clone)]
pub(super) struct Pattern {
    pub query: String,
    pub options: Options,
    pub regex: bool,
    pub scopes: Option<Vec<Range<usize>>>,
}
#[derive(Clone)]
enum Matcher {
    Literal(Arc<Literal>),
    Regex,
}
pub(super) struct Index {
    pub subject: Subject,
    query: Pattern,
    scope: Scope,
    matcher: Matcher,
    pub results: SearchResults,
}
impl Index {
    fn regex_query(&self) -> RegexQuery<'_> {
        RegexQuery {
            pattern: &self.query.query,
            options: self.query.options,
            scope: &self.scope,
        }
    }
    pub(super) fn next(
        &self,
        nav: Navigation,
        cancel: &AtomicBool,
    ) -> Result<(Option<Hit>, bool), String> {
        let mut replay = self.results.navigation(nav);
        if replay.done() {
            return Ok(replay.finish());
        }
        match &self.matcher {
            Matcher::Regex => self
                .subject
                .regex
                .as_ref()
                .unwrap()
                .navigate(self.regex_query(), replay, &Budget::new(cancel))
                .map_err(message),
            Matcher::Literal(literal) => {
                let _ = literal
                    .scan_events(
                        self.subject.text.rope(),
                        replay.start(),
                        &self.scope,
                        || cancel.load(Ordering::Relaxed),
                        |event| match event {
                            ScanEvent::Match(r) => replay.record(r),
                            ScanEvent::Progress { resume, .. } => replay.progress(resume),
                        },
                    )
                    .map_err(message)?;
                Ok(replay.finish())
            }
        }
    }
    pub(super) fn visible(
        &self,
        viewport: Range<usize>,
        cancel: &AtomicBool,
    ) -> Result<Vec<Range<usize>>, String> {
        let mut replay = self.results.viewport(self.subject.text.band(viewport));
        match &self.matcher {
            Matcher::Regex => self
                .subject
                .regex
                .as_ref()
                .unwrap()
                .viewport(self.regex_query(), replay, &Budget::new(cancel))
                .map_err(message),
            Matcher::Literal(literal) => {
                let _ = literal
                    .scan_events(
                        self.subject.text.rope(),
                        replay.start(),
                        &self.scope,
                        || cancel.load(Ordering::Relaxed),
                        |event| match event {
                            ScanEvent::Match(r) => replay.record(r),
                            ScanEvent::Progress { resume, .. } => replay.progress(resume),
                        },
                    )
                    .map_err(message)?;
                Ok(replay.finish())
            }
        }
    }
    pub(super) fn replace(
        &self,
        snapshot: Snapshot,
        template: &str,
        only: Option<Range<usize>>,
        cancel: Arc<AtomicBool>,
    ) -> Result<(u64, PreparedBulk), String> {
        if !matches!(self.subject.text, Text::Source(_)) {
            return Err("替换需要源码模式".into());
        }
        let mut plan = PreparedBulkBuilder::new(snapshot, cancel.clone());
        let count = match &self.matcher {
            Matcher::Regex => self
                .subject
                .regex
                .as_ref()
                .unwrap()
                .replacements(
                    self.regex_query(),
                    template,
                    only,
                    &Budget::new(&cancel),
                    |range, replacement| {
                        plan.replace(
                            range,
                            replacement.pieces().map(|p| match p {
                                Piece::Text(t) => BulkPart::Literal(t),
                                Piece::Source(r) => BulkPart::Original(r),
                            }),
                        )
                    },
                )
                .map_err(|e| match e {
                    VisitError::Search(e) => e.to_string(),
                    VisitError::Visitor(e) => format!("无法准备替换：{e:?}"),
                })?,
            Matcher::Literal(literal) => {
                if let Some(range) = only {
                    plan.replace(range, [BulkPart::Literal(template)])
                        .map_err(bulk_error)?;
                    1
                } else {
                    let mut count = 0;
                    let result = literal
                        .scan(
                            self.subject.text.rope(),
                            0,
                            &self.scope,
                            || cancel.load(Ordering::Relaxed),
                            |range| match plan.replace(range, [BulkPart::Literal(template)]) {
                                Ok(()) => {
                                    count += 1;
                                    ControlFlow::Continue(())
                                }
                                Err(e) => ControlFlow::Break(e),
                            },
                        )
                        .map_err(message)?;
                    if let ControlFlow::Break(e) = result {
                        return Err(bulk_error(e));
                    }
                    count
                }
            }
        };
        Ok((count, plan.finish().map_err(bulk_error)?))
    }
}
fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn bulk_error(error: texzi_document::Error) -> String {
    format!("无法准备替换：{error:?}")
}

pub(super) struct Scan {
    pub snapshot: Snapshot,
    pub rich: Option<Node>,
    pub cached: Option<Subject>,
    pub pattern: Pattern,
    pub anchor: usize,
    pub previous: bool,
    pub viewport: Range<usize>,
    pub select_all: bool,
}
impl Scan {
    pub(super) fn run(
        self,
        cancel: &AtomicBool,
        mut on_first: impl FnMut(&Subject, Range<usize>),
    ) -> Result<Index, String> {
        let mut subject = match self.cached {
            Some(value) => value,
            None => Subject {
                text: match self.rich {
                    Some(root) => Text::Rich(Arc::new(RenderedText::build(&root, cancel)?)),
                    None => Text::Source(self.snapshot.body.source().clone()),
                },
                regex: None,
            },
        };
        let scope = match &self.pattern.scopes {
            None => Scope::Document,
            Some(ranges) => Scope::selections(ranges.iter().map(|r| subject.text.band(r.clone()))),
        };
        let viewport = subject.text.band(self.viewport);
        let anchor = subject.text.byte(self.anchor, self.previous);
        let nav = if self.previous {
            Navigation::PreviousFrom(anchor)
        } else {
            Navigation::NextFrom(anchor)
        };
        let select_cap = self.select_all.then_some(MAX_SELECTIONS);
        let (matcher, results) = if self.pattern.regex {
            let budget = Budget::new(cancel);
            if subject.regex.is_none() {
                subject.regex = Some(Arc::new(
                    RegexSubject::stage(subject.text.rope(), &budget).map_err(message)?,
                ));
            }
            let query = RegexQuery {
                pattern: &self.pattern.query,
                options: self.pattern.options,
                scope: &scope,
            };
            let results = subject
                .regex
                .as_ref()
                .unwrap()
                .search(query, nav, viewport, select_cap, &budget, |r| {
                    on_first(&subject, r)
                })
                .map_err(message)?;
            (Matcher::Regex, results)
        } else {
            let rope = subject.text.rope();
            let literal = Literal::try_new(&self.pattern.query, self.pattern.options, || {
                cancel.load(Ordering::Relaxed)
            })
            .map_err(message)?
            .ok_or("请输入查找文字")?;
            let visible = literal
                .first_visible(rope, viewport.clone(), &scope, || {
                    cancel.load(Ordering::Relaxed)
                })
                .map_err(message)?;
            let previewed = visible.is_some();
            if let Some(hit) = visible {
                on_first(&subject, hit);
            }
            let mut collector = Collector::new(rope.len_bytes(), nav, viewport, select_cap);
            let _ = literal
                .scan_events(
                    rope,
                    0,
                    &scope,
                    || cancel.load(Ordering::Relaxed),
                    |event| {
                        match event {
                            ScanEvent::Match(r) => {
                                let end = r.end;
                                if collector.record(r, Some(end)) && !previewed {
                                    on_first(&subject, collector.first().unwrap().range.clone());
                                }
                            }
                            ScanEvent::Progress { resume, .. } => collector.progress(resume),
                        }
                        ControlFlow::<()>::Continue(())
                    },
                )
                .map_err(message)?;
            (Matcher::Literal(Arc::new(literal)), collector.finish())
        };
        Ok(Index {
            subject,
            query: self.pattern,
            scope,
            matcher,
            results,
        })
    }
}
