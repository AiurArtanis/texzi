use super::{Document, Shell, display_path};
use crate::{
    editor_owner::{MAX_EDIT_BYTES, NativeView, PrepareError, PreparedFile},
    settings,
};
use nsh_base::{
    editor::{EditorApi, ExternalDocument, ExternalSelections},
    rich_editor::view::RichEditorApi,
};
use nsh_core::{desktop::WindowPlacement, geometry::point, prelude::*};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, BufReader, BufWriter, Write},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use texzi_document::{
    Body, Eol,
    files::{Encoding, Format, paged::FileSource},
};

const INTERVAL: Duration = Duration::from_secs(2);
const NEW_WINDOW: &str = "--texzi-new-window";
const RESUME_WINDOW: &str = "--texzi-restore-window=";

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct LocalPath(Vec<u16>);

impl LocalPath {
    fn from_path(path: &Path) -> Self {
        Self(path.as_os_str().encode_wide().collect())
    }

    fn to_path(&self) -> PathBuf {
        OsString::from_wide(&self.0).into()
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct EditorView {
    pub(super) rich: bool,
    wrap: Option<bool>,
    tabs: Option<u32>,
    selection: Option<(Vec<(usize, usize)>, usize)>,
    scroll: Option<(f32, f32)>,
}

impl EditorView {
    fn capture(doc: &Document) -> Self {
        let displayed = doc.displayed_rich();
        let rich = doc.pending_mode.get().unwrap_or(displayed)
            && doc.source.disk_len() <= super::large_file::SOURCE_ONLY_BYTES;
        Self {
            rich,
            wrap: doc.wrap.get(),
            tabs: doc.tab_size.get(),
            selection: doc.gate.recovery_selection().map(|s| (s.ranges, s.primary)),
            scroll: (displayed == rich)
                .then(|| scroll_position(doc))
                .flatten()
                .or_else(|| {
                    doc.retained_scroll
                        .get()
                        .and_then(|(mode, x, y)| (mode == rich).then_some((x, y)))
                }),
        }
    }

    pub(super) fn for_pending(rich: bool, saved: Option<&Self>) -> Self {
        let mut view = saved.cloned().unwrap_or(Self {
            rich,
            wrap: None,
            tabs: None,
            selection: None,
            scroll: None,
        });
        if view.rich != rich {
            view.rich = rich;
            view.scroll = None;
        }
        view
    }

    pub(super) fn restore_source(&self, source: &NativeView) {
        if let Some((ranges, primary)) = &self.selection
            && let Ok(snapshot) = source.snapshot()
        {
            let _ = source.select(
                snapshot.revision,
                snapshot.view_epoch,
                ExternalSelections {
                    ranges: ranges.clone(),
                    primary: *primary,
                },
            );
        }
    }

    pub(super) fn restore_view(&self, doc: &Document) {
        self.restore_in_mode(doc, doc.displayed_rich());
    }

    pub(super) fn restore_in_mode(&self, doc: &Document, rich: bool) {
        doc.wrap.set(self.wrap);
        doc.tab_size
            .set(self.tabs.filter(|width| (1..=8).contains(width)));
        doc.retained_scroll.set(
            self.scroll
                .filter(|_| self.rich == rich)
                .map(|(x, y)| (rich, x, y)),
        );
    }

    pub(super) fn has_scroll(&self) -> bool {
        self.scroll.is_some()
    }
}

fn scroll_position(doc: &Document) -> Option<(f32, f32)> {
    if doc.displayed_rich() {
        doc.rich_reference
            .api::<RichEditorApi>()
            .map(|editor| (0.0, editor.scroll_top()))
    } else {
        doc.reference.api::<EditorApi>().map(|editor| {
            let offset = editor.scroll_offset();
            (offset.x.0, offset.y.0)
        })
    }
}

pub(super) fn keep_scroll(doc: &Document) {
    if let Some((x, y)) = scroll_position(doc) {
        doc.retained_scroll.set(Some((doc.displayed_rich(), x, y)));
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct Draft {
    text: Arc<str>,
    #[serde(flatten)]
    format: FilePreference,
    disk_sha256: Option<[u8; 32]>,
}

impl Draft {
    fn verified_baseline(
        &self,
        source: &FileSource,
        path: &Path,
        limit: u64,
        cancel: &AtomicBool,
    ) -> Option<Arc<[u8]>> {
        let expected = self.disk_sha256?;
        let bytes = source.complete_bytes(limit, cancel).ok()?;
        source.verify_path(path).ok()?;
        (digest(&bytes) == expected).then(|| Arc::from(bytes))
    }

    pub(super) fn prepare(
        &self,
        source: &FileSource,
        path: &Path,
        limit: u64,
        cancel: &AtomicBool,
    ) -> Result<(PreparedFile, bool), PrepareError> {
        PreparedFile::check_recovery_size(self.text.len()).map_err(PrepareError::RecoveryLimit)?;
        let original = self.verified_baseline(source, path, limit, cancel);
        let named = original.is_some();
        let (format, eol) = self.format.restore();
        PreparedFile::from_recovery(self.text.clone(), format, eol, original, cancel)
            .map(|prepared| (prepared, named))
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct FilePreference {
    encoding: String,
    bom: bool,
    eol: String,
}

struct DraftCapture {
    body: Arc<Body>,
    format: Format,
    baseline: Option<Arc<[u8]>>,
}

impl DraftCapture {
    fn materialize(self) -> Draft {
        Draft {
            text: String::from_utf8(self.body.to_utf8())
                .expect("document recovery bytes are UTF-8")
                .into(),
            format: FilePreference::capture(self.format, self.body.default_eol()),
            disk_sha256: self.baseline.as_deref().map(digest),
        }
    }
}

impl FilePreference {
    fn capture(format: Format, eol: Eol) -> Self {
        let encoding = match format.encoding {
            Encoding::Utf8 => "utf-8",
            Encoding::Utf16Le => "utf-16le",
            Encoding::Utf16Be => "utf-16be",
            Encoding::Gbk936 => "gbk-936",
            Encoding::GbkWebEncodingRs0844 => "gbk",
            Encoding::Windows1252 => "windows-1252",
            Encoding::Gb18030_2022 => "gb18030-2022",
            Encoding::Gb18030WebEncodingRs0844 => "gb18030",
        };
        let eol = match eol {
            Eol::Lf => "LF",
            Eol::CrLf => "CRLF",
            Eol::Cr => "CR",
        };
        Self {
            encoding: encoding.into(),
            bom: format.bom,
            eol: eol.into(),
        }
    }

    fn restore(&self) -> (Format, Eol) {
        let (mut format, mut eol) = settings::new_document_format(&self.encoding, &self.eol);
        format.bom = self.bom;
        if self.encoding == "gbk-936" {
            format.encoding = Encoding::Gbk936;
        } else if self.encoding == "gb18030-2022" {
            format.encoding = Encoding::Gb18030_2022;
        }
        if self.eol == "CR" {
            eol = Eol::Cr;
        }
        (format, eol)
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct Tab {
    id: u64,
    path: Option<LocalPath>,
    view: EditorView,
    draft: Option<Draft>,
    blank: Option<FilePreference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archived: Option<ArchivedDraft>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct ArchivedDraft {
    preview_id: u64,
    reason: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct Split {
    pub(super) id: u64,
    down: bool,
    active: bool,
    view: EditorView,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct WindowRecord {
    version: u32,
    folder: Option<LocalPath>,
    sidebar: bool,
    sidebar_width: f32,
    placement: Option<WindowPlacement>,
    tabs: Vec<Tab>,
    active: u64,
    split: Option<Split>,
}

struct Disk {
    path: PathBuf,
    _lease: File,
    committed: Mutex<u64>,
}

impl Disk {
    fn claim(directory: &Path, id: &str) -> io::Result<Option<Arc<Self>>> {
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(format!("{id}.lock")))?;
        if lease.try_lock().is_err() {
            return Ok(None);
        }
        Ok(Some(Arc::new(Self {
            path: directory.join(format!("{id}.json")),
            _lease: lease,
            committed: Mutex::new(0),
        })))
    }

    fn read(&self) -> io::Result<WindowRecord> {
        // serde_json reads one byte at a time from a Read source.
        let record: WindowRecord =
            serde_json::from_reader(BufReader::new(File::open(&self.path)?))?;
        if record.version != 1
            || record
                .tabs
                .iter()
                .any(|tab| tab.id == 0 || tab.id == u64::MAX)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "不支持此恢复记录版本",
            ));
        }
        Ok(record)
    }

    fn update(
        &self,
        sequence: u64,
        make_record: impl FnOnce() -> io::Result<Option<WindowRecord>>,
    ) -> io::Result<()> {
        let mut committed = self
            .committed
            .lock()
            .map_err(|_| io::Error::other("恢复写入锁不可用"))?;
        if sequence <= *committed {
            return Ok(());
        }
        let Some(record) = make_record()? else {
            *committed = sequence;
            return Ok(());
        };
        let temporary = self.path.with_extension("tmp");
        let result = (|| {
            let mut file = BufWriter::new(File::create(&temporary)?);
            serde_json::to_writer(&mut file, &record)?;
            file.write_all(b"\n")?;
            file.flush()?;
            file.get_ref().sync_all()?;
            drop(file);
            fs::rename(&temporary, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        *committed = sequence;
        Ok(())
    }
}

type Signature = (WindowRecord, Vec<(u64, u64)>);

#[derive(Clone, PartialEq)]
pub(super) enum ArchiveState {
    Writing,
    Unavailable,
    Written,
    Failed(String),
}

#[derive(Clone, PartialEq)]
pub(super) struct ArchiveLocation {
    id: u64,
    preview: u64,
    pub(super) reason: String,
    pub(super) original: Option<PathBuf>,
    pub(super) record: PathBuf,
    pub(super) state: ArchiveState,
}

impl ArchiveLocation {
    fn from_tab(tab: &Tab, record: &Path, state: ArchiveState) -> Option<Self> {
        let archive = tab.archived.as_ref()?;
        tab.draft.as_ref()?;
        Some(Self {
            id: tab.id,
            preview: archive.preview_id,
            reason: archive.reason.clone(),
            original: tab.path.as_ref().map(LocalPath::to_path),
            record: record.to_owned(),
            state,
        })
    }

    pub(super) fn record_file(&self) -> io::Result<&Path> {
        if fs::symlink_metadata(&self.record)?.is_file() {
            Ok(&self.record)
        } else {
            Err(io::Error::other("恢复记录位置不是文件"))
        }
    }

    pub(super) fn matches_original(&self, path: Option<&Path>) -> bool {
        match (self.original.as_deref(), path) {
            (Some(original), Some(path)) => {
                display_path(original).eq_ignore_ascii_case(&display_path(path))
            }
            (None, None) => true,
            _ => false,
        }
    }
}

pub(super) struct Session {
    disk: Arc<Disk>,
    sequence: Cell<u64>,
    last: RefCell<Option<Signature>>,
    writing: Cell<bool>,
    retained: RefCell<Vec<Tab>>,
    archives: Signal<Vec<ArchiveLocation>>,
}

impl Session {
    fn new(disk: Arc<Disk>) -> Self {
        Self {
            disk,
            sequence: Cell::new(0),
            last: RefCell::new(None),
            writing: Cell::new(false),
            retained: RefCell::new(Vec::new()),
            archives: signal(Vec::new()),
        }
    }

    fn merge_archived_drafts(&self, record: &mut WindowRecord) -> bool {
        let mut changed = false;
        for tab in self.retained.borrow().iter() {
            let Some(archive) = &tab.archived else {
                continue;
            };
            if record.tabs.iter().any(|entry| entry.id == tab.id) {
                continue;
            }
            if let Some(preview) = record
                .tabs
                .iter_mut()
                .find(|entry| entry.id == archive.preview_id)
                && preview.draft == tab.draft
            {
                preview.draft = None;
            }
            record.tabs.push(tab.clone());
            changed = true;
        }
        changed
    }

    fn archive_checkpoint(&self, record: &WindowRecord, state: ArchiveState) {
        let mut locations = self.archives.get_untracked();
        let mut changed = false;
        for tab in &record.tabs {
            let Some(location) = ArchiveLocation::from_tab(tab, &self.disk.path, state.clone())
            else {
                continue;
            };
            if let Some(existing) = locations.iter_mut().find(|entry| entry.id == location.id) {
                if *existing != location
                    && (existing.state != ArchiveState::Written || state == ArchiveState::Written)
                {
                    *existing = location;
                    changed = true;
                }
            } else {
                locations.push(location);
                changed = true;
            }
        }
        if changed {
            self.archives.set(locations);
        }
    }
}

pub(super) fn archive_location(shell: &Shell, preview: u64) -> Option<ArchiveLocation> {
    let session = shell.session.borrow().clone()?;
    session.archives.with(|locations| {
        locations
            .iter()
            .rfind(|entry| entry.preview == preview)
            .cloned()
    })
}

pub(super) fn restoring_archived_original(shell: &Shell, preview: u64, path: &Path) -> bool {
    archive_location(shell, preview).is_some_and(|entry| {
        entry.state == ArchiveState::Written && entry.matches_original(Some(path))
    })
}

pub(super) fn retain_limited_draft(shell: &Rc<Shell>, doc: &Document, reason: &str) {
    let Some(session) = shell.session.borrow().clone() else {
        return;
    };
    let Some(reader) = &doc.large else { return };
    let Some(draft) = reader.restored_draft.get_untracked() else {
        return;
    };
    let id = shell.next.get();
    let Some(next) = id.checked_add(1) else {
        return;
    };
    shell.next.set(next);
    let tab = Tab {
        id,
        path: doc.path.get().as_deref().map(LocalPath::from_path),
        view: reader.recovery_view(),
        draft: Some(draft),
        blank: None,
        archived: Some(ArchivedDraft {
            preview_id: doc.id,
            reason: reason.into(),
        }),
    };
    let state = if enabled(shell) {
        ArchiveState::Writing
    } else {
        ArchiveState::Unavailable
    };
    if let Some(location) = ArchiveLocation::from_tab(&tab, &session.disk.path, state) {
        session
            .archives
            .update(|locations| locations.push(location));
    }
    session.retained.borrow_mut().push(tab);
    reader.restored_draft.set(None);
    checkpoint(shell, true);
}

pub(super) fn retained_draft_message(shell: &Shell, reason: &str) -> String {
    let path = shell
        .session
        .borrow()
        .as_ref()
        .map(|session| display_path(&session.disk.path))
        .unwrap_or_else(|| "%LOCALAPPDATA%\\texzi\\recovery".into());
    format!("{reason} · 草稿仍保存在恢复记录 {path}")
}
fn enabled(shell: &Shell) -> bool {
    shell.settings.effective(settings::RESTORE, None).0 == true
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 68 && id.bytes().all(|b| b.is_ascii_digit() || b == b'-')
}

pub(super) fn spawn_blank_window() -> io::Result<()> {
    Command::new(std::env::current_exe()?)
        .arg(NEW_WINDOW)
        .spawn()
        .map(|_| ())
}

pub(super) fn start(shell: &Rc<Shell>) -> io::Result<()> {
    if cfg!(test) {
        return Ok(());
    }
    let Some(base) = std::env::var_os("LOCALAPPDATA") else {
        return Ok(());
    };
    let directory = PathBuf::from(base).join("texzi").join("recovery");
    fs::create_dir_all(&directory)?;
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let targeted = args
        .iter()
        .find_map(|arg| arg.to_str()?.strip_prefix(RESUME_WINDOW))
        .filter(|id| valid_id(id));
    let mut queues = Vec::new();
    if enabled(shell) && !args.iter().any(|arg| arg == NEW_WINDOW) {
        if let Some(id) = targeted {
            queues.push(id.to_owned());
        } else {
            let mut files = fs::read_dir(&directory)?
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let path = entry.path();
                    let id = path.file_stem()?.to_str()?.to_owned();
                    (path.extension()?.eq_ignore_ascii_case("json") && valid_id(&id)).then(|| {
                        (
                            entry
                                .metadata()
                                .and_then(|metadata| metadata.modified())
                                .unwrap_or(UNIX_EPOCH),
                            id,
                        )
                    })
                })
                .collect::<Vec<_>>();
            files.sort_unstable_by(|a, b| b.cmp(a));
            queues.extend(files.into_iter().map(|(_, id)| id));
        }
    }

    let mut chosen = None;
    let mut other_windows = Vec::new();
    for id in queues {
        let Some(disk) = Disk::claim(&directory, &id)? else {
            continue;
        };
        match disk.read() {
            Ok(record) if !record.tabs.is_empty() => {
                if chosen.is_none() {
                    chosen = Some((disk, record));
                } else {
                    other_windows.push(id);
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() != io::ErrorKind::NotFound => {
                log::warn!("无法读取恢复记录 {id}：{error}");
            }
            Err(_) => {}
        }
    }
    let (disk, record) = if let Some((disk, record)) = chosen {
        (disk, Some(record))
    } else {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let id = format!("{}-{now}", std::process::id());
        (
            Disk::claim(&directory, &id)?
                .ok_or_else(|| io::Error::other("无法锁定本窗口的恢复记录"))?,
            None,
        )
    };
    *shell.session.borrow_mut() = Some(Rc::new(Session::new(disk)));
    if let Some(record) = record {
        restore(shell, record);
    }
    for id in other_windows {
        if let Err(error) = Command::new(std::env::current_exe()?)
            .arg(format!("{RESUME_WINDOW}{id}"))
            .spawn()
        {
            log::warn!("无法恢复窗口 {id}：{error}");
        }
    }
    periodic(shell);
    Ok(())
}

fn periodic(shell: &Rc<Shell>) {
    let owner = shell.owner;
    let weak = Rc::downgrade(shell);
    owner.run(|| {
        set_timeout(INTERVAL, move || {
            if let Some(shell) = weak.upgrade() {
                checkpoint(&shell, false);
                periodic(&shell);
            }
        })
    });
}

fn collect(shell: &Shell, drafts: bool) -> (Signature, Vec<(u64, DraftCapture)>) {
    let mut revisions = Vec::new();
    let mut bodies = Vec::new();
    let mut tabs: Vec<_> = shell.docs.with(|docs| {
        docs.iter()
            .filter_map(|doc| {
                let path = doc.path.get().as_deref().map(LocalPath::from_path);
                if let Some(large) = &doc.large {
                    let draft = large.restored_draft.get();
                    if !drafts && draft.is_some() {
                        return None;
                    }
                    return Some(Tab {
                        id: doc.id,
                        path,
                        view: large.recovery_view(),
                        draft,
                        blank: None,
                        archived: None,
                    });
                }
                let revision = doc.source.dirty_revision();
                let view = EditorView::capture(doc);
                let mut blank = None;
                if drafts
                    && let Some(revision) = revision
                    && let Some(body) = doc.source.current_body()
                {
                    let (format, baseline) = doc.source.recovery_format();
                    revisions.push((doc.id, revision));
                    bodies.push((
                        doc.id,
                        DraftCapture {
                            body,
                            format,
                            baseline,
                        },
                    ));
                } else if revision.is_some() {
                    return None;
                } else if path.is_none() {
                    if view.wrap.is_none() && view.tabs.is_none() {
                        return None;
                    }
                    let (format, _) = doc.source.recovery_format();
                    let eol = doc.source.current_body()?.default_eol();
                    blank = Some(FilePreference::capture(format, eol));
                }
                Some(Tab {
                    id: doc.id,
                    path,
                    view,
                    draft: None,
                    blank,
                    archived: None,
                })
            })
            .collect()
    });
    if let Some(session) = shell.session.borrow().as_ref() {
        tabs.extend(session.retained.borrow().iter().cloned());
    }
    let record = WindowRecord {
        version: 1,
        folder: shell.root.get().as_deref().map(LocalPath::from_path),
        sidebar: shell.sidebar.get(),
        sidebar_width: shell.sidebar_width.get(),
        placement: shell.window.state().map(|state| state.placement),
        tabs,
        active: shell.active.get(),
        split: shell
            .split
            .get()
            .map(|doc| Split {
                id: doc.id,
                down: shell.split_down.get(),
                active: shell.second_active.get(),
                view: EditorView::capture(&doc),
            })
            .or_else(|| {
                shell
                    .recovery_split
                    .borrow()
                    .as_ref()
                    .map(|(_, split)| split.clone())
            }),
    };
    ((record, revisions), bodies)
}

pub(super) fn checkpoint(shell: &Rc<Shell>, force: bool) {
    let Some(session) = shell.session.borrow().clone() else {
        return;
    };
    if !enabled(shell) || !force && session.writing.get() {
        return;
    }
    let (signature, bodies) = collect(shell, true);
    if !force && session.last.borrow().as_ref() == Some(&signature) {
        return;
    }
    let record = signature.0.clone();
    session.sequence.set(session.sequence.get() + 1);
    let sequence = session.sequence.get();
    let disk = session.disk.clone();
    session.writing.set(true);
    let owner = shell.owner;
    let shell = Rc::downgrade(shell);
    owner.run(|| {
        spawn(async move {
            let result = blocking(move || {
                let mut record = record;
                for (id, body) in bodies {
                    if let Some(tab) = record.tabs.iter_mut().find(|tab| tab.id == id) {
                        tab.draft = Some(body.materialize());
                    }
                }
                disk.update(sequence, || {
                    Ok((!record.tabs.is_empty() || disk.path.is_file()).then_some(record))
                })
            })
            .await;
            if session.sequence.get() != sequence {
                return;
            }
            session.writing.set(false);
            match result {
                Ok(Ok(())) => {
                    session.archive_checkpoint(&signature.0, ArchiveState::Written);
                    *session.last.borrow_mut() = Some(signature);
                }
                Ok(Err(error)) => {
                    session
                        .archive_checkpoint(&signature.0, ArchiveState::Failed(error.to_string()));
                    if let Some(shell) = shell.upgrade() {
                        shell.notice.set(format!("未能写入恢复快照 · {error}"));
                    }
                }
                Err(error) => {
                    session
                        .archive_checkpoint(&signature.0, ArchiveState::Failed(error.to_string()));
                    log::warn!("恢复写入任务退出：{error}");
                }
            }
        })
    });
}

pub(super) fn saved(shell: &Rc<Shell>, doc: &Document) {
    if enabled(shell) {
        checkpoint(shell, true);
        return;
    }
    if doc.source.is_dirty().unwrap_or(true) {
        return;
    }
    let Some(session) = shell.session.borrow().clone() else {
        return;
    };
    session.sequence.set(session.sequence.get() + 1);
    let path = doc.path.get().as_deref().map(LocalPath::from_path);
    let result = session.disk.update(session.sequence.get(), || {
        let mut record = match session.disk.read() {
            Ok(record) => record,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        session.merge_archived_drafts(&mut record);
        if let Some(tab) = record.tabs.iter_mut().find(|tab| tab.id == doc.id) {
            tab.draft = None;
            tab.path = path;
        }
        Ok(Some(record))
    });
    session.writing.set(false);
    *session.last.borrow_mut() = None;
    if let Err(error) = result {
        log::warn!("保存后未能更新恢复记录：{error}");
    }
}

pub(super) fn remove_closed_tab(shell: &Shell, id: u64) -> io::Result<()> {
    let Some(session) = shell.session.borrow().clone() else {
        return Ok(());
    };
    session.sequence.set(session.sequence.get() + 1);
    session.writing.set(false);
    *session.last.borrow_mut() = None;
    session.disk.update(session.sequence.get(), || {
        let mut record = match session.disk.read() {
            Ok(record) => record,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        session.merge_archived_drafts(&mut record);
        record.tabs.retain(|tab| tab.id != id);
        Ok(Some(record))
    })
}

pub(super) fn finish(shell: &Shell, discard_dirty: bool) -> io::Result<()> {
    let Some(session) = shell.session.borrow().clone() else {
        return Ok(());
    };
    session.sequence.set(session.sequence.get() + 1);
    session.writing.set(false);
    *session.last.borrow_mut() = None;
    if enabled(shell) {
        let record = collect(shell, false).0.0;
        return session.disk.update(session.sequence.get(), || {
            Ok((!record.tabs.is_empty() || session.disk.path.is_file()).then_some(record))
        });
    }
    let has_archived = session
        .retained
        .borrow()
        .iter()
        .any(|tab| tab.archived.is_some());
    if discard_dirty || has_archived {
        let discarded: Vec<_> = shell.docs.with(|docs| {
            docs.iter()
                .filter(|doc| discard_dirty && doc.needs_close_confirmation())
                .map(|doc| doc.id)
                .collect()
        });
        session.disk.update(session.sequence.get(), || {
            let mut record = match session.disk.read() {
                Ok(record) => record,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            };
            let archived = session.merge_archived_drafts(&mut record);
            let before = record.tabs.len();
            record.tabs.retain(|tab| !discarded.contains(&tab.id));
            Ok((archived || before != record.tabs.len()).then_some(record))
        })?;
    }
    Ok(())
}

fn restore(shell: &Rc<Shell>, record: WindowRecord) {
    if let Some(session) = shell.session.borrow().as_ref() {
        session.archive_checkpoint(&record, ArchiveState::Written);
    }
    shell.docs.set(Vec::new());
    shell.split.set(None);
    shell.second_active.set(false);
    shell.recovery_split.borrow_mut().take();
    if let Some(path) = record
        .folder
        .as_ref()
        .map(LocalPath::to_path)
        .filter(|path| path.is_dir())
    {
        shell.show_folder(path);
    }
    shell.sidebar.set(record.sidebar);
    shell
        .sidebar_width
        .set(record.sidebar_width.clamp(120.0, 520.0));
    if let Some(placement) = record.placement {
        shell.window.set_placement(placement);
    }
    let mut unavailable = 0;
    let mut retained = Vec::new();
    let mut first_error = None;
    let max_id = record.tabs.iter().map(|tab| tab.id).max().unwrap_or(0);
    for tab in record.tabs {
        if let Some(archive) = &tab.archived {
            first_error.get_or_insert_with(|| archive.reason.clone());
            retained.push(tab);
            continue;
        }
        match restore_tab(shell, &tab) {
            Ok(true) => {}
            Ok(false) => unavailable += 1,
            Err(error) => {
                first_error.get_or_insert(error);
                retained.push(tab);
            }
        }
    }
    shell.next.set(max_id.saturating_add(1));
    if shell.docs.with(|docs| docs.is_empty()) {
        shell.add_blank();
    }
    if shell
        .docs
        .with(|docs| docs.iter().any(|doc| doc.id == record.active))
    {
        shell.active.set(record.active);
    }
    if let Some(split) = record.split
        && let Some(primary) = shell
            .docs
            .with(|docs| docs.iter().find(|doc| doc.id == split.id).cloned())
    {
        if primary.editable_body_ready() {
            restore_split(shell, &primary, record.active, split);
        } else {
            *shell.recovery_split.borrow_mut() = Some((record.active, split));
        }
    }
    let retained_count = retained.len();
    if let Some(session) = shell.session.borrow().as_ref() {
        *session.retained.borrow_mut() = retained;
    }
    if let Some(error) = first_error {
        shell.notice.set(retained_draft_message(
            shell,
            &format!("有 {retained_count} 份草稿未能载入（{error}）"),
        ));
    } else if unavailable > 0 {
        shell.notice.set(format!(
            "会话已恢复 · {unavailable} 个文件无法重新关联，未覆盖现有文件"
        ));
    }
}

fn restore_split(shell: &Rc<Shell>, primary: &Rc<Document>, expected_active: u64, split: Split) {
    if let Some(second) = shell.mount_split(primary, split.down) {
        split.view.restore_source(&second.source);
        let rich = shell.initial_markdown_rich(
            second.path.get().as_deref(),
            second.source.disk_len(),
            Some(split.view.rich),
        );
        split.view.restore_in_mode(&second, rich);
        if rich {
            shell.request_mode_with_position(second, true, split.view.has_scroll());
        }
        if split.active && shell.active.get() == expected_active && !shell.settings_active() {
            shell.second_active.set(true);
        }
    }
}

pub(super) fn restore_waiting_split(shell: &Rc<Shell>, doc: &Rc<Document>) {
    let pending = {
        let mut pending = shell.recovery_split.borrow_mut();
        if !pending
            .as_ref()
            .is_some_and(|(_, split)| split.id == doc.id)
        {
            return;
        }
        pending.take()
    };
    if let Some((expected_active, split)) = pending {
        restore_split(shell, doc, expected_active, split);
    }
}

fn restore_tab(shell: &Rc<Shell>, tab: &Tab) -> Result<bool, String> {
    let mut path = tab.path.as_ref().map(LocalPath::to_path);
    if let Some(path) = &mut path {
        if let Ok(canonical) = path.canonicalize() {
            *path = canonical;
        }
    }
    let named = path.is_some();
    shell.next.set(tab.id);
    if let Some(draft) = &tab.draft {
        PreparedFile::check_recovery_size(draft.text.len())?;
        let pinned = path.as_deref().and_then(|path| FileSource::open(path).ok());
        let baseline = match (path.as_ref(), pinned) {
            (Some(path), Some(file))
                if (super::large_file::LARGE_BYTES..=MAX_EDIT_BYTES).contains(&file.len)
                    && draft.disk_sha256.is_some() =>
            {
                shell.open_large(
                    path.clone(),
                    Arc::new(file),
                    None,
                    Some(tab.view.clone()),
                    Some(draft.clone()),
                );
                return Ok(true);
            }
            (Some(path), Some(file)) => draft.verified_baseline(
                &file,
                path,
                super::large_file::LARGE_BYTES - 1,
                &AtomicBool::new(false),
            ),
            _ => None,
        };
        if baseline.is_none() {
            path = None;
        }
        let (format, eol) = draft.format.restore();
        let source = NativeView::recover_file(draft.text.clone(), format, eol, baseline)?;
        tab.view.restore_source(&source);
        let detached = named && path.is_none();
        shell.add_with_mode(source, path, Some(tab.view.rich && !detached));
        if let Some(doc) = shell.primary() {
            tab.view.restore_view(&doc);
            doc.auto_save.ignore_restored(doc.source.dirty_revision());
        }
        Ok(!detached)
    } else if let Some(path) = path {
        Ok(shell
            .open_canonical_with_view(path, Some(tab.view.clone()))
            .is_ok())
    } else if let Some(blank) = &tab.blank {
        let (format, eol) = blank.restore();
        let source = shell.owner.run(|| NativeView::new_file(format, eol));
        tab.view.restore_source(&source);
        shell.add_with_mode(source, None, Some(false));
        if let Some(doc) = shell.primary() {
            tab.view.restore_view(&doc);
        }
        Ok(true)
    } else {
        Ok(true)
    }
}

pub(super) fn apply_scroll(doc: &Document, rich: bool) {
    let Some((mode, x, y)) = doc.retained_scroll.get() else {
        return;
    };
    if mode != rich {
        return;
    }
    if rich {
        if let Some(editor) = doc.rich_reference.api::<RichEditorApi>() {
            editor.set_scroll_top(y);
            doc.retained_scroll.set(None);
        }
    } else if let Some(editor) = doc.reference.api::<EditorApi>() {
        editor.set_scroll_offset(point(px(x), px(y)));
        doc.retained_scroll.set(None);
    }
}

#[cfg(test)]
#[path = "recovery_save_copy_tests.rs"]
mod save_copy_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_core::{ApproxTextEngine, Core, Size, element::Root};

    thread_local! {
        static RECOVERY_PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    pub(super) fn recovery_tab(path: &Path, bytes: &[u8]) -> Tab {
        Tab {
            id: 4,
            path: Some(LocalPath::from_path(path)),
            view: EditorView {
                rich: false,
                wrap: Some(true),
                tabs: Some(8),
                selection: Some((vec![(2, 6)], 0)),
                scroll: Some((0.0, 20.0)),
            },
            draft: Some(Draft {
                text: "private draft\r\nsecond\n".into(),
                format: FilePreference {
                    encoding: "utf-8".into(),
                    bom: false,
                    eol: "LF".into(),
                },
                disk_sha256: Some(digest(bytes)),
            }),
            blank: None,
            archived: None,
        }
    }

    pub(super) fn record(tab: Tab) -> WindowRecord {
        WindowRecord {
            version: 1,
            folder: None,
            sidebar: true,
            sidebar_width: 240.0,
            placement: None,
            active: tab.id,
            tabs: vec![tab],
            split: None,
        }
    }

    #[component]
    fn RecoveryFixture() -> impl IntoElement {
        let directory = RECOVERY_PATH.with(|slot| slot.borrow_mut().take()).unwrap();
        let path = directory.join("named.txt");
        let disk = Disk::claim(&directory, "410").unwrap().unwrap();
        let initial = recovery_tab(&path, b"published");
        disk.update(1, || Ok(Some(record(initial.clone()))))
            .unwrap();
        let restored = Shell::new();
        restore(&restored, disk.read().unwrap());
        let tab = restored.current().unwrap();
        assert_eq!(tab.id, 4);
        assert_eq!(
            tab.path.get().as_deref(),
            Some(path.canonicalize().unwrap().as_path())
        );
        assert_eq!(
            tab.source.snapshot().unwrap().selections.ranges,
            vec![(2, 6)]
        );
        assert_eq!(restored.tab_size(&tab), 8);
        assert!(tab.source.is_dirty().unwrap());
        assert_eq!(
            tab.source.current_body().unwrap().to_utf8(),
            b"private draft\r\nsecond\n"
        );
        assert_eq!(fs::read(&path).unwrap(), b"published");

        let blank_format = Format {
            encoding: Encoding::Utf16Be,
            bom: true,
        };
        restored.add(NativeView::new_file(blank_format, Eol::Lf), None);
        let blank = restored.current().unwrap();
        blank.tab_size.set(Some(8));
        let ((metadata, _), _) = collect(&restored, false);
        let blank_record = metadata
            .tabs
            .into_iter()
            .find(|entry| entry.id == blank.id)
            .unwrap();
        assert!(blank_record.draft.is_none() && blank_record.blank.is_some());
        let mut with_blank = disk.read().unwrap();
        with_blank.active = blank.id;
        with_blank.tabs.push(blank_record);
        restore(&restored, with_blank);
        let blank = restored.current().unwrap();
        assert!(blank.path.get().is_none() && !blank.source.is_dirty().unwrap());
        assert_eq!(restored.tab_size(&blank), 8);
        assert_eq!(blank.source.recovery_format().0, blank_format);
        assert_eq!(blank.source.current_body().unwrap().default_eol(), Eol::Lf);

        fs::write(&path, b"changed elsewhere").unwrap();
        restore(&restored, disk.read().unwrap());
        let tab = restored.current().unwrap();
        assert!(tab.path.get().is_none());
        assert!(tab.source.is_dirty().unwrap());
        assert_eq!(tab.source.text().unwrap(), "private draft\nsecond\n");
        assert_eq!(fs::read(&path).unwrap(), b"changed elsewhere");

        tab.source.set_composing(true);
        restored.add_blank();
        let independent = restored.current().unwrap();
        let snapshot = independent.source.snapshot().unwrap();
        independent
            .source
            .edit(
                snapshot.revision,
                snapshot.view_epoch,
                vec![(0..0, "other tab".into())],
                ExternalSelections {
                    ranges: vec![(9, 9)],
                    primary: 0,
                },
                nsh_base::editor::ExternalEditKind::Typing,
            )
            .unwrap();
        let (_, captured) = collect(&restored, true);
        assert_eq!(captured.len(), 2);
        assert!(
            captured.iter().any(
                |(id, capture)| *id == independent.id && capture.body.to_utf8() == b"other tab"
            )
        );
        assert!(
            captured.iter().any(|(id, capture)| *id == tab.id
                && capture.body.to_utf8() == b"private draft\r\nsecond\n")
        );
        tab.source.set_composing(false);

        let session = Rc::new(Session::new(disk.clone()));
        session.sequence.set(1);
        *restored.session.borrow_mut() = Some(session);
        assert!(
            restored
                .settings
                .apply(settings::RESTORE, None, Some(false.into()))
        );
        finish(&restored, false).unwrap();
        assert_eq!(disk.read().unwrap().tabs.len(), 1);
        let recovery = restored.session.borrow().clone().unwrap();
        recovery.writing.set(true);
        let obstructed = disk.path.with_extension("tmp");
        fs::create_dir(&obstructed).unwrap();
        assert!(remove_closed_tab(&restored, 4).is_err());
        assert!(!recovery.writing.get());
        assert!(tab.source.is_dirty().unwrap());
        assert_eq!(disk.read().unwrap().tabs.len(), 1);
        fs::remove_dir(&obstructed).unwrap();
        remove_closed_tab(&restored, 4).unwrap();
        assert!(disk.read().unwrap().tabs.is_empty());
        disk.update(1, || Ok(Some(record(initial)))).unwrap();
        assert!(
            disk.read().unwrap().tabs.is_empty(),
            "a stale checkpoint must not restore a discarded tab"
        );

        *restored.session.borrow_mut() = None;
        drop(recovery);
        drop(disk);
        fs::remove_file(path).unwrap();
        fs::remove_file(directory.join("410.json")).unwrap();
        fs::remove_file(directory.join("410.lock")).unwrap();
        fs::remove_dir(directory).unwrap();
        text("")
    }

    #[test]
    fn recovery_restores_a_dirty_draft_and_never_overwrites_an_external_edit() {
        let directory =
            std::env::temp_dir().join(format!("texzi-recovery-test-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("named.txt"), b"published").unwrap();
        RECOVERY_PATH.with(|slot| *slot.borrow_mut() = Some(directory));
        let mut core = Core::new(
            ApproxTextEngine,
            Size {
                width: px(800.0),
                height: px(600.0),
            },
        );
        core.mount_root(&Root::new::<RecoveryFixture>(|| ()));
        core.run_component_tasks();
        let _ = core.commit();
    }

    #[component]
    fn RetainedDraftFixture() -> impl IntoElement {
        let directory = RECOVERY_PATH.with(|slot| slot.borrow_mut().take()).unwrap();
        let path = directory.join("named.txt");
        let disk = Disk::claim(&directory, "411").unwrap().unwrap();
        let mut original_tab = recovery_tab(&path, b"previous disk");
        // 2,000,000 LF boundaries add an empty final line.
        original_tab.draft.as_mut().unwrap().text = "a\n".repeat(2_000_000).into();
        disk.update(1, || Ok(Some(record(original_tab.clone()))))
            .unwrap();
        let restored = Shell::new();
        let session = Rc::new(Session::new(disk.clone()));
        session.sequence.set(1);
        *restored.session.borrow_mut() = Some(session.clone());
        restore(&restored, disk.read().unwrap());

        assert!(
            !restored
                .docs
                .with(|docs| docs.iter().any(|doc| doc.id == original_tab.id))
        );
        assert!(restored.notice.get().contains("2000000 行"));
        assert!(restored.notice.get().contains("411.json"));
        assert_eq!(fs::read(&path).unwrap(), b"published");
        assert!(PreparedFile::check_recovery_size(MAX_EDIT_BYTES as usize + 1).is_err());

        let doc = restored.current().unwrap();
        doc.pending_mode.set(Some(true));
        assert!(!doc.displayed_rich());
        assert!(EditorView::capture(&doc).rich);
        assert!(EditorView::for_pending(true, None).rich);
        assert!(!EditorView::for_pending(false, Some(&EditorView::capture(&doc))).rich);

        let ((checkpoint_record, _), bodies) = collect(&restored, true);
        assert!(bodies.is_empty());
        assert!(checkpoint_record.tabs == vec![original_tab.clone()]);
        disk.update(2, || Ok(Some(checkpoint_record))).unwrap();
        session.sequence.set(2);
        finish(&restored, false).unwrap();
        assert!(disk.read().unwrap().tabs == vec![original_tab]);
        assert_eq!(fs::read(&path).unwrap(), b"published");

        *restored.session.borrow_mut() = None;
        drop(session);
        drop(disk);
        fs::remove_file(path).unwrap();
        fs::remove_file(directory.join("411.json")).unwrap();
        fs::remove_file(directory.join("411.lock")).unwrap();
        fs::remove_dir(directory).unwrap();
        text("")
    }

    #[test]
    fn an_over_limit_draft_is_kept_by_checkpoint_and_graceful_close() {
        let directory =
            std::env::temp_dir().join(format!("texzi-recovery-over-limit-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("named.txt"), b"published").unwrap();
        RECOVERY_PATH.with(|slot| *slot.borrow_mut() = Some(directory));
        let mut core = Core::new(
            ApproxTextEngine,
            Size {
                width: px(800.0),
                height: px(600.0),
            },
        );
        core.mount_root(&Root::new::<RetainedDraftFixture>(|| ()));
        core.run_component_tasks();
        let _ = core.commit();
    }

    #[component]
    fn AsyncArchiveFixture() -> impl IntoElement {
        let dir = RECOVERY_PATH.with(|slot| slot.borrow_mut().take()).unwrap();
        let path = dir.join("named.txt");
        let disk = Disk::claim(&dir, "412").unwrap().unwrap();
        let mut original = recovery_tab(&path, b"published");
        original.draft.as_mut().unwrap().text = "a\n".repeat(2_000_000).into();
        disk.update(1, || Ok(Some(record(original.clone()))))
            .unwrap();
        let shell = Shell::new();
        let session = Rc::new(Session::new(disk.clone()));
        session.sequence.set(1);
        *shell.session.borrow_mut() = Some(session.clone());
        shell.next.set(original.id + 1);

        let file = Arc::new(FileSource::open(&path).unwrap());
        let draft = original.draft.clone().unwrap();
        let Err(PrepareError::RecoveryLimit(reason)) =
            draft.prepare(&file, &path, MAX_EDIT_BYTES, &AtomicBool::new(false))
        else {
            panic!("the async worker must report a typed recovery limit");
        };
        let reader = super::super::large_file::LargeFile::new(
            path.clone(),
            file,
            false,
            Some(original.view.clone()),
            Some(draft.clone()),
        );
        let preview = shell.make_document(
            original.id,
            NativeView::reader_placeholder(),
            Some(path.clone()),
            Some(reader.clone()),
            false,
        );
        shell.show_new(preview.clone());
        assert!(preview.needs_close_confirmation());
        assert!(
            shell
                .settings
                .apply(settings::RESTORE, None, Some(false.into()))
        );
        reader.finish_recovery_limit(&shell, &preview, &reason);
        reader.finish_recovery_limit(&shell, &preview, &reason);
        assert!(!preview.needs_close_confirmation());
        assert!(!preview.editable_body_ready());
        assert!(reader.restored_draft.get().is_none());
        assert_eq!(session.retained.borrow().len(), 1);
        assert!(
            archive_location(&shell, original.id)
                .is_some_and(|entry| entry.state == ArchiveState::Unavailable)
        );
        let ((checkpoint_record, _), bodies) = collect(&shell, false);
        assert!(bodies.is_empty());
        assert_eq!(checkpoint_record.tabs.len(), 2);
        assert!(
            checkpoint_record
                .tabs
                .iter()
                .any(|tab| tab.archived.is_some() && tab.draft.as_ref() == Some(&draft))
        );

        finish(&shell, false).unwrap();
        let saved = disk.read().unwrap();
        assert_eq!(saved.tabs.len(), 2);
        assert!(
            saved
                .tabs
                .iter()
                .any(|tab| tab.id == original.id && tab.draft.is_none())
        );
        let archive = saved
            .tabs
            .iter()
            .find(|tab| tab.archived.is_some())
            .unwrap();
        assert_ne!(archive.id, original.id);
        assert!(archive.draft.as_ref() == Some(&draft));

        restore(&shell, saved);
        assert_eq!(shell.docs.with(Vec::len), 1);
        assert_eq!(shell.current().unwrap().id, original.id);
        assert!(
            archive_location(&shell, original.id)
                .is_some_and(|entry| entry.state == ArchiveState::Written)
        );
        assert_eq!(session.retained.borrow().len(), 1);
        assert_eq!(fs::read(&path).unwrap(), b"published");
        remove_closed_tab(&shell, original.id).unwrap();
        let remaining = disk.read().unwrap().tabs;
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].draft.as_ref() == Some(&draft));

        *shell.session.borrow_mut() = None;
        drop(session);
        drop(preview);
        drop(reader);
        drop(disk);
        fs::remove_file(path).unwrap();
        fs::remove_file(dir.join("412.json")).unwrap();
        fs::remove_file(dir.join("412.lock")).unwrap();
        fs::remove_dir(dir).unwrap();
        text("")
    }

    #[component]
    fn SavedArchiveLinksFixture() -> impl IntoElement {
        let directory = RECOVERY_PATH.with(|slot| slot.borrow_mut().take()).unwrap();
        let paths =
            ["first", "second", "unrelated"].map(|part| directory.join(part).join("同名.txt"));
        let disk = Disk::claim(&directory, "413").unwrap().unwrap();
        let mut tabs = Vec::new();
        for (index, path) in paths.iter().enumerate() {
            let mut preview = recovery_tab(path, b"unchanged");
            preview.id = index as u64 + 4;
            if index < 2 {
                let mut archive = preview.clone();
                archive.id = 20 + index as u64;
                archive.archived = Some(ArchivedDraft {
                    preview_id: preview.id,
                    reason: format!("草稿 {index} 超限"),
                });
                tabs.push(archive);
            }
            preview.draft = None;
            tabs.push(preview);
        }
        let mut persisted = record(tabs[0].clone());
        persisted.tabs = tabs;
        persisted.active = 4;
        disk.update(1, || Ok(Some(persisted))).unwrap();
        let shell = Shell::new();
        let session = Rc::new(Session::new(disk.clone()));
        *shell.session.borrow_mut() = Some(session);
        restore(&shell, disk.read().unwrap());

        let (first, second, unrelated) = shell.docs.with(|docs| {
            let by_id = |id| docs.iter().find(|doc| doc.id == id).cloned().unwrap();
            (by_id(4), by_id(5), by_id(6))
        });
        assert!(first.large.is_some() && second.large.is_some());
        assert!(unrelated.editable_body_ready());
        assert!(archive_location(&shell, 6).is_none());
        assert!(!restoring_archived_original(&shell, 4, &paths[2]));
        let first = archive_location(&shell, 4).unwrap();
        let second = archive_location(&shell, 5).unwrap();
        assert!(first.matches_original(Some(&paths[0])));
        assert!(second.matches_original(Some(&paths[1])));
        assert_ne!(first.original, second.original);
        assert_eq!(first.record_file().unwrap(), disk.path);
        assert_eq!(second.record_file().unwrap(), disk.path);
        let moved = directory.join("moved record.json");
        fs::rename(&disk.path, &moved).unwrap();
        assert_eq!(
            first.record_file().unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        fs::rename(&moved, &disk.path).unwrap();
        assert!(second.record_file().is_ok());
        let saved = disk.read().unwrap();
        assert_eq!(
            saved
                .tabs
                .iter()
                .filter(|tab| tab.archived.is_some() && tab.draft.is_some())
                .count(),
            2
        );
        assert!(
            paths
                .iter()
                .all(|path| fs::read(path).unwrap() == b"unchanged")
        );
        text("")
    }

    #[test]
    fn archive_links_follow_both_saved_previews_and_reject_a_moved_record() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("texzi-archive-links-{nonce}"));
        fs::create_dir(&directory).unwrap();
        for name in ["first", "second", "unrelated"] {
            let parent = directory.join(name);
            fs::create_dir(&parent).unwrap();
            fs::write(parent.join("同名.txt"), b"unchanged").unwrap();
        }
        RECOVERY_PATH.with(|slot| *slot.borrow_mut() = Some(directory.clone()));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut core = Core::new(ApproxTextEngine, Size::new(px(800.0), px(600.0)));
        core.set_task_runtime(runtime.handle().clone());
        core.mount_root(&Root::new::<SavedArchiveLinksFixture>(|| ()));
        core.run_component_tasks();
        let _ = core.commit();
        drop(core);
        drop(runtime);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn async_recovery_limit_archives_draft_without_marking_the_reader_dirty() {
        let dir = std::env::temp_dir().join(format!(
            "texzi-recovery-async-archive-{}",
            std::process::id()
        ));
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("named.txt"), b"published").unwrap();
        RECOVERY_PATH.with(|slot| *slot.borrow_mut() = Some(dir));
        let mut core = Core::new(
            ApproxTextEngine,
            Size {
                width: px(800.0),
                height: px(600.0),
            },
        );
        core.mount_root(&Root::new::<AsyncArchiveFixture>(|| ()));
        core.run_component_tasks();
        let _ = core.commit();
    }
}
