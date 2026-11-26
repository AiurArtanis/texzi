//! Application shell. Native menus and the palette share command identities.
mod large_file;
use crate::editor_owner::SaveError;
use crate::native_menu::{MenuGroup, SHELL_SHORTCUTS};
use crate::{editor_owner::NativeView, rich_owner::ModeGate};
use large_file::{LARGE_BYTES, LargeFile};
use nsh_base::{
    editor::{Editor, EditorApi, EditorCommand, ExternalDocument},
    rich_editor::view::RichEditorApi,
};
use nsh_core::{
    AccessibilityRole, EllipsizeMode, PointerEvents,
    desktop::{WindowAttributes, WindowMode},
    element::{
        Composition, ImageSrc,
        desktop::{MenuItem, WindowRef, current_window, popup_menu_at},
        image, input, scroll, svg,
    },
    file_dialog::{OpenDialog, SaveDialog},
    geometry::point,
    platform::Appearance,
    prelude::*,
};
use std::os::windows::fs::MetadataExt;
use std::{
    cell::{Cell, RefCell},
    io::Read,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(super) const TITLE_BAR_HEIGHT: f32 = 44.0;
const TITLE_ICON_WIDTH: f32 = 44.0;
const TITLE_MENU_WIDTH: f32 = 50.0;
const TITLE_SEARCH_GAP: f32 = 14.0;

#[derive(Clone, PartialEq)]
struct SearchHit {
    path: PathBuf,
    line: usize,
    text: String,
}
struct SearchReport {
    hits: Vec<SearchHit>,
    scanned: usize,
    skipped: usize,
    limited: bool,
    cancelled: bool,
    errors: Vec<String>,
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
}

fn display_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned()
    }
}

fn editor_tab_width(markdown: bool) -> u32 {
    if markdown { 2 } else { 4 }
}

struct Document {
    id: u64,
    path: Signal<Option<PathBuf>>,
    source: Rc<NativeView>,
    gate: Rc<ModeGate>,
    large: Option<Rc<LargeFile>>,
    reference: ComponentRef<Editor>,
    rich_reference: ComponentRef<nsh_base::rich_editor::view::RichEditor>,
    controls: RichControlSet,
    wrap: Signal<bool>,
    pending_mode: Signal<Option<bool>>,
    progress: ElementRef,
    body: ElementRef,
}
impl Document {
    fn displayed_rich(&self) -> bool {
        self.gate.rich_mode.get()
    }

    fn is_markdown(&self) -> bool {
        self.path
            .with(|path| path.as_deref().is_some_and(is_markdown))
    }

    fn viewport_caret_top(&self) -> Option<f32> {
        if self.gate.rich_mode.get() {
            self.rich_reference
                .api::<RichEditorApi>()?
                .viewport_caret_top()
        } else {
            self.reference.api::<EditorApi>()?.viewport_caret_top()
        }
    }

    fn trace_mode_view(&self, phase: &str) {
        let rich = self.gate.rich_mode.get();
        let (position, scroll) = if rich {
            (
                self.gate.rich_cursor_status(),
                self.rich_reference
                    .api::<RichEditorApi>()
                    .map(|api| api.scroll_top()),
            )
        } else {
            (
                self.source.cursor_status(),
                self.reference
                    .api::<EditorApi>()
                    .map(|api| api.scroll_offset().y.0),
            )
        };
        log::info!(
            "TEXZI_MODE_VIEW phase={phase} rich={rich} position={position:?} scroll={scroll:?} caret_top={:?}",
            self.viewport_caret_top(),
        );
    }

    fn name(&self) -> String {
        self.path.with(|path| {
            path.as_deref()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("未命名-{}", self.id))
        })
    }
}

async fn wait_for_editor_frame(reference: ElementRef) {
    // The first task runs after this turn's mount, allowing the ref to point
    // to the new element before observing native presentations.
    if run_on_main_thread(|| ()).await.is_err() {
        return;
    }
    let Some(node) = reference.node() else { return };
    let Ok(Some((baseline, _))) =
        run_on_main_thread(move || nsh_platform_windows::node_presentation(node, None)).await
    else {
        return;
    };
    while reference.node() == Some(node) {
        let _ = next_frame().await;
        if !matches!(
            run_on_main_thread(move || nsh_platform_windows::node_presentation(
                node,
                Some(baseline)
            ))
            .await,
            Ok(Some((_, false)))
        ) {
            break;
        }
    }
}
// Toolbar and command-palette entries share the same per-view drafts and
// confirmation methods. Neither surface owns a second editable document.
struct RichControlSet {
    blocks: Rc<crate::block_controls::BlockControls>,
    resources: Rc<crate::rich_controls::ResourceControls>,
    code: Rc<crate::code_controls::CodeControls>,
    table: Rc<crate::table_controls::TableControls>,
}

#[derive(Clone, Copy)]
enum FormatAction {
    Mark(&'static str),
    Paragraph,
    Link,
    Image,
    Table,
    CodeLanguage,
    ExitCode,
    Indent(bool),
}

const FORMAT_ACTIONS: &[(&str, FormatAction)] = &[
    ("粗体", FormatAction::Mark("texzi.markdown.bold")),
    ("斜体", FormatAction::Mark("texzi.markdown.italic")),
    ("删除线", FormatAction::Mark("texzi.markdown.strikethrough")),
    ("行内代码", FormatAction::Mark("texzi.markdown.inlineCode")),
    ("段落样式", FormatAction::Paragraph),
    ("插入或编辑链接", FormatAction::Link),
    ("图片属性", FormatAction::Image),
    ("表格操作", FormatAction::Table),
    ("代码语言", FormatAction::CodeLanguage),
    ("退出代码块", FormatAction::ExitCode),
    ("缩进", FormatAction::Indent(false)),
    ("反缩进", FormatAction::Indent(true)),
];
impl RichControlSet {
    fn new(gate: &Rc<ModeGate>, source: &Rc<NativeView>) -> Self {
        Self {
            blocks: crate::block_controls::BlockControls::new(gate.clone(), source.clone()),
            resources: crate::rich_controls::ResourceControls::new(gate.clone(), source.clone()),
            code: crate::code_controls::CodeControls::new(gate.clone(), source.clone()),
            table: crate::table_controls::TableControls::new(gate.clone(), source.clone()),
        }
    }
    fn open(&self) -> bool {
        self.blocks.open.get()
            || self.resources.open.get()
            || self.code.open.get()
            || self.table.is_open()
    }
    fn cancel(&self) {
        self.blocks.cancel();
        self.resources.cancel();
        self.code.cancel();
        self.table.cancel();
    }
    fn begin(&self, request: crate::rich_commands::FormRequest) -> Result<(), String> {
        self.cancel();
        match request {
            crate::rich_commands::FormRequest::Link => self
                .resources
                .begin(crate::rich_projection::ResourceKind::Link),
            crate::rich_commands::FormRequest::CodeLanguage => self.code.begin(),
            crate::rich_commands::FormRequest::TableAlignment
            | crate::rich_commands::FormRequest::Table => self.table.begin(),
        }
    }
}
#[cfg(test)]
#[path = "save_overlap_tests.rs"]
mod save_overlap_tests;
#[cfg(test)]
#[path = "shell_markdown_tests.rs"]
mod shell_markdown_tests;
#[cfg(test)]
#[path = "shell_task_tests.rs"]
mod shell_task_tests;
#[cfg(test)]
#[path = "shortcut_regression_tests.rs"]
mod shortcut_regression_tests;

// Keep application shortcuts in the engine keymap as well as the native menu.
// A consumed command stops the Windows menu-shortcut fallback, so this does
// not dispatch a second Open request. The later chord bindings keep priority.
fn bind_shell_shortcuts(run: Rc<dyn Fn(&str)>) {
    for &(key, id) in SHELL_SHORTCUTS {
        let run = run.clone();
        on_key_with(
            key,
            KeyOptions {
                when: nsh_core::keymap::When::Always,
                ..Default::default()
            },
            move |_| run(id),
        );
    }
}

// Command rows and other transient views may dispose their own owner before
// their listener returns. A modal's creation and completion belong to Shell,
// so the dialog answer must not inherit that listener's cancellation scope.
fn await_shell_modal<F: std::future::Future + 'static>(
    owner: &Owner,
    modal: Rc<Cell<bool>>,
    show: impl FnOnce() -> F,
    done: impl FnOnce(F::Output) + 'static,
) {
    owner.run(|| {
        modal.set(true);
        let answer = show();
        spawn(async move {
            let result = answer.await;
            modal.set(false);
            done(result);
        });
    });
}

impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && Rc::ptr_eq(&self.source, &other.source)
    }
}
struct Shell {
    owner: Owner,
    window: WindowRef,
    docs: Signal<Vec<Rc<Document>>>,
    active: Signal<u64>,
    next: Cell<u64>,
    closed: RefCell<Vec<ClosedTab>>,
    imports: large_file::ImportQueue,
    root: Signal<Option<PathBuf>>,
    sidebar: Signal<bool>,
    sidebar_width: Signal<f32>,
    sidebar_drag_start: Signal<f32>,
    notice: Signal<String>,
    panel: Signal<String>,
    query: Signal<String>,
    palette_query: Signal<String>,
    replacement: Signal<String>,
    zen: Signal<bool>,
    zen_exit: Rc<crate::zen_state::ZenState>,
    chord: Signal<bool>,
    chord_generation: Cell<u64>,
    source_ref: Cell<Option<ComponentRef<Editor>>>,
    modal: Rc<Cell<bool>>,
    split: Signal<Option<Rc<Document>>>,
    split_down: Signal<bool>,
    second_active: Signal<bool>,
    expanded: Signal<Vec<PathBuf>>,
    hits: Signal<Vec<SearchHit>>,
    search_notice: Signal<String>,
    searching: Signal<bool>,
    search_cancel: RefCell<Option<Arc<AtomicBool>>>,
    recent: Signal<Vec<PathBuf>>,
    quick_files: Signal<Vec<crate::quick_open::Candidate>>,
    quick_origin: Signal<Option<PathBuf>>,
    quick_notice: Signal<String>,
    quick_scanning: Signal<bool>,
    quick_cancel: RefCell<Option<Arc<AtomicBool>>>,
}
enum ClosedTab {
    Document(Rc<Document>),
    File(PathBuf),
}
pub(crate) fn foreground() -> Hsla {
    if dark() {
        rgb(0xe5e9f0).into()
    } else {
        rgb(0x20242b).into()
    }
}
pub(crate) fn background() -> Hsla {
    if dark() {
        rgb(0x191d23).into()
    } else {
        white()
    }
}
fn frame() -> Hsla {
    if dark() {
        rgb(0x20242b).into()
    } else {
        rgb(0xf3f4f6).into()
    }
}

fn secondary_foreground() -> Hsla {
    rgb(if dark() { 0xa7b0bd } else { 0x616975 }).into()
}

fn divider() -> Hsla {
    rgb(if dark() { 0x434b57 } else { 0xc9ced6 }).into()
}

fn indicator() -> Hsla {
    rgb(if dark() { 0x79aaef } else { 0x3267b0 }).into()
}

