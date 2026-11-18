//! Application shell. Native menus and the palette share command identities.
use crate::{editor_owner::NativeView, rich_owner::ModeGate};
use nsh_base::editor::{Editor, EditorApi, EditorCommand, ExternalDocument};
use nsh_core::{
    desktop::{WindowAttributes, WindowMode},
    element::{
        desktop::{WindowRef, current_window},
        input,
    },
    file_dialog::OpenDialog,
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

struct Document {
    id: u64,
    name: String,
    path: Option<PathBuf>,
    source: Rc<NativeView>,
    gate: Rc<ModeGate>,
    reference: ComponentRef<Editor>,
    rich_reference: ComponentRef<nsh_base::rich_editor::view::RichEditor>,
    controls: RichControlSet,
    wrap: Signal<bool>,
}
// Toolbar and command-palette entries share the same per-view drafts and
// confirmation methods. Neither surface owns a second editable document.
struct RichControlSet {
    blocks: Rc<crate::block_controls::BlockControls>,
    resources: Rc<crate::rich_controls::ResourceControls>,
    code: Rc<crate::code_controls::CodeControls>,
    table: Rc<crate::table_controls::TableControls>,
}
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
#[path = "shell_markdown_tests.rs"]
mod shell_markdown_tests;
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
    closed: RefCell<Vec<Rc<Document>>>,
    root: Signal<Option<PathBuf>>,
    sidebar: Signal<bool>,
    notice: Signal<String>,
    panel: Signal<String>,
    query: Signal<String>,
    replacement: Signal<String>,
    zen: Signal<bool>,
    chord: Signal<bool>,
    source_ref: Cell<Option<ComponentRef<Editor>>>,
    modal: Cell<bool>,
    split: Signal<Option<Rc<Document>>>,
    split_down: Signal<bool>,
    second_active: Signal<bool>,
    expanded: Signal<Vec<PathBuf>>,
    hits: Signal<Vec<SearchHit>>,
    search_notice: Signal<String>,
    searching: Signal<bool>,
    search_cancel: RefCell<Option<Arc<AtomicBool>>>,
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
            root: signal(None),
            sidebar: signal(true),
            notice: signal(String::new()),
            panel: signal(String::new()),
            query: signal(String::new()),
            replacement: signal(String::new()),
            zen: signal(false),
            chord: signal(false),
            source_ref: Cell::new(None),
            modal: Cell::new(false),
            split: signal(None),
            split_down: signal(false),
            second_active: signal(false),
            expanded: signal(vec![]),
            hits: signal(vec![]),
            search_notice: signal(String::new()),
            searching: signal(false),
            search_cancel: RefCell::new(None),
        });
        s.add("", None);
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
        for (key, id) in [
            ("f3", "texzi.find.next"),
            ("shift+f3", "texzi.find.previous"),
            ("mod+shift+w", "texzi.window.close"),
            ("f1", "texzi.commandPalette.open"),
            ("mod+e", "texzi.navigation.quickOpen"),
            ("mod+f4", "texzi.tab.close"),
        ] {
            let s = s.clone();
            on_key(key, move |_| s.run(id));
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
        on_key("escape", move |_| {
            if escape.current().is_some_and(|d| d.gate.busy()) {
                return;
            }
            if !escape.panel.get().is_empty() {
                escape.close_panel();
            } else if escape.zen.get() {
                escape.zen.set(false);
                escape.restore_editor_focus();
            }
        });
        s
    }
    fn add(&self, text: &str, path: Option<PathBuf>) {
        let id = self.next.get();
        self.next.set(id + 1);
        let name = path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("未命名-{id}"));
        let source = self.owner.run(|| NativeView::from_text(text));
        let rich = self.owner.run(|| source.sibling().expect("rich view"));
        let gate = self.owner.run(|| ModeGate::new(source.clone(), rich));
        if path
            .as_ref()
            .and_then(|p| p.extension())
            .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
        {
            self.owner.run(|| gate.to_rich());
        }
        let reference = self.owner.run(component_ref::<Editor>);
        let rich_reference = self
            .owner
            .run(component_ref::<nsh_base::rich_editor::view::RichEditor>);
        let controls = self.owner.run(|| RichControlSet::new(&gate, &source));
        let wrap = self.owner.run(|| signal(false));
        self.docs.update(|ds| {
            ds.push(Rc::new(Document {
                id,
                name,
                path,
                source,
                gate,
                reference,
                rich_reference,
                controls,
                wrap,
            }))
        });
        self.active.set(id);
        self.second_active.set(false);
        self.panel.set(String::new());
    }
    fn blocked(&self, id: &str) -> Option<String> {
        if self.current().is_some_and(|d| d.gate.busy()) {
            return Some("请先完成或取消中文组合".into());
        }
        if id == "texzi.editor.toggleWrap" && self.current().is_some_and(|d| d.gate.rich_mode.get())
        {
            return Some("自动换行开关作用于当前源码视图；正文按块排版。请先切换源码。".into());
        }
        if crate::rich_commands::handles(id) {
            let doc = match self.current() {
                Some(doc) => doc,
                None => return Some("没有活动文档".into()),
            };
            return crate::rich_commands::availability(&doc.gate, &doc.source, id, true).err();
        }
        if matches!(
            id,
            "texzi.window.new"
                | "texzi.group.splitRight"
                | "texzi.group.splitDown"
                | "texzi.group.joinAll"
                | "texzi.find.inFolder"
        ) {
            return None;
        }
        let reason = match id {
            "texzi.file.save"
            | "texzi.file.saveAs"
            | "texzi.file.saveAll"
            | "texzi.file.saveWithEncoding" => {
                "保存受阻：T07 原生文件请求出现 Windows 错误 87，写盘、编码与原子替换尚未通过验收。本次没有创建或覆盖文件，未保存内容仍在内存。"
            }
            "texzi.file.revert" | "texzi.file.reopenWithEncoding" | "texzi.file.changeEol" => {
                "编码、换行与磁盘重载的保护流程尚未接入，当前文档保持原样。"
            }
            "texzi.file.copyPath" => "复制路径暂未接入本轮菜单外壳。未命名文档也没有磁盘路径。",
            "texzi.group.focusDirection"
            | "texzi.tab.moveToNewWindow"
            | "texzi.tab.copyToNewWindow" => {
                "跨窗口移动与方向组导航尚未接入。请用“转到 → 聚焦编辑组”切换现有组；当前文档保持不变。"
            }
            "texzi.settings.open" | "texzi.settings.keybindings" => {
                "设置和快捷键自定义尚未接入；可用“查看 → 命令面板”查询命令及本版本可用入口。"
            }
            "texzi.tab.closeOthers" | "texzi.tab.closeAll" => {
                "批量关闭的保存/放弃/取消流程尚未接入，请逐个关闭标签。"
            }
            _ if supported(id) => return None,
            _ => "此命令尚未完成接线，当前文档保持原样。",
        };
        Some(reason.into())
    }
    fn explain(self: &Rc<Self>, title: &str, message: &str) {
        self.notice.set(message.into());
        self.modal.set(true);
        let answer = nsh_base::alert_dialog()
            .title(title.to_owned())
            .description(message.to_owned())
            .action("ok", "知道了")
            .close_on_escape(true)
            .show();
        let s = self.clone();
        spawn(async move {
            let _ = answer.await;
            s.modal.set(false);
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
        if let Some(doc) = self.docs.with(|ds| {
            ds.iter()
                .find(|d| {
                    d.path
                        .as_ref()
                        .is_some_and(|p| p.as_os_str().eq_ignore_ascii_case(path.as_os_str()))
                })
                .cloned()
        }) {
            self.active.set(doc.id);
            self.second_active.set(false);
            return;
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.explain("读取失败", &e.to_string());
                return;
            }
        };
        match std::str::from_utf8(&bytes){Ok(raw) if !raw.contains('\0')=>self.add(raw,Some(path)),_=>self.explain("编码不支持","当前仅支持严格 UTF-8 文本；未使用替换字符解码，原文件未改变。完整编码与保存验收仍受阻。")}
    }
    fn pick(self: &Rc<Self>, folder: bool) {
        self.modal.set(true);
        let answer = pick_paths(if folder {
            OpenDialog::folders()
        } else {
            OpenDialog::files().multiple(true)
        });
        let s = self.clone();
        spawn(async move {
            let result = answer.await;
            s.modal.set(false);
            match result {
                Ok(Some(paths)) => {
                    for path in paths {
                        if folder {
                            s.root.set(Some(path));
                            s.sidebar.set(true);
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
            }
        });
    }
    fn close_window(self: &Rc<Self>) {
        if self.modal.get() {
            return;
        }
        let dirty = self.docs.with(|ds| {
            ds.iter()
                .filter(|d| d.source.is_dirty().unwrap_or(true))
                .map(|d| d.name.clone())
                .collect::<Vec<_>>()
        });
        if dirty.is_empty() {
            self.window.close();
            return;
        }
        self.modal.set(true);
        let answer = nsh_base::alert_dialog()
            .title("关闭前保留未保存内容")
            .description(format!(
                "{}\n保存与恢复快照尚未通过验收。关闭将丢失这些内存内容。",
                dirty.join("、")
            ))
            .cancel("cancel", "继续编辑")
            .destructive("discard", "放弃并关闭")
            .close_on_escape(true)
            .show();
        let s = self.clone();
        spawn(async move {
            let discard = answer
                .await
                .is_ok_and(|r| r.action.as_deref() == Some("discard"));
            s.modal.set(false);
            if discard {
                s.window.close();
            }
        });
    }
    fn close_tab(self: &Rc<Self>, id: u64) {
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
            self.modal.set(true);
            let answer = nsh_base::alert_dialog()
                .title("关闭未保存标签")
                .description(format!(
                    "{} 尚未保存。保存当前受阻；可以继续编辑，或明确放弃内容。",
                    doc.name
                ))
                .cancel("cancel", "继续编辑")
                .destructive("discard", "放弃并关闭")
                .close_on_escape(true)
                .show();
            let s = self.clone();
            spawn(async move {
                let discard = answer
                    .await
                    .is_ok_and(|r| r.action.as_deref() == Some("discard"));
                s.modal.set(false);
                if discard {
                    s.remove(id);
                }
            });
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
                self.closed.borrow_mut().push(ds.remove(at));
            }
        });
        if self.active.get() == id {
            self.active
                .set(self.docs.with(|ds| ds.last().map_or(0, |d| d.id)));
        }
        if self.docs.with(|ds| ds.is_empty()) {
            self.add("", None);
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
            self.split_down.set(down);
            self.focus_group(2);
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
            name: doc.name.clone(),
            path: doc.path.clone(),
            source,
            gate,
            reference,
            rich_reference,
            controls,
            wrap,
        })));
        self.split_down.set(down);
        self.second_active.set(true);
        self.notice
            .set("同一文档已拆分为两个独立视图，共享正文与撤销历史".into());
    }
    fn focus_group(&self, index: u8) {
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
    fn close_panel(self: &Rc<Self>) {
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
            "texzi.file.new"=>self.add("",None),"texzi.file.open"=>self.pick(false),"texzi.folder.open"=>self.pick(true),
            "texzi.folder.close"=>{self.root.set(None);self.notice.set("已关闭文件夹，已打开标签保持不变".into());},
            "texzi.window.close"=>self.close_window(),"texzi.tab.close"=>if self.second_active.get(){self.split.set(None);self.second_active.set(false);}else{self.close_tab(self.active.get());},
            "texzi.window.new"=>match std::env::current_exe().and_then(|exe|std::process::Command::new(exe).spawn()){Ok(_)=>self.notice.set("已启动新的 texzi 窗口".into()),Err(e)=>self.explain("新建窗口失败",&e.to_string())},
            "texzi.group.splitRight"|"texzi.group.splitDown"=>self.split_editor(id.ends_with("Down")),
            "texzi.group.joinAll"=>{self.split.set(None);self.second_active.set(false);self.notice.set("已合并为单组；同一文档内容与历史保持不变".into());},
            "texzi.tab.reopenClosed"=>{let doc=self.closed.borrow_mut().pop();if let Some(doc)=doc{let id=doc.id;self.docs.update(|ds|ds.push(doc));self.second_active.set(false);self.active.set(id);self.restore_editor_focus();}else{self.notice.set("没有已关闭标签".into());}},
            "texzi.tab.next"|"texzi.tab.previous"=>{if self.second_active.get(){self.notice.set("当前编辑组只有一个标签，已保持当前组".into());}else{self.docs.with(|ds|{if let Some(at)=ds.iter().position(|d|d.id==self.active.get()){self.active.set(ds[if id.ends_with("next"){(at+1)%ds.len()}else{(at+ds.len()-1)%ds.len()}].id);}});}self.restore_editor_focus();},
            "texzi.sidebar.toggle"=>self.sidebar.update(|v|*v=!*v),"texzi.editor.toggleWrap"=>if let Some(doc)=self.current(){doc.wrap.update(|v|*v=!*v);self.notice.set(if doc.wrap.get(){"当前源码视图：自动换行已开启"}else{"当前源码视图：自动换行已关闭"}.into());},
            "texzi.view.toggleZen"=>self.zen.update(|v|*v=!*v),"texzi.view.exitZen"=>self.zen.set(false),
            "texzi.view.toggleFullScreen"=>self.window.set_mode(if self.window.state().is_some_and(|s|s.mode==WindowMode::FullScreen){WindowMode::Normal}else{WindowMode::FullScreen}),
            "texzi.commandPalette.open"|"texzi.navigation.quickOpen"=>{self.query.set(String::new());self.panel.set(if id.ends_with("quickOpen"){"files"}else{"commands"}.into());},
            "texzi.navigation.gotoLine"=>{if let Some(doc)=self.current(){doc.gate.to_source();if doc.gate.rich_mode.get(){self.notice.set(doc.gate.status.get());return;}}self.query.set(String::new());self.panel.set("line".into());self.notice.set("转到行的范围已切换为源码".into());},
            "texzi.group.focusIndex"=>self.panel.set("groups".into()),
            "texzi.markdown.toggleSource"=>if let Some(doc)=self.current(){if doc.gate.rich_mode.get(){doc.gate.to_source();}else{doc.gate.to_rich();}},
            "texzi.find.open"|"texzi.find.replaceOpen"=>{let replace=id.ends_with("replaceOpen");if replace{if let Some(doc)=self.current(){doc.gate.to_source();if doc.gate.rich_mode.get(){self.notice.set(doc.gate.status.get());return;}}}self.panel.set(if replace{"replace"}else{"find"}.into());let scope=if self.current().is_some_and(|d|d.gate.rich_mode.get()){"正文"}else{"源码"};self.notice.set(format!("查找范围：{scope} · 普通文字 · 区分大小写"));},
            "texzi.find.next"=>self.find(false),"texzi.find.previous"=>self.find(true),"texzi.find.close"=>self.close_panel(),
            "texzi.find.replaceOne"|"texzi.find.replaceAll"=>if let Some(doc)=self.current(){doc.gate.to_source();if doc.gate.rich_mode.get(){self.notice.set(doc.gate.status.get());return;}match doc.source.replace_literal(&self.query.get(),&self.replacement.get(),id.ends_with("All")){Ok(n)=>{self.notice.set(format!("源码 · 已替换 {n} 处，可一次撤销"));self.restore_editor_focus();},Err(e)=>self.notice.set(e)}},
            "texzi.appearance.light"=>set_color_scheme(Appearance::Light),"texzi.appearance.dark"=>set_color_scheme(Appearance::Dark),
            "texzi.help.guide"=>self.explain("使用说明","文件：新建、原生打开请求、目录展开、标签与新窗口。编辑/选择：当前文本控件的操作。查看：命令面板、两组分屏、源码/富文本、换行、主题与 Zen；Ctrl+1/2 聚焦编辑组。文件夹搜索会显示范围、预算、跳过项和取消结果。\n保存、完整编码、跨窗口移动与设置尚未完成；菜单会说明原因。此版本内容仅留在内存。"),
            "texzi.help.about"=>self.explain("关于 texzi","texzi\n为日常文字、Markdown 与代码片段设计的 Windows 原生文本编辑器。"),
            _=>if let Some(command)=source_command(id){if let Err(e)=self.source_action(command){self.notice.set(e);}},
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
fn supported(id: &str) -> bool {
    crate::rich_commands::handles(id)
        || source_command(id).is_some()
        || matches!(
            id,
            "texzi.file.new"
                | "texzi.file.open"
                | "texzi.folder.open"
                | "texzi.folder.close"
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
                | "texzi.group.focusIndex"
                | "texzi.markdown.toggleSource"
                | "texzi.find.open"
                | "texzi.find.replaceOpen"
                | "texzi.find.next"
                | "texzi.find.previous"
                | "texzi.find.close"
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

const CHORDS: &[(&str, &str)] = &[
    ("mod+o", "texzi.folder.open"),
    ("f", "texzi.folder.close"),
    ("mod+b", "texzi.sidebar.toggle"),
    ("z", "texzi.view.toggleZen"),
    ("s", "texzi.file.saveAll"),
    ("mod+s", "texzi.settings.keybindings"),
    ("mod+\\", "texzi.group.splitDown"),
];
fn bind_chords(s: Rc<Shell>) {
    let prefix = s.clone();
    on_key("mod+k", move |_| {
        if prefix.modal.get() || prefix.current().is_some_and(|d| d.gate.busy()) {
            return;
        }
        prefix.chord.set(true);
        prefix.notice.set("Ctrl+K：等待第二个按键…".into());
        let chord = prefix.chord;
        let notice = prefix.notice;
        set_timeout(std::time::Duration::from_millis(1500), move || {
            if chord.get() {
                chord.set(false);
                if notice.get() == "Ctrl+K：等待第二个按键…" {
                    notice.set("Ctrl+K 组合已超时取消".into());
                }
            }
        });
    });
    effect(move || {
        if !s.chord.get() {
            return;
        }
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
                let Ok(parsed) = nsh_core::keymap::Keystroke::parse_with(&stroke, apple_keys())
                else {
                    continue;
                };
                if parsed.reserved(os()).is_some() {
                    continue;
                }
                let target = CHORDS
                    .iter()
                    .find(|(key, _)| *key == stroke)
                    .map(|(_, id)| *id);
                let s = s.clone();
                on_key(&stroke, move |_| {
                    s.chord.set(false);
                    if s.notice.get() == "Ctrl+K：等待第二个按键…" {
                        s.notice.set(String::new());
                    }
                    if let Some(id) = target {
                        s.run(id);
                    } else {
                        s.notice
                            .set("已取消：没有这个 Ctrl+K 组合，未写入正文".into());
                    }
                });
            }
        }
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
    let reference = doc.reference;
    let rich_reference = doc.rich_reference;
    let rich_mode = doc.gate.rich_mode;
    let rich = doc.gate.clone();
    let source = doc.gate.clone();
    let status = doc.gate.status;
    let wrap = doc.wrap;
    let src = doc.gate.clone();
    let formatted = doc.gate.clone();
    let source_owner = s.owner;
    let rich_owner = s.owner;
    let source_focus = s.second_active;
    let rich_focus = s.second_active;
    let block_controls = doc.controls.blocks.clone();
    let resource_controls = doc.controls.resources.clone();
    let table_controls = doc.controls.table.clone();
    let code_controls = doc.controls.code.clone();
    let image_document_path = doc.path.clone();
    let controls_owner = s.owner;
    let controls_shell = s.clone();
    let name = doc.name.clone();
    view()
        .flex_1()
        .child(
            view()
                .flex_row()
                .gap(px(8.0))
                .p(px(6.0))
                .bg(frame)
                .child(text(format!("组 {} · {name}", if second { 2 } else { 1 })))
                .child(
                    nsh_base::button()
                        .child(text("源码"))
                        .on_press(move |_: &()| source_owner.run(|| src.to_source())),
                )
                .child(
                    nsh_base::button()
                        .child(text("富文本"))
                        .on_press(move |_: &()| rich_owner.run(|| formatted.to_rich())),
                )
                .child(text(move || status.get())),
        )
        .child(
            when(rich_mode, move || {
                crate::rich_images::install(&rich.session(), image_document_path.clone());
                let shell = controls_shell.clone();
                let focus: Rc<dyn Fn()> =
                    Rc::new(move || shell.focus_group(if second { 2 } else { 1 }));
                let raw_focus = focus.clone();
                view()
                    .flex_1()
                    .child(
                        view()
                            .flex_row()
                            .flex_wrap()
                            .min_w(px(0.0))
                            .gap(px(8.0))
                            .p(px(6.0))
                            .child(crate::block_controls::view(
                                block_controls.clone(),
                                focus.clone(),
                            ))
                            .child(crate::rich_controls::view(
                                resource_controls.clone(),
                                focus.clone(),
                            ))
                            .child(crate::table_controls::view(
                                table_controls.clone(),
                                focus.clone(),
                            ))
                            .child(crate::code_controls::view(code_controls.clone(), focus)),
                    )
                    .child(crate::raw_block_controls::raw_block_controls(
                        rich.clone(),
                        controls_owner,
                        move || raw_focus(),
                    ))
                    .child(
                        nsh_base::rich_editor::view::rich_editor()
                            .component_ref(&rich_reference)
                            .editor(rich.session())
                            .classes(crate::app::rich_classes())
                            .auto_focus(true)
                            .on_focus(move |_: &()| rich_focus.set(second))
                            .flex_1(),
                    )
            })
            .otherwise(move || {
                source.source_mounted();
                nsh_base::editor::editor()
                    .component_ref(&reference)
                    .auto_focus(true)
                    .on_focus(move |_: &()| source_focus.set(second))
                    .language(
                        if doc
                            .path
                            .as_ref()
                            .and_then(|p| p.extension())
                            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
                        {
                            "markdown"
                        } else {
                            "plaintext"
                        },
                    )
                    .soft_wrap(move || wrap.get())
                    .font_size(px(16.0))
                    .line_height(px(24.0))
                    .size_full()
            }),
        )
}

fn groups(s: Rc<Shell>) -> AnyElement {
    let down = s.split_down.get();
    let primary = s.clone();
    let second = s.clone();
    (if down { view() } else { view().flex_row() })
        .flex_1()
        .child(dynamic(move || {
            primary
                .primary()
                .map(|doc| editor(primary.clone(), doc, false))
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
    let s = Shell::new();
    let toolbar = s.clone();
    let tabs = s.clone();
    let side = s.clone();
    let content = s.clone();
    let panels = s.clone();
    let notice = s.notice;
    let zen = s.zen;
    let docs = s.docs;
    let active = s.active;
    view()
        .size_full()
        .bg(background)
        .text_color(foreground)
        .child(when(
            move || !zen.get(),
            move || {
                view()
                    .flex_row()
                    .gap(px(8.0))
                    .p(px(6.0))
                    .bg(frame)
                    .child(button("新建", toolbar.clone(), "texzi.file.new"))
                    .child(button("打开文件", toolbar.clone(), "texzi.file.open"))
                    .child(button("打开文件夹", toolbar.clone(), "texzi.folder.open"))
                    .child(button(
                        "搜索文件或命令",
                        toolbar.clone(),
                        "texzi.navigation.quickOpen",
                    ))
            },
        ))
        .child(
            view()
                .flex_row()
                .flex_1()
                .child(when(
                    {
                        let s = side.clone();
                        move || s.sidebar.get() && !zen.get()
                    },
                    move || {
                        let s = side.clone();
                        view()
                            .w(px(232.0))
                            .p(px(8.0))
                            .gap(px(8.0))
                            .bg(frame)
                            .child(text("资源管理器"))
                            .child(button("打开文件夹", s.clone(), "texzi.folder.open"))
                            .child(dynamic(move || folder(s.clone())))
                    },
                ))
                .child(
                    view()
                        .flex_1()
                        .child(
                            view()
                                .flex_row()
                                .gap(px(4.0))
                                .p(px(4.0))
                                .bg(frame)
                                .child(each(
                                    move || docs.get(),
                                    |d: &Rc<Document>| d.id,
                                    move |doc, _| {
                                        let doc = doc.get_untracked();
                                        let s = tabs.clone();
                                        let close = tabs.clone();
                                        let id = doc.id;
                                        view()
                                            .flex_row()
                                            .child(
                                                nsh_base::button()
                                                    .child(text(move || {
                                                        doc.source.track();
                                                        format!(
                                                            "{}{}{}",
                                                            if active.get() == id {
                                                                "▎"
                                                            } else {
                                                                ""
                                                            },
                                                            doc.name,
                                                            if doc.source.is_dirty().unwrap_or(true)
                                                            {
                                                                " ●"
                                                            } else {
                                                                ""
                                                            }
                                                        )
                                                    }))
                                                    .on_press(move |_: &()| {
                                                        s.active.set(id);
                                                        s.second_active.set(false);
                                                    }),
                                            )
                                            .child(
                                                nsh_base::button()
                                                    .child(text("×"))
                                                    .on_press(move |_: &()| close.close_tab(id)),
                                            )
                                    },
                                )),
                        )
                        .child(dynamic(move || groups(content.clone())))
                        .child(dynamic(move || panel(panels.clone()))),
                ),
        )
        .child(
            view()
                .h(px(24.0))
                .bg(frame)
                .child(text(move || notice.get())),
        )
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
    scroll()
        .flex_1()
        .child(
            view()
                .child(text(root.display().to_string()))
                .child(text(errors.join("\n")))
                .child(each(
                    move || rows.clone(),
                    |r: &(PathBuf, usize, bool)| r.0.clone(),
                    move |row, _| {
                        let (path, depth, directory) = row.get_untracked();
                        let label = format!(
                            "{}{}{}",
                            "  ".repeat(depth),
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
                        nsh_base::button()
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

fn panel(s: Rc<Shell>) -> Option<AnyElement> {
    let mode = s.panel.get();
    if mode.is_empty() {
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
    if mode == "commands" || mode == "files" {
        let markdown = if mode == "commands" {
            s.current()
                .map(|doc| crate::rich_commands::availability_all(&doc.gate, &doc.source, true))
        } else {
            None
        };
        let entries = if mode == "commands" {
            crate::native_menu::commands()
                .into_iter()
                .map(|c| {
                    // One immutable source projection per palette build, not
                    // one full parse for every Markdown command row.
                    let blocked = markdown
                        .as_ref()
                        .and_then(|results| results.get(&c.id))
                        .map_or_else(|| s.blocked(&c.id).is_some(), Result::is_err);
                    let keys = if c.keys.is_empty() {
                        String::new()
                    } else {
                        format!("    {}", c.keys.join(" / "))
                    };
                    nsh_base::CommandItem::new(
                        c.id.clone(),
                        format!(
                            "{}{}{}",
                            c.name,
                            if blocked { "（暂不可用）" } else { "" },
                            keys
                        ),
                    )
                    .keywords([c.id])
                    .group(c.group)
                })
                .collect::<Vec<_>>()
        } else {
            s.docs.with(|ds| {
                ds.iter()
                    .map(|d| nsh_base::CommandItem::new(format!("file:{}", d.id), d.name.clone()))
                    .collect()
            })
        };
        let run = s.clone();
        let cancel = s.clone();
        let search = s.clone();
        let files = mode == "files";
        panel = panel.child(
            nsh_base::command()
                .items(entries)
                .placeholder(if files {
                    "搜索已打开文件，输入 > 查找命令"
                } else {
                    "搜索命令名称或 ID"
                })
                .auto_focus(true)
                .list_class("max-h-64")
                .on_query_change(move |q: &SharedString| {
                    if files && q.starts_with('>') {
                        search.panel.set("commands".into());
                    }
                })
                .on_select(move |id: &SharedString| {
                    run.panel.set(String::new());
                    if let Some(id) = id.strip_prefix("file:").and_then(|s| s.parse::<u64>().ok()) {
                        run.active.set(id);
                        run.second_active.set(false);
                        run.restore_editor_focus();
                    } else {
                        run.run(id);
                        if run.panel.get().is_empty()
                            && !run.current().is_some_and(|d| d.controls.open())
                        {
                            run.restore_editor_focus();
                        }
                    }
                })
                .on_cancel(move |_: &()| cancel.close_panel()),
        );
    } else if mode == "groups" {
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
                            if let Some(doc) = s.current().filter(|d| d.path == target) {
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
                                d.source.go_to_line_column(line, column)
                            });
                        match result {
                            Ok(()) => {
                                run.close_panel();
                                run.notice.set("已定位源码行列".into());
                            }
                            Err(e) => run.notice.set(e),
                        }
                    }),
            );
    } else {
        let find = s.clone();
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
            panel = panel
                .child(
                    input()
                        .value(move || replacement.get())
                        .placeholder("替换为")
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
