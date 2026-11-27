//! PCRE2 10.46 (Unicode 16.0.0), 8-bit interpreter over the frozen, LF-normalized
//! editing subject. Call these blocking APIs on a background task. Publish final
//! results / prepared document changes only after rechecking that task's identity.
mod engine;
mod process;
mod template;
mod wire;
mod worker;

use std::{
    convert::Infallible,
    env, fmt,
    fs::File,
    io::{self, BufWriter, Write},
    ops::Range,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use crate::{
    Hit, Navigation, NavigationReplay, Options, Rope, Scope, SearchResults, VisibleReplay,
};
use serde::{Deserialize, Serialize};
use template::Parsed;
use wire::{Operation, Request};

pub use template::{Piece, Replacement};
pub use worker::worker_main;

pub const WORKER_FLAG: &str = "--texzi-search-worker";

/// Numeric receipt from the resident interpreter. No query, subject, or path is recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerStats {
    pub pid: u32,
    pub subject_bytes_read: u64,
    pub pattern_compilations: u64,
    pub completed_operations: u64,
}
pub const MAX_SUBJECT_BYTES: usize = 512 * 1024 * 1024;
const MAX_EXPRESSION_BYTES: usize = 64 * 1024;
const MAX_SCOPES: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemplateIssue {
    InvalidName,
    UnknownGroup,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegexError {
    Cancelled,
    Timeout,
    Syntax { code: i32, byte: usize },
    Template { byte: usize, issue: TemplateIssue },
    EngineLimit { code: i32 },
    Engine { code: i32 },
    Resource,
    Staging,
    InvalidSubject,
    Launch,
    Worker,
}

impl fmt::Display for RegexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("已取消查找"),
            Self::Timeout => f.write_str("正则查找超过 2 秒，已停止本次计算"),
            Self::Syntax { code, byte } => write!(
                f,
                "正则表达式在字节 {byte} 处有误（{}）",
                engine::error_text(*code)
            ),
            Self::Template { byte, issue } => write!(
                f,
                "替换模板在字节 {byte} 处{}",
                match issue {
                    TemplateIssue::InvalidName => "需要有效的组名和右花括号",
                    TemplateIssue::UnknownGroup => "引用了不存在的捕获组",
                }
            ),
            Self::EngineLimit { code } => {
                write!(f, "已达正则匹配资源限制（{}）", engine::error_text(*code))
            }
            Self::Engine { code } => write!(f, "正则匹配失败（{}）", engine::error_text(*code)),
            Self::Resource => f.write_str("搜索请求或结果超过资源额度"),
            Self::Staging => f.write_str("无法准备只读搜索快照"),
            Self::InvalidSubject => f.write_str("搜索快照的长度或 UTF-8 编码无效"),
            Self::Launch => f.write_str("无法启动隔离搜索进程"),
            Self::Worker => f.write_str("隔离搜索进程未能返回完整结果"),
        }
    }
}
impl std::error::Error for RegexError {}
impl From<io::Error> for RegexError {
    fn from(_: io::Error) -> Self {
        Self::Worker
    }
}

#[derive(Debug)]
pub enum VisitError<E> {
    Search(RegexError),
    Visitor(E),
}
impl<E> From<RegexError> for VisitError<E> {
    fn from(error: RegexError) -> Self {
        Self::Search(error)
    }
}

impl VisitError<Infallible> {
    fn into_search(self) -> RegexError {
        match self {
            Self::Search(error) => error,
            Self::Visitor(never) => match never {},
        }
    }
}

/// Use the same budget when first staging the snapshot and starting that query.
/// Reusing a subject for a later query takes a fresh budget.
pub struct Budget<'a> {
    cancel: &'a AtomicBool,
    deadline: Instant,
}

impl<'a> Budget<'a> {
    pub fn new(cancel: &'a AtomicBool) -> Self {
        Self {
            cancel,
            deadline: Instant::now() + Duration::from_secs(2),
        }
    }

    fn check(&self) -> Result<(), RegexError> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(RegexError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(RegexError::Timeout);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct RegexQuery<'a> {
    pub pattern: &'a str,
    pub options: Options,
    pub scope: &'a Scope,
}

/// Share this handle between requests for the same frozen document revision.
/// It is independent of the document's original on-disk path and encoding.
pub struct RegexSubject {
    sessions: process::Sessions,
    file: File,
    len: usize,
}

impl RegexSubject {
    pub fn stage(rope: &Rope, budget: &Budget<'_>) -> Result<Self, RegexError> {
        budget.check()?;
        let len = rope.len_bytes();
        if len > MAX_SUBJECT_BYTES {
            return Err(RegexError::Resource);
        }
        let file = process::stage(|file| {
            let mut out = BufWriter::with_capacity(wire::BATCH_BYTES, file);
            for chunk in rope.chunks() {
                budget.check()?;
                out.write_all(chunk.as_bytes())
                    .map_err(|_| RegexError::Staging)?;
            }
            out.flush().map_err(|_| RegexError::Staging)?;
            Ok(())
        })
        .map_err(|error| {
            if error == RegexError::Worker {
                RegexError::Staging
            } else {
                error
            }
        })?;
        budget.check()?;
        Ok(Self {
            file,
            len,
            sessions: process::Sessions::default(),
        })
    }

    /// Invalidates an active query and releases an idle worker without waiting on the UI.
    /// Ordinary F3 and viewport changes keep the same compiled program and subject.
    pub fn retire(&self) {
        self.sessions.retire();
    }

