//! Pre-permit staging only. Never replaces/deletes a target or reports save success.
use super::PreparedSave;
use crate::{SaveStage, Store};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone)]
pub enum ContentGuard {
    Absent,
    Exact(Arc<[u8]>),
}
#[derive(Debug)]
pub enum StageIssue {
    Io(std::io::Error),
    Document(crate::Error),
    Conflict,
    InvalidTarget,
    TempNameExhausted,
}
#[derive(Debug)]
pub struct StageFailure {
    pub issue: StageIssue,
    pub recovery_temp: Option<PathBuf>,
}
pub struct StagedSave {
    temp: PathBuf,
    target: PathBuf,
}
impl StagedSave {
    pub fn recovery_temp(&self) -> &Path {
        &self.temp
    }
    pub fn target(&self) -> &Path {
        &self.target
    }
}
impl ContentGuard {
    /// Content comparison only; not a Windows file-ID/reparse resolver or CAS.
    pub fn verify(&self, target: &Path) -> Result<(), StageIssue> {
        let metadata = match fs::symlink_metadata(target) {
            Ok(m) => Some(m),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(StageIssue::Io(e)),
        };
        match (self, metadata) {
            (Self::Absent, None) => Ok(()),
            (Self::Exact(expected), Some(m)) if m.is_file() && !m.file_type().is_symlink() => {
                let actual = fs::read(target).map_err(StageIssue::Io)?;
                if actual.as_slice() == expected.as_ref() {
                    Ok(())
                } else {
                    Err(StageIssue::Conflict)
                }
            }
            _ => Err(StageIssue::Conflict),
        }
    }
}
impl PreparedSave {
    /// Reserves an exclusive sibling, writes the captured payload, flushes and
    /// rereads it, then rechecks the target. Errors keep reservation and temp.
    /// There is deliberately no publish/commit/success function before identity
    /// coordination and the post-permit failure API have been reviewed.
    pub fn stage_temp(
        &self,
        store: &mut Store,
        target: &Path,
        guard: &ContentGuard,
    ) -> Result<StagedSave, StageFailure> {
        let fail = |issue| StageFailure {
            issue,
            recovery_temp: None,
        };
        if !target.is_absolute() || target.file_name().is_none() {
            return Err(fail(StageIssue::InvalidTarget));
        }
        let parent = target
            .parent()
            .ok_or_else(|| fail(StageIssue::InvalidTarget))?;
        guard.verify(target).map_err(fail)?;
        store
            .advance_save(&self.capture, SaveStage::WritingTemp)
            .map_err(|e| fail(StageIssue::Document(e)))?;
        let mut chosen = None;
        for attempt in 0..64 {
            let path = parent.join(format!(
                ".texzi-{}-{}-{attempt}.tmp",
                std::process::id(),
                self.capture.operation
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    chosen = Some((path, file));
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(fail(StageIssue::Io(e))),
            }
        }
        let (temp, mut file) = chosen.ok_or_else(|| fail(StageIssue::TempNameExhausted))?;
        let work = (|| -> Result<(), StageIssue> {
            file.write_all(&self.bytes).map_err(StageIssue::Io)?;
            file.sync_all().map_err(StageIssue::Io)?;
            let actual = fs::read(&temp).map_err(StageIssue::Io)?;
            if actual.as_slice() != self.bytes.as_ref() {
                return Err(StageIssue::Conflict);
            }
            guard.verify(target)?;
            // Stay WritingTemp until a filesystem identity coordinator verifies
            // the target/parent/epoch and explicitly authorizes ReadyToReplace.
            Ok(())
        })();
        work.map_err(|issue| StageFailure {
            issue,
            recovery_temp: Some(temp.clone()),
        })?;
        Ok(StagedSave {
            temp,
            target: target.to_owned(),
        })
    }
}
