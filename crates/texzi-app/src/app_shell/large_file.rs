use super::*;
use crate::editor_owner::{
    MAX_EDIT_BYTES, PrepareError, PreparedFile, SourcePreparation, line_limit_message,
};
use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};
use texzi_document::files::{
    Encoding,
    paged::{Calibration, FileSource, Page, PagedFile, Position},
};

const MIB: u64 = 1024 * 1024;
pub(super) const LARGE_BYTES: u64 = 20 * MIB;
const HUGE_BYTES: u64 = 200 * MIB;
// Editing owns decoded text, provenance, a Rope and a save-conflict baseline.
// Only one such import runs at a time. Files above this bound stay seekable.
pub(super) const SOURCE_ONLY_BYTES: u64 = 50_000_000;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(125);

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImportState {
    Idle,
    Loading,
    Cancelling,
    Cancelled,
    LineLimit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImportProgress {
    PreparingPreview,
    Queued,
    VerifyingSource,
    RestoringDraft,
    PreparingSource(SourcePreparation),
    Attaching,
}

impl ImportProgress {
    fn read_amount(self, total: u64) -> Option<u64> {
        match self {
            Self::PreparingSource(SourcePreparation::Reading(read))
                if total > 0 && read <= total =>
            {
                Some(read)
            }
            _ => None,
        }
    }

    fn read_percent(self, total: u64) -> Option<u32> {
        self.read_amount(total)
            .map(|read| (u128::from(read) * 100 / u128::from(total)) as u32)
    }

    fn label(self, total: u64) -> String {
        match self {
            Self::PreparingPreview => "正在准备首屏预览…".into(),
            Self::Queued => "等待完整加载…".into(),
            Self::VerifyingSource => "正在校验原文件…".into(),
            Self::RestoringDraft => "正在准备恢复草稿…".into(),
            Self::PreparingSource(SourcePreparation::Reading(read)) => {
                self.read_percent(total).map_or_else(
                    || "正在读取文件…".into(),
                    |percent| format!("正在读取 · {read} / {total} B · {percent}%"),
                )
            }
            Self::PreparingSource(SourcePreparation::Decoding) => "正在解码文件…".into(),
            Self::PreparingSource(SourcePreparation::Building) => "正在构建编辑模型…".into(),
            Self::Attaching => "正在打开编辑视图…".into(),
        }
    }
}

pub(super) fn requires_editable(id: &str) -> bool {
    crate::rich_commands::handles(id)
        || (source_command(id).is_some()
            && !matches!(id, "texzi.edit.copy" | "texzi.edit.selectAll"))
        || matches!(
            id,
            "texzi.file.save"
                | "texzi.file.saveAs"
                | "texzi.group.splitRight"
                | "texzi.group.splitDown"
                | "texzi.editor.toggleWrap"
                | "texzi.markdown.toggleSource"
                | "texzi.navigation.gotoLine"
        )
        || (id.starts_with("texzi.find.")
            && !matches!(id, "texzi.find.inFolder" | "texzi.find.close"))
}

#[derive(PartialEq)]
struct VisiblePage {
    data: Page,
    encoding: Encoding,
    first: u64,
}

#[derive(Clone, Default, PartialEq, Eq)]
struct InspectorMessage {
    text: String,
    is_error: bool,
}

impl InspectorMessage {
    fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: false,
        }
    }

    fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            is_error: true,
        }
    }
}

pub(super) struct LargeFile {
    source: Arc<FileSource>,
    choice: Cell<Option<Encoding>>,
    limited_encodings: RefCell<Vec<Encoding>>,
    pending_seek: Signal<Option<Position>>,
    path: PathBuf,
    len: u64,
    initial_rich: bool,
    pub(super) restored_view: Option<session_recovery::EditorView>,
    pub(super) restored_draft: Signal<Option<session_recovery::Draft>>,
    reader: RefCell<Option<Arc<PagedFile>>>,
    page: Signal<Option<Rc<VisiblePage>>>,
    message: Signal<InspectorMessage>,
    reading: Signal<bool>,
    import_state: Signal<ImportState>,
    import_progress: Signal<Option<Arc<Mutex<ImportProgress>>>>,
    generation: Cell<u64>,
    cancel: RefCell<Option<Arc<AtomicBool>>>,
}

