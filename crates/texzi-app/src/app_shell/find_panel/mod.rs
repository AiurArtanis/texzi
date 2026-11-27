//! The find panel belongs to a concrete editor view, including in split panes.
#[cfg(test)]
pub(crate) mod fixtures;
mod jobs;
mod view;

use super::*;
use crate::editor_owner::SearchScope;
use jobs::{Index, Pattern, Scan, Subject};
use std::{
    ops::Range,
    sync::{
        Mutex,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    time::Duration,
};
use texzi_search::{Hit, Navigation, Options};
pub(super) use view::{overlay, source_marks, source_viewport};

#[derive(Clone, Copy, PartialEq)]
enum Intent {
    Count,
    Navigate(bool),
    AfterReplace(usize),
    SelectAll,
}
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Toggle {
    Case,
    Word,
    Regex,
    Selection,
}

impl Toggle {
    pub(super) fn command(id: &str) -> Option<Self> {
        Some(match id {
            "texzi.find.toggleCase" => Self::Case,
            "texzi.find.toggleWholeWord" => Self::Word,
            "texzi.find.toggleRegex" => Self::Regex,
            "texzi.find.toggleSelection" => Self::Selection,
            _ => return None,
        })
    }
}

pub(super) struct State {
    query: Signal<String>,
    replacement: Signal<String>,
    case: Signal<bool>,
    word: Signal<bool>,
    regex: Signal<bool>,
    scoped: Signal<bool>,
    has_scope: Signal<bool>,
    composing_query: Signal<bool>,
    composing_replacement: Signal<bool>,
    status: Signal<String>,
    query_input: ElementRef,
    target: RefCell<Option<Rc<Document>>>,
    outside_panel: Cell<bool>,
    last_scope: RefCell<Option<(std::rc::Weak<Document>, Rc<RefCell<SearchScope>>)>>,
    scope: RefCell<Option<Rc<RefCell<SearchScope>>>>,
    version: Cell<Option<(u64, bool)>>,
    subject: RefCell<Option<Subject>>,
    index: RefCell<Option<Arc<Index>>>,
    current: RefCell<Option<Hit>>,
    viewport: RefCell<Range<usize>>,
    visible: RefCell<Vec<Range<usize>>>,
    current_mark: RefCell<Option<Range<usize>>>,
    marks_rev: Signal<u64>,
    generation: Cell<u64>,
    cancel: RefCell<Option<Arc<AtomicBool>>>,
    viewport_number: Cell<u64>,
    viewport_cancel: RefCell<Option<Arc<AtomicBool>>>,
    serial: Arc<Mutex<()>>,
    trace: bool,
    started: Cell<Option<std::time::Instant>>,
    frame_probe: Cell<Option<u64>>,
}
impl State {
    pub(super) fn new() -> Self {
        Self {
            query: signal(String::new()),
            replacement: signal(String::new()),
            case: signal(false),
            word: signal(false),
            regex: signal(false),
            scoped: signal(false),
            has_scope: signal(false),
            composing_query: signal(false),
            composing_replacement: signal(false),
            status: signal(String::new()),
            query_input: element_ref(),
            target: RefCell::default(),
            outside_panel: Cell::new(false),
            last_scope: RefCell::default(),
            scope: RefCell::default(),
            version: Cell::new(None),
            subject: RefCell::default(),
            index: RefCell::default(),
            current: RefCell::default(),
            viewport: RefCell::new(0..0),
            visible: RefCell::default(),
            current_mark: RefCell::default(),
            marks_rev: signal(0),
            generation: Cell::default(),
            cancel: RefCell::default(),
            viewport_number: Cell::default(),
            viewport_cancel: RefCell::default(),
            serial: Arc::new(Mutex::new(())),
            trace: std::env::var_os("TEXZI_FIND_METRICS").as_deref()
                == Some(std::ffi::OsStr::new("1")),
            started: Cell::new(None),
            frame_probe: Cell::new(None),
        }
    }
    fn stop_viewport(&self) {
        self.viewport_number
            .set(self.viewport_number.get().wrapping_add(1));
        if let Some(flag) = self.viewport_cancel.take() {
            flag.store(true, Ordering::Release);
        }
    }
    fn supersede(&self) -> u64 {
        self.stop_viewport();
        if let Some(flag) = self.cancel.take() {
            self.metric("cancel_requested", 0);
            flag.store(true, Ordering::Release);
        }
        let n = self.generation.get().wrapping_add(1);
        self.generation.set(n);
        n
    }
    fn clear_marks(&self) {
        self.visible.borrow_mut().clear();
        self.current_mark.take();
        self.marks_rev.update(|n| *n += 1);
    }
    pub(super) fn close(&self) {
        self.supersede();
        self.retire_query();
        self.outside_panel.set(false);
        if let Some(doc) = self.target.take() {
            self.last_scope.replace(if self.scoped.get() {
                self.scope
                    .borrow()
                    .clone()
                    .map(|range| (Rc::downgrade(&doc), range))
            } else {
                None
            });
            if let Some(api) = doc.rich_reference.api::<RichEditorApi>() {
                api.set_external_decorations(Vec::new());
            }
            self.clear_marks();
        }
        self.subject.take();
        self.index.take();
        self.current.take();
        self.scope.take();
        self.version.set(None);
        self.viewport.replace(0..0);
        self.composing_query.set(false);
        self.composing_replacement.set(false);
    }
    fn editing(&self) -> bool {
        self.composing_query.get() || self.composing_replacement.get()
    }
    fn metric(&self, event: &str, count: u64) {
        if self.trace
            && let Some(start) = self.started.get()
        {
            log::info!(
                "TEXZI_FIND event={event} request={} elapsed_us={} count={count}",
                self.generation.get(),
                start.elapsed().as_micros()
            );
        }
    }
    fn retire_query(&self) {
        if let Some(subject) = self.subject.borrow().as_ref() {
            subject.retire();
        }
    }
}
pub(super) fn is_open(mode: &str) -> bool {
    matches!(mode, "find" | "replace")
}
fn bound(s: &Shell, doc: &Rc<Document>) -> bool {
    s.current().is_some_and(|current| Rc::ptr_eq(&current, doc))
        && s.find_panel
            .target
            .borrow()
            .as_ref()
            .is_some_and(|target| Rc::ptr_eq(target, doc))
}
pub(super) fn active(s: &Shell, doc: &Rc<Document>) -> bool {
    is_open(&s.panel.get()) && bound(s, doc)
}
fn target(s: &Shell) -> Option<Rc<Document>> {
    let panel = s.panel.get();
    if !is_open(&panel) && !(panel.is_empty() && s.find_panel.outside_panel.get()) {
        return None;
    }
    s.current()
        .filter(|doc| doc.editable_body_ready() && bound(s, doc))
}
fn source_mode(s: &Rc<Shell>, doc: &Rc<Document>) -> bool {
    if doc.gate.busy() || s.find_panel.editing() {
        s.find_panel.status.set("请先完成或取消中文组合".into());
        return false;
    }
    if !doc.displayed_rich() {
        return true;
    }
    let caret_top = doc.viewport_caret_top();
    let viewport_anchor = if caret_top.is_none() {
        doc.rich_reference
            .api::<RichEditorApi>()
            .and_then(|api| api.visible_document_range())
            .and_then(|range| {
                doc.gate
                    .find_source_position(range.start.midpoint(range.end))
            })
    } else {
        None
    };
    doc.gate.to_source();
    if doc.displayed_rich() {
        s.find_panel.status.set(doc.gate.status.get());
        return false;
    }
    s.notice.set("已切换到源码查找".into());
    if let Ok((snapshot, epoch)) = doc.source.search_snapshot() {
        let revision = snapshot.revision;
        let anchor = viewport_anchor.map(|byte| view::position(snapshot.body.source(), byte));
        let shell = Rc::downgrade(s);
        let doc = doc.clone();
        s.owner.run(|| {
            spawn(async move {
                let _ = next_frame().await;
                let Some(shell) = shell.upgrade() else { return };
                if !bound(&shell, &doc)
                    || doc.displayed_rich()
                    || shell.modal.get()
                    || !doc
                        .source
                        .search_snapshot()
                        .is_ok_and(|(current, view_epoch)| {
                            current.revision == revision && view_epoch == epoch
                        })
                {
                    return;
                }
                if let Some(api) = doc.reference.api::<EditorApi>() {
                    if let Some(anchor) = anchor {
                        api.reveal_position(anchor, true);
                    } else {
                        api.reveal_primary_caret(caret_top);
                    }
                }
            });
        });
    }
    true
}
pub(super) fn open(s: &Rc<Shell>, replace: bool) {
    let Some(doc) = s.current() else { return };
    if !doc.editable_body_ready() {
        s.notice
            .set("只读分页未加载全文，当前无法执行文内查找".into());
        return;
    }
    let st = &s.find_panel;
    let already = active(s, &doc);
    if !already {
        st.close();
        // Capture the actual selections before focusing the panel or switching mode.
        let selection = doc
            .gate
            .recovery_selection()
            .map(|selection| selection.ranges)
            .unwrap_or_default();
        let has_scope = selection.iter().any(|&(a, b)| a != b);
        match doc.source.freeze_search_scope(&selection) {
            Ok(scope) => {
                st.scope.replace(Some(scope));
            }
            Err(error) => {
                s.notice.set(error);
                return;
            }
        }
        st.has_scope.set(has_scope);
        st.scoped.set(false);
        st.regex.set(false);
        st.word.set(false);
        st.case.set(false);
        st.target.replace(Some(doc.clone()));
    }
    if (replace || st.regex.get()) && !source_mode(s, &doc) {
        s.notice.set(st.status.get());
        if !already {
            st.close();
        }
        return;
    }
    s.panel.set(if replace { "replace" } else { "find" }.into());
    st.query_input.focus();
    if !already {
        set_version(s, &doc);
        refresh(s, Intent::Count, false);
    }
}

pub(super) fn install(s: &Rc<Shell>) {
    let weak = Rc::downgrade(s);
    effect(move || {
        let Some(s) = weak.upgrade() else { return };
        let panel = s.panel.get();
        let current = s.current();
        let bound = s.find_panel.target.borrow().clone();
        let Some(bound) = bound else { return };
        if (!is_open(&panel) && !(s.find_panel.outside_panel.get() && panel.is_empty()))
            || current.as_ref().is_none_or(|doc| !Rc::ptr_eq(&bound, doc))
        {
            untracked(|| {
                s.find_panel.close();
                if is_open(&panel) {
                    s.panel.set(String::new());
                }
            });
            return;
        }
        bound.source.track();
        let rich = bound.displayed_rich();
        let Ok((snapshot, _)) = bound.source.search_snapshot() else {
            return;
        };
        let ver = (snapshot.revision.0, rich);
        if s.find_panel.version.get() != Some(ver) {
            untracked(|| {
                if s.find_panel.outside_panel.get() {
                    s.find_panel.close();
                    return;
                }
                if rich && (s.find_panel.regex.get() || panel == "replace") {
                    s.find_panel.regex.set(false);
                    s.panel.set("find".into());
                    s.notice.set("查找范围已切换为正文".into());
                }
                set_version(&s, &bound);
                refresh(&s, Intent::Count, false);
            });
        }
    });
}
fn set_version(s: &Shell, doc: &Document) {
    let ver = doc
        .source
        .search_snapshot()
        .ok()
        .map(|(v, _)| (v.revision.0, doc.displayed_rich()));
    if s.find_panel.version.replace(ver) != ver {
        s.find_panel.retire_query();
        s.find_panel.subject.take();
        s.find_panel.viewport.replace(0..0);
    }
}
fn valid(s: &Shell, doc: &Rc<Document>, generation: u64) -> bool {
    let st = &s.find_panel;
    st.generation.get() == generation
        && target(s).is_some_and(|target| Rc::ptr_eq(&target, doc))
        && doc
            .source
            .search_snapshot()
            .ok()
            .is_some_and(|(v, _)| st.version.get() == Some((v.revision.0, doc.displayed_rich())))
}
pub(super) fn change(s: &Rc<Shell>, which: Toggle) {
    let Some(doc) = target(s) else { return };
    let st = &s.find_panel;
    if st.editing() {
        return;
    }
    let flag = match which {
        Toggle::Case => st.case,
        Toggle::Word => st.word,
        Toggle::Regex => st.regex,
        Toggle::Selection => st.scoped,
    };
    if which == Toggle::Selection && !st.has_scope.get() {
        st.status.set("请先在正文中选择文字，再打开查找".into());
        return;
    }
    if which == Toggle::Regex && !flag.get() && !source_mode(s, &doc) {
        return;
    }
    st.retire_query();
    flag.update(|v| *v = !*v);
    set_version(s, &doc);
    refresh(s, Intent::Count, false);
}
fn query_changed(s: &Rc<Shell>, query: &str) {
    if s.find_panel.query.with(|old| old == query) {
        return;
    }
    s.find_panel.retire_query();
    s.find_panel.query.set(query.to_owned());
    if s.find_panel.composing_query.get() {
        s.find_panel.supersede();
        s.find_panel.index.take();
        s.find_panel.clear_marks();
    } else {
        refresh(s, Intent::Count, true);
    }
}

fn refresh(s: &Rc<Shell>, intent: Intent, debounce: bool) {
    let Some(doc) = target(s) else { return };
    set_version(s, &doc);
    let st = &s.find_panel;
    let generation = st.supersede();
    st.index.take();
    st.current.take();
    st.clear_marks();
    st.started.set(st.trace.then(std::time::Instant::now));
    if st.query.get().is_empty() {
        st.status.set("请输入查找文字".into());
        return;
    }
    st.status.set("正在查找…".into());
    let s = s.clone();
    let run = move || {
        if !valid(&s, &doc, generation) || s.find_panel.editing() {
            return;
        }
        let st = &s.find_panel;
        let Ok((snapshot, epoch)) = doc.source.search_snapshot() else {
            return;
        };
        let rich = doc.displayed_rich();
        let Some(selection) = doc.gate.recovery_selection() else {
            st.status.set("正文尚未就绪".into());
            return;
        };
        let (root, anchor) = if rich {
            let Some((node, range)) = doc.gate.find_rich_root() else {
                st.status.set("正文尚未就绪".into());
                return;
            };
            (
                Some(node),
                if intent == Intent::Navigate(false) {
                    range.end
                } else {
                    range.start
                },
            )
        } else {
            let (a, b) = selection
                .ranges
                .get(selection.primary)
                .copied()
                .unwrap_or((0, 0));
            (
                None,
                if intent == Intent::Navigate(false) {
                    a.max(b)
                } else {
                    a.min(b)
                },
            )
        };
        let anchor = if let Intent::AfterReplace(at) = intent {
            at
        } else {
            anchor
        };
        let scopes = if st.scoped.get() {
            let scopes = st
                .scope
                .borrow()
                .as_ref()
                .ok_or_else(|| "查找选区已关闭".to_string())
                .and_then(|scope| scope.borrow().ranges(snapshot.revision))
                .and_then(|ranges| {
                    if rich {
                        doc.gate.find_rich_scope(&ranges)
                    } else {
                        Ok(ranges)
                    }
                });
            match scopes {
                Ok(r) => Some(r),
                Err(e) => {
                    st.status.set(e);
                    return;
                }
            }
        } else {
            None
        };
        let band =
            view::visible_band(&doc, &snapshot).unwrap_or_else(|| st.viewport.borrow().clone());
        st.viewport.replace(band.clone());
        let scan = Scan {
            snapshot,
            rich: root,
            cached: st.subject.borrow().clone(),
            pattern: Pattern {
                query: st.query.get(),
                options: Options {
                    case_sensitive: st.case.get(),
                    whole_word: st.word.get(),
                },
                regex: st.regex.get(),
                scopes,
            },
            anchor,
            previous: intent == Intent::Navigate(true),
            viewport: band.clone(),
            select_all: intent == Intent::SelectAll,
        };
        launch(&s, doc, generation, move |cancel, tx| {
            let result = scan.run(&cancel, |subject, range| {
                if let Some(view) = subject.text.view(&range) {
                    let _ = tx.send(Event::First(view));
                }
            })?;
            Ok(Event::Found(Arc::new(result), band, intent, epoch))
        });
    };
    if debounce {
        set_timeout(Duration::from_millis(45), run);
    } else {
        run();
    }
}

enum Event {
    First(Range<usize>),
    Found(Arc<Index>, Range<usize>, Intent, u64),
    Moved(Option<Hit>, bool, u64),
    Prepared(
        u64,
        texzi_document::PreparedBulk,
        u64,
        Option<texzi_document::SourceByte>,
        bool,
    ),
    Failed(String),
}
fn launch(
    s: &Rc<Shell>,
    doc: Rc<Document>,
    generation: u64,
    work: impl FnOnce(Arc<AtomicBool>, &Sender<Event>) -> Result<Event, String> + Send + 'static,
) {
    let st = &s.find_panel;
    let cancel = Arc::new(AtomicBool::new(false));
    st.cancel.replace(Some(cancel.clone()));
    let (tx, rx) = mpsc::channel();
    let serial = st.serial.clone();
    let trace_start = st.started.get();
    if let Err(error) = std::thread::Builder::new()
        .name("texzi-find".into())
        .spawn(move || {
            let stopped = || {
                if let Some(start) = trace_start {
                    log::info!(
                        "TEXZI_FIND event=worker_stopped request={generation} elapsed_us={}",
                        start.elapsed().as_micros()
                    );
                }
            };
            let Ok(_guard) = serial.lock() else {
                let _ = tx.send(Event::Failed("查找线程已退出".into()));
                return;
            };
            if cancel.load(Ordering::Acquire) {
                stopped();
                return;
            }
            let event = work(cancel.clone(), &tx).unwrap_or_else(Event::Failed);
            if !cancel.load(Ordering::Acquire) {
                let _ = tx.send(event);
            } else {
                stopped();
            }
        })
    {
        st.status.set(error.to_string());
        return;
    }
    let shell = Rc::downgrade(s);
    s.owner.run(|| poll(shell, doc, generation, rx));
}
fn poll(weak: std::rc::Weak<Shell>, doc: Rc<Document>, generation: u64, rx: Receiver<Event>) {
    set_timeout(Duration::from_millis(12), move || {
        let Some(s) = weak.upgrade() else { return };
        if !valid(&s, &doc, generation) {
            return;
        }
        match rx.try_recv() {
            Ok(Event::First(hit)) => {
                s.find_panel.metric("first_result", 1);
                s.find_panel.status.set("已找到匹配，正在计数…".into());
                s.find_panel.current_mark.replace(Some(hit));
                s.find_panel.marks_rev.update(|n| *n += 1);
                view::note_present(&s, &doc);
            }
            Ok(event) => {
                finish(&s, &doc, event);
                return;
            }
            Err(TryRecvError::Disconnected) => {
                s.find_panel.status.set("查找任务提前结束".into());
                return;
            }
            Err(TryRecvError::Empty) => {}
        }
        poll(weak, doc, generation, rx);
    });
}
fn finish(s: &Rc<Shell>, doc: &Rc<Document>, event: Event) {
    let st = &s.find_panel;
    match event {
        Event::Failed(error) => {
            st.status.set(error);
            st.clear_marks();
        }
        Event::Found(index, band, intent, epoch) => {
            st.metric("count_complete", index.results.total);
            let may_select = doc.source.search_snapshot().is_ok_and(|(_, e)| e == epoch);
            st.subject.replace(Some(index.subject.clone()));
            st.index.replace(Some(index.clone()));
            let current = index.results.current.clone();
            st.visible
                .replace(to_view(&index, index.results.visible.iter()));
            present(
                s,
                doc,
                current,
                index.results.wrapped,
                may_select && intent != Intent::SelectAll,
            );
            if intent == Intent::SelectAll && may_select {
                match &index.results.selected {
                    Some(Ok(ranges)) if !ranges.is_empty() => {
                        if let Err(e) = select(s, doc, ranges) {
                            st.status.set(e);
                        } else {
                            s.notice.set(format!("已选中全部 {} 处", ranges.len()));
                            s.close_panel();
                        }
                    }
                    Some(Err(limit)) => st.status.set(format!(
                        "共 {} 处，超过 {} 个选区上限",
                        index.results.total, limit.limit
                    )),
                    _ => {}
                }
            } else if *st.viewport.borrow() != band {
                view::request_marks(s, doc, true);
            }
        }
        Event::Moved(hit, wrap, epoch) => {
            if doc.source.search_snapshot().is_ok_and(|(_, e)| e == epoch) {
                present(s, doc, hit, wrap, true);
            }
        }
        Event::Prepared(count, plan, epoch, marker, single) => {
            if s.modal.get() || st.editing() || doc.gate.busy() {
                st.status.set("编辑状态已变化，替换已取消".into());
                return;
            }
            match doc.source.submit_find_replace_at(epoch, plan, marker) {
                Ok((changed, next)) => {
                    let msg = if changed {
                        format!("已替换 {count} 处，可一次撤销")
                    } else {
                        format!("匹配 {count} 处，正文未变化")
                    };
                    s.notice.set(msg.clone());
                    st.status.set(msg);
                    if single {
                        refresh(s, Intent::AfterReplace(next.unwrap_or(0)), false);
                    } else if changed {
                        refresh(s, Intent::Count, false);
                    }
                }
                Err(error) => st.status.set(error),
            }
        }
        Event::First(_) => unreachable!(),
    }
}
fn to_view<'a>(index: &Index, ranges: impl Iterator<Item = &'a Range<usize>>) -> Vec<Range<usize>> {
    ranges.filter_map(|r| index.subject.text.view(r)).collect()
}
fn present(s: &Rc<Shell>, doc: &Rc<Document>, hit: Option<Hit>, wrapped: bool, navigate: bool) {
    let st = &s.find_panel;
    let Some(index) = st.index.borrow().clone() else {
        return;
    };
    let pos = hit.as_ref().map_or(0, |h| h.ordinal);
    let view_hit = hit.as_ref().and_then(|h| index.subject.text.view(&h.range));
    st.current.replace(hit);
    st.current_mark.replace(view_hit.clone());
    st.marks_rev.update(|v| *v += 1);
    view::note_present(s, doc);
    let scope = if st.scoped.get() {
        "选区"
    } else if doc.displayed_rich() {
        "正文"
    } else {
        "源码"
    };
    let wrap = if wrapped { " · 已回绕" } else { "" };
    let message = format!("{scope} · {pos}/{}{wrap}", index.results.total);
    st.status.set(message.clone());
    if navigate && (wrapped || st.outside_panel.get()) {
        s.notice.set(message);
    }
    if navigate
        && let Some(r) = view_hit
        && let Err(e) = select(s, doc, &[r])
    {
        st.status.set(e);
    }
}
fn select(s: &Rc<Shell>, doc: &Rc<Document>, ranges: &[Range<usize>]) -> Result<(), String> {
    if doc.gate.busy() || s.find_panel.editing() || s.modal.get() {
        return Err("请先完成或取消当前操作".into());
    }
    let rev = s.find_panel.version.get().ok_or("正文尚未就绪")?.0;
    if doc.displayed_rich() {
        return doc.gate.find_rich_select(rev, ranges[0].clone());
    }
    doc.source.find_select(rev, ranges)?;
    let shell = Rc::downgrade(s);
    let doc = doc.clone();
    s.owner.run(|| {
        spawn(async move {
            let _ = next_frame().await;
            if let Some(s) = shell.upgrade()
                && s.current().is_some_and(|d| Rc::ptr_eq(&d, &doc))
                && !doc.displayed_rich()
                && let Some(api) = doc.reference.api::<EditorApi>()
            {
                api.reveal_primary_caret(None);
            }
        })
    });
    Ok(())
}
pub(super) fn navigate(s: &Rc<Shell>, previous: bool) {
    if target(s).is_none() {
        let st = &s.find_panel;
        if st.query.get().is_empty() {
            s.notice.set("请输入查找文字".into());
            return;
        }
        let Some(doc) = s.current().filter(|d| d.editable_body_ready()) else {
            return;
        };
        if !s.panel.get().is_empty() {
            return;
        }
        st.close();
        let frozen = st.last_scope.borrow().as_ref().and_then(|(owner, range)| {
            owner
                .upgrade()
                .filter(|d| Rc::ptr_eq(d, &doc))
                .map(|_| range.clone())
        });
        st.scoped.set(frozen.is_some());
        st.scope.replace(frozen);
        if st.regex.get() && !source_mode(s, &doc) {
            return;
        }
        st.outside_panel.set(true);
        st.target.replace(Some(doc));
        refresh(s, Intent::Navigate(previous), false);
        return;
    }
    let Some(doc) = target(s) else { return };
    let st = &s.find_panel;
    if st.editing() || doc.gate.busy() {
        return;
    }
    let cached = st.index.borrow().clone();
    let current = st.current.borrow().clone();
    if let (Some(index), Some(hit)) = (cached, current) {
        let nav = if previous {
            Navigation::PreviousBefore(hit.ordinal)
        } else {
            Navigation::NextAfter(hit.ordinal)
        };
        let Ok((_, epoch)) = doc.source.search_snapshot() else {
            return;
        };
        let generation = st.supersede();
        launch(s, doc, generation, move |cancel, _| {
            let (hit, wrap) = index.next(nav, &cancel)?;
            Ok(Event::Moved(hit, wrap, epoch))
        });
    } else {
        refresh(s, Intent::Navigate(previous), false);
    }
}
pub(super) fn select_all(s: &Rc<Shell>) {
    let Some(doc) = target(s) else { return };
    if source_mode(s, &doc) {
        refresh(s, Intent::SelectAll, false);
    }
}
pub(super) fn replace(s: &Rc<Shell>, all: bool) {
    let Some(doc) = target(s) else { return };
    let st = &s.find_panel;
    if st.editing() || doc.displayed_rich() || doc.gate.busy() {
        return;
    }
    let Some(index) = st.index.borrow().clone() else {
        st.status.set("等待查找完成后可替换".into());
        return;
    };
    let only = if all {
        None
    } else {
        let Some(hit) = st.current.borrow().clone() else {
            st.status.set("没有可替换的匹配".into());
            return;
        };
        Some(hit.range)
    };
    let Ok((snapshot, epoch)) = doc.source.search_snapshot() else {
        return;
    };
    if st.version.get() != Some((snapshot.revision.0, false)) {
        refresh(s, Intent::Count, false);
        return;
    }
    // At an empty hit advance across one original scalar. A hit at EOF wraps.
    let marker = only.as_ref().and_then(|r| {
        let rope = snapshot.body.source();
        let at = if !r.is_empty() {
            r.end
        } else if r.end < rope.len_bytes() {
            rope.char_to_byte(rope.byte_to_char(r.end) + 1)
        } else {
            return None;
        };
        Some(texzi_document::SourceByte::at(&snapshot, at))
    });
    let template = st.replacement.get();
    let generation = st.supersede();
    st.status.set("正在准备替换…".into());
    launch(s, doc, generation, move |cancel, _| {
        let (count, plan) = index.replace(snapshot, &template, only, cancel)?;
        Ok(Event::Prepared(count, plan, epoch, marker, !all))
    });
}
