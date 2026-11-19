use super::*;
use doc::files::{
    LoadedBytes,
    windows_identity::{IdentityError, ObservedTarget, OpenOutcome},
    windows_save::{
        BoundFailure, BoundIssue, LockFailure, PublishFailure, recovery::PublicationRecovery,
    },
};
use std::path::{Path, PathBuf};

pub(super) struct FileState {
    binding: Option<doc::files::FileBinding>,
    recovery: Option<Recovery>,
    #[cfg(test)]
    before_publish: Option<BeforePublish>,
}

#[cfg(test)]
type BeforePublish = Box<dyn FnOnce(&doc::SaveCapture) + Send>;

// Keep the original handles alive while the owner has an unresolved failure.
enum Recovery {
    Write {
        _failure: BoundFailure,
    },
    Lock {
        _failure: LockFailure,
    },
    Publish {
        _failure: PublishFailure,
    },
    WorkerLost {
        _publications: Vec<PublicationRecovery>,
    },
}

enum StagingFailure {
    Write(BoundFailure),
    Lock(LockFailure),
    Prepare(PublishFailure),
}

impl FileState {
    pub(super) fn new(binding: Option<doc::files::FileBinding>) -> Self {
        Self {
            binding,
            recovery: None,
            #[cfg(test)]
            before_publish: None,
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

    #[cfg(test)]
    pub(crate) fn before_next_publication(&self, hook: BeforePublish) {
        self.owner.files.borrow_mut().before_publish = Some(hook);
    }

    // The product has a verified new-target publisher. For a dirty bound
    // file, run current-target guards and stop before staging an overwrite.
    pub async fn check_current_save(&self, path: PathBuf) -> Result<(), String> {
        let target = inspect_destination(path).await?;
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

    pub async fn save_new_destination(&self, path: PathBuf) -> Result<(), String> {
        let target = inspect_destination(path).await?;
        if target.version().is_some() {
            return Err("目标文件已存在。请选择新文件名。".into());
        }
        let (capture, work) = {
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
            let work = prepared.begin(&mut store).map_err(|failure| {
                retain_staging_failure(
                    &mut files,
                    &mut store,
                    &capture,
                    StagingFailure::Write(failure),
                )
            })?;
            (capture, work)
        };
        let staged = blocking(move || {
            let staged = work.write().map_err(StagingFailure::Write)?;
            let locked = staged.lock_temporary().map_err(StagingFailure::Lock)?;
            locked
                .prepare_new_owner_only()
                .map_err(StagingFailure::Prepare)
        })
        .await
        .map_err(|error| self.record_lost_save_worker(&capture, false, error.to_string()))?;
        let permitted = {
            let mut files = self.owner.files.borrow_mut();
            let mut store = self.owner.store.borrow_mut();
            let ready = staged.map_err(|failure| {
                retain_staging_failure(&mut files, &mut store, &capture, failure)
            })?;
            ready
                .permit(&mut store)
                .map_err(|failure| files.retain_publication(failure, &mut store))?
        };
        #[cfg(test)]
        let before_publish = self.owner.files.borrow_mut().before_publish.take();
        let published = blocking(move || {
            #[cfg(test)]
            if let Some(hook) = before_publish {
                hook(permitted.capture());
            }
            permitted.run()
        })
        .await
        .map_err(|error| self.record_lost_save_worker(&capture, true, error.to_string()))?;
        {
            let mut files = self.owner.files.borrow_mut();
            let mut store = self.owner.store.borrow_mut();
            let committed = published
                .and_then(|published| published.accept(&mut store))
                .map_err(|failure| files.retain_publication(failure, &mut store))?;
            files.binding = Some(committed.binding);
        }
        self.owner.changed.update(|revision| *revision += 1);
        Ok(())
    }

    fn record_lost_save_worker(
        &self,
        capture: &doc::SaveCapture,
        permitted: bool,
        error: String,
    ) -> String {
        let mut store = self.owner.store.borrow_mut();
        let publications = if permitted {
            store.drain_dropped_publications()
        } else {
            Vec::new()
        };
        let details = format!("保存后台任务未返回：{error}");
        let current = publications
            .iter()
            .find(|notice| notice.capture().operation == capture.operation);
        let recorded = if let Some(notice) = current {
            notice.owner_error.clone().map_or(Ok(()), Err)
        } else {
            store.finish_save_failure(
                capture,
                doc::SaveFailure {
                    outcome: if permitted {
                        doc::SaveFailureOutcome::OutcomeUnknown
                    } else {
                        doc::SaveFailureOutcome::BeforeReplace
                    },
                    conflict: permitted,
                    os_error: None,
                    message: details.clone(),
                    recovery_paths: Vec::new(),
                },
            )
        };
        self.owner.files.borrow_mut().recovery = Some(Recovery::WorkerLost {
            _publications: publications,
        });
        match recorded {
            Ok(()) => format!("{details}\n已记录恢复状态，文档仍保留。"),
            Err(error) => format!("{details}\n无法记录恢复状态：{error:?}。文档仍保留。"),
        }
    }
}

async fn inspect_destination(path: PathBuf) -> Result<ObservedTarget, String> {
    blocking(move || ObservedTarget::inspect(&path))
        .await
        .map_err(|error| format!("无法完成保存路径检查：{error}"))?
        .map_err(|error| format!("{error:?}"))
}

fn retain_staging_failure(
    files: &mut FileState,
    store: &mut doc::Store,
    capture: &doc::SaveCapture,
    failure: StagingFailure,
) -> String {
    match failure {
        StagingFailure::Write(failure) => {
            let conflict = matches!(failure.issue, BoundIssue::Identity(IdentityError::Changed));
            let path = failure.recovery_temp.clone();
            let details = format!("{failure:?}");
            files.retain(
                Recovery::Write { _failure: failure },
                store,
                capture,
                conflict,
                path,
                details,
            )
        }
        StagingFailure::Lock(failure) => {
            let conflict = matches!(failure.issue, IdentityError::Changed);
            let path = Some(failure.staged.recovery_temp().to_owned());
            let details = format!("{failure:?}");
            files.retain(
                Recovery::Lock { _failure: failure },
                store,
                capture,
                conflict,
                path,
                details,
            )
        }
        StagingFailure::Prepare(failure) => files.retain_publication(failure, store),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_as_preserves_format_updates_shared_dirty_and_rechecks_bound_file() {
        let worker = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
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
        assert!(
            worker
                .block_on(source.save_new_destination(original.clone()))
                .is_err()
        );
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        assert!(rich.is_dirty().unwrap());
        worker
            .block_on(rich.save_new_destination(destination.clone()))
            .unwrap();
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
            worker
                .block_on(source.check_current_save(destination.clone()))
                .unwrap_err()
                .contains("原位替换")
        );
        std::fs::write(&destination, b"external").unwrap();
        assert!(
            worker
                .block_on(rich.check_current_save(destination.clone()))
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
