//! Application shell. Native menus and the palette share command identities.
mod draft_close;
mod large_file;
mod splitter;
use crate::editor_owner::SaveError;
use crate::native_menu::{MenuGroup, SHELL_SHORTCUTS};
use crate::settings;
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

#[path = "auto_save.rs"]
mod auto_save;
#[path = "minimap.rs"]
mod minimap;
#[path = "session_recovery.rs"]
mod session_recovery;
#[path = "settings_view.rs"]
mod settings_view;

use crate::origami::{CONTROL_STATES, border_width, color, current_color, font_size, metric};
const TITLE_ICON_WIDTH: f32 = 42.0;
const TITLE_MENU_WIDTH: f32 = 50.0;
const TITLE_SEARCH_GAP: f32 = 10.0;
const TITLE_DRAG_MIN: f32 = 18.0;

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

fn save_path_dialog(original: Option<&Path>, save_as: bool) -> SaveDialog {
    let mut dialog = SaveDialog::new().title(if save_as || original.is_some() {
        "另存为新文件"
    } else {
        "保存新文件"
    });
    if let Some(path) = original {
        if let Some(parent) = path.parent() {
            dialog = dialog.directory(parent);
        }
        if let Some(stem) = path.file_stem() {
            let extension = path
                .extension()
                .map(|ext| format!(".{}", ext.to_string_lossy()))
                .unwrap_or_default();
            dialog = dialog.file_name(format!("{}-副本{extension}", stem.to_string_lossy()));
        }
    }
    dialog
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
    wrap: Signal<Option<bool>>,
    tab_size: Signal<Option<u32>>,
    retained_scroll: Signal<Option<(bool, f32, f32)>>,
    auto_save: Rc<auto_save::State>,
    pending_mode: Signal<Option<bool>>,
    import_complete: Cell<bool>,
    progress: ElementRef,
    body: ElementRef,
}
impl Document {
    fn editable_body_ready(&self) -> bool {
        self.large.is_none()
    }

    fn needs_close_confirmation(&self) -> bool {
        self.large.as_ref().map_or_else(
            || self.source.is_dirty().unwrap_or(true),
            |large| large.restored_draft.get().is_some(),
        )
    }

    fn source_wrap_limited(&self) -> bool {
        self.source.track();
        self.source.disk_len() > large_file::SOURCE_ONLY_BYTES
            || self
                .source
                .current_body()
                .is_some_and(|body| body.len_bytes() as u64 > large_file::SOURCE_ONLY_BYTES)
    }

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsTab {
    Closed,
    Inactive,
    Active,
}

struct Shell {
    owner: Owner,
    window: WindowRef,
    docs: Signal<Vec<Rc<Document>>>,
    active: Signal<u64>,
    settings: Rc<settings::Settings>,
    settings_view: Rc<settings_view::SettingsView>,
    settings_tab: Signal<SettingsTab>,
    session: RefCell<Option<Rc<session_recovery::Session>>>,
    recovery_split: RefCell<Option<(u64, session_recovery::Split)>>,
    next: Cell<u64>,
    closed: RefCell<Vec<ClosedTab>>,
    imports: large_file::ImportQueue,
    root: Signal<Option<PathBuf>>,
    sidebar: Signal<bool>,
    sidebar_width: Signal<f32>,
    compact_drawer: Signal<bool>,
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
    draft_close_focus_epoch: Cell<u64>,
    auto_saving: Signal<Option<u64>>,
    split: Signal<Option<Rc<Document>>>,
    split_down: Signal<bool>,
    split_share: Signal<f32>,
    second_active: Signal<bool>,
    expanded: Signal<Vec<PathBuf>>,
    hits: Signal<Vec<SearchHit>>,
    search_query: Signal<String>,
    search_origin: Signal<Option<(PathBuf, String)>>,
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
    current_color("paper-subtle")
}

fn secondary_foreground() -> Hsla {
    current_color("muted-readable")
}

fn divider() -> Hsla {
    current_color("chrome-edge")
}

fn indicator() -> Hsla {
    current_color("iris")
}

fn selected_surface() -> Hsla {
    current_color("selected-bg")
}

impl Shell {
    fn primary(&self) -> Option<Rc<Document>> {
        self.docs
            .with(|ds| ds.iter().find(|d| d.id == self.active.get()).cloned())
    }
    fn current(&self) -> Option<Rc<Document>> {
        if self.settings_active() {
            return None;
        }
        if self.second_active.get() {
            self.split.get().or_else(|| self.primary())
        } else {
            self.primary()
        }
    }

    fn settings_active(&self) -> bool {
        self.settings_tab.get() == SettingsTab::Active
    }

    fn show_settings(self: &Rc<Self>) {
        if self.modal.get() || self.current().is_some_and(|doc| doc.gate.busy()) {
            return;
        }
        self.zen_exit.reset();
        self.cancel_quick(false);
        self.panel.set(String::new());
        self.leave_editor_document(None);
        self.settings_tab.set(SettingsTab::Active);
        self.restore_editor_focus();
    }

    fn dismiss_settings(self: &Rc<Self>, close: bool) {
        let active = self.settings_active();
        self.settings_tab.set(if close {
            SettingsTab::Closed
        } else {
            SettingsTab::Inactive
        });
        if active {
            self.restore_editor_focus();
        }
    }

    fn show_documents(&self) {
        if self.settings_active() {
            self.settings_tab.set(SettingsTab::Inactive);
        }
    }

    fn activate_primary(&self, id: u64) {
        self.compact_drawer.set(false);
        self.leave_editor_document(Some(id));
        if let Some(previous) = self.primary().filter(|previous| previous.id != id) {
            session_recovery::keep_scroll(&previous);
        }
        self.active.set(id);
        self.second_active.set(false);
        self.show_documents();
    }

    fn leave_editor_document(&self, next: Option<u64>) {
        if let Some(previous) = self.current().filter(|doc| Some(doc.id) != next) {
            previous.auto_save.leave_document();
        }
    }

    fn body_blurred(&self, doc: &Document, second: bool, rich: bool) {
        let same_document_switch = self.current().is_some_and(|current| {
            current.id == doc.id
                && (self.second_active.get() != second || current.displayed_rich() != rich)
        });
        doc.auto_save
            .blur_editor(second, rich, same_document_switch);
    }

    fn body_focused(&self, doc: &Rc<Document>, second: bool, rich: bool) {
        let shown = if second {
            self.split.get()
        } else {
            self.primary()
        };
        if !self.settings_active()
            && shown.is_some_and(|shown| Rc::ptr_eq(&shown, doc))
            && doc.displayed_rich() == rich
        {
            self.leave_editor_document(Some(doc.id));
            self.second_active.set(second);
            doc.auto_save.focus_editor(second, rich);
        }
    }

    fn leave_second_group(&self) {
        self.leave_editor_document(self.primary().map(|doc| doc.id));
        self.split.set(None);
        self.recovery_split.borrow_mut().take();
        self.second_active.set(false);
    }

    fn step_document_tab(self: &Rc<Self>, forward: bool) {
        if self.settings_active() {
            let target = self
                .docs
                .with(|docs| (if forward { docs.first() } else { docs.last() }).map(|doc| doc.id));
            if let Some(target) = target {
                self.activate_primary(target);
            }
        } else if self.second_active.get() {
            self.notice
                .set("当前编辑组只有一个标签，已保持当前组".into());
        } else {
            let target = self.docs.with(|docs| {
                let at = docs.iter().position(|doc| doc.id == self.active.get())?;
                let next = if forward { at + 1 } else { at + docs.len() - 1 };
                Some(docs[next % docs.len()].id)
            });
            if let Some(target) = target {
                self.activate_primary(target);
            }
        }
        self.restore_editor_focus();
    }

    fn preference(&self, id: &str) -> serde_json::Value {
        self.settings.effective(id, self.root.get().as_deref()).0
    }

    fn source_wrap(&self, doc: &Document) -> bool {
        !doc.source_wrap_limited() && self.source_wrap_preference(doc)
    }

    fn source_wrap_preference(&self, doc: &Document) -> bool {
        doc.wrap.get().unwrap_or_else(|| {
            let id = if doc.is_markdown() {
                settings::WRAP_MARKDOWN
            } else if doc.path.with(|path| {
                path.as_deref().is_some_and(|path| {
                    nsh_base::editor::known_language(&path.to_string_lossy()).is_some()
                })
            }) {
                settings::WRAP_CODE
            } else {
                settings::WRAP_TEXT
            };
            self.preference(id) == "on"
        })
    }

    fn source_font_size(&self) -> f32 {
        self.preference(settings::FONT_SIZE).as_u64().unwrap() as f32
    }

    fn source_font_family(&self) -> String {
        let family = self.preference(settings::FONT_FAMILY);
        settings::source_font_stack(family.as_str().unwrap())
    }

    fn zen_reading_width(&self, rich: bool) -> Points {
        let (family, size) = if rich {
            ("system-ui".to_owned(), font_size("richBody"))
        } else {
            (self.source_font_family(), self.source_font_size())
        };
        let family = nsh_core::css_font_families(&family);
        let size = size * nsh_core::element::font_scale();
        nsh_core::element::measure_text(&format!("{size}px {family}"), &"0".repeat(68))
    }

    fn tab_size(&self, doc: &Document) -> u32 {
        doc.tab_size
            .get()
            .unwrap_or_else(|| self.preference(settings::TAB_SIZE).as_u64().unwrap() as u32)
    }

    fn set_theme(&self, theme: &str) {
        if !self
            .settings
            .apply(settings::THEME, None, Some(theme.into()))
        {
            self.notice
                .set(self.settings.error.get().unwrap_or_default());
        }
    }