    pub fn worker_stats(&self) -> Option<WorkerStats> {
        self.sessions.stats()
    }

    fn request(&self, q: RegexQuery<'_>, operation: Operation) -> Result<Request, RegexError> {
        if q.pattern.len() > MAX_EXPRESSION_BYTES
            || q.scope.ranges().is_some_and(|s| s.len() > MAX_SCOPES)
        {
            return Err(RegexError::Resource);
        }
        Ok(Request {
            subject_len: self.len,
            pattern: q.pattern.to_owned(),
            options: q.options,
            scope: q.scope.clone(),
            operation,
        })
    }

    pub fn search(
        &self,
        query: RegexQuery<'_>,
        navigation: Navigation,
        viewport: Range<usize>,
        select_cap: Option<usize>,
        budget: &Budget<'_>,
        mut on_first: impl FnMut(Range<usize>),
    ) -> Result<SearchResults, RegexError> {
        let request = self.request(
            query,
            Operation::Search {
                navigation,
                viewport,
                select_cap,
            },
        )?;
        let executable = env::current_exe().map_err(|_| RegexError::Launch)?;
        self.sessions
            .exchange(
                &executable,
                &self.file,
                &request,
                budget,
                |reader, active| {
                    let mut buf = Vec::new();
                    let mut first = true;
                    loop {
                        match wire::next_frame(reader, &mut buf)? {
                            wire::FIRST if first => {
                                let range =
                                    wire::get_range(&buf, self.len)?.ok_or(RegexError::Worker)?;
                                active()?;
                                on_first(range);
                                first = false;
                            }
                            wire::SUMMARY => return Ok(wire::parse(&buf)?),
                            _ => return Err(RegexError::Worker.into()),
                        }
                    }
                },
            )
            .map_err(VisitError::into_search)
    }

    /// Pass replay state from the completed summary for the exact same request.
    pub fn navigate(
        &self,
        query: RegexQuery<'_>,
        replay: NavigationReplay,
        budget: &Budget<'_>,
    ) -> Result<(Option<Hit>, bool), RegexError> {
        budget.check()?;
        if replay.done() {
            return Ok(replay.finish());
        }
        self.replay(query, Operation::Navigate(replay), budget)
    }

    pub fn viewport(
        &self,
        query: RegexQuery<'_>,
        replay: VisibleReplay,
        budget: &Budget<'_>,
    ) -> Result<Vec<Range<usize>>, RegexError> {
        self.replay(query, Operation::Viewport(replay), budget)
    }

    fn replay<T: serde::de::DeserializeOwned>(
        &self,
        query: RegexQuery<'_>,
        operation: Operation,
        budget: &Budget<'_>,
    ) -> Result<T, RegexError> {
        let request = self.request(query, operation)?;
        let executable = env::current_exe().map_err(|_| RegexError::Launch)?;
        self.sessions
            .exchange(&executable, &self.file, &request, budget, |reader, _| {
                let mut buf = Vec::new();
                if wire::next_frame(reader, &mut buf)? != wire::SUMMARY {
                    return Err(RegexError::Worker.into());
                }
                Ok(wire::parse(&buf)?)
            })
            .map_err(VisitError::into_search)
    }

    /// The visitor prepares a private transaction. Only a successful final count
    /// permits committing it. An absent or superseded current match yields zero.
    pub fn replacements<E>(
        &self,
        query: RegexQuery<'_>,
        template: &str,
        only: Option<Range<usize>>,
        budget: &Budget<'_>,
        mut visit: impl FnMut(Range<usize>, Replacement<'_>) -> Result<(), E>,
    ) -> Result<u64, VisitError<E>> {
        let parsed = Parsed::new(template)?;
        let stride = wire::record_bytes(parsed.groups.len())?;
        let request = self.request(
            query,
            Operation::Replace {
                template: template.to_owned(),
                only,
            },
        )?;
        let executable = env::current_exe().map_err(|_| RegexError::Launch)?;
        self.sessions.exchange(
            &executable,
            &self.file,
            &request,
            budget,
            |reader, active| {
                let mut buf = Vec::new();
                let mut captures = Vec::with_capacity(parsed.groups.len());
                let mut seen = 0u64;
                let mut next_min = 0usize;
                loop {
                    match wire::next_frame(reader, &mut buf)? {
                        wire::BATCH if !buf.is_empty() && buf.len().is_multiple_of(stride) => {
                            for row in buf.chunks_exact(stride) {
                                active()?;
                                let range = wire::get_range(&row[..8], self.len)?
                                    .ok_or(RegexError::Worker)?;
                                if range.start < next_min {
                                    return Err(RegexError::Worker.into());
                                }
                                next_min = range.end + usize::from(range.is_empty());
                                captures.clear();
                                for pair in row[8..].chunks_exact(8) {
                                    captures.push(wire::get_range(pair, self.len)?);
                                }
                                visit(
                                    range,
                                    Replacement {
                                        template: &parsed,
                                        captures: &captures,
                                    },
                                )
                                .map_err(VisitError::Visitor)?;
                                seen += 1;
                            }
                        }
                        wire::COUNT if buf.as_slice() == seen.to_le_bytes() => return Ok(seen),
                        _ => return Err(RegexError::Worker.into()),
                    }
                }
            },
        )
    }
}