fn selected_surface() -> Hsla {
    rgb(if dark() { 0x303e54 } else { 0xdce9fc }).into()
}
impl Shell {
    fn primary(&self) -> Option<Rc<Document>> {
        self.docs
            .with(|ds| ds.iter().find(|d| d.id == self.active.get()).cloned())
    }
    fn current(&self) -> Option<Rc<Document>> {
        if self.second_active.get() {
            self.split.get().or_else(|| self.primary())
        } else {
            self.primary()
        }
    }
    fn new() -> Rc<Self> {
        let s = Rc::new(Self {
            owner: Owner::current(),
            window: current_window(),
            docs: signal(vec![]),
            active: signal(0),
            next: Cell::new(1),
            closed: RefCell::new(vec![]),
            imports: Default::default(),
            root: signal(None),
            sidebar: signal(true),
            sidebar_width: signal(232.0),
            sidebar_drag_start: signal(232.0),
            notice: signal(String::new()),
            panel: signal(String::new()),
            query: signal(String::new()),
            palette_query: signal(String::new()),
            replacement: signal(String::new()),
            zen: signal(false),
            zen_exit: crate::zen_state::ZenState::new(),
            chord: signal(false),
            chord_generation: Cell::new(0),
            source_ref: Cell::new(None),
            modal: Rc::new(Cell::new(false)),
            split: signal(None),
            split_down: signal(false),
            second_active: signal(false),
            expanded: signal(vec![]),
            hits: signal(vec![]),
            search_notice: signal(String::new()),
            searching: signal(false),
            search_cancel: RefCell::new(None),
            recent: signal(vec![]),
            quick_files: signal(vec![]),
            quick_origin: signal(None),
            quick_notice: signal(String::new()),
            quick_scanning: signal(false),
            quick_cancel: RefCell::new(None),
        });
        // Notify observers must precede actual Shell bindings: NSH chooses
        // one newest match, so later registration could shadow a real command.
        let reset = s.zen_exit.clone();
        crate::zen_state::bind_reset_on_other_keys(reset.armed, Rc::new(move || reset.reset()));
        let shell = s.clone();
        effect(move || {
            if !shell.window.state().is_some_and(|state| state.focused)
                || shell.zen.get() && shell.current().is_some_and(|doc| doc.gate.busy())
            {
                untracked(|| shell.zen_exit.reset());
            }
        });
        let quick = s.clone();
        effect(move || {
            let root = quick.root.get();
            if quick.panel.get() == "files" {
                quick.start_quick(root);
            } else {
                quick.cancel_quick(false);
            }
        });
        s.add_blank();
        let mut attributes = WindowAttributes::default();
        attributes.close_on_press = Some(false);
        s.window.set(attributes);
        let close = s.clone();
        s.window.on_close_press(move |_| close.close_window());
        let menus = s.clone();
        effect(move || {
            let _ = menus.active.get();
            let _ = menus.docs.get();
            let run = menus.clone();
            crate::native_menu::install(
                Rc::new(move |id| run.run(id)),
                |id| menus.blocked(id),
                menus.chord.get(),
            );
        });
        bind_chords(s.clone());
        let shortcuts = s.clone();
        bind_shell_shortcuts(Rc::new(move |id| shortcuts.run(id)));
        let menus = s.clone();
        on_key(crate::native_menu::MENU_KEY, move |_| {
            menus.show_title_menu(None)
        });
        for group in MenuGroup::ALL {
            let menus = s.clone();
            on_key_with(
                group.shortcut(),
                KeyOptions {
                    when: nsh_core::keymap::When::Always,
                    ..Default::default()
                },
                move |_| menus.show_title_menu(Some(group)),
            );
        }
        let source_sidebar = s.clone();
        effect(move || {
            if source_sidebar
                .current()
                .is_some_and(|d| !d.gate.rich_mode.get())
            {
                let s = source_sidebar.clone();
                on_key("mod+b", move |_| s.run("texzi.sidebar.toggle"));
            }
        });
        for index in 1..=8u8 {
            let s = s.clone();
            on_key(&format!("mod+{index}"), move |_| {
                if !s.modal.get() && !s.current().is_some_and(|d| d.gate.busy()) {
                    s.focus_group(index);
                }
            });
        }
        let escape = s.clone();
        on_key("escape", move |_| escape.handle_escape());
        let source_escape = {
            let escape = s.clone();
            computed(move || escape.zen_source_escape())
        };
        let escape = s.clone();
        effect(move || {
            if !source_escape.get() || escape.escape_blocked() {
                return;
            }
            let escape = escape.clone();
            // Windows treats Escape as a text key. Let Zen observe it before
            // the focused input's native Escape handling can blur the field.
            on_key_with(
                "escape",
                KeyOptions {
                    when: nsh_core::keymap::When::Always,
                    pass_through: true,
                    ..Default::default()
                },
                move |_| {
                    if source_escape.get() {
                        escape.handle_escape();
                    }
                },
            );
        });
        s
    }
    fn add_blank(&self) {
        self.add(self.owner.run(NativeView::new_file), None);
    }
    fn title_room(&self) -> f32 {
        let width = self
            .window
            .state()
            .map_or(880.0, |state| state.content_bounds.size.width.0);
        width - self.window.title_bar().right.0
    }
    fn compact_menu(&self) -> bool {
        self.title_room()
            < TITLE_ICON_WIDTH
                + TITLE_MENU_WIDTH * MenuGroup::ALL.len() as f32
                + TITLE_SEARCH_GAP
                + 64.0
    }
    fn title_search_left(&self) -> f32 {
        let count = if self.compact_menu() {
            1
        } else {
            MenuGroup::ALL.len()
        };
        TITLE_ICON_WIDTH + TITLE_MENU_WIDTH * count as f32 + TITLE_SEARCH_GAP
    }
    fn title_search_width(&self) -> Points {
        px((self.title_room() - self.title_search_left() - 18.0).clamp(0.0, 460.0))
    }
    fn show_title_menu(self: &Rc<Self>, group: Option<MenuGroup>) {
        if self.modal.get() || self.current().is_some_and(|doc| doc.gate.busy()) {
            return;
        }
        let run = self.clone();
        let run: Rc<dyn Fn(&str)> = Rc::new(move |id| run.run(id));
        let blocked = |id: &str| self.blocked(id);
        let pending = self.chord.get();
        let entries = match group {
            Some(group) => crate::native_menu::entries(group.commands(), &run, &blocked, pending),
            None => crate::native_menu::groups(&run, &blocked, pending),
        };
        let at = point(
            px(TITLE_ICON_WIDTH
                + TITLE_MENU_WIDTH
                    * if self.compact_menu() {
                        0.0
                    } else {
                        group.map_or(0.0, MenuGroup::position)
                    }),
            px(TITLE_BAR_HEIGHT),
        );
        self.owner.run(|| {
            let _ = popup_menu_at(entries, at);
        });
    }
    fn add(&self, source: Rc<NativeView>, path: Option<PathBuf>) {
        let id = self.next.get();
        self.next.set(id + 1);
        let doc = self.make_document(id, source, path, None, true);
        self.show_new(doc);
    }
    fn make_document(
        &self,
        id: u64,
        source: Rc<NativeView>,
        path: Option<PathBuf>,
        large: Option<Rc<LargeFile>>,
        start_rich: bool,
    ) -> Rc<Document> {
        let rich = self.owner.run(|| source.sibling().expect("rich view"));
        let gate = self.owner.run(|| ModeGate::new(source.clone(), rich));
        if start_rich && path.as_deref().is_some_and(is_markdown) {
            self.owner.run(|| gate.to_rich());
        }
        let reference = self.owner.run(component_ref::<Editor>);
        let rich_reference = self
            .owner
            .run(component_ref::<nsh_base::rich_editor::view::RichEditor>);
        let controls = self.owner.run(|| RichControlSet::new(&gate, &source));
        let wrap = self.owner.run(|| signal(false));
        let path = self.owner.run(|| signal(path));
        Rc::new(Document {
            id,
            path,
            source,
            gate,
            large,
            reference,
            rich_reference,
            controls,
            wrap,
            pending_mode: self.owner.run(|| signal(None)),
            progress: self.owner.run(element_ref),
            body: self.owner.run(element_ref),
        })
    }
    fn show_new(&self, doc: Rc<Document>) {
        let id = doc.id;
        self.docs.update(|ds| ds.push(doc));
        self.active.set(id);
        self.second_active.set(false);
        self.panel.set(String::new());
        self.notice.set(String::new());
    }
    fn blocked(&self, id: &str) -> Option<String> {
        if self.current().is_some_and(|doc| doc.large.is_some())
            && large_file::requires_editable(id)
        {
            return Some("此标签尚未载入完整编辑模型，分页内容仅供查看和复制。".into());
        }
        if self.current().is_some_and(|d| d.gate.busy()) {
            return Some("请先完成或取消中文组合".into());
        }
        if matches!(id, "texzi.group.splitRight" | "texzi.group.splitDown")
            && self.split.get().is_some()
        {
            return Some(
                "当前实现只有两个编辑组，继续新建组尚未完成；本次未新增组，也不改变原分屏方向。"
                    .into(),
            );
        }
        if crate::rich_commands::handles(id) {
            let doc = match self.current() {
                Some(doc) => doc,
                None => return Some("没有活动文档".into()),
            };
            return crate::rich_commands::availability(&doc.gate, &doc.source, id, true).err();
        }
        (!supported(id)).then(|| "此命令尚未完成接线，当前文档保持原样。".into())
    }
    fn explain(self: &Rc<Self>, title: &str, message: &str) {
        self.notice.set(message.into());
        // A palette selection unmounts its own owner. Both the native dialog
        // request and its answer task must outlive that transient command row;
        // otherwise its cleanup cancels the task and leaves `modal` set forever.
        let s = self.clone();
        await_shell_modal(
            &self.owner,
            self.modal.clone(),
            || {
                if std::env::var_os("TEXZI_TRACE_IME").as_deref() == Some(std::ffi::OsStr::new("1"))
                {
                    log::info!("TX_SHELL_MODAL action=open");
                }
                nsh_base::alert_dialog()
                    .title(title.to_owned())
                    .description(message.to_owned())
                    .action("ok", "知道了")
                    .close_on_escape(true)
                    .show()
            },
            move |result| {
                if std::env::var_os("TEXZI_TRACE_IME").as_deref() == Some(std::ffi::OsStr::new("1"))
                {
                    log::info!(
                        "TX_SHELL_MODAL action=answer ok={} modal=false",
                        result.is_ok()
                    );
                }
                s.restore_editor_focus();
            },
        );
    }
    fn zen_stamp(&self) -> Option<crate::zen_state::Stamp> {
        let doc = self.current()?;
        let snapshot = doc.source.snapshot().ok()?;
        let rich_mode = doc.gate.rich_mode.get();
        Some(crate::zen_state::Stamp {
            document: doc.id,
            group: if self.second_active.get() { 2 } else { 1 },
            rich_mode,
            revision: snapshot.revision,
            view_epoch: snapshot.view_epoch,
            rich_state_revision: rich_mode.then(|| doc.gate.session().state().revision()),
        })
    }
    fn remember(&self, path: &Path) {
        self.recent.update(|paths| {
            paths.retain(|old| !old.as_os_str().eq_ignore_ascii_case(path.as_os_str()));
            paths.insert(0, path.to_path_buf());
            paths.truncate(50);
        });
    }
    fn cancel_quick(&self, explain: bool) {
        if let Some(cancel) = self.quick_cancel.borrow_mut().take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.quick_scanning.set(false);
        if explain {
            self.quick_notice
                .set("已取消目录扫描；已打开和最近文件仍可选择".into());
        }
    }
    fn start_quick(self: &Rc<Self>, root: Option<PathBuf>) {
        self.cancel_quick(false);
        self.quick_files.set(vec![]);
        self.quick_origin.set(root.clone());
        let Some(root) = root else {
            self.quick_notice
                .set("未打开文件夹；可搜索已打开和本次会话最近文件".into());
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        *self.quick_cancel.borrow_mut() = Some(cancel.clone());
        self.quick_scanning.set(true);
        self.quick_notice
            .set("正在列出当前文件夹的文件；可以先搜索已打开和最近文件".into());
        let worker_root = root.clone();
        let worker_cancel = cancel.clone();
        let work = blocking(move || {
            crate::quick_open::scan(
                worker_root,
                crate::quick_open::ScanLimits::default(),
                worker_cancel,
            )
        });
        let shell = self.clone();
        spawn(async move {
            let result = work.await;
            let current = shell
                .quick_cancel
                .borrow()
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, &cancel));
            if !current || shell.root.get() != Some(root) || shell.panel.get() != "files" {
                return;
            }
            shell.quick_scanning.set(false);
            match result {
                Ok(report) => {
                    shell.quick_notice.set(format!(
                        "{}：{} 个文件候选，检查 {} 项，跳过 {} 项{}。单次最多 2000 项；不跟随链接。{}{}",
                        if report.cancelled { "扫描已取消" } else { "扫描结束" },
                        report.files.len(), report.scanned, report.skipped,
                        if report.limited { "；结果不完整，已达扫描预算" } else { "" },
                        report.errors.join("\n"),
                        if report.errors_omitted > 0 { format!("；另有 {} 个错误未展开", report.errors_omitted) } else { String::new() },
                    ));
                    shell.quick_files.set(report.files);
                }
                Err(error) => shell.quick_notice.set(format!("目录候选扫描失败：{error}")),
            }
        });
    }
    fn open(self: &Rc<Self>, path: &Path) {
        let path = match path.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                self.explain("无法打开文件", &e.to_string());
                return;
            }
        };
        if let Err(error) = self.open_canonical_file(path) {
            self.explain("读取失败", &error);
        }
    }

    fn open_canonical_file(self: &Rc<Self>, path: PathBuf) -> Result<(), String> {
        if let Some(doc) = self.docs.with(|ds| {
            ds.iter()
                .find(|d| {
                    d.path
                        .get()
                        .as_ref()
                        .is_some_and(|p| p.as_os_str().eq_ignore_ascii_case(path.as_os_str()))
                })
                .cloned()
        }) {
            self.remember(&path);
            self.active.set(doc.id);
            self.second_active.set(false);
            self.restore_editor_focus();
            return Ok(());
        }
        let file =
            texzi_document::files::paged::FileSource::open(&path).map_err(|e| e.to_string())?;
        if file.len >= LARGE_BYTES {
            self.open_large(path, Arc::new(file));
            return Ok(());
        }
        let prepared = crate::editor_owner::PreparedFile::from_source(
            &file,
            LARGE_BYTES - 1,
            &AtomicBool::new(false),
            None,
        )?;
        let source = self.owner.run(|| prepared.attach())?;
        self.remember(&path);
        self.add(source, Some(path));
        Ok(())
    }
    fn open_folder(self: &Rc<Self>, path: &Path) {
        match path.canonicalize() {
            Ok(path) => self.show_folder(path),
            Err(error) => self.explain("无法打开文件夹", &error.to_string()),
        }
    }

    fn show_folder(&self, path: PathBuf) {
        self.root.set(Some(path));
        self.sidebar.set(true);
    }

    fn open_dropped(self: &Rc<Self>, uris: &[String]) {
        let mut folders = Vec::new();
        let mut failures = Vec::new();
        for uri in uris {
            let path = url::Url::parse(uri)
                .ok()
                .filter(|url| {
                    url.scheme() == "file"
                        && url
                            .host_str()
                            .is_none_or(|host| host.eq_ignore_ascii_case("localhost"))
                })
                .and_then(|url| url.to_file_path().ok());
            let Some(path) = path else {
                failures.push(format!("{uri}：仅支持本地文件和文件夹"));
                continue;
            };
            let result = path.canonicalize().and_then(|canonical| {
                let entry = canonical.metadata()?;
                Ok((canonical, entry))
            });
            match result {
                Ok((canonical, entry)) if entry.is_dir() => folders.push(canonical),
                Ok((canonical, entry)) if entry.is_file() => {
                    if let Err(error) = self.open_canonical_file(canonical) {
                        failures.push(format!("{}：{error}", path.display()));
                    }
                }
                Ok(_) => failures.push(format!("{}：不是普通文件或文件夹", path.display())),
                Err(error) => failures.push(format!("{}：{error}", path.display())),
            }
        }
        match folders.len() {
            0 => {}
            1 => self.show_folder(folders.pop().unwrap()),
            _ => failures.push("一个窗口一次只能打开一个文件夹，已保留原文件树。".into()),
        }
        if !failures.is_empty() {
            self.explain("部分拖入项未打开", &failures.join("\n"));
        }
    }

    fn pick(self: &Rc<Self>, folder: bool) {
        let s = self.clone();
        await_shell_modal(
            &self.owner,
            self.modal.clone(),
            || {
                pick_paths(if folder {
                    OpenDialog::folders()
                } else {
                    OpenDialog::files().multiple(true)
                })
            },
            move |result| match result {
                Ok(Some(paths)) => {
                    for path in paths {
                        if folder {
                            s.open_folder(&path);
                        } else {
                            s.open(&path);
                        }
                    }
                }
                Ok(None) => s.notice.set("已取消打开，文档保持不变".into()),
                Err(e) => s.explain(
                    "原生打开请求失败",
                    &format!("{e}\n未读取或写入文件；原生文件请求仍待修复。"),
                ),
            },
        );
    }
    fn save(self: &Rc<Self>, all: bool, save_as: bool) {
        let targets = if all {
            self.docs.with(|docs| {
                docs.iter()
                    .filter(|doc| doc.large.is_none() && doc.source.is_dirty().unwrap_or(true))
                    .cloned()
                    .collect::<Vec<_>>()
            })
        } else {
            self.current().into_iter().collect()
        };
        if targets.is_empty() {
            self.notice.set("没有待保存的文档".into());
            return;
        }
        if targets.iter().any(|doc| doc.gate.busy())
            || self.split.get().is_some_and(|split| {
                split.gate.busy() && targets.iter().any(|doc| doc.id == split.id)
            })
        {
            self.notice.set("请先完成或取消中文组合".into());
            return;
        }
        let shell = self.clone();
        self.modal.set(true);
        spawn(async move {
            let result = shell.save_documents(targets, save_as).await;
            shell.modal.set(false);
            if let Err(error) = result {
                shell.explain("保存失败", &error);
            }
        });
    }
    async fn save_documents(
        self: &Rc<Self>,
        targets: Vec<Rc<Document>>,
        save_as: bool,
    ) -> Result<(), String> {
        let mut saved = 0;
        for doc in targets {
            if doc.large.is_some() {
                return Err("只读分页或尚在加载的文件不能保存".into());
            }
            let original = doc.path.get();
            let mut choose_path = save_as || original.is_none();
            let mut use_utf8 = false;
            let mut destination = original.clone();
            loop {
                if choose_path {
                    let mut dialog = SaveDialog::new().title(if save_as || original.is_some() {
                        "另存为新文件"
                    } else {
                        "保存新文件"
                    });
                    if let Some(path) = &original {
                        if let Some(parent) = path.parent() {
                            dialog = dialog.directory(parent);
                        }
                        if let Some(stem) = path.file_stem() {
                            let extension = path
                                .extension()
                                .map(|ext| format!(".{}", ext.to_string_lossy()))
                                .unwrap_or_default();
                            dialog = dialog
                                .file_name(format!("{}-副本{extension}", stem.to_string_lossy()));
                        }
                    }
                    destination = self.owner.run(|| pick_save_path(dialog)).await?;
                    if destination.is_none() {
                        self.show_cancelled_save(saved);
                        return Ok(());
                    }
                    choose_path = false;
                }
                let path = destination.as_ref().unwrap();
                let replace = original.as_ref().is_some_and(|old| old == path);
                self.restore_editor_focus();
                self.notice.set(format!("正在保存 {}", doc.name()));
                match doc.source.save_file(path.clone(), replace, use_utf8).await {
                    Ok(()) => {
                        self.remember(path);
                        doc.path.set(Some(path.clone()));
                        saved += 1;
                        self.show_saved(&doc);
                        break;
                    }
                    Err(SaveError::Encoding) => {
                        let answer = self
                            .owner
                            .run(|| {
                                nsh_base::alert_dialog()
                                    .title("当前编码无法保存部分字符")
                                    .action("utf8", "改存 UTF-8")
                                    .cancel("cancel", "取消")
                                    .close_on_escape(true)
                                    .show()
                            })
                            .await
                            .map_err(|error| error.to_string())?;
                        if answer.action.as_deref() != Some("utf8") {
                            self.show_cancelled_save(saved);
                            return Ok(());
                        }
                        use_utf8 = true;
                    }
                    Err(SaveError::Other(message)) => {
                        let answer = self
                            .owner
                            .run(|| {
                                nsh_base::alert_dialog()
                                    .title("保存失败")
                                    .description(format!("{message}\n保存未完成，编辑内容仍在。"))
                                    .action("retry", "重试")
                                    .action("save-as", "另存为")
                                    .cancel("cancel", "取消")
                                    .close_on_escape(true)
                                    .show()
                            })
                            .await
                            .map_err(|error| error.to_string())?;
                        match answer.action.as_deref() {
                            Some("retry") => {}
                            Some("save-as") => choose_path = true,
                            _ => {
                                self.show_cancelled_save(saved);
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn show_cancelled_save(self: &Rc<Self>, saved: usize) {
        self.restore_editor_focus();
        self.notice.set(if saved == 0 {
            "已取消保存".into()
        } else {
            format!("已保存 {saved} 个文档，已取消剩余保存")
        });
    }
    fn show_saved(&self, doc: &Document) {
        let newer = if doc.source.is_dirty().unwrap_or(true) {
            " · 仍有较新的未保存修改"
        } else {
            ""
        };
        self.notice.set(format!("已保存 {}{newer}", doc.name()));
    }
    fn close_window(self: &Rc<Self>) {
        if self.modal.get() {
            return;
        }
        let dirty = self.docs.with(|ds| {
            ds.iter()
                .filter(|d| d.source.is_dirty().unwrap_or(true))
                .map(|d| d.name())
                .collect::<Vec<_>>()
        });
        if dirty.is_empty() {
            self.window.close();
            return;
        }
        let s = self.clone();
        // ClosePress is an effect; another close request reruns that effect.
        // Its cleanup must not cancel the first pending confirmation.
        await_shell_modal(
            &self.owner,
            self.modal.clone(),
            || {
                nsh_base::alert_dialog()
                    .title("关闭前保留未保存内容")
                    .description(format!(
                        "{}\n关闭将丢失这些未保存的修改。",
                        dirty.join("、")
                    ))
                    .cancel("cancel", "继续编辑")
                    .destructive("discard", "放弃并关闭")
                    .close_on_escape(true)
                    .show()
            },
            move |result| {
                if result.is_ok_and(|r| r.action.as_deref() == Some("discard")) {
                    s.window.close();
                } else {
                    s.restore_editor_focus();
                }
            },
        );
    }
    fn close_tab(self: &Rc<Self>, id: u64) {
        if self.modal.get() {
            return;
        }
        let Some(doc) = self.docs.with(|ds| ds.iter().find(|d| d.id == id).cloned()) else {
            return;
        };
        if doc.gate.busy() {
            self.notice.set("请先完成或取消中文组合".into());
            return;
        }
        if let Some(second) = self.split.get().filter(|d| d.id == id) {
            self.docs.update(|ds| {
                if let Some(d) = ds.iter_mut().find(|d| d.id == id) {
                    *d = second;
                }
            });
            self.split.set(None);
            self.second_active.set(false);
            self.active.set(id);
            return;
        }
        if doc.source.is_dirty().unwrap_or(true) {
            let s = self.clone();
            await_shell_modal(
                &self.owner,
                self.modal.clone(),
                || {
                    nsh_base::alert_dialog()
                        .title("关闭未保存标签")
                        .description(format!(
                            "{} 尚未保存。可以继续编辑，或明确放弃内容。",
                            doc.name()
                        ))
                        .cancel("cancel", "继续编辑")
                        .destructive("discard", "放弃并关闭")
                        .close_on_escape(true)
                        .show()
                },
                move |result| {
                    if result.is_ok_and(|r| r.action.as_deref() == Some("discard")) {
                        s.remove(id);
                    }
                    s.restore_editor_focus();
                },
            );
        } else {
            self.remove(id);
        }
    }
    fn remove(&self, id: u64) {
        if self.split.with(|d| d.as_ref().is_some_and(|d| d.id == id)) {
            self.split.set(None);
            self.second_active.set(false);
        }
        self.docs.update(|ds| {
            if let Some(at) = ds.iter().position(|d| d.id == id) {
                let doc = ds.remove(at);
                if let Some(large) = &doc.large {
                    large.close();
                }
                let reopen = if doc.large.is_some()
                    || (doc.source.disk_len() >= LARGE_BYTES
                        && !doc.source.is_dirty().unwrap_or(true))
                {
                    doc.path.get().map(ClosedTab::File)
                } else {
                    None
                };
                self.closed
                    .borrow_mut()
                    .push(reopen.unwrap_or(ClosedTab::Document(doc)));
            }
        });
        if self.active.get() == id {
            self.active
                .set(self.docs.with(|ds| ds.last().map_or(0, |d| d.id)));
        }
        if self.docs.with(|ds| ds.is_empty()) {
            self.add_blank();
        }
    }
    fn find(self: &Rc<Self>, previous: bool) {
        let Some(doc) = self.current() else {
            return;
        };
        let scope = if doc.gate.rich_mode.get() {
            "正文"
        } else {
            "源码"
        };
        match doc.gate.find_literal(&self.query.get(), previous) {
            Ok((n, total)) => {
                self.notice.set(format!("{scope} · {n}/{total}"));
                if n > 0 {
                    self.restore_editor_focus();
                }
            }
            Err(e) => self.notice.set(e),
        }
    }
    fn source_action(&self, command: EditorCommand) -> Result<(), String> {
        let doc = self.current().ok_or("没有活动文档")?;
        if command == EditorCommand::Undo || command == EditorCommand::Redo {
            return doc.gate.shell_history(command == EditorCommand::Undo);
        }
        if matches!(
            command,
            EditorCommand::SelectNextOccurrence
                | EditorCommand::SelectAllOccurrences
                | EditorCommand::DuplicateLinesDown
                | EditorCommand::SingleSelection
        ) {
            let transferred = doc.gate.rich_mode.get();
            if doc.gate.rich_mode.get() {
                doc.gate.to_source();
                if doc.gate.rich_mode.get() {
                    return Err(doc.gate.status.get());
                }
            }
            let result = match command {
                EditorCommand::SelectNextOccurrence => doc.source.select_next_match(false),
                EditorCommand::SelectAllOccurrences => doc.source.select_next_match(true),
                EditorCommand::DuplicateLinesDown => doc.source.duplicate_line(),
                _ => doc.source.clear_extra_selections(),
            };
            if result.is_ok()
                && std::env::var_os("TEXZI_TRACE_IME").as_deref() == Some(std::ffi::OsStr::new("1"))
            {
                if let Ok(snapshot) = doc.source.snapshot() {
                    log::info!(
                        "TX_SHELL_SELECTION command={command:?} revision={} epoch={} count={}",
                        snapshot.revision,
                        snapshot.view_epoch,
                        snapshot.selections.ranges.len()
                    );
                }
            }
            if transferred && result.is_ok() {
                self.notice
                    .set("此操作已切换到源码；继续使用同一正文和撤销历史。".into());
            }
            return result;
        }
        if doc.gate.rich_mode.get() {
            return Err("此源码操作请先切换源码；当前富文本与选区已保留。".into());
        }
        let api = doc
            .reference
            .api::<EditorApi>()
            .ok_or("源码编辑区尚未就绪")?;
        api.run(command);
        api.focus();
        Ok(())
    }
    fn split_editor(self: &Rc<Self>, down: bool) {
        if self.split.get().is_some() {
            self.notice
                .set("继续新建编辑组尚未完成，已保留当前两组和原布局".into());
            return;
        }
        let Some(doc) = self.current() else {
            return;
        };
        if doc.gate.rich_mode.get() {
            doc.gate.to_source();
            if doc.gate.rich_mode.get() {
                self.notice.set(doc.gate.status.get());
                return;
            }
        }
        let source = match doc.source.sibling_source() {
            Ok(v) => v,
            Err(e) => {
                self.explain("无法拆分", &e);
                return;
            }
        };
        let rich = match source.sibling() {
            Ok(v) => v,
            Err(e) => {
                self.explain("无法拆分", &e);
                return;
            }
        };
        let gate = ModeGate::new(source.clone(), rich);
        let reference = component_ref::<Editor>();
        let rich_reference = component_ref::<nsh_base::rich_editor::view::RichEditor>();
        let controls = RichControlSet::new(&gate, &source);
        let wrap = signal(doc.wrap.get());
        self.split.set(Some(Rc::new(Document {
            id: doc.id,
            path: doc.path,
            source,
            gate,
            large: None,
            reference,
            rich_reference,
            controls,
            wrap,
            pending_mode: signal(None),
            progress: element_ref(),
            body: element_ref(),
        })));
        self.split_down.set(down);
        self.second_active.set(true);
        self.notice
            .set("同一文档已拆分为两个独立视图，共享正文与撤销历史".into());
    }
    fn focus_group(&self, index: u8) {
        self.zen_exit.reset();
        if index > 2 {
            self.notice
                .set(format!("编辑组 {index} 不存在，未创建新文档"));
            return;
        }
        if index == 2 && self.split.get().is_none() {
            self.notice.set("编辑组 2 不存在，未创建新文档".into());
            return;
        }
        self.second_active.set(index == 2);
        if let Some(doc) = self.current() {
            if doc.gate.rich_mode.get() {
                if let Some(api) = doc
                    .rich_reference
                    .api::<nsh_base::rich_editor::view::RichEditorApi>()
                {
                    api.focus();
                }
            } else if let Some(api) = doc.reference.api::<EditorApi>() {
                api.focus();
            }
        }
    }
    fn restore_editor_focus(self: &Rc<Self>) {
        self.focus_group(if self.second_active.get() { 2 } else { 1 });
        let s = self.clone();
        self.owner.run(|| {
            spawn(async move {
                let _ = next_frame().await;
                s.focus_group(if s.second_active.get() { 2 } else { 1 });
            })
        });
    }
    fn escape_blocked(&self) -> bool {
        self.modal.get()
            || self
                .current()
                .is_some_and(|d| d.gate.busy() || d.controls.open())
    }

    fn zen_source_escape(&self) -> bool {
        self.zen.get()
            && !self.chord.get()
            && self.panel.get().is_empty()
            // A newer binding would mask the Editor's selection collapse.
            && self.current().is_some_and(|doc| {
                !doc.gate.rich_mode.get() && doc.source.has_single_caret()
            })
    }

    fn handle_rich_escape(self: &Rc<Self>, document: u64, second: bool) {
        if self.zen.get()
            && !self.chord.get()
            && self.second_active.get() == second
            && self
                .current()
                .is_some_and(|doc| doc.id == document && doc.gate.rich_mode.get())
        {
            self.handle_escape();
        }
    }

    fn handle_escape(self: &Rc<Self>) {
        if self.escape_blocked() {
            self.zen_exit.escape(None, true);
            return;
        }
        if !self.panel.get().is_empty() {
            self.zen_exit.escape(None, true);
            self.close_panel();
        } else {
            let stamp = self.zen.get().then(|| self.zen_stamp()).flatten();
            match self.zen_exit.escape(stamp, false) {
                crate::zen_state::EscapeAction::Exit => {
                    self.zen.set(false);
                    self.notice.set(String::new());
                    self.restore_editor_focus();
                }
                crate::zen_state::EscapeAction::Armed => {
                    self.notice.set("再按一次 Esc 退出 Zen".into())
                }
                crate::zen_state::EscapeAction::Ignored => {}
            }
        }
    }

    fn close_panel(self: &Rc<Self>) {
        self.zen_exit.reset();
        self.cancel_quick(false);
        self.panel.set(String::new());
        self.restore_editor_focus();
    }
    fn search_folder(self: &Rc<Self>) {
        let Some(root) = self.root.get() else {
            self.search_notice.set("请先打开文件夹".into());
            return;
        };
        let query = self.query.get();
        if query.is_empty() {
            self.search_notice.set("请输入查找文字".into());
            return;
        }
        if let Some(previous) = self.search_cancel.borrow_mut().take() {
            previous.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *self.search_cancel.borrow_mut() = Some(cancel.clone());
        self.searching.set(true);
        self.hits.set(vec![]);
        self.search_notice.set("正在搜索根目录；可以取消".into());
        let worker_cancel = cancel.clone();
        let worker_root = root.clone();
        let work = blocking(move || scan_folder(worker_root, query, worker_cancel));
        let s = self.clone();
        spawn(async move {
            let result = work.await;
            let current = s
                .search_cancel
                .borrow()
                .as_ref()
                .is_some_and(|c| Arc::ptr_eq(c, &cancel));
            if !current {
                return;
            }
            s.searching.set(false);
            if s.root.get() != Some(root) {
                s.search_notice.set("根目录已改变，旧结果已丢弃".into());
                return;
            }
            match result {
                Ok(report) => {
                    s.search_notice.set(format!("{}：已读 {} 个 UTF-8 文本，{} 行命中，跳过 {} 项{}。上限 2000 目录/文件、200 行结果、单文件 1 MiB；跳过链接/重解析点、非UTF-8、二进制、大文件和无法读取项。{}",if report.cancelled{"已取消"}else{"本次搜索结束"},report.scanned,report.hits.len(),report.skipped,if report.limited{"；已达预算，结果不完整"}else{""},report.errors.join("\n")));
                    s.hits.set(report.hits);
                }
                Err(e) => s.search_notice.set(format!("目录搜索失败：{e}")),
            }
        });
    }
    fn run(self: &Rc<Self>, id: &str) {
        self.zen_exit.reset();
        if std::env::var_os("TEXZI_TRACE_IME").as_deref() == Some(std::ffi::OsStr::new("1")) {
            log::info!("TX_SHELL_COMMAND id={id} modal={}", self.modal.get());
        }
        if self.modal.get() {
            return;
        }
        if self.current().is_some_and(|d| d.gate.busy()) {
            self.notice.set("请先完成或取消中文组合".into());
            return;
        }
        if let Some(reason) = self.blocked(id) {
            self.explain("此动作暂不可用", &reason);
            return;
        }
        self.owner.run(|| self.run_ready(id));
    }
    fn run_ready(self: &Rc<Self>, id: &str) {
        if crate::rich_commands::handles(id) {
            self.markdown_command(id);
            return;
        }
        match id {
            "texzi.find.inFolder"=>{self.query.set(String::new());self.panel.set("folder".into());},
            "texzi.file.new"=>self.add_blank(),"texzi.file.open"=>self.pick(false),"texzi.folder.open"=>self.pick(true),
            "texzi.file.save" => self.save(false,false),
            "texzi.file.saveAs" => self.save(false,true),
            "texzi.file.saveAll" => self.save(true,false),
            "texzi.folder.close"=>{self.root.set(None);self.notice.set("已关闭文件夹，已打开标签保持不变".into());},
            "texzi.window.close"=>self.close_window(),"texzi.tab.close"=>if self.second_active.get(){self.split.set(None);self.second_active.set(false);}else{self.close_tab(self.active.get());},
            "texzi.window.new"=>match std::env::current_exe().and_then(|exe|std::process::Command::new(exe).spawn()){Ok(_)=>self.notice.set("已启动新的 texzi 窗口".into()),Err(e)=>self.explain("新建窗口失败",&e.to_string())},
            "texzi.group.splitRight"|"texzi.group.splitDown"=>self.split_editor(id.ends_with("Down")),
            "texzi.group.joinAll"=>{self.split.set(None);self.second_active.set(false);self.notice.set("已合并为单组；同一文档内容与历史保持不变".into());},
            "texzi.tab.reopenClosed"=>{let doc=self.closed.borrow_mut().pop();match doc{Some(ClosedTab::Document(doc))=>{let id=doc.id;self.docs.update(|ds|ds.push(doc));self.second_active.set(false);self.active.set(id);self.restore_editor_focus();},Some(ClosedTab::File(path))=>self.open(&path),None=>self.notice.set("没有已关闭标签".into())}},
            "texzi.tab.next"|"texzi.tab.previous"=>{if self.second_active.get(){self.notice.set("当前编辑组只有一个标签，已保持当前组".into());}else{self.docs.with(|ds|{if let Some(at)=ds.iter().position(|d|d.id==self.active.get()){self.active.set(ds[if id.ends_with("next"){(at+1)%ds.len()}else{(at+ds.len()-1)%ds.len()}].id);}});}self.restore_editor_focus();},
            "texzi.sidebar.toggle"=>self.sidebar.update(|v|*v=!*v),"texzi.editor.toggleWrap"=>if let Some(doc)=self.current(){doc.wrap.update(|v|*v=!*v);self.notice.set(if doc.wrap.get(){"当前源码视图：自动换行已开启"}else{"当前源码视图：自动换行已关闭"}.into());},
            "texzi.view.toggleZen"|"texzi.view.exitZen"=>{if id.ends_with("exitZen"){self.zen.set(false);}else{self.zen.update(|v|*v=!*v);}if self.notice.get()=="再按一次 Esc 退出 Zen"{self.notice.set(String::new());}},
            "texzi.view.toggleFullScreen"=>self.window.set_mode(if self.window.state().is_some_and(|s|s.mode==WindowMode::FullScreen){WindowMode::Normal}else{WindowMode::FullScreen}),
            "texzi.commandPalette.open"|"texzi.navigation.quickOpen"=>{self.palette_query.set(String::new());let files=id.ends_with("quickOpen");let already=self.panel.get()=="files";if files{self.quick_files.set(vec![]);self.quick_notice.set("正在准备文件候选".into());}self.panel.set(if files{"files"}else{"commands"}.into());if files&&already{self.start_quick(self.root.get());}},
            "texzi.navigation.gotoLine"=>{if let Some(doc)=self.current(){doc.gate.to_source();if doc.gate.rich_mode.get(){self.notice.set(doc.gate.status.get());return;}}self.query.set(String::new());self.panel.set("line".into());self.notice.set("转到行的范围已切换为源码".into());},
            "texzi.group.focusIndex"=>self.panel.set("groups".into()),
            "texzi.markdown.toggleSource"=>if let Some(doc)=self.current(){let target = !doc.displayed_rich(); self.request_mode(doc, target);},
            "texzi.find.open"|"texzi.find.replaceOpen"=>{let replace=id.ends_with("replaceOpen");if replace{if let Some(doc)=self.current(){doc.gate.to_source();if doc.gate.rich_mode.get(){self.notice.set(doc.gate.status.get());return;}}}self.panel.set(if replace{"replace"}else{"find"}.into());let scope=if self.current().is_some_and(|d|d.gate.rich_mode.get()){"正文"}else{"源码"};self.notice.set(format!("查找范围：{scope} · 普通文字 · 区分大小写"));},
            "texzi.find.next"=>self.find(false),"texzi.find.previous"=>self.find(true),"texzi.find.close"=>self.close_panel(),
            "texzi.find.replaceOne"|"texzi.find.replaceAll"=>if let Some(doc)=self.current(){doc.gate.to_source();if doc.gate.rich_mode.get(){self.notice.set(doc.gate.status.get());return;}match doc.source.replace_literal(&self.query.get(),&self.replacement.get(),id.ends_with("All")){Ok(n)=>{self.notice.set(format!("源码 · 已替换 {n} 处，可一次撤销"));self.restore_editor_focus();},Err(e)=>self.notice.set(e)}},
            "texzi.appearance.light"=>set_color_scheme(Appearance::Light),"texzi.appearance.dark"=>set_color_scheme(Appearance::Dark),
            "texzi.help.guide"=>self.explain("使用说明","文件菜单可新建、打开、保存和另存文档。编辑菜单提供查找和替换，外观菜单提供侧栏、自动换行和主题。\nCtrl+P 搜索文件，Ctrl+Shift+P 搜索命令。可搜索分屏或 Zen，亦可按 Ctrl+K Z 进入 Zen，连按两次 Esc 退出。编辑器右上可切换源码和富文本。"),
            "texzi.help.about"=>self.explain("关于 texzi","texzi\n为日常文字、Markdown 与代码片段设计的 Windows 原生文本编辑器。"),
            _=>if let Some(command)=source_command(id){if let Err(e)=self.source_action(command){self.notice.set(e);}},
        }
    }
    fn request_mode(self: &Rc<Self>, doc: Rc<Document>, target_rich: bool) {
        if target_rich && doc.source.disk_len() > large_file::SOURCE_ONLY_BYTES {
            self.notice
                .set("此大文件使用源码模式，已关闭语法高亮".into());
            return;
        }
        if doc.pending_mode.get().is_some() || doc.gate.rich_mode.get() == target_rich {
            return;
        }
        doc.pending_mode.set(Some(target_rich));
        let shell = self.clone();
        self.owner.run(|| {
            spawn(async move {
                wait_for_editor_frame(doc.progress).await;
                let still_open = || {
                    shell
                        .docs
                        .with(|docs| docs.iter().any(|entry| Rc::ptr_eq(entry, &doc)))
                        || shell.split.with(|entry| {
                            entry.as_ref().is_some_and(|entry| Rc::ptr_eq(entry, &doc))
                        })
                };
                if still_open() {
                    let viewport_top = doc.viewport_caret_top();
                    let trace = std::env::var_os("TEXZI_TRACE_IME").as_deref()
                        == Some(std::ffi::OsStr::new("1"));
                    if trace {
                        doc.trace_mode_view("before");
                    }
                    if target_rich {
                        doc.gate.to_rich_in_background(&still_open).await;
                    } else {
                        doc.gate.to_source();
                    }
                    if doc.gate.rich_mode.get() == target_rich {
                        wait_for_editor_frame(doc.body).await;
                        if !still_open() {
                            doc.pending_mode.set(None);
                            return;
                        }
                        let active = shell
                            .current()
                            .is_some_and(|current| Rc::ptr_eq(&current, &doc));
                        if target_rich {
                            if let Some(api) = doc.rich_reference.api::<RichEditorApi>() {
                                api.reveal_primary_caret(viewport_top);
                                if active {
                                    api.focus();
                                }
                            }
                        } else if let Some(api) = doc.reference.api::<EditorApi>() {
                            api.reveal_primary_caret(viewport_top);
                            if active {
                                api.focus();
                            }
                        }
                        if trace {
                            let trace_doc = doc.clone();
                            spawn(async move {
                                for _ in 0..10 {
                                    let _ = next_frame().await;
                                    if trace_doc.viewport_caret_top().is_some() {
                                        break;
                                    }
                                }
                                trace_doc.trace_mode_view("after");
                            });
                        }
                    }
                }
                doc.pending_mode.set(None);
            });
        });
    }

    fn show_format_menu(self: &Rc<Self>, doc: Rc<Document>, anchor: ElementRef) {
        let shell = self.clone();
        self.owner.run(|| {
            spawn(async move {
                let Some(bounds) = anchor.measure().await else {
                    return;
                };
                let items = FORMAT_ACTIONS
                    .iter()
                    .map(|&(label, action)| {
                        let shell = shell.clone();
                        let doc = doc.clone();
                        MenuItem::new(label).on_select(move || shell.format_action(&doc, action))
                    })
                    .collect();
                shell.zen_exit.reset();
                let _ = popup_menu_at(
                    items,
                    point(bounds.page.x, bounds.page.y + bounds.frame.size.height),
                );
            });
        });
    }

    fn format_action(self: &Rc<Self>, doc: &Rc<Document>, action: FormatAction) {
        if !doc.gate.rich_mode.get()
            || !self
                .current()
                .is_some_and(|current| Rc::ptr_eq(&current, doc))
        {
            return;
        }
        use crate::rich_projection::ResourceKind;
        let result = match action {
            FormatAction::Mark(id) => return self.run(id),
            FormatAction::Paragraph => doc.controls.blocks.begin(),
            FormatAction::Link => doc.controls.resources.begin(ResourceKind::Link),
            FormatAction::Image => doc.controls.resources.begin(ResourceKind::Image),
            FormatAction::Table => doc.controls.table.begin(),
            FormatAction::CodeLanguage => doc.controls.code.begin(),
            FormatAction::ExitCode => doc.controls.code.exit(),
            FormatAction::Indent(outdent) => doc.controls.code.indent(outdent),
        };
        if let Err(error) = result {
            doc.gate.status.set(error.clone());
            self.notice.set(error);
        }
        if !doc.controls.open() {
            self.restore_editor_focus();
        }
    }
    fn markdown_command(self: &Rc<Self>, id: &str) {
        let Some(doc) = self.current() else { return };
        let outcome = crate::rich_commands::run(&doc.gate, &doc.source, id, true);
        let result = match outcome {
            Ok(crate::rich_commands::Outcome::Applied { .. }) => {
                self.notice.set(String::new());
                self.restore_editor_focus();
                Ok(())
            }
            Ok(crate::rich_commands::Outcome::Form(request)) => doc.controls.begin(request),
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            doc.gate.status.set(error.clone());
            self.notice.set(error);
        }
    }
}
pub(crate) fn supported(id: &str) -> bool {
    crate::rich_commands::handles(id)
        || source_command(id).is_some()
        || matches!(
            id,
            "texzi.file.new"
                | "texzi.file.open"
                | "texzi.file.save"
                | "texzi.file.saveAs"
                | "texzi.file.saveAll"
                | "texzi.folder.open"
                | "texzi.folder.close"
                | "texzi.window.new"
                | "texzi.window.close"
                | "texzi.tab.close"
                | "texzi.tab.reopenClosed"
                | "texzi.tab.next"
                | "texzi.tab.previous"
                | "texzi.sidebar.toggle"
                | "texzi.editor.toggleWrap"
                | "texzi.view.toggleZen"
                | "texzi.view.exitZen"
                | "texzi.view.toggleFullScreen"
                | "texzi.commandPalette.open"
                | "texzi.navigation.quickOpen"
                | "texzi.navigation.gotoLine"
                | "texzi.group.splitRight"
                | "texzi.group.splitDown"
                | "texzi.group.joinAll"
                | "texzi.group.focusIndex"
                | "texzi.markdown.toggleSource"
                | "texzi.find.open"
                | "texzi.find.replaceOpen"
                | "texzi.find.next"
                | "texzi.find.previous"
                | "texzi.find.close"
                | "texzi.find.inFolder"
                | "texzi.find.replaceOne"
                | "texzi.find.replaceAll"
                | "texzi.appearance.light"
                | "texzi.appearance.dark"
                | "texzi.help.guide"
                | "texzi.help.about"
        )
}
fn line_column(value: &str) -> Result<(usize, usize), String> {
    let value = value.trim();
    let (line, column) = value.split_once(':').map_or((value, "1"), |pair| pair);
    let parse = |value: &str| value.trim().parse::<usize>().ok().filter(|n| *n > 0);
    match (parse(line), parse(column)) {
        (Some(line), Some(column)) => Ok((line, column)),
        _ => Err("请输入从 1 开始的行或行:列，例如 12 或 12:8".into()),
    }
}
fn source_command(id: &str) -> Option<EditorCommand> {
    use EditorCommand::*;
    Some(match id {
        "texzi.edit.undo" => Undo,
        "texzi.edit.redo" => Redo,
        "texzi.edit.cut" => Cut,
        "texzi.edit.copy" => Copy,
        "texzi.edit.paste" | "texzi.edit.pastePlain" => Paste,
        "texzi.edit.selectAll" => SelectAll,
        "texzi.edit.copyLineDown" => DuplicateLinesDown,
        "texzi.edit.copyLineUp" => DuplicateLinesUp,
        "texzi.edit.moveLineUp" => MoveLinesUp,
        "texzi.edit.moveLineDown" => MoveLinesDown,
        "texzi.edit.deleteLine" => DeleteLine,
        "texzi.edit.insertLineAbove" => NewlineAbove,
        "texzi.edit.insertLineBelow" => NewlineBelow,
        "texzi.edit.indent" => Indent,
        "texzi.edit.outdent" => Outdent,
        "texzi.selection.line" => SelectLine,
        "texzi.multicursor.addNext" => SelectNextOccurrence,
        "texzi.multicursor.selectAllOccurrences" => SelectAllOccurrences,
        "texzi.multicursor.above" => AddCaretAbove,
        "texzi.multicursor.below" => AddCaretBelow,
        "texzi.multicursor.undo" => UndoSelection,
        "texzi.multicursor.clear" => SingleSelection,
        _ => return None,
    })
}
fn button(label: &'static str, s: Rc<Shell>, id: &'static str) -> impl IntoElement {
    nsh_base::button()
        .child(text(label))
        .on_press(move |_: &()| s.run(id))
}

const CHORD_PREFIX: &str = "mod+k";
const CHORDS: &[(&str, &str)] = &[
    ("mod+o", "texzi.folder.open"),
    ("f", "texzi.folder.close"),
    ("mod+b", "texzi.sidebar.toggle"),
    ("z", "texzi.view.toggleZen"),
    ("s", "texzi.file.saveAll"),
    ("mod+\\", "texzi.group.splitDown"),
];
// Installed only while the prefix is pending. Its plain second keys must be
// commands even with a text input focused, rather than typing into the document.
fn bind_chord_continuations(run: Rc<dyn Fn(Option<&'static str>)>) {
    for base in "abcdefghijklmnopqrstuvwxyz0123456789"
        .chars()
        .map(|c| c.to_string())
        .chain(
            [
                "escape",
                "left",
                "right",
                "up",
                "down",
                "backspace",
                "delete",
                "enter",
                "space",
                "tab",
                "\\",
            ]
            .into_iter()
            .map(str::to_owned),
        )
    {
        for modifier in ["", "mod+", "shift+", "mod+shift+"] {
            let stroke = format!("{modifier}{base}");
            // Let the prefix handler start a new timer generation on a repeated Ctrl+K.
            if stroke == CHORD_PREFIX {
                continue;
            }
            let Ok(parsed) = nsh_core::keymap::Keystroke::parse_with(&stroke, apple_keys()) else {
                continue;
            };
            if parsed.reserved(os()).is_some() {
                continue;
            }
            let target = CHORDS
                .iter()
                .find(|(key, _)| *key == stroke)
                .map(|(_, id)| *id);
            let run = run.clone();
            on_key_with(
                &stroke,
                KeyOptions {
                    when: nsh_core::keymap::When::Always,
                    ..Default::default()
                },
                move |_| run(target),
            );
        }
    }
}

// This opt-in snapshot must not subscribe a binding effect to extra signals.
// UTC microseconds use the same SystemTime epoch as the native key trace.
fn trace_chord_shell(stage: &'static str, s: &Shell, command: Option<&'static str>) {
    if std::env::var_os("TEXZI_TRACE_CHORD").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return;
    }
    untracked(|| {
        let current = s.current();
        let utc_us = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_micros());
        log::info!(
            "TX_CHORD_SHELL utc_us={utc_us} stage={stage} prefix_pending={} modal={} active_document={} ime_busy={} known_command={} command={}",
            s.chord.get(),
            s.modal.get(),
            current.is_some(),
            current.is_some_and(|d| d.gate.busy()),
            command.is_some(),
            command.unwrap_or("none"),
        );
    });
}

fn bind_chords(s: Rc<Shell>) {
    let prefix = s.clone();
    on_key(CHORD_PREFIX, move |_| {
        prefix.zen_exit.reset();
        if prefix.modal.get() || prefix.current().is_some_and(|d| d.gate.busy()) {
            trace_chord_shell("prefix_rejected", &prefix, None);
            return;
        }
        prefix.chord.set(true);
        let generation = prefix.chord_generation.get().wrapping_add(1);
        prefix.chord_generation.set(generation);
        prefix.notice.set("Ctrl+K：等待第二个按键…".into());
        trace_chord_shell("prefix_begin", &prefix, None);
        let trace_shell = Rc::downgrade(&prefix);
        set_timeout(std::time::Duration::from_millis(1500), move || {
            if let Some(shell) = trace_shell.upgrade()
                && shell.chord_generation.get() == generation
                && shell.chord.get()
            {
                trace_chord_shell("timeout", &shell, None);
                shell.zen_exit.reset();
                shell.chord.set(false);
                if shell.notice.get() == "Ctrl+K：等待第二个按键…" {
                    shell.notice.set("Ctrl+K 组合已超时取消".into());
                }
            }
        });
    });
    effect(move || {
        if !s.chord.get() {
            return;
        }
        let shell = s.clone();
        bind_chord_continuations(Rc::new(move |target| {
            trace_chord_shell("continuation_hit", &shell, target);
            shell.zen_exit.reset();
            shell.chord.set(false);
            if shell.notice.get() == "Ctrl+K：等待第二个按键…" {
                shell.notice.set(String::new());
            }
            if let Some(id) = target {
                shell.run(id);
            } else {
                shell
                    .notice
                    .set("已取消：没有这个 Ctrl+K 组合，未写入正文".into());
            }
        }));
        // This observes engine registration, not delivery of MountItem::Keymap
        // to Windows. A native trace is still needed to establish that boundary.
        trace_chord_shell("continuations_registered", &s, None);
    });
}

fn scan_folder(root: PathBuf, query: String, cancel: Arc<AtomicBool>) -> SearchReport {
    let mut result = SearchReport {
        hits: vec![],
        scanned: 0,
        skipped: 0,
        limited: false,
        cancelled: false,
        errors: vec![],
    };
    let mut pending = vec![root];
    let mut seen = 0usize;
    while let Some(path) = pending.pop() {
        if cancel.load(Ordering::Relaxed) {
            result.cancelled = true;
            break;
        }
        seen += 1;
        if seen > 2000 || result.hits.len() >= 200 {
            result.limited = true;
            break;
        }
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                result.skipped += 1;
                if result.errors.len() < 20 {
                    result.errors.push(format!("{}：{e}", path.display()));
                }
                continue;
            }
        };
        if meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0 {
            result.skipped += 1;
            continue;
        }
        if meta.is_dir() {
            match std::fs::read_dir(&path) {
                Ok(entries) => {
                    let mut children = vec![];
                    for entry in entries.take(2001) {
                        match entry {
                            Ok(e) => children.push(e.path()),
                            Err(_) => result.skipped += 1,
                        }
                    }
                    children.sort();
                    pending.extend(children.into_iter().rev());
                }
                Err(_) => result.skipped += 1,
            }
            continue;
        }
        if !meta.is_file() || meta.len() > 1024 * 1024 {
            result.skipped += 1;
            continue;
        }
        let mut bytes = vec![];
        let read = std::fs::File::open(&path)
            .and_then(|f| f.take(1024 * 1024 + 1).read_to_end(&mut bytes));
        if let Err(e) = &read {
            if result.errors.len() < 20 {
                result.errors.push(format!("{}：{e}", path.display()));
            }
        }
        if read.is_err() || bytes.len() > 1024 * 1024 || bytes.contains(&0) {
            result.skipped += 1;
            continue;
        }
        let text = match std::str::from_utf8(&bytes) {
            Ok(t) => t,
            Err(_) => {
                result.skipped += 1;
                continue;
            }
        };
        result.scanned += 1;
        for (index, line) in text.lines().enumerate() {
            if line.contains(&query) {
                result.hits.push(SearchHit {
                    path: path.clone(),
                    line: index + 1,
                    text: line.chars().take(160).collect(),
                });
                if result.hits.len() >= 200 {
                    result.limited = true;
                    break;
                }
            }
        }
    }
    result
}

fn editor(s: Rc<Shell>, doc: Rc<Document>, second: bool) -> impl IntoElement {
    provide_context(doc.source.buffer());
    let document = doc.id;
    let reference = doc.reference;
    let rich_reference = doc.rich_reference;
    let rich_mode = doc.gate.rich_mode;
    let rich = doc.gate.clone();
    let source = doc.gate.clone();
    let wrap = doc.wrap;
    let body = doc.body;
    let source_focus = s.second_active;
    let rich_focus = s.second_active;
    let block_controls = doc.controls.blocks.clone();
    let resource_controls = doc.controls.resources.clone();
    let table_controls = doc.controls.table.clone();
    let code_controls = doc.controls.code.clone();
    let image_document_path = doc.path;
    let controls_owner = s.owner;
    let controls_shell = s.clone();
    let show_path = doc.clone();
    let path = doc.path;
    let root = s.root;
    let status_doc = doc.clone();
    let mode_doc = doc.clone();
    let format_doc = doc.clone();
    let format_shell = s.clone();
    let markdown = move || path.with(|value| value.as_deref().is_some_and(is_markdown));
    let large_source = doc.source.disk_len() > large_file::SOURCE_ONLY_BYTES;

    let content = view()
        .relative()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .child(
            when(rich_mode, move || {
                let session = rich.session();
                effect(move || crate::rich_images::install(&session, image_document_path.get()));
                let shell = controls_shell.clone();
                let escape = shell.clone();
                let focus: Rc<dyn Fn()> =
                    Rc::new(move || shell.focus_group(if second { 2 } else { 1 }));
                let raw_focus = focus.clone();
                view()
                    .flex_1()
                    .w_full()
                    .max_w(relative(1.0))
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .child(crate::block_controls::dialog(
                        block_controls.clone(),
                        focus.clone(),
                    ))
                    .child(crate::rich_controls::dialog(
                        resource_controls.clone(),
                        focus.clone(),
                    ))
                    .child(crate::table_controls::dialog(
                        table_controls.clone(),
                        focus.clone(),
                    ))
                    .child(crate::code_controls::dialog(code_controls.clone(), focus))
                    .child(
                        scroll()
                            .w_full()
                            .max_h(relative(0.48))
                            .min_h(px(0.0))
                            .child(crate::raw_block_controls::raw_block_controls(
                                rich.clone(),
                                controls_owner,
                                move || raw_focus(),
                            )),
                    )
                    .child(
                        nsh_base::rich_editor::view::rich_editor()
                            .element_ref(&body)
                            .component_ref(&rich_reference)
                            .editor(rich.session())
                            .application_shortcuts(crate::native_menu::application_shortcuts())
                            .classes(crate::app::rich_classes())
                            .auto_focus(move || rich_focus.get() == second)
                            .on_focus(move |_: &()| rich_focus.set(second))
                            .on_unhandled_escape(move |_: &()| {
                                escape.handle_rich_escape(document, second)
                            })
                            .flex_1()
                            .min_w(px(0.0))
                            .min_h(px(0.0)),
                    )
            })
            .otherwise(move || {
                source.source_mounted();
                nsh_base::editor::editor()
                    .element_ref(&body)
                    .component_ref(&reference)
                    .auto_focus(move || source_focus.get() == second)
                    .on_focus(move |_: &()| source_focus.set(second))
                    .language(move || {
                        Some(SharedString::from(if markdown() {
                            "markdown"
                        } else {
                            "plaintext"
                        }))
                    })
                    .tab_size(move || Some(editor_tab_width(markdown())))
                    .hard_tabs(false)
                    .soft_wrap(move || wrap.get())
                    .highlighting(!large_source)
                    .font_size(px(16.0))
                    .line_height(px(24.0))
                    .w_full()
                    .flex_1()
                    .max_w(relative(1.0))
                    .min_w(px(0.0))
                    .min_h(px(0.0))
            }),
        )
        .child(
            view()
                .absolute()
                .top(px(12.0))
                .left(px(16.0))
                .right(px(60.0))
                .z_index(1)
                .pointer_events(PointerEvents::None)
                .element_ref(&doc.progress)
                .child(
                    text(move || match status_doc.pending_mode.get() {
                        Some(true) => "正在切换到富文本…".into(),
                        Some(false) => "正在切换到源码…".into(),
                        None => status_doc.gate.status.get(),
                    })
                    .number_of_lines(1)
                    .text_overflow(EllipsizeMode::Tail),
                ),
        );

    view()
        .relative()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .overflow_hidden()
        .child(when(
            move || path.get().is_some() && !show_path.displayed_rich(),
            move || {
                view()
                    .h(px(28.0))
                    .flex_shrink_0()
                    .justify_center()
                    .pl(px(12.0))
                    .pr(px(60.0))
                    .border_b(px(1.0))
                    .border_color(divider)
                    .child(
                        text(move || {
                            let Some(path) = path.get() else {
                                return String::new();
                            };
                            if let Some(root) = root.get() {
                                if let (Some(folder), Ok(relative)) =
                                    (root.file_name(), path.strip_prefix(&root))
                                {
                                    return Path::new(folder)
                                        .join(relative)
                                        .to_string_lossy()
                                        .into_owned();
                                }
                            }
                            display_path(&path)
                        })
                        .text_color(secondary_foreground)
                        .text_size(px(12.0))
                        .number_of_lines(1)
                        .text_overflow(EllipsizeMode::Middle),
                    )
            },
        ))
        .child(content)
        .child(when(rich_mode, move || {
            format_button(format_shell.clone(), format_doc.clone(), second)
        }))
        .child(when(
            move || {
                mode_doc.is_markdown()
                    || mode_doc.displayed_rich()
                    || mode_doc.pending_mode.get().is_some()
            },
            move || mode_button(s.clone(), doc.clone(), second),
        ))
}

fn mode_button(shell: Rc<Shell>, doc: Rc<Document>, second: bool) -> impl IntoElement {
    let selected = doc.clone();
    let name = doc.clone();
    let disabled = doc.clone();
    let backdrop = doc.clone();
    icon_button(SOURCE_ICON, "切换到源码", 32.0, 18.0)
        .absolute()
        .h(px(28.0))
        .top(px(12.0))
        .right(px(16.0))
        .z_index(1)
        .border(px(1.0))
        .border_color(divider)
        .bg(move || {
            if backdrop.displayed_rich() {
                frame()
            } else {
                selected_surface()
            }
        })
        .accessibility_label(move || {
            Some(SharedString::from(if name.displayed_rich() {
                "切换到源码"
            } else {
                "返回所见即所得"
            }))
        })
        .selected(move || !selected.displayed_rich())
        .disabled(move || {
            disabled.pending_mode.get().is_some()
                || (!disabled.displayed_rich()
                    && disabled.source.disk_len() > large_file::SOURCE_ONLY_BYTES)
        })
        .on_press(move |_: &()| {
            shell.second_active.set(second);
            shell.request_mode(doc.clone(), !doc.displayed_rich());
        })
}

fn format_button(shell: Rc<Shell>, doc: Rc<Document>, second: bool) -> impl IntoElement {
    let anchor = element_ref();
    let state = doc.clone();
    icon_button(FORMAT_ICON, "格式命令", 32.0, 18.0)
        .element_ref(&anchor)
        .absolute()
        .h(px(28.0))
        .top(px(12.0))
        .right(px(56.0))
        .z_index(1)
        .border(px(1.0))
        .border_color(divider)
        .bg(frame)
        .disabled(move || {
            state.pending_mode.get().is_some() || state.gate.busy() || state.controls.open()
        })
        .on_press(move |_: &()| {
            shell.second_active.set(second);
            shell.show_format_menu(doc.clone(), anchor);
        })
}

fn groups(s: Rc<Shell>) -> AnyElement {
    let down = s.split_down.get();
    let primary = s.clone();
    let second = s.clone();
    (if down { view() } else { view().flex_row() })
        .flex_1()
        .min_w(px(0.0))
        .child(dynamic(move || {
            primary.primary().map(|doc| {
                if let Some(large) = &doc.large {
                    large_file::viewer(primary.clone(), doc.clone(), large.clone()).into_element()
                } else {
                    editor(primary.clone(), doc, false).into_element()
                }
            })
        }))
        .child(dynamic(move || {
            second
                .split
                .get()
                .map(|doc| editor(second.clone(), doc, true))
        }))
        .into_element()
}

pub fn view_shell() -> impl IntoElement {
    render_shell(Shell::new())
}

fn title_menu_button(
    label: &'static str,
    shell: Rc<Shell>,
    group: Option<MenuGroup>,
) -> impl IntoElement {
    view()
        .w(px(TITLE_MENU_WIDTH))
        .h(px(32.0))
        .items_center()
        .justify_center()
        .app_class("rounded-sm hover:bg-accent")
        .text_size(px(14.0))
        .focusable(false)
        .window_drag(false)
        .role(AccessibilityRole::Button)
        .child(text(label))
        .on_press(move |_: &()| shell.show_title_menu(group))
}

fn title_bar(shell: Rc<Shell>) -> impl IntoElement {
    let window = shell.window.clone();
    let search = shell.clone();
    let width = shell.clone();
    let panel = shell.panel;
    let menus = shell.clone();
    let icon = ImageSrc::rgba(
        32,
        32,
        include_bytes!(concat!(env!("OUT_DIR"), "/app-icon.rgba")),
    );
    view()
        .flex_row()
        .items_center()
        .h(px(TITLE_BAR_HEIGHT))
        .flex_shrink_0()
        .pr(move || window.title_bar().right)
        .bg(frame)
        .border_b(px(1.0))
        .border_color(divider)
        .window_drag(true)
        .child(
            view()
                .w(px(TITLE_ICON_WIDTH))
                .items_center()
                .justify_center()
                .child(image(icon).w(px(20.0)).h(px(20.0))),
        )
        .child(dynamic(move || {
            let mut row = view().flex_row().flex_shrink_0();
            if menus.compact_menu() {
                row = row.child(title_menu_button("菜单", menus.clone(), None));
            } else {
                for group in MenuGroup::ALL {
                    let label = group.label().split_once('(').unwrap().0;
                    row = row.child(title_menu_button(label, menus.clone(), Some(group)));
                }
            }
            row
        }))
        .child(
            view()
                .w(move || width.title_search_width())
                .min_w(px(0.0))
                .ml(px(TITLE_SEARCH_GAP))
                .h(px(32.0))
                .window_drag(false)
                .child(when(
                    move || !is_quick_panel(&panel.get()),
                    move || {
                        let search = search.clone();
                        view()
                            .size_full()
                            .px(px(8.0))
                            .flex_row()
                            .gap(px(6.0))
                            .items_center()
                            .overflow_hidden()
                            .app_class("rounded-sm hover:bg-accent focus-visible:bg-accent")
                            .border(px(1.0))
                            .border_color(divider)
                            .focusable(true)
                            .role(AccessibilityRole::Button)
                            .label("搜索文件或命令")
                            .child(shell_glyph(SEARCH_ICON, 16.0))
                            .child(
                                text("搜索文件或命令…")
                                    .min_w(px(0.0))
                                    .number_of_lines(1)
                                    .text_overflow(EllipsizeMode::Tail)
                                    .text_size(px(14.0)),
                            )
                            .on_press(move |_: &()| search.run("texzi.navigation.quickOpen"))
                    },
                )),
        )
        .child(view().flex_1().min_w(px(18.0)).h_full())
}

fn activity_bar(s: Rc<Shell>) -> impl IntoElement {
    let files = s.clone();
    let search = s.clone();
    view()
        .w(px(44.0))
        .flex_shrink_0()
        .items_center()
        .bg(frame)
        .border_r(px(1.0))
        .border_color(divider)
        .child(activity_icon(
            FILES_ICON,
            "资源管理器",
            {
                let s = s.clone();
                move || s.sidebar.get() && s.panel.get() != "folder"
            },
            move || {
                if files.panel.get() == "folder" {
                    files.close_panel();
                    files.sidebar.set(true);
                } else {
                    files.run("texzi.sidebar.toggle");
                }
            },
        ))
        .child(activity_icon(
            SEARCH_ICON,
            "在文件夹中搜索",
            move || s.panel.get() == "folder",
            move || search.run("texzi.find.inFolder"),
        ))
}

fn activity_icon(
    path: &'static str,
    label: &'static str,
    selected: impl Fn() -> bool + Clone + 'static,
    activate: impl Fn() + 'static,
) -> impl IntoElement {
    let surface = selected.clone();
    view()
        .relative()
        .w(px(44.0))
        .h(px(44.0))
        .items_center()
        .justify_center()
        .child(
            icon_button(path, label, 36.0, 20.0)
                .bg(move || {
                    if surface() {
                        selected_surface()
                    } else {
                        frame()
                    }
                })
                .selected(selected.clone())
                .on_press(move |_: &()| activate()),
        )
        .child(when(selected, || {
            view()
                .absolute()
                .left(px(0.0))
                .top(px(8.0))
                .bottom(px(8.0))
                .w(px(2.0))
                .bg(indicator)
        }))
}

const FOLDER_ICON: &str = "M3 6h7l2 3h9v10H3z";
const SPLIT_ICON: &str = "M3 5h18v14H3zM12 5v14";
const FILES_ICON: &str = "M7 5V3h8l4 4v12h-2M15 3v5h4M5 7h10v14H5z";
const SEARCH_ICON: &str = "M15.5 15.5L21 21M17.5 10.5a7 7 0 1 1-14 0 7 7 0 0 1 14 0";
const CLOSE_ICON: &str = "M6 6l12 12M18 6L6 18";
const FORMAT_ICON: &str = "M3 18l5-12 5 12M5 14h6M16 8h5M16 12h5M16 16h5";
const SOURCE_ICON: &str = "M4 6l6 5-6 5M12 17h8";
const ADD_ICON: &str = "M5 12h14M12 5v14";

fn shell_glyph(path: &str, size: f32) -> impl IntoElement {
    let source = ImageSrc::svg_markup(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><path d="{path}" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>"#
    ));
    svg(source)
        .w(px(size))
        .h(px(size))
        .flex_shrink_0()
        .text_color(foreground)
}

fn icon_button(
    path: &str,
    label: &'static str,
    target: f32,
    glyph: f32,
) -> nsh_base::ButtonElement {
    nsh_base::button()
        .w(px(target))
        .h(px(target))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .app_class("rounded-sm hover:bg-accent focus-visible:bg-accent")
        .accessibility_label(label)
        .child(shell_glyph(path, glyph))
}

fn file_sidebar(s: Rc<Shell>) -> impl IntoElement {
    let width = s.sidebar_width;
    let start = s.sidebar_drag_start;
    let window = s.window.clone();
    let max_width = move || {
        window.state().map_or(560.0, |state| {
            (state.content_bounds.size.width.0 - 244.0).clamp(160.0, 560.0)
        })
    };
    let layout_max = max_width.clone();
    let drag_max = max_width.clone();
    view()
        .relative()
        .w(move || px(width.get().min(layout_max())))
        .flex_shrink_0()
        .bg(frame)
        .border_r(px(1.0))
        .border_color(divider)
        .child(
            view()
                .h(px(38.0))
                .px(px(8.0))
                .flex_shrink_0()
                .justify_center()
                .border_b(px(1.0))
                .border_color(divider)
                .child(text("资源管理器")),
        )
        .child(
            view()
                .flex_1()
                .min_h(px(0.0))
                .p(px(8.0))
                .gap(px(8.0))
                .child(button("打开文件夹", s.clone(), "texzi.folder.open"))
                .child(dynamic(move || folder(s.clone()))),
        )
        .child(
            nsh_base::resize_handle()
                .edge(nsh_base::HandleEdge::Trailing)
                .accessibility_label("调整资源管理器宽度")
                .on_drag_start(move || start.set(width.get_untracked().min(max_width())))
                .on_drag_move(move |offset: &Points| {
                    width.set((start.get_untracked() + offset.0).clamp(160.0, drag_max()));
                }),
        )
}

fn tab_bar(s: Rc<Shell>) -> impl IntoElement {
    let docs = s.docs;
    let active = s.active;
    let new_tab = s.clone();
    let split = s.split;
    let split_tab = s.clone();
    view()
        .h(px(38.0))
        .flex_shrink_0()
        .items_center()
        .flex_row()
        .bg(frame)
        .border_b(px(1.0))
        .border_color(divider)
        .child(
            scroll()
                .horizontal()
                .scrollbar(false)
                .flex_1()
                .min_w(px(0.0))
                .child(view().flex_row().child(each(
                    move || docs.get(),
                    |doc: &Rc<Document>| doc.id,
                    move |doc, _| {
                        let doc = doc.get_untracked();
                        let tab = s.clone();
                        let close = s.clone();
                        let id = doc.id;
                        let label = doc.clone();
                        view()
                            .relative()
                            .h(px(38.0))
                            .min_w(px(104.0))
                            .max_w(px(220.0))
                            .pr(px(6.0))
                            .flex_row()
                            .items_center()
                            .bg(move || {
                                if active.get() == id {
                                    background()
                                } else {
                                    frame()
                                }
                            })
                            .border_r(px(1.0))
                            .border_color(divider)
                            .child(
                                nsh_base::button()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .h_full()
                                    .px(px(10.0))
                                    .flex_row()
                                    .items_center()
                                    .gap(px(6.0))
                                    .app_class("hover:bg-accent focus-visible:bg-accent")
                                    .selected(move || active.get() == id)
                                    .child(
                                        text(move || label.name())
                                            .min_w(px(0.0))
                                            .number_of_lines(1)
                                            .text_overflow(EllipsizeMode::Tail),
                                    )
                                    .child(when(
                                        move || {
                                            doc.source.track();
                                            doc.source.is_dirty().unwrap_or(true)
                                        },
                                        || {
                                            view()
                                                .w(px(6.0))
                                                .h(px(6.0))
                                                .flex_shrink_0()
                                                .app_class("rounded-full")
                                                .bg(indicator)
                                        },
                                    ))
                                    .on_press(move |_: &()| {
                                        tab.active.set(id);
                                        tab.second_active.set(false);
                                    }),
                            )
                            .child(
                                icon_button(CLOSE_ICON, "关闭此标签", 24.0, 14.0)
                                    .on_press(move |_: &()| close.close_tab(id)),
                            )
                            .child(when(
                                move || active.get() == id,
                                || {
                                    view()
                                        .absolute()
                                        .bottom(px(0.0))
                                        .left(px(0.0))
                                        .right(px(0.0))
                                        .h(px(2.0))
                                        .bg(indicator)
                                },
                            ))
                    },
                ))),
        )
        .child(
            icon_button(ADD_ICON, "新建标签", 32.0, 16.0)
                .ml(px(4.0))
                .on_press(move |_: &()| new_tab.run("texzi.file.new")),
        )
        .child(when(
            move || split.get().is_none(),
            move || {
                let split_tab = split_tab.clone();
                icon_button(SPLIT_ICON, "向右拆分编辑组", 32.0, 16.0)
                    .mr(px(4.0))
                    .on_press(move |_: &()| split_tab.run("texzi.group.splitRight"))
            },
        ))
}

fn render_shell(s: Rc<Shell>) -> impl IntoElement {
    let toolbar = s.clone();
    let tabs = s.clone();
    let side = s.clone();
    let activity = s.clone();
    let content = s.clone();
    let panels = s.clone();
    let quick = s.clone();
    let status_shell = s.clone();
    let drop_shell = s.clone();
    let zen = s.zen;
    let exit_zen = s.clone();
    let zen_window = s.window.clone();
    let zen_armed = s.zen_exit.armed;
    view()
        .relative()
        .size_full()
        .bg(background)
        .text_color(foreground)
        .on_file_drop(move |event: &FileDropEvent| drop_shell.open_dropped(&event.uris))
        .child(when(move || !zen.get(), move || title_bar(toolbar.clone())))
        .child(when(
            move || zen.get(),
            move || {
                let exit = exit_zen.clone();
                let window = zen_window.clone();
                view()
                    .group_named("zen-exit")
                    .h(px(TITLE_BAR_HEIGHT))
                    .w_full()
                    .flex_row()
                    .pr(move || window.title_bar().right)
                    .justify_end()
                    .items_center()
                    .child(view().flex_1().h_full().window_drag(true))
                    .child(text(move || {
                        if zen_armed.get() {
                            "再按一次 Esc 退出 Zen"
                        } else {
                            ""
                        }
                    }))
                    .child(
                        nsh_base::button()
                            .opacity(0.0)
                            .group_hover_named("zen-exit", |style| style.opacity(1.0))
                            .group_focus_within_named("zen-exit", |style| style.opacity(1.0))
                            .child(text("退出 Zen"))
                            .on_press(move |_: &()| {
                                exit.run("texzi.view.exitZen");
                                exit.restore_editor_focus();
                            }),
                    )
            },
        ))
        .child(
            view()
                .flex_row()
                .flex_1()
                .child(when(
                    move || !zen.get(),
                    move || activity_bar(activity.clone()),
                ))
                .child(when(
                    {
                        let s = side.clone();
                        move || s.sidebar.get() && !zen.get()
                    },
                    move || file_sidebar(side.clone()),
                ))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(when(move || !zen.get(), move || tab_bar(tabs.clone())))
                        .child(dynamic(move || groups(content.clone())))
                        .child(dynamic(move || {
                            (!is_quick_panel(&panels.panel.get()))
                                .then(|| panel(panels.clone()))
                                .flatten()
                        })),
                ),
        )
        .child(when(
            move || !zen.get(),
            move || status_bar(status_shell.clone()),
        ))
        .child(dynamic(move || {
            let mode = quick.panel.get();
            if !is_quick_panel(&mode) {
                return None;
            }
            let palette = quick_panel(quick.clone(), mode == "files");
            Some(if zen.get() {
                view()
                    .absolute()
                    .top(px(6.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .z_index(1)
                    .flex_row()
                    .justify_center()
                    .child(view().w(px(460.0)).max_w(relative(1.0)).child(palette))
                    .into_element()
            } else {
                view()
                    .absolute()
                    .top(px(6.0))
                    .left(px(quick.title_search_left()))
                    .z_index(1)
                    .w(quick.title_search_width())
                    .window_drag(false)
                    .child(palette)
                    .into_element()
            })
        }))
}

fn status_bar(shell: Rc<Shell>) -> impl IntoElement {
    let notice = shell.notice;
    let document = shell.clone();
    view()
        .h(px(24.0))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(8.0))
        .bg(frame)
        .border_t(px(1.0))
        .border_color(divider)
        .text_color(secondary_foreground)
        .text_size(px(12.0))
        .child(
            text(move || {
                let message = notice.get();
                let dirty = document.current().is_some_and(|doc| {
                    doc.source.track();
                    doc.source.is_dirty().unwrap_or(true)
                });
                if dirty {
                    if message.is_empty() || message.starts_with("已保存") {
                        "未保存".into()
                    } else {
                        format!("{message} · 未保存")
                    }
                } else if message.is_empty() {
                    "就绪".into()
                } else {
                    message
                }
            })
            .flex_1()
            .min_w(px(0.0))
            .number_of_lines(1)
            .text_overflow(EllipsizeMode::Tail),
        )
        .child(dynamic(move || {
            let doc = shell.current()?;
            if let Some(large) = &doc.large {
                return Some(view().child(text(large.status())).into_element());
            }
            let rich = doc.displayed_rich();
            let markdown = doc.is_markdown();
            let position = if rich {
                doc.gate.rich_cursor_status()
            } else {
                doc.source.cursor_status()
            };
            let (encoding, newline) = doc.source.file_status()?;
            let language = if rich {
                "Markdown · 富文本"
            } else if markdown {
                "Markdown · 源码"
            } else {
                "纯文本"
            };
            let mut fields = view()
                .flex_row()
                .items_center()
                .min_w(px(0.0))
                .gap(px(12.0));
            if let Some(position) = position {
                fields = fields.child(text(position).number_of_lines(1));
            }
            if !rich {
                if doc.source.disk_len() > large_file::SOURCE_ONLY_BYTES {
                    fields = fields.child(text("大文件 · 无高亮").number_of_lines(1));
                }
                fields = fields.child(
                    text(format!("空格：{}", editor_tab_width(markdown))).number_of_lines(1),
                );
            }
            Some(
                fields
                    .child(text(encoding).number_of_lines(1))
                    .child(text(newline).number_of_lines(1))
                    .child(text(language).number_of_lines(1))
                    .into_element(),
            )
        }))
}

fn is_quick_panel(mode: &str) -> bool {
    matches!(mode, "files" | "commands")
}

fn tree_rows(
    root: &Path,
    expanded: &[PathBuf],
    depth: usize,
    rows: &mut Vec<(PathBuf, usize, bool)>,
    errors: &mut Vec<String>,
) {
    if depth > 24 || rows.len() >= 1000 {
        errors.push("已达到目录显示预算；可关闭部分目录后重试".into());
        return;
    }
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(e) => {
            errors.push(format!("{}：{e}", root.display()));
            return;
        }
    };
    let mut entries = entries
        .filter_map(|entry| match entry {
            Ok(e) => Some(e),
            Err(e) => {
                errors.push(e.to_string());
                None
            }
        })
        .take(1001)
        .collect::<Vec<_>>();
    entries.sort_by_key(|e| {
        (
            !e.file_type().is_ok_and(|t| t.is_dir()),
            e.file_name().to_string_lossy().to_ascii_lowercase(),
        )
    });
    for entry in entries {
        if rows.len() >= 1000 {
            errors.push("已显示 1000 项；其余未加载".into());
            break;
        }
        let path = entry.path();
        let directory = entry.file_type().is_ok_and(|t| t.is_dir());
        rows.push((path.clone(), depth, directory));
        if directory && expanded.contains(&path) {
            if std::fs::symlink_metadata(&path)
                .is_ok_and(|m| m.file_type().is_symlink() || m.file_attributes() & 0x400 != 0)
            {
                errors.push(format!("{}：链接或重解析点未展开", path.display()));
            } else {
                tree_rows(&path, expanded, depth + 1, rows, errors);
            }
        }
    }
}
fn folder(s: Rc<Shell>) -> AnyElement {
    let Some(root) = s.root.get() else {
        return text("未打开文件夹").into_element();
    };
    let expanded = s.expanded.get();
    let mut rows = vec![];
    let mut errors = vec![];
    tree_rows(&root, &expanded, 0, &mut rows, &mut errors);
    let root_path = display_path(&root);
    let root_label = root.file_name().map_or_else(
        || root_path.clone(),
        |name| name.to_string_lossy().into_owned(),
    );
    scroll()
        .flex_1()
        .child(
            view()
                .child(
                    view()
                        .h(px(28.0))
                        .flex_row()
                        .gap(px(6.0))
                        .items_center()
                        .label(root_path)
                        .child(shell_glyph(FOLDER_ICON, 16.0))
                        .child(
                            text(root_label)
                                .min_w(px(0.0))
                                .number_of_lines(1)
                                .text_overflow(EllipsizeMode::Tail),
                        ),
                )
                .child((!errors.is_empty()).then(|| text(errors.join("\n"))))
                .child(each(
                    move || rows.clone(),
                    |r: &(PathBuf, usize, bool)| r.0.clone(),
                    move |row, _| {
                        let (path, depth, directory) = row.get_untracked();
                        let label = format!(
                            "{}{}",
                            if directory {
                                if expanded.contains(&path) {
                                    "▾ "
                                } else {
                                    "▸ "
                                }
                            } else {
                                ""
                            },
                            path.file_name().unwrap_or_default().to_string_lossy()
                        );
                        let s = s.clone();
                        let highlighted = s.clone();
                        let row_path = path.clone();
                        nsh_base::button()
                            .w_full()
                            .justify_start()
                            .pl(px(8.0 + depth as f32 * 14.0))
                            .bg(move || {
                                if highlighted.current().is_some_and(|d| {
                                    d.path.with(|p| {
                                        p.as_ref().is_some_and(|current| {
                                            current
                                                .as_os_str()
                                                .eq_ignore_ascii_case(row_path.as_os_str())
                                        })
                                    })
                                }) {
                                    selected_surface()
                                } else {
                                    frame()
                                }
                            })
                            .child(text(label))
                            .on_press(move |_: &()| {
                                if directory {
                                    s.expanded.update(|es| {
                                        if let Some(at) = es.iter().position(|p| p == &path) {
                                            es.remove(at);
                                        } else {
                                            es.push(path.clone());
                                        }
                                    });
                                } else {
                                    s.open(&path);
                                }
                            })
                    },
                )),
        )
        .into_element()
}

fn quick_panel(s: Rc<Shell>, files: bool) -> impl IntoElement {
    let query = s.palette_query;
    let markdown = if !files {
        s.current()
            .map(|doc| crate::rich_commands::availability_all(&doc.gate, &doc.source, true))
    } else {
        None
    };
    let mut quick_paths: Vec<PathBuf> = vec![];
    let entries = if !files {
        crate::native_menu::commands()
            .into_iter()
            .filter_map(|c| {
                // One immutable source projection per palette build, not
                // one full parse for every Markdown command row.
                let blocked = markdown
                    .as_ref()
                    .and_then(|results| results.get(&c.id))
                    .map_or_else(|| s.blocked(&c.id).is_some(), Result::is_err);
                if blocked {
                    return None;
                }
                let keys = if c.keys.is_empty() {
                    String::new()
                } else {
                    format!("    {}", c.keys.join(" / "))
                };
                Some(
                    nsh_base::CommandItem::new(c.id.clone(), format!("{}{keys}", c.name))
                        .keywords([c.id])
                        .group(c.group),
                )
            })
            .collect::<Vec<_>>()
    } else {
        let docs = s.docs.get();
        let mut known = docs.iter().filter_map(|d| d.path.get()).collect::<Vec<_>>();
        let mut entries = docs
            .iter()
            .map(|d| nsh_base::CommandItem::new(format!("file:{}", d.id), d.name()).group("已打开"))
            .collect::<Vec<_>>();
        let mut add = |path: PathBuf, display: String, group: &'static str| {
            if known
                .iter()
                .any(|old| old.as_os_str().eq_ignore_ascii_case(path.as_os_str()))
            {
                return;
            }
            known.push(path.clone());
            let id = format!("quick:{}", quick_paths.len());
            let keyword = path.to_string_lossy().into_owned();
            quick_paths.push(path);
            entries.push(
                nsh_base::CommandItem::new(id, display)
                    .keywords([keyword])
                    .group(group),
            );
        };
        for path in s.recent.get() {
            add(path.clone(), path.display().to_string(), "最近打开");
        }
        if s.quick_origin.get() == s.root.get() {
            for file in s.quick_files.get() {
                add(file.path, file.display, "当前文件夹");
            }
        }
        entries
    };
    let run = s.clone();
    let cancel = s.clone();
    let search = s.clone();
    let notice = s.quick_notice;
    let scanning = s.quick_scanning;
    let stop = s.clone();
    view()
        .bg(frame)
        .border(px(1.0))
        .border_color(divider)
        .child(
            nsh_base::command()
                .items(entries)
                .query(move || Some(query.get().into()))
                .placeholder(if files {
                    "搜索文件或命令…（> 查找命令）"
                } else {
                    "搜索命令名称或 ID"
                })
                .auto_focus(true)
                .search_class("h-8")
                .search_accessibility_label("搜索文件或命令")
                .list_class("max-h-64")
                .on_query_change(move |q: &SharedString| {
                    if let Some(command) = files.then(|| q.strip_prefix('>')).flatten() {
                        query.set(command.trim_start().to_owned());
                        search.panel.set("commands".into());
                    } else {
                        query.set(q.to_string());
                    }
                })
                .on_select(move |id: &SharedString| {
                    run.cancel_quick(false);
                    run.panel.set(String::new());
                    if let Some(id) = id.strip_prefix("file:").and_then(|s| s.parse::<u64>().ok()) {
                        run.active.set(id);
                        run.second_active.set(false);
                        run.restore_editor_focus();
                    } else if let Some(path) = id
                        .strip_prefix("quick:")
                        .and_then(|s| s.parse::<usize>().ok())
                        .and_then(|i| quick_paths.get(i))
                    {
                        run.open(path);
                        run.restore_editor_focus();
                    } else {
                        run.run(id);
                        if run.panel.get().is_empty()
                            && !run.modal.get()
                            && !run.current().is_some_and(|d| d.controls.open())
                        {
                            run.restore_editor_focus();
                        }
                    }
                })
                .on_cancel(move |_: &()| cancel.close_panel()),
        )
        .child(when(
            move || files,
            move || {
                let stop = stop.clone();
                view()
                    .p(px(8.0))
                    .border_t(px(1.0))
                    .border_color(divider)
                    .child(text(move || notice.get()))
                    .child(when(scanning, move || {
                        let stop = stop.clone();
                        nsh_base::button()
                            .child(text("取消目录扫描"))
                            .on_press(move |_: &()| stop.cancel_quick(true))
                    }))
            },
        ))
}

fn panel(s: Rc<Shell>) -> Option<AnyElement> {
    let mode = s.panel.get();
    if mode.is_empty() || is_quick_panel(&mode) {
        return None;
    }
    let query = s.query;
    let replacement = s.replacement;
    let close = s.clone();
    let mut panel = view().p(px(8.0)).gap(px(8.0)).bg(frame).child(
        nsh_base::button()
            .child(text("关闭面板（Esc）"))
            .on_press(move |_: &()| close.close_panel()),
    );
    if mode == "groups" {
        let first = s.clone();
        let second = s.clone();
        let exists = s.split.get().is_some();
        panel = panel
            .child(
                nsh_base::button()
                    .child(text("编辑组 1"))
                    .on_press(move |_: &()| {
                        first.focus_group(1);
                        first.panel.set(String::new());
                    }),
            )
            .child(
                nsh_base::button()
                    .child(text(if exists {
                        "编辑组 2"
                    } else {
                        "编辑组 2（不存在）"
                    }))
                    .disabled(!exists)
                    .on_press(move |_: &()| {
                        second.focus_group(2);
                        second.panel.set(String::new());
                    }),
            );
    } else if mode == "folder" {
        let search = s.clone();
        let submit = s.clone();
        let cancel = s.clone();
        let status = s.search_notice;
        let results = s.clone();
        let hits = s.hits;
        panel = panel
            .child(text("当前根目录 · UTF-8 普通文字 · 区分大小写"))
            .child(
                input()
                    .value(move || query.get())
                    .placeholder("文件夹搜索文字")
                    .on_change(move |v: &str| query.set(v.into()))
                    .auto_focus(true)
                    .on_submit(move |_: &str| submit.search_folder()),
            )
            .child(
                nsh_base::button()
                    .child(text("开始搜索"))
                    .on_press(move |_: &()| search.search_folder()),
            )
            .child(
                nsh_base::button()
                    .child(text("取消搜索"))
                    .on_press(move |_: &()| {
                        if let Some(c) = cancel.search_cancel.borrow().as_ref() {
                            c.store(true, Ordering::Relaxed);
                        }
                    }),
            )
            .child(text(move || status.get()))
            .child(scroll().h(px(180.0)).child(each(
                move || hits.get(),
                |h: &SearchHit| (h.path.clone(), h.line),
                move |hit, _| {
                    let hit = hit.get_untracked();
                    let s = results.clone();
                    nsh_base::button()
                        .child(text(format!(
                            "{}:{}  {}",
                            hit.path.display(),
                            hit.line,
                            hit.text
                        )))
                        .on_press(move |_: &()| {
                            s.open(&hit.path);
                            let target = hit.path.canonicalize().ok();
                            if let Some(doc) = s.current().filter(|d| d.path.get() == target) {
                                doc.gate.to_source();
                                if !doc.gate.rich_mode.get() {
                                    if let Err(e) = doc.source.go_to_line(hit.line) {
                                        s.notice.set(e);
                                    }
                                }
                            }
                        })
                },
            )));
    } else if mode == "line" {
        let run = s.clone();
        panel = panel
            .child(text("源码行或行:列（从 1 开始；列按字素簇，Tab 算 1 列）"))
            .child(
                input()
                    .value(move || query.get())
                    .on_change(move |v: &str| query.set(v.into()))
                    .auto_focus(true)
                    .on_submit(move |_: &str| {
                        let result = run
                            .current()
                            .ok_or("没有活动文档".to_owned())
                            .and_then(|d| {
                                let (line, column) = line_column(&query.get())?;
                                d.gate.to_source();
                                if d.gate.rich_mode.get() {
                                    return Err(d.gate.status.get());
                                }
                                d.source.go_to_line_column(line, column)?;
                                Ok(d)
                            });
                        match result {
                            Ok(doc) => {
                                let second = run.second_active.get();
                                run.close_panel();
                                run.notice.set("已定位源码行列".into());
                                let shell = run.clone();
                                run.owner.run(|| {
                                    spawn(async move {
                                        // The native editor consumes the external selection at commit.
                                        let _ = next_frame().await;
                                        if shell.second_active.get() == second
                                            && shell
                                                .current()
                                                .is_some_and(|current| Rc::ptr_eq(&current, &doc))
                                            && !doc.displayed_rich()
                                            && let Some(api) = doc.reference.api::<EditorApi>()
                                        {
                                            api.reveal_primary_caret(None);
                                        }
                                    })
                                });
                            }
                            Err(e) => run.notice.set(e),
                        }
                    }),
            );
    } else {
        let find = s.clone();
        let query_composing = signal(false);
        let replacement_composing = signal(false);
        let scope = if s.current().is_some_and(|d| d.gate.rich_mode.get()) {
            "正文"
        } else {
            "源码"
        };
        panel = panel
            .child(text(format!("{scope} · 普通文字 · 区分大小写")))
            .child(
                input()
                    .value(move || query.get())
                    .placeholder("查找文字")
                    .on_change(move |v: &str| query.set(v.into()))
                    .on_composition_change(move |event: &Composition| {
                        query_composing.set(event.composing)
                    })
                    .auto_focus(true)
                    .on_submit(move |_: &str| find.find(false)),
            )
            .child(
                view()
                    .flex_row()
                    .gap(px(8.0))
                    .child(button("上一个", s.clone(), "texzi.find.previous"))
                    .child(button("下一个", s.clone(), "texzi.find.next")),
            );
        if mode == "replace" {
            let escape = s.clone();
            effect(move || {
                if escape.chord.get() {
                    return;
                }
                let escape = escape.clone();
                // Windows treats Escape as a text key. Keep receiving it while
                // the panel is open, and let native input cancel composition.
                on_key_with(
                    "escape",
                    KeyOptions {
                        when: nsh_core::keymap::When::Always,
                        pass_through: true,
                        ..Default::default()
                    },
                    move |_| {
                        if escape.panel.get() == "replace"
                            && !escape.chord.get()
                            && !escape.escape_blocked()
                            && !query_composing.get()
                            && !replacement_composing.get()
                        {
                            escape.close_panel();
                        }
                    },
                );
            });
            panel = panel
                .child(
                    input()
                        .value(move || replacement.get())
                        .placeholder("替换为")
                        .on_composition_change(move |event: &Composition| {
                            replacement_composing.set(event.composing)
                        })
                        .on_change(move |v: &str| replacement.set(v.into())),
                )
                .child(
                    view()
                        .flex_row()
                        .gap(px(8.0))
                        .child(button("替换当前", s.clone(), "texzi.find.replaceOne"))
                        .child(button("全部替换", s, "texzi.find.replaceAll")),
                );
        }
    }
    Some(panel.into_element())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chord_strokes_are_individual_valid_keystrokes() {
        nsh_core::keymap::Keystroke::parse_with("mod+k", false).unwrap();
        for (stroke, _) in CHORDS {
            assert!(!stroke.contains(' '));
            nsh_core::keymap::Keystroke::parse_with(stroke, false).unwrap();
        }
    }
    #[test]
    fn directory_search_reports_real_file_hits_and_honors_cancellation() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/app_shell.rs");
        let report = scan_folder(
            path.clone(),
            "fn scan_folder".into(),
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(report.scanned, 1);
        assert!(
            report
                .hits
                .iter()
                .any(|hit| hit.path == path && hit.line > 1)
        );
        assert!(!report.cancelled);
        let report = scan_folder(
            path,
            "fn scan_folder".into(),
            Arc::new(AtomicBool::new(true)),
        );
        assert!(report.cancelled);
        assert!(report.hits.is_empty());
        assert_eq!(report.scanned, 0);
    }
}