impl LargeFile {
    pub(super) fn new(
        path: PathBuf,
        source: Arc<FileSource>,
        initial_rich: bool,
        restored_view: Option<session_recovery::EditorView>,
        restored_draft: Option<session_recovery::Draft>,
    ) -> Rc<Self> {
        let len = source.len;
        Rc::new(Self {
            source,
            choice: Cell::new(None),
            limited_encodings: RefCell::new(Vec::new()),
            pending_seek: signal(None),
            path,
            len,
            initial_rich,
            restored_view,
            restored_draft: signal(restored_draft),
            reader: Default::default(),
            page: signal(None),
            message: signal(InspectorMessage::info(if len >= HUGE_BYTES {
                ""
            } else {
                "正在准备首屏…"
            })),
            reading: signal(false),
            import_state: signal(ImportState::Idle),
            import_progress: signal_with(None, SignalEquals::Never),
            generation: Cell::new(0),
            cancel: Default::default(),
        })
    }

    pub(super) fn status(&self) -> String {
        if self.restored_draft.get().is_some()
            && matches!(
                self.import_state.get(),
                ImportState::Idle | ImportState::Cancelled
            )
        {
            return "待恢复草稿 · 原文件预览只读".into();
        }
        match self.import_state.get() {
            ImportState::Loading => "正在加载完整文档 · 预览只读".into(),
            ImportState::Cancelling => "正在取消完整加载 · 预览只读".into(),
            ImportState::Cancelled => "已取消完整加载 · 预览只读".into(),
            ImportState::LineLimit => "行数超限 · 只读分页".into(),
            ImportState::Idle => format!(
                "大文件 · 只读分页 · {:.1} MiB",
                self.len as f64 / MIB as f64
            ),
        }
    }

    pub(super) fn recovery_view(&self) -> session_recovery::EditorView {
        session_recovery::EditorView::for_pending(self.initial_rich, self.restored_view.as_ref())
    }

    pub(super) fn finish_recovery_limit(&self, shell: &Rc<Shell>, doc: &Document, reason: &str) {
        session_recovery::retain_limited_draft(shell, doc, reason);
        self.message.set(InspectorMessage::error(
            session_recovery::retained_draft_message(shell, &format!("草稿未能载入（{reason}）")),
        ));
    }

    fn cancel_import(&self) {
        if let Some(cancel) = self.cancel.borrow_mut().take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.import_state.set(ImportState::Idle);
        self.import_progress.set(None);
    }

    fn mark_line_limit(self: &Rc<Self>, owner: Owner, encoding: Encoding) {
        if !self.limited_encodings.borrow().contains(&encoding) {
            self.limited_encodings.borrow_mut().push(encoding);
        }
        self.choice.set(Some(encoding));
        self.import_state.set(ImportState::LineLimit);
        if self
            .reader
            .borrow()
            .as_ref()
            .is_some_and(|reader| reader.encoding != encoding)
        {
            // The full decode can identify a legacy codec beyond an ASCII-only first page.
            // Invalidate an in-flight navigation before replacing the sampled reader.
            let at = self
                .page
                .with(|page| page.as_ref().map_or(0, |page| page.data.start));
            self.generation.set(self.generation.get().wrapping_add(1));
            self.reading.set(false);
            self.reader.borrow_mut().take();
            self.page.set(None);
            self.load(owner, Position::Near(at), None);
        }
    }