    fn new() -> Rc<Self> {
        let s = Rc::new(Self {
            owner: Owner::current(),
            window: current_window(),
            docs: signal(vec![]),
            active: signal(0),
            settings: Rc::new(settings::Settings::new()),
            settings_view: Rc::new(settings_view::SettingsView::new(CHORDS)),
            settings_tab: signal(SettingsTab::Closed),
            session: RefCell::new(None),
            recovery_split: RefCell::new(None),
            next: Cell::new(1),
            closed: RefCell::new(vec![]),
            imports: Default::default(),
            root: signal(None),
            sidebar: signal(true),
            sidebar_width: signal(metric("sidebar")),
            compact_drawer: signal(false),
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
            draft_close_focus_epoch: Cell::new(0),
            auto_saving: signal(None),
            split: signal(None),
            split_down: signal(false),
            split_share: signal(splitter::EQUAL_SHARE),
            second_active: signal(false),
            expanded: signal(vec![]),
            hits: signal(vec![]),
            search_query: signal(String::new()),
            search_origin: signal(None),
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
        let preferences = s.clone();
        let window_focused = computed(move || {
            preferences
                .window
                .state()
                .is_some_and(|state| state.focused)
        });
        let preferences = s.settings.clone();
        effect(move || {
            if window_focused.get() {
                untracked(|| preferences.refresh());
            }
        });
        settings::Settings::watch(&s.settings, s.owner);
        let preferences = s.settings.clone();
        let theme =
            computed(
                move || match preferences.effective(settings::THEME, None).0.as_str() {
                    Some("light") => Appearance::Light,
                    Some("dark") => Appearance::Dark,
                    _ => Appearance::System,
                },
            );
        effect(move || set_color_scheme(theme.get()));
        auto_save::install(&s);
        s.watch_draft_close_focus();
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
        if let Err(error) = session_recovery::start(&s) {
            s.notice.set(format!("无法启用会话恢复 · {error}"));
        }
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
        let encoding = self.preference(settings::DEFAULT_ENCODING);
        let eol = self.preference(settings::DEFAULT_EOL);
        let (format, eol) =
            settings::new_document_format(encoding.as_str().unwrap(), eol.as_str().unwrap());
        self.add(self.owner.run(|| NativeView::new_file(format, eol)), None);
    }
    fn content_width(&self) -> f32 {
        self.window
            .state()
            .map_or(880.0, |state| state.content_bounds.size.width.0)
    }

    fn title_room(&self) -> f32 {
        self.content_width() - self.window.title_bar().right.0
    }

    fn quick_panel_width(&self) -> Points {
        px(metric("spotWidth").min((self.content_width() - 28.0).max(0.0)))
    }

    fn compact_menu(&self) -> bool {
        self.content_width() < metric("compactBelow")
            || self.title_room()
                < TITLE_ICON_WIDTH
                    + TITLE_MENU_WIDTH * MenuGroup::ALL.len() as f32
                    + TITLE_SEARCH_GAP
                    + 64.0
    }

    fn title_menu_end(&self) -> f32 {
        let count = if self.compact_menu() {
            1
        } else {
            MenuGroup::ALL.len()
        };
        TITLE_ICON_WIDTH + TITLE_MENU_WIDTH * count as f32
    }

    fn title_search_width(&self) -> Points {
        let room = self.title_room() - self.title_menu_end() - TITLE_SEARCH_GAP - TITLE_DRAG_MIN;
        let desired = if self.compact_menu() {
            metric("field")
        } else {
            360.0
        };
        px(room.clamp(0.0, desired))
    }

    fn title_search_left(&self) -> f32 {
        let after_menu = self.title_menu_end() + TITLE_SEARCH_GAP;
        if self.compact_menu() {
            return after_menu;
        }
        let search = self.title_search_width().0;
        let centered = (self.content_width() - search) / 2.0;
        centered
            .min(self.title_room() - search - TITLE_DRAG_MIN)
            .max(after_menu)
    }

    fn drawer(&self) -> Signal<bool> {
        if self.compact_menu() {
            self.compact_drawer
        } else {
            self.sidebar
        }
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
            px(metric("title")),
        );
        self.owner.run(|| {
            let _ = popup_menu_at(entries, at);
        });
    }
    fn add(&self, source: Rc<NativeView>, path: Option<PathBuf>) {
        self.add_with_mode(source, path, None);
    }
    fn add_with_mode(
        &self,
        source: Rc<NativeView>,
        path: Option<PathBuf>,
        restored_mode: Option<bool>,
    ) -> Rc<Document> {
        let id = self.next.get();
        self.next.set(id + 1);
        let start_rich =
            self.initial_markdown_rich(path.as_deref(), source.disk_len(), restored_mode);
        let doc = self.make_document(id, source, path, None, start_rich);
        self.show_new(doc.clone());
        doc
    }
    fn initial_markdown_rich(
        &self,
        path: Option<&Path>,
        bytes: u64,
        restored: Option<bool>,
    ) -> bool {
        path.is_some_and(is_markdown)
            && bytes <= large_file::SOURCE_ONLY_BYTES
            && restored.unwrap_or_else(|| self.preference(settings::MARKDOWN_VIEW) == "wysiwyg")
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
        if start_rich {
            self.owner.run(|| gate.to_rich());
        }
        let reference = self.owner.run(component_ref::<Editor>);
        let rich_reference = self
            .owner
            .run(component_ref::<nsh_base::rich_editor::view::RichEditor>);
        let controls = self.owner.run(|| RichControlSet::new(&gate, &source));
        let wrap = self.owner.run(|| signal(None));
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
            tab_size: self.owner.run(|| signal(None)),
            retained_scroll: self.owner.run(|| signal(None)),
            auto_save: self.owner.run(|| Rc::new(auto_save::State::new())),
            pending_mode: self.owner.run(|| signal(None)),
            import_complete: Cell::new(false),
            progress: self.owner.run(element_ref),
            body: self.owner.run(element_ref),
        })
    }
    fn show_new(&self, doc: Rc<Document>) {
        let id = doc.id;
        self.docs.update(|ds| ds.push(doc));
        self.activate_primary(id);
        self.panel.set(String::new());
        self.notice.set(String::new());
    }
    fn blocked(&self, id: &str) -> Option<String> {
        if self.settings_active() && document_action(id) {
            return Some("请先切回文档".into());
        }
        if self.current().is_some_and(|doc| !doc.editable_body_ready())
            && large_file::requires_editable(id)
        {
            return Some("此标签尚未载入完整编辑模型，分页内容仅供查看和复制。".into());
        }
        if id == "texzi.editor.toggleWrap"
            && self.current().is_some_and(|doc| doc.source_wrap_limited())
        {
            return Some("大文档已暂时关闭自动换行，设置保持原值。".into());
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
        self.open_canonical_with_view(path, None).map(|_| ())
    }

    fn open_canonical_with_view(
        self: &Rc<Self>,
        path: PathBuf,
        restored_view: Option<session_recovery::EditorView>,
    ) -> Result<Rc<Document>, String> {
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
            self.activate_primary(doc.id);
            self.restore_editor_focus();
            return Ok(doc);
        }
        let file =
            texzi_document::files::paged::FileSource::open(&path).map_err(|e| e.to_string())?;
        if file.len >= LARGE_BYTES {
            return Ok(self.open_large(path, Arc::new(file), None, restored_view, None));
        }
        let prepared = match crate::editor_owner::PreparedFile::from_source(
            &file,
            LARGE_BYTES - 1,
            &AtomicBool::new(false),
            None,
        ) {
            Ok(prepared) => prepared,
            Err(crate::editor_owner::PrepareError::LineLimit(encoding)) => {
                return Ok(self.open_large(
                    path,
                    Arc::new(file),
                    Some(encoding),
                    restored_view,
                    None,
                ));
            }
            Err(e) => return Err(e.to_string()),
        };
        let source = self.owner.run(|| prepared.attach())?;
        if let Some(view) = &restored_view {
            view.restore_source(&source);
        }
        self.remember(&path);
        let doc = self.add_with_mode(
            source,
            Some(path),
            restored_view.as_ref().map(|view| view.rich),
        );
        if let Some(view) = restored_view {
            view.restore_view(&doc);
        }
        Ok(doc)
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
        self.drawer().set(true);
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
                    .filter(|doc| {
                        doc.editable_body_ready() && doc.source.is_dirty().unwrap_or(true)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
        } else {
            self.current()
                .into_iter()
                .filter(|doc| doc.editable_body_ready())
                .collect()
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
            while shell.auto_saving.get().is_some() {
                let _ = next_frame().await;
            }
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
                    let dialog = save_path_dialog(original.as_deref(), save_as);
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
                        doc.auto_save.after_explicit_save();
                        session_recovery::saved(self, &doc);
                        saved += 1;
                        self.show_saved(&doc);
                        break;
                    }
                    Err(SaveError::Encoding) => {
                        if !self.confirm_save_as_utf8().await? {
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
    async fn confirm_save_as_utf8(&self) -> Result<bool, String> {
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
            .await?;
        Ok(answer.action.as_deref() == Some("utf8"))
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
        if self.auto_saving.get().is_some() {
            self.notice.set("自动保存完成后可关闭窗口".into());
            return;
        }
        let dirty = self.docs.with(|ds| {
            ds.iter()
                .filter(|d| d.needs_close_confirmation())
                .map(|d| d.name())
                .collect::<Vec<_>>()
        });
        if dirty.is_empty() {
            self.finish_close(false);
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
                    s.finish_close(true);
                } else {
                    s.restore_editor_focus();
                }
            },
        );
    }
    fn finish_close(self: &Rc<Self>, discard_dirty: bool) {
        match session_recovery::finish(self, discard_dirty) {
            Ok(()) => self.window.close(),
            Err(error) if discard_dirty => self.explain("无法更新恢复记录", &error.to_string()),
            Err(error) => {
                log::warn!("关闭时未能写入会话记录：{error}");
                self.window.close();
            }
        }
    }
    fn close_tab(self: &Rc<Self>, id: u64) {
        if self.modal.get() {
            return;
        }
        if self.auto_saving.get() == Some(id) {
            self.notice.set("自动保存完成后可关闭此标签".into());
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
        if doc.needs_close_confirmation() {
            if doc.large.is_some() || doc.source.is_recovered_draft() {
                self.confirm_recovered_close(doc);
                return;
            }
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
                        match session_recovery::remove_closed_tab(&s, id) {
                            Ok(()) => s.remove(id),
                            Err(error) => s.explain("无法更新恢复记录", &error.to_string()),
                        }
                    }
                    s.restore_editor_focus();
                },
            );
        } else {
            self.remove(id);
            session_recovery::checkpoint(self, true);
        }
    }
    fn remove(&self, id: u64) {
        self.remove_with_reopen(id, None);
    }
    fn remove_with_reopen(&self, id: u64, reopen: Option<ClosedTab>) {
        if self
            .recovery_split
            .borrow()
            .as_ref()
            .is_some_and(|(_, split)| split.id == id)
        {
            self.recovery_split.borrow_mut().take();
        }
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
                let reopen = reopen.or_else(|| {
                    if doc.large.is_some()
                        || (doc.source.disk_len() >= LARGE_BYTES
                            && !doc.source.is_dirty().unwrap_or(true))
                    {
                        doc.path.get().map(ClosedTab::File)
                    } else {
                        None
                    }
                });
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
        if self.mount_split(&doc, down).is_some() {
            self.recovery_split.borrow_mut().take();
            self.second_active.set(true);
            self.notice
                .set("同一文档已拆分为两个独立视图，共享正文与撤销历史".into());
        }
    }

