use super::*;
use doc::files::{
    LoadedBytes,
    windows_identity::{IdentityError, ObservedTarget, OpenOutcome},
    windows_save::{BoundFailure, BoundIssue, LockFailure, PublishFailure},
};
use std::path::{Path, PathBuf};

pub(super) struct FileState {
    binding: Option<doc::files::FileBinding>,
    recovery: Option<Recovery>,
}

// Keep the original handles alive while the owner has an unresolved failure.
enum Recovery {
    Write { _failure: BoundFailure },
    Lock { _failure: LockFailure },
    Publish { _failure: PublishFailure },
}

impl FileState {
    pub(super) fn new(binding: Option<doc::files::FileBinding>) -> Self {
        Self {
            binding,
            recovery: None,
        }
    }

    fn retain(
        &mut self,
        recovery: Recovery,
        store: &mut doc::Store,
        capture: &doc::SaveCapture,
        conflict: bool,
        path: Option<PathBuf>,
        details: String,
    ) -> String {
        self.recovery = Some(recovery);
        let result = store.finish_save_failure(
            capture,
            doc::SaveFailure {
                outcome: doc::SaveFailureOutcome::BeforeReplace,
                conflict,
                os_error: None,
                message: details.clone(),
                recovery_paths: path.into_iter().collect(),
            },
        );
        failure_message(details, result)
    }

    fn retain_publication(&mut self, failure: PublishFailure, store: &mut doc::Store) -> String {
        let details = format!("{failure:?}");
        let result = failure.record_failure(store);
        self.recovery = Some(Recovery::Publish { _failure: failure });
        failure_message(details, result)
    }
}

fn failure_message(details: String, recorded: Result<(), doc::Error>) -> String {
    match recorded {
        Ok(()) => format!("{details}\n已保留保存恢复信息和文档，当前需要检查恢复结果。"),
        Err(error) => format!("{details}\n恢复状态记录失败：{error:?}。已保留恢复句柄。"),
    }
}

impl NativeView {
    pub fn new_file() -> Rc<Self> {
        let mut store = doc::Store::new(doc::HistoryBudget::default());
        let (snapshot, binding) = LoadedBytes::decode(std::sync::Arc::from([]), None)
            .and_then(|loaded| loaded.attach(&mut store))
            .expect("empty UTF-8 file");
        Self::from_store(store, snapshot, Some(binding)).expect("empty source view")
    }

    pub fn open_file(path: &Path) -> Result<Rc<Self>, String> {
        let mut store = doc::Store::new(doc::HistoryBudget::default());
        let captured = ObservedTarget::inspect(path)
            .map_err(|e| format!("{e:?}"))?
            .open_existing(None)
            .map_err(|e| format!("{e:?}"))?;
        match captured.attach(&mut store).map_err(|e| format!("{e:?}"))? {
            OpenOutcome::Imported { snapshot, binding } => {
                Self::from_store(store, snapshot, Some(binding))
            }
            OpenOutcome::Existing { .. } => Err("文件已在此 Store 中打开".into()),
        }
    }

    pub fn needs_save_recovery(&self) -> bool {
        self.owner.files.borrow().recovery.is_some()
    }

    // The product has a verified new-target publisher. For a dirty bound
    // file, run current-target guards and stop before staging an overwrite.
    pub fn check_current_save(&self, path: &Path) -> Result<(), String> {
        let target = ObservedTarget::inspect(path).map_err(|e| format!("{e:?}"))?;
        let files = self.owner.files.borrow();
        if files.recovery.is_some() {
            return Err("请先检查上次保存的恢复结果".into());
        }
        let binding = files.binding.as_ref().ok_or("缺少磁盘绑定")?;
        let mut store = self.owner.store.borrow_mut();
        let prepared = target
            .prepare_current(binding, &mut store)
            .map_err(|e| format!("{e:?}"))?;
        store.cancel_save(prepared.capture()).map_err(err)?;
        Err("此候选版本尚未提供安全的原位替换。请另存为一个新文件。".into())
    }