    fn cancelled(&self, token: &Arc<AtomicBool>) {
        // Closing the tab, choosing another encoding or retrying can supersede this job.
        if self
            .cancel
            .borrow()
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, token))
        {
            self.cancel_import();
            self.import_state.set(ImportState::Cancelled);
        }
    }

    pub(super) fn close(&self) {
        self.cancel_import();
        self.generation.set(self.generation.get().wrapping_add(1));
        self.reader.borrow_mut().take();
        self.page.set(None);
        self.reading.set(false);
    }

    fn show_page(&self, data: Page, reader: &PagedFile) {
        self.page.set(Some(Rc::new(VisiblePage {
            data,
            encoding: reader.encoding,
            first: reader.first,
        })));
        self.message.set(InspectorMessage::default());
    }

    fn calibrate(self: &Rc<Self>, owner: Owner) {
        let Some(pos) = self.pending_seek.get() else {
            return;
        };
        let Some(reader) = self.reader.borrow().clone() else {
            return;
        };
        if self.reading.get() {
            return;
        }
        let view = self.clone();
        let generation = self.generation.get();
        let encoding = reader.encoding;
        let label = encoding_name(encoding);
        self.reading.set(true);
        owner.run(|| {
            spawn(async move {
                if view.generation.get() != generation {
                    return;
                }
                let mut from = reader.scan_start(pos);
                let mut sampled_at = Instant::now();
                view.message.set(InspectorMessage::info(format!(
                    "正在校准 {label} 字符边界 · 已到字节 {from}"
                )));
                loop {
                    let scan = reader.clone();
                    let step = blocking(move || scan.calibrate_step(pos, from)).await;
                    if view.generation.get() != generation {
                        return;
                    }
                    match step {
                        Ok(Ok(Calibration::More(at))) => {
                            from = at;
                            if sampled_at.elapsed() >= PROGRESS_INTERVAL {
                                sampled_at = Instant::now();
                                view.message.set(InspectorMessage::info(format!(
                                    "正在校准 {label} 字符边界 · 已到字节 {at}"
                                )));
                            }
                            // Yield between bounded reads, with a visible and cancellable operation.
                            let _ = next_frame().await;
                            if view.generation.get() != generation {
                                return;
                            }
                        }
                        Ok(Ok(Calibration::Ready(page))) => {
                            view.show_page(page, &reader);
                            view.pending_seek.set(None);
                            break;
                        }
                        Ok(Err(e)) => {
                            view.message
                                .set(InspectorMessage::error(format!("校准失败：{e}")));
                            break;
                        }
                        Err(e) => {
                            view.message
                                .set(InspectorMessage::error(format!("校准未完成：{e}")));
                            break;
                        }
                    }
                }
                view.reading.set(false);
            })
        });
    }

    fn load(self: &Rc<Self>, owner: Owner, pos: Position, encoding: Option<Encoding>) {
        if self.reading.get() {
            return;
        }
        let current = self.reader.borrow().clone();
        if let Some(selected) = encoding {
            if current
                .as_ref()
                .is_some_and(|reader| reader.first != 0 && reader.encoding != selected)
            {
                self.message.set(InspectorMessage::error(
                    "所选编码与文件 BOM 不一致，已保留当前编码和页面。",
                ));
                return;
            }
            self.cancel_import();
            if self.limited_encodings.borrow().contains(&selected) {
                self.import_state.set(ImportState::LineLimit);
            }
        }
        self.pending_seek.set(None);
        let source = self.source.clone();
        let choice = encoding.or(self.choice.get());
        let generation = self.generation.get();
        self.reading.set(true);
        self.message
            .set(InspectorMessage::info("正在读取请求的页面…"));
        let view = self.clone();
        owner.run(|| {
            spawn(async move {
                let result = blocking(move || {
                    let reader = match (current, encoding) {
                        (Some(file), None) => file,
                        _ => Arc::new(PagedFile::from_source(source, choice)?),
                    };
                    let page = reader.read(pos);
                    Ok::<_, std::io::Error>((reader, page))
                })
                .await;
                if generation != view.generation.get() {
                    return;
                }
                // A first codec selection may precede the first page and its BOM detection.
                if encoding.is_some()
                    && let Ok(Ok((reader, _))) = &result
                {
                    view.choice.set(Some(reader.encoding));
                    if view.limited_encodings.borrow().contains(&reader.encoding) {
                        view.cancel_import();
                        view.import_state.set(ImportState::LineLimit);
                    }
                }
                view.reading.set(false);
                match result {
                    Ok(Ok((reader, Err(error)))) => {
                        if error.kind() == std::io::ErrorKind::WouldBlock {
                            view.pending_seek.set(Some(pos));
                            let at = match pos {
                                Position::Boundary(at)
                                | Position::Near(at)
                                | Position::Before(at) => at,
                            };
                            view.message
                                .set(InspectorMessage::info(format!("请求位置 {at} B · {error}")));
                        } else {
                            view.message.set(InspectorMessage::error(error.to_string()));
                        }
                        *view.reader.borrow_mut() = Some(reader);
                    }
                    Ok(Ok((reader, Ok(page)))) => {
                        *view.reader.borrow_mut() = Some(reader.clone());
                        view.show_page(page, &reader);
                    }
                    Ok(Err(e)) => view
                        .message
                        .set(InspectorMessage::error(format!("读取失败：{e}"))),
                    Err(e) => view
                        .message
                        .set(InspectorMessage::error(format!("读取任务未完成：{e}"))),
                }
            })
        });
    }
}

fn encoding_name(encoding: Encoding) -> &'static str {
    match encoding {
        Encoding::Utf8 => "UTF-8",
        Encoding::Utf16Le => "UTF-16 LE",
        Encoding::Utf16Be => "UTF-16 BE",
        Encoding::GbkWebEncodingRs0844 => "GBK",
        _ => "GB18030",
    }
}

struct Import {
    doc: Rc<Document>,
    large: Rc<LargeFile>,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<ImportProgress>>,
}