    fn mount_split(self: &Rc<Self>, doc: &Rc<Document>, down: bool) -> Option<Rc<Document>> {
        if !doc.editable_body_ready() || self.split.get().is_some() {
            return None;
        }
        let source = match doc.source.sibling_source() {
            Ok(v) => v,
            Err(e) => {
                self.explain("无法拆分", &e);
                return None;
            }
        };
        let rich = match source.sibling() {
            Ok(v) => v,
            Err(e) => {
                self.explain("无法拆分", &e);
                return None;
            }
        };
        let gate = ModeGate::new(source.clone(), rich);
        let reference = component_ref::<Editor>();
        let rich_reference = component_ref::<nsh_base::rich_editor::view::RichEditor>();
        let controls = RichControlSet::new(&gate, &source);
        let wrap = signal(doc.wrap.get());
        let second = Rc::new(Document {
            id: doc.id,
            path: doc.path,
            source,
            gate,
            large: None,
            reference,
            rich_reference,
            controls,
            wrap,
            tab_size: doc.tab_size,
            retained_scroll: signal(None),
            auto_save: doc.auto_save.clone(),
            pending_mode: signal(None),
            import_complete: Cell::new(doc.import_complete.get()),
            progress: element_ref(),
            body: element_ref(),
        });
        self.split_share.set(splitter::EQUAL_SHARE);
        self.split.set(Some(second.clone()));
        self.split_down.set(down);
        Some(second)
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
        let next = if index == 2 {
            self.split.get()
        } else {
            self.primary()
        };
        self.leave_editor_document(next.map(|doc| doc.id));
        self.show_documents();
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
    fn focus_active_page(&self) {
        if self.settings_active() {
            self.settings_view.search.focus();
        } else {
            self.focus_group(if self.second_active.get() { 2 } else { 1 });
        }
    }
    fn restore_editor_focus(self: &Rc<Self>) {
        self.focus_active_page();
        let s = self.clone();
        self.owner.run(|| {
            spawn(async move {
                let _ = next_frame().await;
                s.focus_active_page();
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
        let query = self.search_query.get();
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
        self.search_origin.set(Some((root.clone(), query.clone())));
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
        if self.settings_active() && document_action(id) {
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
            "texzi.find.inFolder"=>self.panel.set("folder".into()),
            "texzi.file.new"=>self.add_blank(),"texzi.file.open"=>self.pick(false),"texzi.folder.open"=>self.pick(true),
            "texzi.file.save" => self.save(false,false),
            "texzi.file.saveAs" => self.save(false,true),
            "texzi.file.saveAll" => self.save(true,false),
            "texzi.folder.close"=>{self.root.set(None);self.notice.set("已关闭文件夹，已打开标签保持不变".into());},
            "texzi.window.close"=>self.close_window(),"texzi.tab.close"=>if self.settings_active(){self.dismiss_settings(true);}else if self.second_active.get(){self.leave_second_group();}else{self.close_tab(self.active.get());},
            "texzi.window.new"=>match session_recovery::spawn_blank_window(){Ok(())=>self.notice.set("已启动新的 texzi 窗口".into()),Err(e)=>self.explain("新建窗口失败",&e.to_string())},
            "texzi.group.splitRight"|"texzi.group.splitDown"=>self.split_editor(id.ends_with("Down")),
            "texzi.group.joinAll"=>{self.leave_second_group();self.notice.set("已合并为单组；同一文档内容与历史保持不变".into());},
            "texzi.tab.reopenClosed"=>{let doc=self.closed.borrow_mut().pop();match doc{Some(ClosedTab::Document(doc))=>{let id=doc.id;self.docs.update(|ds|ds.push(doc));self.activate_primary(id);self.restore_editor_focus();},Some(ClosedTab::File(path))=>self.open(&path),None=>self.notice.set("没有已关闭标签".into())}},
            "texzi.tab.next"|"texzi.tab.previous" => self.step_document_tab(id.ends_with("next")),
            "texzi.sidebar.toggle"=>self.drawer().update(|v|*v=!*v),"texzi.editor.toggleWrap"=>if let Some(doc)=self.current(){let wrap=!self.source_wrap(&doc);doc.wrap.set(Some(wrap));self.notice.set(if wrap{"当前源码视图：自动换行已开启"}else{"当前源码视图：自动换行已关闭"}.into());},
            "texzi.view.toggleZen"|"texzi.view.exitZen"=>{if id.ends_with("exitZen"){self.zen.set(false);}else{self.zen.update(|v|*v=!*v);}if self.notice.get()=="再按一次 Esc 退出 Zen"{self.notice.set(String::new());}},
            "texzi.view.toggleFullScreen"=>self.window.set_mode(if self.window.state().is_some_and(|s|s.mode==WindowMode::FullScreen){WindowMode::Normal}else{WindowMode::FullScreen}),
            "texzi.commandPalette.open"|"texzi.navigation.quickOpen"=>{self.palette_query.set(String::new());let files=id.ends_with("quickOpen");let already=self.panel.get()=="files";if files{self.quick_files.set(vec![]);self.quick_notice.set("正在准备文件候选".into());}self.panel.set(if files{"files"}else{"commands"}.into());if files&&already{self.start_quick(self.root.get());}},
            "texzi.navigation.gotoLine"=>{if let Some(doc)=self.current(){doc.gate.to_source();if doc.gate.rich_mode.get(){self.notice.set(doc.gate.status.get());return;}}self.query.set(String::new());self.panel.set("line".into());self.notice.set("转到行的范围已切换为源码".into());},
            "texzi.group.focusIndex"=>self.panel.set("groups".into()),
            "texzi.markdown.toggleSource"=>if let Some(doc)=self.current(){let target = !doc.displayed_rich(); self.request_mode(doc, target);},
            "texzi.find.open"|"texzi.find.replaceOpen"=>{let replace=id.ends_with("replaceOpen");if replace{if let Some(doc)=self.current(){doc.gate.to_source();if doc.gate.rich_mode.get(){self.notice.set(doc.gate.status.get());return;}}}self.panel.set(if replace{"replace"}else{"find"}.into());let scope=if self.current().is_some_and(|d|d.gate.rich_mode.get()){"正文"}else{"源码"};self.notice.set(format!("查找范围：{scope} · 普通文字 · 区分大小写"));},
            "texzi.find.next"=>self.find(false),"texzi.find.previous"=>self.find(true),"texzi.find.close"=>self.close_panel(),
            "texzi.find.replaceOne"|"texzi.find.replaceAll"=>if let Some(doc)=self.current(){doc.gate.to_source();if doc.gate.rich_mode.get(){self.notice.set(doc.gate.status.get());return;}match doc.source.replace_literal(&self.query.get(),&self.replacement.get(),id.ends_with("All")){Ok(n)=>{self.notice.set(format!("源码 · 已替换 {n} 处，可一次撤销"));self.restore_editor_focus();},Err(e)=>self.notice.set(e)}},
            "texzi.appearance.light"=>self.set_theme("light"),"texzi.appearance.dark"=>self.set_theme("dark"),
            "texzi.help.guide"=>self.explain("使用说明","文件菜单可新建、打开、保存和另存文档。编辑菜单提供查找和替换，外观菜单提供侧栏、自动换行和主题。\nCtrl+P 搜索文件，Ctrl+Shift+P 搜索命令。可搜索分屏或 Zen，亦可按 Ctrl+K Z 进入 Zen，连按两次 Esc 退出。编辑器右上可切换源码和富文本。"),
            "texzi.help.about"=>self.explain("关于 texzi","texzi\n为日常文字、Markdown 与代码片段设计的 Windows 原生文本编辑器。"),
            _=>if let Some(command)=source_command(id){if let Err(e)=self.source_action(command){self.notice.set(e);}},
        }
    }
    fn request_mode(self: &Rc<Self>, doc: Rc<Document>, target_rich: bool) {
        self.request_mode_with_position(doc, target_rich, false);
    }

    fn request_mode_with_position(
        self: &Rc<Self>,
        doc: Rc<Document>,
        target_rich: bool,
        restore_position: bool,
    ) {
        if target_rich && doc.source.disk_len() > large_file::SOURCE_ONLY_BYTES {
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
                                if restore_position {
                                    session_recovery::apply_scroll(&doc, true);
                                } else {
                                    api.reveal_primary_caret(viewport_top);
                                }
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

    fn show_format_menu(
        self: &Rc<Self>,
        doc: Rc<Document>,
        second: bool,
        anchor: ElementRef,
        button_focused: Rc<Cell<bool>>,
    ) {
        let shell = self.clone();
        self.owner.run(|| {
            spawn(async move {
                let Some(bounds) = anchor.measure().await else {
                    return;
                };
                let can_open = || {
                    !shell.settings_active()
                        && shell.second_active.get() == second
                        && shell
                            .current()
                            .is_some_and(|current| Rc::ptr_eq(&current, &doc))
                        && doc.displayed_rich()
                        && doc.pending_mode.get().is_none()
                        && !shell.escape_blocked()
                };
                if !can_open() {
                    return;
                }
                let items = FORMAT_ACTIONS
                    .iter()
                    .map(|&(label, action)| {
                        let shell = shell.clone();
                        let doc = doc.clone();
                        MenuItem::new(label).on_select(move || shell.format_action(&doc, action))
                    })
                    .collect();
                shell.zen_exit.reset();
                popup_menu_at(
                    items,
                    point(bounds.page.x, bounds.page.y + bounds.frame.size.height),
                )
                .await;
                if button_focused.get()
                    && can_open()
                    && shell.panel.get().is_empty()
                    && shell.window.state().is_some_and(|state| state.focused)
                    && let Some(editor) = doc.rich_reference.api::<RichEditorApi>()
                {
                    editor.focus();
                }
            });
        });
    }

    fn show_tab_size_menu(self: &Rc<Self>, doc: Rc<Document>, anchor: ElementRef) {
        let shell = self.clone();
        self.owner.run(|| {
            spawn(async move {
                let Some(bounds) = anchor.measure().await else {
                    return;
                };
                if !shell
                    .current()
                    .is_some_and(|current| Rc::ptr_eq(&current, &doc))
                    || doc.gate.busy()
                {
                    return;
                }
                let selected = doc.tab_size.get();
                let entries = std::iter::once(None)
                    .chain((1..=8).map(Some))
                    .map(|width| {
                        let label = width
                            .map_or_else(|| "继承设置".to_owned(), |width| format!("{width} 列"));
                        let label = if width == selected {
                            format!("✓ {label}")
                        } else {
                            label
                        };
                        let doc = doc.clone();
                        let shell = shell.clone();
                        MenuItem::new(label).on_select(move || {
                            if shell
                                .current()
                                .is_some_and(|current| Rc::ptr_eq(&current, &doc))
                                && !doc.gate.busy()
                            {
                                doc.tab_size.set(width);
                                shell.restore_editor_focus();
                            }
                        })
                    })
                    .collect();
                shell.zen_exit.reset();
                let _ = popup_menu_at(entries, point(bounds.page.x, bounds.page.y));
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
fn document_action(id: &str) -> bool {
    source_command(id).is_some()
        || crate::rich_commands::handles(id)
        || matches!(
            id,
            "texzi.file.save"
                | "texzi.file.saveAs"
                | "texzi.find.open"
                | "texzi.find.replaceOpen"
                | "texzi.find.next"
                | "texzi.find.previous"
                | "texzi.find.replaceOne"
                | "texzi.find.replaceAll"
                | "texzi.editor.toggleWrap"
                | "texzi.navigation.gotoLine"
                | "texzi.markdown.toggleSource"
                | "texzi.view.toggleZen"
                | "texzi.group.splitRight"
                | "texzi.group.splitDown"
                | "texzi.group.joinAll"
        )
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
                    // While the prefix is pending its continuation belongs to
                    // the shell, including keys normally bound by the editor.
                    over_focused: true,
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
    let wrap_doc = doc.clone();
    let wrap_shell = s.clone();
    let tab_shell = s.clone();
    let tab_doc = doc.clone();
    let size_shell = s.clone();
    let height_shell = s.clone();
    let font_shell = s.clone();
    let map_shell = s.clone();
    let map_source = doc.source.clone();
    let map_document = doc.clone();
    let lines_shell = s.clone();
    let body = doc.body;
    let source_focus = s.second_active;
    let rich_focus = s.second_active;
    let rich_focus_shell = s.clone();
    let rich_focus_doc = doc.clone();
    let source_focus_shell = s.clone();
    let source_focus_doc = doc.clone();
    let rich_restore = doc.clone();
    let source_restore = doc.clone();
    let settings_tab = s.settings_tab;
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
    let column_shell = s.clone();
    let reading_width = computed(move || {
        column_shell
            .zen
            .get()
            .then(|| column_shell.zen_reading_width(rich_mode.get()))
    });

    let content = view()
        .test_id("texzi.editor.reading-column")
        .relative()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .w(move || {
            reading_width
                .get()
                .map_or_else(|| relative(1.0).into(), Length::from)
        })
        .max_w(relative(1.0))
        .self_center()
        .child(
            when(rich_mode, move || {
                restore_editor_scroll(rich_restore.clone(), true);
                let session = rich.session();
                effect(move || crate::rich_images::install(&session, image_document_path.get()));
                let shell = controls_shell.clone();
                let escape = shell.clone();
                let focus_shell = rich_focus_shell.clone();
                let focus_doc = rich_focus_doc.clone();
                let blur_shell = focus_shell.clone();
                let blur_doc = focus_doc.clone();
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
                            .auto_focus(move || {
                                settings_tab.get() != SettingsTab::Active
                                    && rich_focus.get() == second
                            })
                            .on_focus(move |_: &()| {
                                focus_shell.body_focused(&focus_doc, second, true)
                            })
                            .on_blur(move |_: &()| blur_shell.body_blurred(&blur_doc, second, true))
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
                restore_editor_scroll(source_restore.clone(), false);
                let tab_shell = tab_shell.clone();
                let tab_doc = tab_doc.clone();
                let size_shell = size_shell.clone();
                let height_shell = height_shell.clone();
                let font_shell = font_shell.clone();
                let focus_shell = source_focus_shell.clone();
                let focus_doc = source_focus_doc.clone();
                let blur_shell = focus_shell.clone();
                let blur_doc = focus_doc.clone();
                let lines_shell = lines_shell.clone();
                let wrap_shell = wrap_shell.clone();
                let wrap_doc = wrap_doc.clone();
                let wrap_limit_doc = wrap_doc.clone();
                let wrap_limited = computed(move || wrap_limit_doc.source_wrap_limited());
                let map_shell = map_shell.clone();
                let map_source = map_source.clone();
                let map_document = map_document.clone();
                let map_enabled = computed(move || {
                    map_document.editable_body_ready()
                        && map_shell.preference(settings::MINIMAP) == true
                });
                let viewport = signal((0, 0));
                let syntax_doc = source_focus_doc.clone();
                let syntax = computed(move || {
                    let rust = syntax_doc.path.with(|path| {
                        path.as_ref()
                            .and_then(|path| path.extension())
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
                    });
                    if rust {
                        syntax_doc.source.track();
                        if syntax_doc.source.disk_len() <= large_file::SOURCE_ONLY_BYTES {
                            return "rust";
                        }
                    }
                    "plaintext"
                });
                let editor = nsh_base::editor::editor()
                    .element_ref(&body)
                    .component_ref(&reference)
                    .anchor_wrap_reflow(true)
                    .viewport_feedback(move || map_enabled.get())
                    .on_viewport_change(move |range: &nsh_base::editor::EditorRange| {
                        viewport.set((range.start.line, range.end.line));
                    })
                    .auto_focus(move || {
                        settings_tab.get() != SettingsTab::Active && source_focus.get() == second
                    })
                    .on_focus(move |_: &()| focus_shell.body_focused(&focus_doc, second, false))
                    .on_blur(move |_: &()| blur_shell.body_blurred(&blur_doc, second, false))
                    .language(move || {
                        Some(SharedString::from(if markdown() {
                            "markdown"
                        } else {
                            "plaintext"
                        }))
                    })
                    .highlight_language(move || Some(SharedString::from(syntax.get())))
                    .tab_size(move || Some(tab_shell.tab_size(&tab_doc)))
                    .hard_tabs(false)
                    .soft_wrap(move || {
                        !wrap_limited.get() && wrap_shell.source_wrap_preference(&wrap_doc)
                    })
                    .font_family(move || Some(SharedString::from(font_shell.source_font_family())))
                    .font_size(move || px(size_shell.source_font_size()))
                    .line_height(move || px(height_shell.source_font_size() * 1.5))
                    .line_numbers(move || lines_shell.preference(settings::LINE_NUMBERS) == true)
                    .w_full()
                    .flex_1()
                    .max_w(relative(1.0))
                    .min_w(px(0.0))
                    .min_h(px(0.0));
                view()
                    .flex_row()
                    .size_full()
                    .min_h(px(0.0))
                    .min_w(px(0.0))
                    .child(editor)
                    .child(when(
                        move || map_enabled.get(),
                        move || minimap::render(map_source.clone(), viewport),
                    ))
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
        .rounded(px(metric("paperRadius")))
        .overflow_hidden()
        .child(when(
            move || path.get().is_some() && !show_path.displayed_rich(),
            move || {
                view()
                    .h(px(28.0))
                    .bg(frame)
                    .flex_shrink_0()
                    .justify_center()
                    .pl(px(12.0))
                    .pr(px(60.0))
                    .border_b(border_width)
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
                        .text_size(px(font_size("meta")))
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

fn restore_editor_scroll(doc: Rc<Document>, rich: bool) {
    if !doc
        .retained_scroll
        .get_untracked()
        .is_some_and(|(mode, _, _)| mode == rich)
    {
        return;
    }
    spawn(async move {
        wait_for_editor_frame(doc.body).await;
        if doc.displayed_rich() == rich {
            session_recovery::apply_scroll(&doc, rich);
        }
    });
}

fn mode_button(shell: Rc<Shell>, doc: Rc<Document>, second: bool) -> impl IntoElement {
    let selected = doc.clone();
    let name = doc.clone();
    let disabled = doc.clone();
    let backdrop = doc.clone();
    icon_button(SOURCE_ICON, "切换到源码", metric("modeWidth"), 18.0)
        .absolute()
        .h(px(metric("modeHeight")))
        .top(px(12.0))
        .right(px(16.0))
        .z_index(1)
        .border(border_width)
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
    let button_focused = Rc::new(Cell::new(false));
    let focused = button_focused.clone();
    let blurred = button_focused.clone();
    view()
        .element_ref(&anchor)
        .role(AccessibilityRole::Button)
        .label("格式命令")
        .accessibility_state(nsh_core::AccessibilityState {
            has_popup: true,
            ..Default::default()
        })
        .items_center()
        .justify_center()
        .app_class(CONTROL_STATES)
        .rounded_full()
        .absolute()
        .w(px(metric("modeWidth")))
        .h(px(metric("modeHeight")))
        .top(px(12.0))
        .right(px(56.0))
        .z_index(1)
        .border(border_width)
        .border_color(divider)
        .bg(frame)
        .disabled(move || {
            state.pending_mode.get().is_some() || state.gate.busy() || state.controls.open()
        })
        .on_focus(move |_: &()| focused.set(true))
        .on_blur(move |_: &()| blurred.set(false))
        .on_press(move |_: &()| {
            if shell.second_active.get() != second {
                // Native menus preserve their text target. On a group change
                // that target must first become this pane's own editor.
                shell.focus_group(if second { 2 } else { 1 });
            }
            shell.show_format_menu(doc.clone(), second, anchor, button_focused.clone());
        })
        .child(shell_glyph(FORMAT_ICON, 18.0))
}

fn document_paper(
    content: impl IntoElement,
    active: impl Fn() -> bool + 'static,
) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .rounded(px(metric("paperRadius")))
        .border(border_width)
        .border_color(move || {
            color(if active() {
                "active-paper-edge"
            } else {
                "chrome-edge"
            })
        })
        .bg(|| color("paper"))
        .child(content)
}

fn groups(s: Rc<Shell>) -> AnyElement {
    let down = s.split_down.get();
    let second_active = s.second_active;
    let primary = s.clone();
    let second = s.clone();
    splitter::groups(
        s,
        down,
        dynamic(move || {
            primary.primary().map(|doc| {
                let content = if let Some(large) = &doc.large {
                    large_file::viewer(primary.clone(), doc.clone(), large.clone()).into_element()
                } else {
                    editor(primary.clone(), doc, false).into_element()
                };
                document_paper(content, move || !second_active.get())
            })
        }),
        move || {
            let second = second.clone();
            dynamic(move || {
                second.split.get().map(|doc| {
                    document_paper(editor(second.clone(), doc, true), move || {
                        second_active.get()
                    })
                })
            })
        },
    )
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
        .h(px(metric("menuHit")))
        .items_center()
        .justify_center()
        .app_class(CONTROL_STATES)
        .rounded(px(6.0))
        .bg(|| color("control-face"))
        .text_color(|| color("control-text"))
        .text_size(px(font_size("bodyUI")))
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
    let offset = shell.clone();
    let compact = shell.clone();
    let menus = shell.clone();
    let icon = ImageSrc::rgba(
        32,
        32,
        include_bytes!(concat!(env!("OUT_DIR"), "/app-icon.rgba")),
    );
    view()
        .flex_row()
        .items_center()
        .h(px(metric("title")))
        .flex_shrink_0()
        .pr(move || window.title_bar().right)
        .bg(|| color("mica-shell"))
        .border_b(border_width)
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
            let mut row = view()
                .flex_row()
                .flex_shrink_0()
                .role(AccessibilityRole::Menubar);
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
                .ml(move || px(offset.title_search_left() - offset.title_menu_end()))
                .h(px(metric("menuHit")))
                .window_drag(false)
                .child(
                    view()
                        .size_full()
                        .px(px(8.0))
                        .flex_row()
                        .gap(px(6.0))
                        .items_center()
                        .overflow_hidden()
                        .app_class(CONTROL_STATES)
                        .rounded(px(6.0))
                        .border(border_width)
                        .border_color(divider)
                        .focusable(true)
                        .role(AccessibilityRole::Button)
                        .label("搜索文件或命令")
                        .child(shell_glyph(SEARCH_ICON, 16.0))
                        .child(when(
                            move || !compact.compact_menu(),
                            || {
                                text("搜索文件或命令…")
                                    .min_w(px(0.0))
                                    .number_of_lines(1)
                                    .text_overflow(EllipsizeMode::Tail)
                                    .text_size(px(font_size("bodyUI")))
                            },
                        ))
                        .on_press(move |_: &()| search.run("texzi.navigation.quickOpen")),
                ),
        )
        .child(view().flex_1().min_w(px(TITLE_DRAG_MIN)).h_full())
}

fn activity_bar(s: Rc<Shell>) -> impl IntoElement {
    let files = s.clone();
    let search = s.clone();
    let settings = s.clone();
    let settings_selected = s.settings_tab;
    view()
        .w(px(metric("rail")))
        .flex_shrink_0()
        .items_center()
        .bg(|| color("mica-shell"))
        .border_r(border_width)
        .border_color(divider)
        .child(activity_icon(
            FILES_ICON,
            "资源管理器",
            {
                let s = s.clone();
                move || !s.settings_active() && s.drawer().get() && s.panel.get() != "folder"
            },
            move || {
                if files.panel.get() == "folder" {
                    files.close_panel();
                    files.drawer().set(true);
                } else if files.settings_active() {
                    files.dismiss_settings(false);
                    files.drawer().set(true);
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
        .child(view().flex_1().min_h(px(0.0)))
        .child(activity_icon(
            SETTINGS_ICON,
            "设置",
            move || settings_selected.get() == SettingsTab::Active,
            move || settings.show_settings(),
        ))
}

fn activity_icon(
    path: &'static str,
    label: &'static str,
    selected: impl Fn() -> bool + Clone + 'static,
    activate: impl Fn() + 'static,
) -> impl IntoElement {
    let surface = selected.clone();
    let ink = selected.clone();
    view()
        .relative()
        .w(px(metric("rail")))
        .h(px(metric("rail")))
        .items_center()
        .justify_center()
        .child(
            icon_button(path, label, 34.0, metric("railIcon"))
                .bg(move || {
                    if surface() {
                        selected_surface()
                    } else {
                        rgba(0).into()
                    }
                })
                .text_color(move || color(if ink() { "selected-text" } else { "ink-1" }))
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
const SETTINGS_ICON: &str = "M10 3h4l.5 2.5 2 .9 2.2-1.3 2.2 2.2-1.3 2.2.9 2L23 12l-2.5.5-.9 2 1.3 2.2-2.2 2.2-2.2-1.3-2 .9L14 21h-4l-.5-2.5-2-.9-2.2 1.3-2.2-2.2 1.3-2.2-.9-2L1 12l2.5-.5.9-2-1.3-2.2 2.2-2.2 2.2 1.3 2-.9L10 3zM15 12a3 3 0 1 1-6 0 3 3 0 1 1 6 0";
const CLOSE_ICON: &str = "M6 6l12 12M18 6L6 18";
const FORMAT_ICON: &str = "M3 18l5-12 5 12M5 14h6M16 8h5M16 12h5M16 16h5";
const SOURCE_ICON: &str = "M4 6l6 5-6 5M12 17h8";
const ADD_ICON: &str = "M5 12h14M12 5v14";

fn shell_glyph(path: &str, size: f32) -> impl IntoElement {
    let source = ImageSrc::svg_markup(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><path d="{path}" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>"#
    ));
    svg(source).w(px(size)).h(px(size)).flex_shrink_0()
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
        .app_class(CONTROL_STATES)
        .rounded(px(6.0))
        .accessibility_label(label)
        .child(shell_glyph(path, glyph))
}

fn file_sidebar(s: Rc<Shell>, compact: bool) -> impl IntoElement {
    let close = s.clone();
    let panel = s.panel;
    let zen = s.zen;
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
        .style_when(
            move || compact,
            |style| {
                style
                    .absolute()
                    .left(move || px(if zen.get() { 0.0 } else { metric("rail") }))
                    .top(px(0.0))
                    .bottom(px(0.0))
                    .z_index(2)
            },
        )
        .w(move || px(width.get().min(layout_max())))
        .max_w(relative(1.0))
        .flex_shrink_0()
        .bg(move || {
            color(if compact {
                "shell-solid"
            } else {
                "mica-sidebar"
            })
        })
        .border_r(border_width)
        .border_color(divider)
        .child(
            view()
                .h(px(38.0))
                .px(px(8.0))
                .flex_shrink_0()
                .flex_row()
                .items_center()
                .justify_between()
                .border_b(border_width)
                .border_color(divider)
                .child(text(move || {
                    if panel.get() == "folder" {
                        "文件夹搜索"
                    } else {
                        "资源管理器"
                    }
                }))
                .child(dynamic(move || {
                    let searching_folder = panel.get() == "folder";
                    let close = close.clone();
                    (compact || searching_folder).then(|| {
                        let label = if searching_folder {
                            "关闭文件夹搜索"
                        } else {
                            "关闭资源管理器"
                        };
                        icon_button(CLOSE_ICON, label, 26.0, 14.0).on_press(move |_: &()| {
                            if searching_folder {
                                close.close_panel();
                            } else {
                                close.compact_drawer.set(false);
                                close.restore_editor_focus();
                            }
                        })
                    })
                })),
        )
        .child(dynamic(move || {
            if panel.get() == "folder" {
                folder_search_panel(s.clone()).into_element()
            } else {
                let s = s.clone();
                view()
                    .flex_1()
                    .min_h(px(0.0))
                    .p(px(8.0))
                    .gap(px(8.0))
                    .child(button("打开文件夹", s.clone(), "texzi.folder.open"))
                    .child(dynamic(move || folder(s.clone())))
                    .into_element()
            }
        }))
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

const TAB_CLOSE_GROUP: &str = "workbench-tab";

fn tab_close_button(
    shell: Rc<Shell>,
    label: &'static str,
    active: impl Fn() -> bool + 'static,
    close: impl Fn() + 'static,
) -> impl IntoElement {
    let active_shell = shell.clone();
    icon_button(CLOSE_ICON, label, 24.0, 14.0)
        .style_when(
            move || active_shell.preference(settings::TAB_CLOSE) == "active" && !active(),
            |style| style.invisible(),
        )
        .style_when(
            move || shell.preference(settings::TAB_CLOSE) == "hover",
            |style| style.opacity(0.0),
        )
        .group_hover_named(TAB_CLOSE_GROUP, |style| style.opacity(1.0))
        .group_focus_within_named(TAB_CLOSE_GROUP, |style| style.opacity(1.0))
        .on_press(move |_: &()| close())
}

fn tab_ribbon() -> impl IntoElement {
    crate::origami::ribbon(
        view()
            .absolute()
            .bottom(px(0.0))
            .left(px(6.0))
            .right(px(6.0))
            .h(px(metric("tabMark")))
            .rounded_full()
            .pointer_events(PointerEvents::None),
    )
}

fn tab_bar(s: Rc<Shell>) -> impl IntoElement {
    let docs = s.docs;
    let active = s.active;
    let settings_tab = s.settings_tab;
    let preferences_tab = s.clone();
    let new_tab = s.clone();
    let split = s.split;
    let split_tab = s.clone();
    view()
        .h(px(metric("tabTrack")))
        .flex_shrink_0()
        .items_center()
        .flex_row()
        .child(
            scroll()
                .horizontal()
                .scrollbar(false)
                .flex_1()
                .min_w(px(0.0))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .h(px(metric("tabTrack")))
                        .gap(px(4.0))
                        .pl(px(metric("splitGap")))
                        .child(each(
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
                                    .group_named(TAB_CLOSE_GROUP)
                                    .h(px(metric("tab")))
                                    .rounded(px(metric("tabRadius")))
                                    .min_w(px(104.0))
                                    .max_w(px(metric("tabLimit")))
                                    .pr(px(6.0))
                                    .flex_row()
                                    .items_center()
                                    .bg(move || {
                                        if active.get() == id
                                            && settings_tab.get() != SettingsTab::Active
                                        {
                                            current_color("tab-active-bg")
                                        } else {
                                            Hsla::from(rgba(0))
                                        }
                                    })
                                    .child(
                                        nsh_base::button()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .h_full()
                                            .px(px(10.0))
                                            .flex_row()
                                            .items_center()
                                            .gap(px(6.0))
                                            .app_class(CONTROL_STATES)
                                            .text_color(move || {
                                                color(
                                                    if active.get() == id
                                                        && settings_tab.get() != SettingsTab::Active
                                                    {
                                                        "selected-text"
                                                    } else {
                                                        "ink-1"
                                                    },
                                                )
                                            })
                                            .accessibility_label({
                                                let label = label.clone();
                                                move || Some(SharedString::from(label.name()))
                                            })
                                            .selected(move || {
                                                active.get() == id
                                                    && settings_tab.get() != SettingsTab::Active
                                            })
                                            .child(
                                                text(move || label.name())
                                                    .text_size(px(font_size("label")))
                                                    .min_w(px(0.0))
                                                    .number_of_lines(1)
                                                    .text_overflow(EllipsizeMode::Tail),
                                            )
                                            .child(when(
                                                move || {
                                                    doc.source.track();
                                                    doc.needs_close_confirmation()
                                                },
                                                || {
                                                    view()
                                                        .w(px(6.0))
                                                        .h(px(6.0))
                                                        .flex_shrink_0()
                                                        .app_class("rounded-full")
                                                        .bg(|| color("apricot"))
                                                },
                                            ))
                                            .on_press(move |_: &()| {
                                                let from_settings = tab.settings_active();
                                                tab.activate_primary(id);
                                                if from_settings {
                                                    tab.restore_editor_focus();
                                                }
                                            }),
                                    )
                                    .child(tab_close_button(
                                        close.clone(),
                                        "关闭此标签",
                                        move || {
                                            active.get() == id
                                                && settings_tab.get() != SettingsTab::Active
                                        },
                                        move || close.close_tab(id),
                                    ))
                                    .child(when(
                                        move || {
                                            active.get() == id
                                                && settings_tab.get() != SettingsTab::Active
                                        },
                                        tab_ribbon,
                                    ))
                            },
                        )),
                ),
        )
        .child(when(
            move || settings_tab.get() != SettingsTab::Closed,
            move || {
                let open = preferences_tab.clone();
                let close = preferences_tab.clone();
                view()
                    .relative()
                    .group_named(TAB_CLOSE_GROUP)
                    .h(px(metric("tab")))
                    .rounded(px(metric("tabRadius")))
                    .flex_row()
                    .items_center()
                    .pr(px(6.0))
                    .bg(move || {
                        if settings_tab.get() == SettingsTab::Active {
                            current_color("tab-active-bg")
                        } else {
                            Hsla::from(rgba(0))
                        }
                    })
                    .child(
                        nsh_base::button()
                            .h_full()
                            .px(px(10.0))
                            .app_class(CONTROL_STATES)
                            .text_color(move || {
                                color(if settings_tab.get() == SettingsTab::Active {
                                    "selected-text"
                                } else {
                                    "ink-1"
                                })
                            })
                            .accessibility_label("设置标签")
                            .selected(move || settings_tab.get() == SettingsTab::Active)
                            .on_press(move |_: &()| open.show_settings())
                            .child(text("设置").text_size(px(font_size("label")))),
                    )
                    .child(tab_close_button(
                        close.clone(),
                        "关闭设置标签",
                        move || settings_tab.get() == SettingsTab::Active,
                        move || close.dismiss_settings(true),
                    ))
                    .child(when(
                        move || settings_tab.get() == SettingsTab::Active,
                        tab_ribbon,
                    ))
            },
        ))
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
    crate::origami::install();
    let toolbar = s.clone();
    let tabs = s.clone();
    let side = s.clone();
    let compact_side = s.clone();
    let activity = s.clone();
    let content = s.clone();
    let settings_side = s.clone();
    let preferences_side = s.clone();
    let preferences_page = s.clone();
    let settings_tab = s.settings_tab;
    let narrow = signal(false);
    let editor_width = signal(None::<Points>);
    let layout = s.clone();
    effect(move || {
        let width = layout.content_width();
        let categories = if width < metric("settingsStackBelow") {
            0.0
        } else {
            metric("sidebar")
        };
        narrow.set(width - categories - metric("rail") < metric("settingsStackBelow"));
    });
    let panels = s.clone();
    let quick = s.clone();
    let status_shell = s.clone();
    let drop_shell = s.clone();
    let zen = s.zen;
    let exit_zen = s.clone();
    let zen_window = s.window.clone();
    let zen_armed = s.zen_exit.armed;
    let nav_panel = s.panel;
    view()
        .relative()
        .size_full()
        .text_color(|| color("ink-1"))
        .text_size(px(font_size("bodyUI")))
        .on_file_drop(move |event: &FileDropEvent| drop_shell.open_dropped(&event.uris))
        .child(when(move || !zen.get(), move || title_bar(toolbar.clone())))
        .child(when(
            move || zen.get(),
            move || {
                let exit = exit_zen.clone();
                let window = zen_window.clone();
                view()
                    .group_named("zen-exit")
                    .bg(|| color("mica-shell"))
                    .h(px(metric("title")))
                    .w_full()
                    .flex_row()
                    .pr(move || window.title_bar().right)
                    .justify_end()
                    .items_center()
                    .child(view().flex_1().h_full().window_drag(true))
                    .child(when(zen_armed, || {
                        view()
                            .relative()
                            .min_w(px(0.0))
                            .px(px(8.0))
                            .py(px(4.0))
                            .rounded(px(8.0))
                            .border(border_width)
                            .border_color(|| color("chrome-edge"))
                            .bg(|| color("paper-subtle"))
                            .child(text("再按一次 Esc 退出 Zen").number_of_lines(1))
                            .child(tab_ribbon())
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
                .relative()
                .flex_row()
                .flex_1()
                .child(when(
                    move || !zen.get(),
                    move || activity_bar(activity.clone()),
                ))
                .child(when(
                    {
                        let s = side.clone();
                        move || {
                            !s.compact_menu()
                                && (nav_panel.get() == "folder"
                                    || (s.sidebar.get() && !zen.get() && !s.settings_active()))
                        }
                    },
                    move || file_sidebar(side.clone(), false),
                ))
                .child(when(
                    move || {
                        !zen.get()
                            && nav_panel.get() != "folder"
                            && settings_tab.get() == SettingsTab::Active
                            && settings_side.content_width() >= metric("settingsStackBelow")
                    },
                    move || settings_view::sidebar(preferences_side.settings_view.clone(), false),
                ))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .bg(|| color("mica-shell"))
                        .child(when(move || !zen.get(), move || tab_bar(tabs.clone())))
                        .child(
                            view()
                                .relative()
                                .flex_1()
                                .min_h(px(0.0))
                                .child(
                                    view()
                                        .size_full()
                                        .w(move || {
                                            if settings_tab.get() == SettingsTab::Active {
                                                editor_width
                                                    .get()
                                                    .map(Length::from)
                                                    .unwrap_or_else(|| relative(1.0).into())
                                            } else {
                                                relative(1.0).into()
                                            }
                                        })
                                        .on_layout(move |bounds: &Bounds<Points>| {
                                            if settings_tab.get_untracked() != SettingsTab::Active {
                                                editor_width.set(Some(bounds.size.width));
                                            }
                                        })
                                        .style_when(
                                            move || settings_tab.get() == SettingsTab::Active,
                                            |style| style.invisible(),
                                        )
                                        .child(dynamic(move || groups(content.clone()))),
                                )
                                .child(when(
                                    move || settings_tab.get() == SettingsTab::Active,
                                    move || {
                                        let compact = preferences_page.clone();
                                        let categories = preferences_page.settings_view.clone();
                                        view()
                                            .absolute()
                                            .top(px(metric("splitGap")))
                                            .left(px(metric("splitGap")))
                                            .right(px(metric("splitGap")))
                                            .bottom(px(metric("splitGap")))
                                            .min_h(px(0.0))
                                            .rounded(px(metric("paperRadius")))
                                            .border(border_width)
                                            .border_color(divider)
                                            .bg(|| color("paper"))
                                            .overflow_hidden()
                                            .child(when(
                                                move || {
                                                    compact.content_width()
                                                        < metric("settingsStackBelow")
                                                },
                                                move || {
                                                    settings_view::sidebar(categories.clone(), true)
                                                },
                                            ))
                                            .child(settings_view::content(
                                                preferences_page.settings.clone(),
                                                preferences_page.settings_view.clone(),
                                                preferences_page.root,
                                                narrow,
                                            ))
                                    },
                                )),
                        )
                        .child(dynamic(move || {
                            (!is_quick_panel(&panels.panel.get()))
                                .then(|| panel(panels.clone()))
                                .flatten()
                        })),
                )
                .child(when(
                    {
                        let shell = compact_side.clone();
                        move || {
                            shell.compact_menu()
                                && (nav_panel.get() == "folder"
                                    || (shell.compact_drawer.get()
                                        && !zen.get()
                                        && !shell.settings_active()))
                        }
                    },
                    move || file_sidebar(compact_side.clone(), true),
                )),
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
            let width = quick.clone();
            let position = quick.clone();
            Some(
                view()
                    .absolute()
                    .top(px(metric("spotTop")))
                    .left(move || {
                        px((position.content_width() - position.quick_panel_width().0) / 2.0)
                    })
                    .w(move || width.quick_panel_width())
                    .z_index(3)
                    .window_drag(false)
                    .child(quick_panel(quick.clone(), mode == "files"))
                    .into_element(),
            )
        }))
}

const LARGE_SOURCE_NO_HIGHLIGHT: &str = "大文件 · 无高亮";
const LARGE_SOURCE_NO_WRAP: &str = "已暂停自动换行";
const LARGE_IMPORT_COMPLETE: &str = "大文件加载完成 · 可编辑";

fn source_protection(doc: &Document) -> (bool, bool) {
    doc.source.track();
    (
        doc.source.disk_len() > large_file::SOURCE_ONLY_BYTES,
        doc.source_wrap_limited(),
    )
}

fn workbench_status(document: &Shell) -> (String, &'static str) {
    let mut message = document.notice.get();
    let current = document.current();
    let import_complete = message.is_empty()
        && current
            .as_ref()
            .is_some_and(|doc| doc.import_complete.get());
    if import_complete {
        message = LARGE_IMPORT_COMPLETE.to_owned();
    }
    let dirty = current.as_ref().is_some_and(|doc| {
        doc.source.track();
        doc.needs_close_confirmation()
    });
    let large = current.as_ref().and_then(|doc| doc.large.as_ref());
    let important = |message: &str| {
        ["失败", "未能", "无法", "错误"]
            .iter()
            .any(|part| message.contains(part))
    };
    let tone = if important(&message) {
        "error-fg"
    } else if dirty || large.is_some() {
        "warning-fg"
    } else if message.starts_with("已保存") {
        "success-fg"
    } else {
        "muted-readable"
    };
    let message = match (dirty, message.is_empty() || message.starts_with("已保存")) {
        (true, true) => "未保存".to_owned(),
        (true, false) => format!("{message} · 未保存"),
        (false, _) if message.is_empty() && large.is_none() => "就绪".to_owned(),
        _ => message,
    };
    let message = if let Some(large) = large {
        let state = large.status();
        if message.is_empty() {
            state
        } else if important(&message) {
            format!("{message} · {state}")
        } else {
            format!("{state} · {message}")
        }
    } else {
        message
    };
    if document.compact_menu()
        && let Some(doc) = current
            .as_ref()
            .filter(|doc| doc.large.is_none() && !doc.displayed_rich())
    {
        let hints = match source_protection(doc) {
            (true, true) => Some(format!(
                "{LARGE_SOURCE_NO_HIGHLIGHT} · {LARGE_SOURCE_NO_WRAP}"
            )),
            (true, false) => Some(LARGE_SOURCE_NO_HIGHLIGHT.to_owned()),
            (false, true) => Some(LARGE_SOURCE_NO_WRAP.to_owned()),
            (false, false) => None,
        };
        if let Some(hints) = hints {
            return if important(&message) {
                (format!("{message} · {hints}"), tone)
            } else if dirty {
                (format!("未保存 · {hints}"), "warning-fg")
            } else if import_complete {
                (format!("{hints} · {LARGE_IMPORT_COMPLETE}"), "warning-fg")
            } else {
                (hints, "warning-fg")
            };
        }
    }
    (message, tone)
}

fn status_bar(shell: Rc<Shell>) -> impl IntoElement {
    let document = shell.clone();
    let status = computed(move || workbench_status(&document));
    view()
        .h(px(metric("status")))
        .flex_shrink_0()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(8.0))
        .bg(|| color("mica-statusbar"))
        .border_t(border_width)
        .border_color(divider)
        .text_color(secondary_foreground)
        .text_size(px(font_size("meta")))
        .tabular_nums()
        .child(
            text(move || status.get().0)
                .text_color(move || color(status.get().1))
                .flex_1()
                .min_w(px(0.0))
                .number_of_lines(1)
                .text_overflow(EllipsizeMode::Tail),
        )
        .child(dynamic(move || {
            let doc = shell.current()?;
            let compact = shell.compact_menu();
            let show_optional = !compact || status.get().1 == "muted-readable";
            if doc.large.is_some() {
                return None;
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
            if show_optional && let Some(position) = position {
                fields = fields.child(text(position).number_of_lines(1));
            }
            if !rich {
                let anchor = element_ref();
                let activate = shell.clone();
                let label_shell = shell.clone();
                let label_doc = doc.clone();
                if !compact {
                    let (highlighting, wrapping) = source_protection(&doc);
                    if highlighting {
                        fields = fields.child(text(LARGE_SOURCE_NO_HIGHLIGHT).number_of_lines(1));
                    }
                    if wrapping {
                        fields = fields.child(text(LARGE_SOURCE_NO_WRAP).number_of_lines(1));
                    }
                }
                fields = fields.child(
                    nsh_base::button()
                        .element_ref(&anchor)
                        .accessibility_label("当前文档缩进宽度")
                        .child(
                            text(move || format!("空格：{}", label_shell.tab_size(&label_doc)))
                                .number_of_lines(1),
                        )
                        .on_press(move |_: &()| activate.show_tab_size_menu(doc.clone(), anchor)),
                );
            }
            if compact {
                if show_optional {
                    fields = fields.child(text(encoding).number_of_lines(1));
                }
                return Some(fields.into_element());
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
    let compact = s.compact_menu();
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
                if !supported(&c.id) {
                    return None;
                }
                let reason = markdown
                    .as_ref()
                    .and_then(|results| results.get(&c.id))
                    .map_or_else(|| s.blocked(&c.id), |result| result.as_ref().err().cloned());
                let label = reason
                    .as_ref()
                    .map_or_else(|| c.name.clone(), |reason| format!("{} · {reason}", c.name));
                let mut item = nsh_base::CommandItem::new(c.id.clone(), label)
                    .keywords([c.id])
                    .group(c.group)
                    .disabled(reason.is_some());
                if !c.keys.is_empty() {
                    let keys = c.keys.join(" / ");
                    item = item.shortcut(keys.clone()).keywords([keys]);
                }
                Some(item)
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
        .rounded(px(12.0))
        .overflow_hidden()
        .bg(|| color("paper"))
        .border(border_width)
        .border_color(|| color("chrome-edge"))
        .child(
            nsh_base::command()
                .items(entries)
                .accessibility_label(if files { "文件搜索结果" } else { "命令搜索结果" })
                .query(move || Some(query.get().into()))
                .placeholder(if files {
                    "搜索文件或命令…（> 查找命令）"
                } else {
                    "搜索命令名称或 ID"
                })
                .auto_focus(true)
                .loading(move || files && scanning.get())
                .search_class("h-8")
                .search_bar_style(move |s| {
                    s.h(px(44.0)).px(px(if compact { 2.0 } else { 12.0 })).gap(px(8.0))
                })
                .when(!compact, |command| {
                    command
                        .search_prefix(shell_glyph(SEARCH_ICON, 16.0))
                        .search_suffix(text(if files { "文件" } else { "命令" }).text_color(secondary_foreground))
                })
                .search_accessibility_label("搜索文件或命令")
                .list_class("max-h-64")
                .item_class("focus:bg-sidebar-accent focus:text-sidebar-accent-foreground disabled:text-(--control-disabled)")
                .item_style(|s| s.mx(px(6.0)).px(px(8.0)).py(px(7.0)).rounded(px(6.0)))
                .group_style(|s| s.px(px(12.0)).py(px(4.0)).text_color(secondary_foreground))
                .render_item(|entry: &nsh_base::CommandEntry| {
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(text(entry.label.clone()).min_w(px(0.0)).flex_1().number_of_lines(2))
                        .child(entry.shortcut_label.clone().or_else(|| entry.shortcut.clone()).map(|keys| {
                            text(keys)
                                .flex_shrink_0()
                                .px(px(5.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .bg(|| color("paper-subtle"))
                                .text_color(secondary_foreground)
                                .text_size(px(font_size("meta")))
                        }))
                })
                .empty(text("没有匹配结果").p(px(12.0)).text_color(secondary_foreground))
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
                        run.activate_primary(id);
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
                    .child(text(move || notice.get()).text_color(secondary_foreground).text_size(px(font_size("meta"))))
                    .child(when(scanning, move || {
                        let stop = stop.clone();
                        nsh_base::button()
                            .child(text("取消目录扫描"))
                            .on_press(move |_: &()| stop.cancel_quick(true))
                    }))
            },
        ))
        .child(
            text("↑↓ 选择 · Enter 打开或执行 · Esc 关闭")
                .px(px(12.0))
                .py(px(6.0))
                .text_color(secondary_foreground)
                .text_size(px(font_size("meta"))),
        )
}

fn folder_search_panel(s: Rc<Shell>) -> impl IntoElement {
    let root = s.root;
    let query = s.search_query;
    let origin = s.search_origin;
    let hits = s.hits;
    let notice = s.search_notice;
    let searching = s.searching;
    let submit = s.clone();
    let enter = s.clone();
    let cancel = s.clone();
    let results = s.clone();
    let same_query = move || match (root.get(), origin.get()) {
        (Some(root), Some((searched_root, searched_query))) => {
            root == searched_root && query.get() == searched_query
        }
        _ => false,
    };

    view()
        .flex_1()
        .min_h(px(0.0))
        .p(px(8.0))
        .gap(px(8.0))
        .child(
            input()
                .h(px(metric("field")))
                .px(px(8.0))
                .rounded(px(6.0))
                .border(border_width)
                .border_color(divider)
                .bg(|| color("paper"))
                .app_class(CONTROL_STATES)
                .label("文件夹搜索文字")
                .placeholder("搜索文件中的文字")
                .value(move || query.get())
                .on_change(move |value: &str| query.set(value.into()))
                .auto_focus(true)
                .on_submit(move |_: &str| enter.search_folder()),
        )
        .child(
            text("UTF-8 · 普通文字 · 区分大小写")
                .text_size(px(font_size("meta")))
                .text_color(secondary_foreground),
        )
        .child(when(
            move || root.with(Option::is_none),
            move || button("打开文件夹", s.clone(), "texzi.folder.open"),
        ))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(6.0))
                .child(
                    folder_search_button("开始搜索")
                        .disabled(move || {
                            root.with(Option::is_none) || query.with(String::is_empty)
                        })
                        .on_press(move |_: &()| submit.search_folder()),
                )
                .child(
                    folder_search_button("取消搜索")
                        .disabled(move || !searching.get())
                        .on_press(move |_: &()| {
                            if let Some(active) = cancel.search_cancel.borrow().as_ref() {
                                active.store(true, Ordering::Relaxed);
                                cancel.search_notice.set("正在取消目录搜索…".into());
                            }
                        }),
                ),
        )
        .child(scroll().flex_1().min_h(px(0.0)).child(dynamic(move || {
            if !same_query() {
                return None;
            }
            let root = root.get()?;
            let mut files: Vec<(PathBuf, Vec<SearchHit>)> = Vec::new();
            for hit in hits.get() {
                if let Some((path, matches)) = files.last_mut()
                    && *path == hit.path
                {
                    matches.push(hit);
                } else {
                    files.push((hit.path.clone(), vec![hit]));
                }
            }
            Some(
                view()
                    .gap(px(8.0))
                    .children(files.into_iter().map(|(path, matches)| {
                        let relative = path.strip_prefix(&root).unwrap_or(&path);
                        let title = display_path(relative);
                        view()
                            .rounded(px(6.0))
                            .border(border_width)
                            .border_color(divider)
                            .bg(|| color("paper"))
                            .child(
                                text(title)
                                    .label(display_path(&path))
                                    .p(px(6.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(font_size("meta")))
                                    .number_of_lines(2)
                                    .text_overflow(EllipsizeMode::Middle),
                            )
                            .children(matches.into_iter().map(|hit| {
                                let shell = results.clone();
                                nsh_base::button()
                                    .min_h(px(metric("field")))
                                    .w_full()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(6.0))
                                    .p(px(6.0))
                                    .border_t(border_width)
                                    .border_color(divider)
                                    .app_class(CONTROL_STATES)
                                    .accessibility_label(format!(
                                        "{} 第 {} 行，{}",
                                        display_path(relative),
                                        hit.line,
                                        hit.text
                                    ))
                                    .child(
                                        text(hit.line.to_string())
                                            .flex_shrink_0()
                                            .tabular_nums()
                                            .text_size(px(font_size("meta")))
                                            .text_color(secondary_foreground),
                                    )
                                    .child(
                                        text(hit.text.clone())
                                            .min_w(px(0.0))
                                            .flex_1()
                                            .number_of_lines(2)
                                            .text_overflow(EllipsizeMode::Tail),
                                    )
                                    .on_press(move |_: &()| open_folder_result(&shell, &hit))
                            }))
                    })),
            )
        })))
        .child(
            scroll().max_h(relative(0.35)).min_h(px(0.0)).child(
                text(move || {
                    if root.with(Option::is_none) {
                        return "请先打开文件夹".into();
                    }
                    if origin.with(Option::is_some) && !same_query() {
                        return "搜索条件已更改，按 Enter 开始搜索".into();
                    }
                    let status = notice.get();
                    if status.is_empty() {
                        "按 Enter 搜索当前文件夹".into()
                    } else {
                        status
                    }
                })
                .text_size(px(font_size("meta")))
                .text_color(secondary_foreground)
                .tabular_nums()
                .rounded(px(6.0))
                .bg(|| color("paper-subtle"))
                .p(px(8.0)),
            ),
        )
}

fn folder_search_button(label: &'static str) -> nsh_base::ButtonElement {
    nsh_base::button()
        .h(px(metric("field")))
        .px(px(7.0))
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .border(border_width)
        .border_color(divider)
        .bg(|| color("paper"))
        .app_class(CONTROL_STATES)
        .child(text(label).text_size(px(font_size("meta"))))
}

fn open_folder_result(shell: &Rc<Shell>, hit: &SearchHit) {
    let result = hit
        .path
        .canonicalize()
        .map_err(|error| error.to_string())
        .and_then(|path| shell.open_canonical_with_view(path, None));
    let doc = match result {
        Ok(doc) => doc,
        Err(error) => {
            shell
                .search_notice
                .set(format!("打开搜索结果失败：{error}"));
            return;
        }
    };
    // A newly opened document clears the old editor panel. The folder
    // results belong to the sidebar and remain available after navigation.
    shell.panel.set("folder".into());
    if !doc.editable_body_ready() {
        shell
            .search_notice
            .set("文件已改为只读分页，当前无法按行定位。".into());
        return;
    }
    doc.gate.to_source();
    if doc.gate.rich_mode.get() {
        shell.search_notice.set(doc.gate.status.get());
        return;
    }
    if let Err(error) = doc.source.go_to_line(hit.line) {
        shell.search_notice.set(format!("定位失败：{error}"));
        return;
    }
    shell.restore_editor_focus();
    let owner = shell.owner;
    let shell = shell.clone();
    owner.run(|| {
        spawn(async move {
            let _ = next_frame().await;
            if shell
                .current()
                .is_some_and(|current| Rc::ptr_eq(&current, &doc))
                && !doc.displayed_rich()
                && let Some(editor) = doc.reference.api::<EditorApi>()
            {
                editor.reveal_primary_caret(None);
            }
        });
    });
}

fn panel(s: Rc<Shell>) -> Option<AnyElement> {
    let mode = s.panel.get();
    if mode.is_empty() || mode == "folder" || is_quick_panel(&mode) {
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
