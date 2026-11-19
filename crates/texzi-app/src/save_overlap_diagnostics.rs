//! Opt-in, one-process diagnostic for a generated Windows save fixture.
use nsh_core::prelude::{blocking, timer};
use serde_json::json;
use std::{
    fs, io,
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use texzi_document::{Revision, SaveCapture, files::windows_identity::ObservedTarget};

const SWITCH: &str = "--tx07-save-overlap";
const FIXTURE_PREFIX: &str = "texzi-tx07-overlap-";
const SAVE_NAME: &str = "capture.txt";
const REPARSE_POINT: u32 = 0x400;
const WAIT: Duration = Duration::from_secs(45);
const POLL: Duration = Duration::from_millis(40);
static ARMED: OnceLock<Gate> = OnceLock::new();

pub(crate) struct Gate {
    target: ObservedTarget,
    root: PathBuf,
    used: AtomicBool,
}

#[derive(Clone, Copy)]
pub(crate) struct EditState {
    pub(crate) revision: Revision,
    pub(crate) dirty: bool,
    pub(crate) fixture_edit: bool,
}

pub(crate) fn configure() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let Some(_) = args.find(|arg| arg == SWITCH) else {
        return Ok(());
    };
    let root = args.next().ok_or("missing generated fixture directory")?;
    if args.any(|arg| arg == SWITCH) {
        return Err("duplicate diagnostic switch".into());
    }
    arm(Path::new(&root))
}

fn arm(root: &Path) -> Result<(), String> {
    ARMED
        .set(Gate::open(root)?)
        .map_err(|_| "diagnosis was already armed".into())
}

#[cfg(test)]
pub(crate) fn arm_test_fixture(root: &Path) -> Result<(), String> {
    arm(root)
}

impl Gate {
    fn open(root: &Path) -> Result<Self, String> {
        let nonce = root
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix(FIXTURE_PREFIX));
        if !root.is_absolute()
            || !nonce.is_some_and(|nonce| {
                nonce.len() == 32 && nonce.as_bytes().iter().all(u8::is_ascii_hexdigit)
            })
        {
            return Err("expected a generated, absolute TX07 fixture directory".into());
        }
        let metadata = fs::symlink_metadata(root).map_err(|error| error.to_string())?;
        if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
            return Err("fixture directory must be a normal directory".into());
        }
        let root = fs::canonicalize(root).map_err(|error| error.to_string())?;
        let temp = fs::canonicalize(std::env::temp_dir()).map_err(|error| error.to_string())?;
        if root.parent() != Some(temp.as_path())
            || fs::read_dir(&root)
                .map_err(|error| error.to_string())?
                .next()
                .is_some()
        {
            return Err("fixture directory must be an empty direct child of Windows TEMP".into());
        }
        let target =
            ObservedTarget::inspect(&root.join(SAVE_NAME)).map_err(|error| format!("{error:?}"))?;
        let root = target
            .resolved_path()
            .parent()
            .ok_or("missing pinned fixture directory")?
            .to_owned();
        Ok(Self {
            target,
            root,
            used: AtomicBool::new(false),
        })
    }

    pub(crate) async fn wait(
        &self,
        capture: &SaveCapture,
        observe: impl Fn() -> Result<EditState, String>,
    ) -> Result<Option<Trace>, String> {
        if self.used.swap(true, Ordering::Relaxed) {
            return Ok(None);
        }
        if capture.binding_epoch != 0
            || capture.snapshot.body.len_bytes() != 1
            || capture.snapshot.body.to_utf8() != b"N"
        {
            return Err("诊断保存只接受生成的未绑定单字符文档".into());
        }
        let mut trace = Trace::new(capture);
        trace.emit("captured", None);
        let deadline = Instant::now() + WAIT;
        let expected_edit = capture.snapshot.revision.0.checked_add(1);
        let mut observed_edit = false;
        loop {
            let root = self.root.clone();
            let markers = blocking(move || {
                Ok::<_, io::Error>((marker(&root, "cancel")?, marker(&root, "release")?))
            })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
            let state = observe()?;
            let next_edit =
                expected_edit == Some(state.revision.0) && state.dirty && state.fixture_edit;
            if markers.0 || Instant::now() >= deadline {
                trace.end(if markers.0 { "cancelled" } else { "timed_out" }, state);
                return Err("诊断保存已取消，编辑内容保留".into());
            }
            if markers.1 {
                if !observed_edit || !next_edit {
                    trace.end("premature_release", state);
                    return Err("诊断放行时没有已确认的下一修订，保存已取消".into());
                }
                trace.emit("released", Some(state));
                return Ok(Some(trace));
            }
            if next_edit && !observed_edit {
                trace.emit("edited", Some(state));
                observed_edit = true;
            }
            timer(POLL).await;
        }
    }
}

fn marker(root: &Path, name: &str) -> io::Result<bool> {
    match fs::symlink_metadata(root.join(name)) {
        Ok(entry)
            if entry.is_file()
                && entry.len() == 0
                && entry.file_attributes() & REPARSE_POINT == 0 =>
        {
            Ok(true)
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid diagnostic marker",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(crate) fn for_destination(target: &ObservedTarget) -> Option<&'static Gate> {
    ARMED.get().filter(|gate| {
        target.version().is_none()
            && target.parent_identity() == gate.target.parent_identity()
            && target.resolved_path() == gate.target.resolved_path()
    })
}

pub(crate) struct Trace {
    operation: u64,
    revision: u64,
    terminal: bool,
}

impl Trace {
    fn new(capture: &SaveCapture) -> Self {
        Self {
            operation: capture.operation,
            revision: capture.snapshot.revision.0,
            terminal: false,
        }
    }

    fn emit(&self, event: &str, state: Option<EditState>) {
        let utc_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis());
        eprintln!(
            "TX07_SAVE_OVERLAP {}",
            json!({
                "event": event,
                "utc_ms": utc_ms,
                "pid": std::process::id(),
                "operation": self.operation,
                "captured_revision": self.revision,
                "current_revision": state.map(|state| state.revision.0),
                "dirty": state.map(|state| state.dirty),
            })
        );
    }

    fn end(&mut self, event: &str, state: EditState) {
        self.terminal = true;
        self.emit(event, Some(state));
    }

    pub(crate) fn complete(mut self, state: EditState) {
        self.end("completed", state);
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        if !self.terminal {
            self.emit("failed", None);
        }
    }
}
