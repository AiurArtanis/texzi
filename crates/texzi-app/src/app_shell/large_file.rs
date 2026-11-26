use super::*;
use std::collections::VecDeque;
use texzi_document::files::{
    Encoding,
    paged::{Calibration, FileSource, Page, PagedFile, Position},
};

const MIB: u64 = 1024 * 1024;
pub(super) const LARGE_BYTES: u64 = 20 * MIB;
const HUGE_BYTES: u64 = 200 * MIB;
// Editing owns decoded text, provenance, a Rope and a save-conflict baseline.
// Only one such import runs at a time. Files above this bound stay seekable.
const MAX_EDIT_BYTES: u64 = 256 * MIB;
pub(super) const SOURCE_ONLY_BYTES: u64 = 50_000_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImportState {
    Idle,
    Loading,
    Cancelling,
    Cancelled,
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

pub(super) struct LargeFile {
    source: Arc<FileSource>,
    choice: Cell<Option<Encoding>>,
    pending_seek: Signal<Option<Position>>,
    path: PathBuf,
    len: u64,
    reader: RefCell<Option<Arc<PagedFile>>>,
    page: Signal<Option<Rc<Page>>>,
    message: Signal<String>,
    reading: Signal<bool>,
    import_state: Signal<ImportState>,
    generation: Cell<u64>,
    cancel: RefCell<Option<Arc<AtomicBool>>>,
}

impl LargeFile {
    fn new(path: PathBuf, source: Arc<FileSource>) -> Rc<Self> {
        let len = source.len;
        Rc::new(Self {
            source,
            choice: Cell::new(None),
            pending_seek: signal(None),
            path,
            len,
            reader: Default::default(),
            page: signal(None),
            message: signal(if len >= HUGE_BYTES {
                "选择只读分页查看，或尝试载入完整编辑模型。".into()
            } else {
                "正在准备首屏…".into()
            }),
            reading: signal(false),
            import_state: signal(ImportState::Idle),
            generation: Cell::new(0),
            cancel: Default::default(),
        })
    }

    pub(super) fn status(&self) -> String {
        match self.import_state.get() {
            ImportState::Loading => "正在加载完整文档 · 预览只读".into(),
            ImportState::Cancelling => "正在取消完整加载 · 预览只读".into(),
            ImportState::Cancelled => "已取消完整加载 · 预览只读".into(),
            ImportState::Idle => format!(
                "大文件 · 只读分页 · {:.1} MiB",
                self.len as f64 / MIB as f64
            ),
        }
    }

    fn cancel_import(&self) {
        if let Some(cancel) = self.cancel.borrow_mut().take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.import_state.set(ImportState::Idle);
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
            self.message.set("已取消完整加载，可继续分页查看".into());
        }
    }

    pub(super) fn close(&self) {
        self.cancel_import();
        self.generation.set(self.generation.get().wrapping_add(1));
        self.reader.borrow_mut().take();
        self.page.set(None);
        self.reading.set(false);
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
        self.reading.set(true);
        owner.run(|| {
            spawn(async move {
                let mut from = reader.scan_start(pos);
                loop {
                    let reader = reader.clone();
                    let step = blocking(move || reader.calibrate_step(pos, from)).await;
                    if view.generation.get() != generation {
                        return;
                    }
                    match step {
                        Ok(Ok(Calibration::More(at))) => {
                            from = at;
                            view.message
                                .set(format!("正在校准 GB18030 字符边界 · 已到字节 {at}"));
                            // Yield between bounded reads, with a visible and cancellable operation.
                            let _ = next_frame().await;
                            if view.generation.get() != generation {
                                return;
                            }
                        }
                        Ok(Ok(Calibration::Ready(page))) => {
                            view.message.set(format!(
                                "GB18030 · 字节 {}–{} / {}",
                                page.start, page.end, view.len
                            ));
                            view.page.set(Some(Rc::new(page)));
                            view.pending_seek.set(None);
                            break;
                        }
                        Ok(Err(e)) => {
                            view.message.set(format!("校准失败：{e}"));
                            break;
                        }
                        Err(e) => {
                            view.message.set(format!("校准未完成：{e}"));
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
        let source = self.source.clone();
        self.pending_seek.set(None);
        if encoding.is_some() {
            self.cancel_import();
            self.choice.set(encoding);
        }
        let choice = self.choice.get();
        let generation = self.generation.get();
        self.reading.set(true);
        self.message.set("正在读取请求的页面…".into());
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
                view.reading.set(false);
                match result {
                    Ok(Ok((reader, Err(error)))) => {
                        if error.kind() == std::io::ErrorKind::WouldBlock {
                            view.pending_seek.set(Some(pos));
                        }
                        *view.reader.borrow_mut() = Some(reader);
                        view.message.set(error.to_string());
                    }
                    Ok(Ok((reader, Ok(page)))) => {
                        let label = match reader.encoding {
                            Encoding::Utf8 => "UTF-8",
                            Encoding::Utf16Le => "UTF-16 LE",
                            Encoding::Utf16Be => "UTF-16 BE",
                            _ => "GB18030",
                        };
                        view.message.set(format!(
                            "{label} · 字节 {}–{} / {}",
                            page.start, page.end, reader.len
                        ));
                        *view.reader.borrow_mut() = Some(reader);
                        view.page.set(Some(Rc::new(page)));
                    }
                    Ok(Err(e)) => view.message.set(format!("读取失败：{e}")),
                    Err(e) => view.message.set(format!("读取任务未完成：{e}")),
                }
            })
        });
    }
}

struct Import {
    doc: Rc<Document>,
    large: Rc<LargeFile>,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub(super) struct ImportQueue {
    jobs: RefCell<VecDeque<Import>>,
    active: RefCell<Option<Arc<AtomicBool>>>,
}

impl Shell {
    pub(super) fn open_large(self: &Rc<Self>, path: PathBuf, source: Arc<FileSource>) {
        let size = source.len;
        self.remember(&path);
        let id = self.next.get();
        self.next.set(id + 1);
        let (doc, large) = self.owner.run(|| {
            let large = LargeFile::new(path.clone(), source);
            let doc = self.make_document(
                id,
                NativeView::new_file(),
                Some(path),
                Some(large.clone()),
                false,
            );
            (doc, large)
        });
        self.show_new(doc.clone());
        if size < HUGE_BYTES {
            large.load(self.owner, Position::Boundary(0), None);
            self.queue_import(doc, large);
        }
    }

    fn queue_import(self: &Rc<Self>, doc: Rc<Document>, large: Rc<LargeFile>) {
        if large.len > MAX_EDIT_BYTES {
            large.message.set(format!(
                "文件超过 {} MiB 完整编辑预算，可只读分页查看。",
                MAX_EDIT_BYTES / MIB
            ));
            return;
        }
        if matches!(
            large.import_state.get(),
            ImportState::Loading | ImportState::Cancelling
        ) {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *large.cancel.borrow_mut() = Some(cancel.clone());
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
                shell
                    .imports
                    .jobs
                    .borrow_mut()
                    .push_back(Import { doc, large, cancel });
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
            file.message.set("正在取消完整加载…".into());
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
                let result = blocking(move || {
                    source.verify_path(&path).map_err(|e| e.to_string())?;
                    crate::editor_owner::PreparedFile::from_source(
                        &source,
                        MAX_EDIT_BYTES,
                        &cancel,
                        encoding,
                    )
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
                        .map_err(|e| e.to_string())
                        .and_then(|r| r)
                        .and_then(|r| {
                            job.large
                                .source
                                .verify_path(&job.large.path)
                                .map(|_| r)
                                .map_err(|e| e.to_string())
                        })
                        .and_then(|data| shell.owner.run(|| data.attach()))
                    {
                        Ok(source) => {
                            let path = job.large.path.clone();
                            let markdown = is_markdown(&path);
                            let doc =
                                shell.make_document(job.doc.id, source, Some(path), None, false);
                            job.large.close();
                            shell.docs.update(|docs| {
                                if let Some(slot) =
                                    docs.iter_mut().find(|d| Rc::ptr_eq(d, &job.doc))
                                {
                                    *slot = doc.clone();
                                }
                            });
                            if markdown && job.large.len <= SOURCE_ONLY_BYTES {
                                shell.request_mode(doc.clone(), true);
                            }
                            if shell
                                .current()
                                .is_some_and(|current| Rc::ptr_eq(&current, &doc))
                            {
                                shell.notice.set(if job.large.len > SOURCE_ONLY_BYTES {
                                    "大文件已用源码打开 · 已关闭语法高亮".into()
                                } else {
                                    "大文件加载完成 · 可编辑".into()
                                });
                            }
                        }
                        Err(error) => job.large.message.set(format!("完整编辑加载失败：{error}")),
                    }
                }
                shell.imports.active.borrow_mut().take();
                shell.next_import();
            })
        });
    }
}

fn action(label: &'static str, click: impl Fn() + 'static) -> impl IntoElement {
    nsh_base::button()
        .child(text(label))
        .on_press(move |_: &()| click())
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
    let bytes = file.len;
    let open = file.clone();
    let import = file.clone();
    let start = shell.clone();
    let cancel = file.clone();
    let cancel_shell = shell.clone();
    let calibration = file.clone();
    let close = shell.clone();
    let show_bytes = file.clone();
    let navigator = file.clone();
    let navigate = move |label, get: fn(&Page, u64) -> Position| {
        let file = navigator.clone();
        action(label, move || {
            if let Some(page) = file.page.get() {
                file.load(owner, get(&page, bytes), None);
            }
        })
    };
    let jump = {
        let file = file.clone();
        move || match input.get().trim().parse::<u64>() {
            Ok(at) if at < bytes => file.load(owner, Position::Near(at), None),
            _ => file.message.set(format!(
                "请输入 0 至 {} 之间的字节位置",
                bytes.saturating_sub(1)
            )),
        }
    };
    let codec = |label, encoding| {
        let file = file.clone();
        action(label, move || {
            let start = file.page.with(|p| p.as_ref().map_or(0, |p| p.start));
            file.load(owner, Position::Near(start), Some(encoding));
        })
    };
    view()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .overflow_hidden()
        .p(px(8.))
        .gap(px(4.))
        .child(
            text(format!(
                "{} · {:.1} MiB",
                doc.name(),
                bytes as f64 / MIB as f64
            ))
            .number_of_lines(1),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(4.))
                .child(action("只读分页查看", move || {
                    open.load(owner, Position::Boundary(0), None)
                }))
                .child(action("尝试完整编辑", move || {
                    start.queue_import(doc.clone(), import.clone())
                }))
                .child(when(
                    move || import_state.get() == ImportState::Loading,
                    move || {
                        action("取消完整加载", {
                            let cancel = cancel.clone();
                            let shell = cancel_shell.clone();
                            move || shell.cancel_full_import(&cancel)
                        })
                    },
                ))
                .child(action("关闭文件", move || close.close_tab(id))),
        )
        .child(text(move || message.get()).number_of_lines(2))
        .child(when(move || calibration.pending_seek.get().is_some(), {
            let file = file.clone();
            move || {
                let begin = file.clone();
                let cancel = file.clone();
                view()
                    .flex_row()
                    .child(action("校准并定位", move || begin.calibrate(owner)))
                    .child(action("取消定位", move || {
                        cancel
                            .generation
                            .set(cancel.generation.get().wrapping_add(1));
                        cancel.pending_seek.set(None);
                        cancel.reading.set(false);
                        cancel.message.set("已取消定位，保留当前页".into());
                    }))
            }
        }))
        .child(when(
            move || page.with(Option::is_some),
            move || {
                let jump_submit = jump.clone();
                let jump_button = jump.clone();
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(4.))
                    .child(navigate("首页", |_, _| Position::Boundary(0)))
                    .child(navigate("上一页", |p, _| {
                        if p.start < 4 {
                            Position::Boundary(0)
                        } else {
                            Position::Before(p.start)
                        }
                    }))
                    .child(navigate("下一页", |p, len| {
                        if p.end < len {
                            Position::Boundary(p.end)
                        } else {
                            Position::Boundary(p.start)
                        }
                    }))
                    .child(navigate("末页", |_, len| Position::Before(len)))
                    .child(
                        nsh_core::element::input()
                            .label("文件字节位置")
                            .w(px(145.))
                            .placeholder("字节位置")
                            .value(move || input.get())
                            .on_change(move |s: &str| input.set(s.into()))
                            .on_submit(move |_: &str| jump_submit()),
                    )
                    .child(action("转到", jump_button))
            },
        ))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(4.))
                .child(codec("UTF-8", Encoding::Utf8))
                .child(codec("GB18030", Encoding::Gb18030WebEncodingRs0844))
                .child(codec("UTF-16 LE", Encoding::Utf16Le))
                .child(codec("UTF-16 BE", Encoding::Utf16Be)),
        )
        .child(dynamic(move || {
            let page = show_bytes.page.get()?;
            Some(
                nsh_base::editor::editor()
                    .element_ref(&body)
                    .component_ref(&reference)
                    .default_value(Some(SharedString::from(page.text.clone())))
                    .read_only(true)
                    .line_numbers(false)
                    .soft_wrap(false)
                    .on_focus(move |_: &()| focus.set(false))
                    .flex_1()
                    .w_full()
                    .min_h(px(0.))
                    .min_w(px(0.)),
            )
        }))
}