#[derive(Default)]
pub(super) struct ImportQueue {
    jobs: RefCell<VecDeque<Import>>,
    active: RefCell<Option<Arc<AtomicBool>>>,
}

impl Shell {
    pub(super) fn open_large(
        self: &Rc<Self>,
        path: PathBuf,
        source: Arc<FileSource>,
        limited_encoding: Option<Encoding>,
        restored_view: Option<session_recovery::EditorView>,
        restored_draft: Option<session_recovery::Draft>,
    ) -> Rc<Document> {
        let size = source.len;
        self.remember(&path);
        let id = self.next.get();
        self.next.set(id + 1);
        let initial_rich = self.initial_markdown_rich(
            Some(&path),
            size,
            restored_view.as_ref().map(|view| view.rich),
        );
        let (doc, large) = self.owner.run(|| {
            let large = LargeFile::new(
                path.clone(),
                source,
                initial_rich,
                restored_view,
                restored_draft,
            );
            if let Some(encoding) = limited_encoding {
                large.mark_line_limit(self.owner, encoding);
            }
            let doc = self.make_document(
                id,
                NativeView::reader_placeholder(),
                Some(path),
                Some(large.clone()),
                false,
            );
            (doc, large)
        });
        self.show_new(doc.clone());
        if size < HUGE_BYTES {
            large.load(self.owner, Position::Boundary(0), None);
            self.queue_import(doc.clone(), large);
        }
        doc
    }

    fn queue_import(self: &Rc<Self>, doc: Rc<Document>, large: Rc<LargeFile>) {
        if large.len > MAX_EDIT_BYTES {
            large
                .message
                .set(InspectorMessage::error(byte_limit_message()));
            return;
        }
        if matches!(
            large.import_state.get(),
            ImportState::Loading | ImportState::Cancelling | ImportState::LineLimit
        ) {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(ImportProgress::PreparingPreview));
        *large.cancel.borrow_mut() = Some(cancel.clone());
        large.import_progress.set(Some(progress.clone()));
        large.import_state.set(ImportState::Loading);
        if large.page.with(Option::is_none) && !large.reading.get() {
            large.load(self.owner, Position::Boundary(0), None);
        }
        let shell = self.clone();
        self.owner.run(|| {
            spawn(async move {
                while large.reading.get() && !cancel.load(Ordering::Relaxed) {
                    let _ = next_frame().await;
                }
                if !cancel.load(Ordering::Relaxed)
                    && large.page.with(Option::is_some)
                    && shell
                        .current()
                        .is_some_and(|current| Rc::ptr_eq(&current, &doc))
                {
                    wait_for_editor_frame(doc.body).await;
                }
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                *progress.lock().unwrap() = ImportProgress::Queued;
                shell.imports.jobs.borrow_mut().push_back(Import {
                    doc,
                    large,
                    cancel,
                    progress,
                });
                shell.next_import();
            })
        });
    }