    pub fn save_new_destination(&self, path: &Path) -> Result<(), String> {
        let target = ObservedTarget::inspect(path).map_err(|e| format!("{e:?}"))?;
        if target.version().is_some() {
            return Err("目标文件已存在。请选择新文件名。".into());
        }
        let mut files = self.owner.files.borrow_mut();
        if files.recovery.is_some() {
            return Err("请先检查上次保存的恢复结果".into());
        }
        let binding = files.binding.as_ref().ok_or("缺少文档绑定")?;
        let mut store = self.owner.store.borrow_mut();
        let prepared = target
            .prepare_save_as(binding, &mut store)
            .map_err(|e| format!("{e:?}"))?;
        let capture = prepared.capture().clone();
        let staged = prepared.begin(&mut store).and_then(|work| work.write());
        let staged = staged.map_err(|failure| {
            let conflict = matches!(failure.issue, BoundIssue::Identity(IdentityError::Changed));
            let path = failure.recovery_temp.clone();
            let details = format!("{failure:?}");
            files.retain(
                Recovery::Write { _failure: failure },
                &mut store,
                &capture,
                conflict,
                path,
                details,
            )
        })?;
        let locked = staged.lock_temporary().map_err(|failure| {
            let conflict = matches!(failure.issue, IdentityError::Changed);
            let path = Some(failure.staged.recovery_temp().to_owned());
            let details = format!("{failure:?}");
            files.retain(
                Recovery::Lock { _failure: failure },
                &mut store,
                &capture,
                conflict,
                path,
                details,
            )
        })?;
        let committed = locked
            .prepare_new_owner_only()
            .and_then(|ready| ready.permit(&mut store))
            .and_then(|permitted| permitted.run())
            .and_then(|published| published.accept(&mut store))
            .map_err(|failure| files.retain_publication(failure, &mut store))?;
        files.binding = Some(committed.binding);
        drop(store);
        drop(files);
        self.owner.changed.update(|revision| *revision += 1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_as_preserves_format_updates_shared_dirty_and_rechecks_bound_file() {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        let dir = std::env::temp_dir().join(format!(
            "texzi-save-component-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let original = dir.join("original.txt");
        let destination = dir.join("saved.txt");
        let bytes = [0xff, 0xfe, 0x4e, 0, 0x0d, 0, 0x0a, 0];
        std::fs::write(&original, bytes).unwrap();
        let source = NativeView::open_file(&original).unwrap();
        let rich = source.sibling().unwrap();
        let before = source.snapshot().unwrap();
        source
            .edit(
                before.revision,
                before.view_epoch,
                vec![(1..1, "A".into())],
                ExternalSelections {
                    primary: 0,
                    ranges: vec![(2, 2)],
                },
                ExternalEditKind::Typing,
            )
            .unwrap();
        assert!(source.is_dirty().unwrap());
        assert!(source.save_new_destination(&original).is_err());
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        assert!(rich.is_dirty().unwrap());
        rich.save_new_destination(&destination).unwrap();
        assert!(!rich.is_dirty().unwrap());
        assert!(!source.is_dirty().unwrap());
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            [0xff, 0xfe, 0x4e, 0, 0x41, 0, 0x0d, 0, 0x0a, 0]
        );
        assert_eq!(
            NativeView::open_file(&destination).unwrap().text().unwrap(),
            "NA\n"
        );
        let before = source.snapshot().unwrap();
        source
            .edit(
                before.revision,
                before.view_epoch,
                vec![(2..2, "B".into())],
                ExternalSelections {
                    primary: 0,
                    ranges: vec![(3, 3)],
                },
                ExternalEditKind::Typing,
            )
            .unwrap();
        assert!(
            source
                .check_current_save(&destination)
                .unwrap_err()
                .contains("原位替换")
        );
        std::fs::write(&destination, b"external").unwrap();
        assert!(
            rich.check_current_save(&destination)
                .unwrap_err()
                .contains("Changed")
        );
        assert!(source.is_dirty().unwrap());
        assert_eq!(std::fs::read(&destination).unwrap(), b"external");
        drop(rich);
        drop(source);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