    fn cancel_full_import(&self, file: &LargeFile) {
        let Some(token) = file.cancel.borrow().clone() else {
            return;
        };
        token.store(true, Ordering::Relaxed);
        if self
            .imports
            .active
            .borrow()
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, &token))
        {
            file.import_state.set(ImportState::Cancelling);
            file.import_progress.set(None);
        } else {
            // Neither a preview still preparing nor a queued import owns a full model.
            self.imports
                .jobs
                .borrow_mut()
                .retain(|job| !Arc::ptr_eq(&token, &job.cancel));
            file.cancelled(&token);
        }
    }

    fn next_import(self: &Rc<Self>) {
        if self.imports.active.borrow().is_some() {
            return;
        }
        let job = loop {
            let Some(job) = self.imports.jobs.borrow_mut().pop_front() else {
                return;
            };
            if !job.cancel.load(Ordering::Relaxed) {
                break job;
            }
        };
        *self.imports.active.borrow_mut() = Some(job.cancel.clone());
        let shell = self.clone();
        self.owner.run(|| {
            spawn(async move {
                let path = job.large.path.clone();
                let cancel = job.cancel.clone();
                let source = job.large.source.clone();
                let encoding = job.large.choice.get();
                let draft = job.large.restored_draft.get_untracked();
                let progress = job.progress.clone();
                let result = blocking(move || {
                    let report = |stage| *progress.lock().unwrap() = stage;
                    let prepared = if let Some(draft) = draft {
                        report(ImportProgress::RestoringDraft);
                        draft.prepare(&source, &path, MAX_EDIT_BYTES, &cancel)
                    } else {
                        report(ImportProgress::VerifyingSource);
                        source.verify_path(&path).map_err(|e| e.to_string())?;
                        PreparedFile::from_source_with_progress(
                            &source,
                            MAX_EDIT_BYTES,
                            &cancel,
                            encoding,
                            |stage| report(ImportProgress::PreparingSource(stage)),
                        )
                        .map(|prepared| (prepared, true))
                    }?;
                    report(ImportProgress::Attaching);
                    Ok(prepared)
                })
                .await;
                if job.cancel.load(Ordering::Relaxed) {
                    // A complete model may already be in transit. Release it away from the UI
                    // before acknowledging cancellation or starting another full import.
                    let _ = blocking(move || drop(result)).await;
                    job.large.cancelled(&job.cancel);
                } else if shell
                    .docs
                    .with(|docs| docs.iter().any(|d| Rc::ptr_eq(&job.doc, d)))
                {
                    job.large.cancel_import();
                    match result
                        .map_err(|e| PrepareError::Other(e.to_string()))
                        .and_then(|r| r)
                        .and_then(|(mut data, mut named)| {
                            if named
                                && let Err(error) = job.large.source.verify_path(&job.large.path)
                            {
                                if job.large.restored_draft.get_untracked().is_none() {
                                    return Err(PrepareError::Other(error.to_string()));
                                }
                                data.detach_baseline();
                                named = false;
                            }
                            Ok((data, named))
                        })
                        .and_then(|(data, named)| {
                            shell
                                .owner
                                .run(|| data.attach())
                                .map(|source| (source, named))
                                .map_err(PrepareError::Other)
                        }) {
                        Ok((source, named)) => {
                            let path = named.then(|| job.large.path.clone());
                            let start_rich = named && job.large.initial_rich;
                            let restored = job.large.restored_view.as_ref();
                            if let Some(view) = restored {
                                view.restore_source(&source);
                            }
                            let doc = shell.make_document(job.doc.id, source, path, None, false);
                            doc.import_complete.set(named);
                            if let Some(view) = restored {
                                view.restore_in_mode(&doc, start_rich);
                            }
                            if job.large.restored_draft.get_untracked().is_some() {
                                doc.auto_save.ignore_restored(doc.source.dirty_revision());
                            }
                            job.large.close();
                            shell.docs.update(|docs| {
                                if let Some(slot) =
                                    docs.iter_mut().find(|d| Rc::ptr_eq(d, &job.doc))
                                {
                                    *slot = doc.clone();
                                }
                            });
                            session_recovery::restore_waiting_split(&shell, &doc);
                            if start_rich {
                                shell.request_mode_with_position(
                                    doc.clone(),
                                    true,
                                    restored.is_some_and(|view| view.rich && view.has_scroll()),
                                );
                            }
                            if !named
                                && shell
                                    .current()
                                    .is_some_and(|current| Rc::ptr_eq(&current, &doc))
                            {
                                shell
                                    .notice
                                    .set("原文件无法重新关联 · 草稿已恢复到未命名标签".into());
                            }
                        }
                        Err(PrepareError::LineLimit(encoding)) => {
                            job.large.mark_line_limit(shell.owner, encoding)
                        }
                        Err(PrepareError::RecoveryLimit(reason)) => {
                            // A confirmation already showing for this draft must settle
                            // before it stops belonging to the visible reader.
                            while shell.modal.get() {
                                let _ = next_frame().await;
                            }
                            if shell
                                .docs
                                .with(|docs| docs.iter().any(|doc| Rc::ptr_eq(doc, &job.doc)))
                            {
                                job.large.finish_recovery_limit(&shell, &job.doc, &reason);
                            }
                        }
                        Err(error) => {
                            let message = if job.large.restored_draft.get_untracked().is_some() {
                                session_recovery::retained_draft_message(
                                    &shell,
                                    &format!("草稿未能载入（{error}）"),
                                )
                            } else {
                                format!("完整编辑加载失败：{error}")
                            };
                            job.large.message.set(InspectorMessage::error(message));
                        }
                    }
                }
                shell.imports.active.borrow_mut().take();
                shell.next_import();
            })
        });
    }
}

fn byte_limit_message() -> String {
    format!(
        "文件超过 {} MiB 完整编辑预算，可只读分页查看。",
        MAX_EDIT_BYTES / MIB
    )
}

fn action(label: &'static str, click: impl Fn() + 'static) -> nsh_base::ButtonElement {
    nsh_base::button()
        .h(px(metric("field")))
        .px(px(8.0))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .border(border_width)
        .border_color(|| color("chrome-edge"))
        .bg(|| color("paper"))
        .text_color(|| color("ink-1"))
        .text_size(px(font_size("meta")))
        .app_class(CONTROL_STATES)
        .accessibility_label(label)
        .child(text(label))
        .on_press(move |_: &()| click())
}

fn import_progress(
    source: Signal<Option<Arc<Mutex<ImportProgress>>>>,
    total: u64,
) -> impl IntoElement {
    let progress = signal(ImportProgress::PreparingPreview);
    effect(move || {
        if source.with(Option::is_some) {
            let sample = move || {
                let Some(stage) = source
                    .with_untracked(|source| source.as_ref()?.try_lock().ok().map(|stage| *stage))
                else {
                    return;
                };
                if progress.get_untracked() != stage {
                    progress.set(stage);
                }
            };
            sample();
            let timer = set_interval(PROGRESS_INTERVAL, sample);
            cleanup(move || timer.cancel());
        }
    });
    view()
        .gap(px(4.0))
        .child(
            text(move || progress.get().label(total))
                .tabular_nums()
                .text_size(px(font_size("meta")))
                .text_color(|| color("muted-readable")),
        )
        .child(
            nsh_base::progress()
                .accessibility_label(move || {
                    let stage = progress.get();
                    let label = stage.read_amount(total).map_or_else(
                        || format!("完整加载阶段，{}", stage.label(total)),
                        |read| format!("源文件读取进度，已读 {read} 字节，共 {total} 字节"),
                    );
                    Some(SharedString::from(label))
                })
                .value(move || progress.get().read_percent(total).unwrap_or(0) as f32)
                .indeterminate(move || progress.get().read_percent(total).is_none())
                .track_style(|s| {
                    s.h(px(4.0))
                        .rounded_full()
                        .overflow_hidden()
                        .bg(|| color("seam-strong"))
                })
                .indicator_style(move |s| {
                    let s = crate::origami::ribbon(s.h_full().rounded_full());
                    if progress.get().read_percent(total).is_none() {
                        s.w(px(0.0))
                    } else {
                        s
                    }
                }),
        )
}

pub(super) fn viewer(shell: Rc<Shell>, doc: Rc<Document>, file: Rc<LargeFile>) -> impl IntoElement {
    let owner = shell.owner;
    let id = doc.id;
    let reference = doc.reference;
    let body = doc.body;
    let focus = shell.second_active;
    let input = signal(String::new());
    let page = file.page;
    let message = file.message;
    let import_state = file.import_state;
    let progress = file.import_progress;
    let bytes = file.len;
    let name = doc.name();
    let inspector_label = format!("大文件导入检视舱，{name}");
    let open = file.clone();
    let start = shell.clone();
    let import = file.clone();
    let cancel = file.clone();
    let cancel_shell = shell.clone();
    let close = shell.clone();

    let heading = view()
        .flex_row()
        .flex_wrap()
        .justify_between()
        .items_center()
        .gap(px(8.0))
        .child(
            view()
                .max_w(relative(1.0))
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(8.0))
                .child(
                    text(name)
                        .min_w(px(0.0))
                        .max_w(relative(1.0))
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Tail)
                        .font_weight(FontWeight::BOLD)
                        .text_color(|| color("ink-1")),
                )
                .child(
                    text(format!("{:.1} MiB", bytes as f64 / MIB as f64))
                        .tabular_nums()
                        .text_size(px(font_size("meta")))
                        .text_color(|| color("success-fg")),
                )
                .child(
                    text("只读分页")
                        .text_size(px(font_size("meta")))
                        .text_color(|| color("muted-readable")),
                )
                .child(when(
                    move || {
                        matches!(
                            import_state.get(),
                            ImportState::Cancelling | ImportState::Cancelled
                        )
                    },
                    move || {
                        text(move || {
                            if import_state.get() == ImportState::Cancelling {
                                "正在取消完整加载"
                            } else {
                                "已取消完整加载"
                            }
                        })
                        .text_size(px(font_size("meta")))
                        .text_color(|| color("muted-readable"))
                    },
                )),
        )
        .child(
            view()
                .max_w(relative(1.0))
                .flex_row()
                .flex_wrap()
                .gap(px(8.0))
                .child(when(
                    move || page.with(Option::is_none),
                    move || {
                        let open = open.clone();
                        action("只读分页查看", move || {
                            open.load(owner, Position::Boundary(0), None);
                        })
                    },
                ))
                .child(when(
                    move || {
                        bytes <= MAX_EDIT_BYTES
                            && matches!(
                                import_state.get(),
                                ImportState::Idle | ImportState::Cancelled
                            )
                    },
                    move || {
                        let start = start.clone();
                        let doc = doc.clone();
                        let import = import.clone();
                        action("尝试完整编辑", move || {
                            start.queue_import(doc.clone(), import.clone());
                        })
                    },
                ))
                .child(when(
                    move || {
                        matches!(
                            import_state.get(),
                            ImportState::Loading | ImportState::Cancelling
                        )
                    },
                    move || {
                        let cancel = cancel.clone();
                        let shell = cancel_shell.clone();
                        action("取消完整加载", move || {
                            shell.cancel_full_import(&cancel)
                        })
                        .disabled(move || import_state.get() == ImportState::Cancelling)
                        .border_color(|| color("error-fg"))
                        .text_color(|| color("error-fg"))
                    },
                ))
                .child(action("关闭文件", move || close.close_tab(id))),
        );

    let page_details = when(
        move || page.with(Option::is_some),
        move || {
            let range = move || page.with(|page| page.as_ref().map(|p| (p.data.start, p.data.end)));
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(8.0))
                .child(
                    text(move || {
                        range().map_or_else(String::new, |(start, end)| {
                            format!("字节 [{start}, {end}) / {bytes} B")
                        })
                    })
                    .label(move || {
                        range().map_or_else(String::new, |(start, end)| {
                            format!("从 {start} 字节到 {end} 字节之前，源文件共 {bytes} 字节")
                        })
                    })
                    .tabular_nums()
                    .text_size(px(font_size("meta")))
                    .text_color(|| color("muted-readable")),
                )
                .child(
                    text(move || {
                        page.with(|page| {
                            page.as_ref().map_or_else(String::new, |p| {
                                format!("当前编码 · {}", encoding_name(p.encoding))
                            })
                        })
                    })
                    .text_size(px(font_size("meta")))
                    .text_color(|| color("muted-readable")),
                )
        },
    );

    let navigator = file.clone();
    let navigate = move |label, toward_end, get: fn(&Page, u64) -> Position| {
        let file = navigator.clone();
        action(label, move || {
            if let Some(page) = file.page.get() {
                file.load(owner, get(&page.data, bytes), None);
            }
        })
        .disabled(move || {
            page.with(|page| {
                page.as_ref().is_none_or(|p| {
                    if toward_end {
                        p.data.end >= bytes
                    } else {
                        p.data.start <= p.first
                    }
                })
            })
        })
    };
    let jump = {
        let file = file.clone();
        move || match input.get().trim().parse::<u64>() {
            Ok(at) if at < bytes => file.load(owner, Position::Near(at), None),
            _ => file.message.set(InspectorMessage::error(format!(
                "请输入 0 至 {} 之间的字节位置",
                bytes.saturating_sub(1),
            ))),
        }
    };
    let codec_file = file.clone();
    let codec = move |label, encoding| {
        let file = codec_file.clone();
        let current = move || page.with(|p| p.as_ref().is_some_and(|p| p.encoding == encoding));
        action(label, move || {
            let start = file.page.with(|p| p.as_ref().map_or(0, |p| p.data.start));
            file.load(owner, Position::Near(start), Some(encoding));
        })
        .selected(current)
        .selected_style(|s| {
            s.bg(|| color("selected-bg"))
                .text_color(|| color("selected-text"))
        })
    };
    let calibration = file.clone();
    let controls = view()
        .flex_row()
        .flex_wrap()
        .justify_between()
        .items_center()
        .gap(px(8.0))
        .child(when(
            move || page.with(Option::is_some),
            move || {
                let jump_submit = jump.clone();
                let jump_button = jump.clone();
                view()
                    .max_w(relative(1.0))
                    .flex_row()
                    .flex_wrap()
                    .gap(px(8.0))
                    .child(navigate("首页", false, |_, _| Position::Boundary(0)))
                    .child(navigate("上一页", false, |p, _| {
                        if p.start < 4 {
                            Position::Boundary(0)
                        } else {
                            Position::Before(p.start)
                        }
                    }))
                    .child(navigate("下一页", true, |p, _| {
                        Position::Boundary(p.end)
                    }))
                    .child(navigate("末页", true, |_, len| Position::Before(len)))
                    .child(
                        nsh_core::element::input()
                            .label("文件字节位置")
                            .h(px(metric("field")))
                            .w(px(145.0))
                            .max_w(relative(1.0))
                            .px(px(8.0))
                            .tabular_nums()
                            .rounded(px(6.0))
                            .border(border_width)
                            .border_color(|| color("chrome-edge"))
                            .bg(|| color("paper"))
                            .text_color(|| color("ink-1"))
                            .app_class(CONTROL_STATES)
                            .placeholder("字节位置")
                            .value(move || input.get())
                            .on_change(move |s: &str| input.set(s.into()))
                            .on_submit(move |_: &str| jump_submit()),
                    )
                    .child(action("转到", jump_button).accessibility_hint("转到填入的文件字节位置"))
            },
        ))
        .child(
            view()
                .max_w(relative(1.0))
                .flex_row()
                .flex_wrap()
                .gap(px(8.0))
                .accessible(true)
                .role(AccessibilityRole::Group)
                .label("分页编码")
                .child(codec("UTF-8", Encoding::Utf8))
                .child(codec("GB18030", Encoding::Gb18030WebEncodingRs0844))
                .child(codec("UTF-16 LE", Encoding::Utf16Le))
                .child(codec("UTF-16 BE", Encoding::Utf16Be)),
        );
    let calibration_actions = when(move || calibration.pending_seek.get().is_some(), {
        let file = file.clone();
        move || {
            let begin = file.clone();
            let cancel = file.clone();
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(8.0))
                .child(action("校准并定位", move || begin.calibrate(owner)))
                .child(action("取消定位", move || {
                    cancel
                        .generation
                        .set(cancel.generation.get().wrapping_add(1));
                    cancel.pending_seek.set(None);
                    cancel.reading.set(false);
                    cancel
                        .message
                        .set(InspectorMessage::info("已取消定位，保留当前页"));
                }))
        }
    });

    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .rounded(px(metric("paperRadius")))
        .overflow_hidden()
        .child(
            scroll()
                .w_full()
                .max_h(relative(0.60))
                .min_h(px(0.0))
                .child(
                    view().px(px(12.0)).py(px(8.0)).child(
                        view()
                            .accessible(true)
                            .role(AccessibilityRole::Group)
                            .label(inspector_label)
                            .rounded(px(metric("paperRadius")))
                            .border(border_width)
                            .border_color(|| color("chrome-edge"))
                            .bg(|| color("paper-subtle"))
                            .p(px(14.0))
                            .gap(px(8.0))
                            .child(heading)
                            .child(when(
                                move || import_state.get() == ImportState::Loading,
                                move || import_progress(progress, bytes),
                            ))
                            .child(page_details)
                            .child(controls)
                            .child(when(bytes > MAX_EDIT_BYTES, || {
                                text(byte_limit_message())
                                    .text_size(px(font_size("meta")))
                                    .text_color(|| color("error-fg"))
                            }))
                            .child(when(
                                move || import_state.get() == ImportState::LineLimit,
                                || {
                                    text(line_limit_message())
                                        .text_size(px(font_size("meta")))
                                        .text_color(|| color("error-fg"))
                                },
                            ))
                            .child(when(
                                move || message.with(|message| !message.text.is_empty()),
                                move || {
                                    text(move || message.with(|message| message.text.clone()))
                                        .text_size(px(font_size("meta")))
                                        .text_color(move || {
                                            color(if message.with(|message| message.is_error) {
                                                "error-fg"
                                            } else {
                                                "muted-readable"
                                            })
                                        })
                                },
                            ))
                            .child(calibration_actions),
                    ),
                ),
        )
        .child(dynamic(move || {
            let page = file.page.get()?;
            Some(
                nsh_base::editor::editor()
                    .element_ref(&body)
                    .component_ref(&reference)
                    .default_value(Some(SharedString::from(page.data.text.clone())))
                    .read_only(true)
                    .line_numbers(false)
                    .soft_wrap(false)
                    .on_focus(move |_: &()| focus.set(false))
                    .flex_1()
                    .w_full()
                    .min_h(px(0.0))
                    .min_w(px(0.0)),
            )
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn determinate_progress_describes_only_a_valid_read_for_this_source() {
        let reading = ImportProgress::PreparingSource(SourcePreparation::Reading(38));
        assert_eq!(reading.read_amount(100), Some(38));
        assert_eq!(reading.read_percent(100), Some(38));
        assert_eq!(reading.label(100), "正在读取 · 38 / 100 B · 38%");
        assert_eq!(reading.read_percent(38), Some(100));
        assert_eq!(reading.read_percent(37), None);
        assert_eq!(reading.read_percent(0), None);
        let decoding = ImportProgress::PreparingSource(SourcePreparation::Decoding);
        assert_eq!(decoding.read_percent(100), None);
        assert_eq!(decoding.label(100), "正在解码文件…");
    }
}
