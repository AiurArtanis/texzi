//! Generated-file CreationDefault fixtures. Source only; never use user documents.
use super::*;
use crate::files::{Encoding, LoadedBytes, windows_identity};
use std::sync::Mutex;
use windows_sys::Win32::Security::{
    GetSecurityDescriptorControl, GetSecurityDescriptorDacl, SE_DACL_PROTECTED,
};

const CAPTURED_UTF16: [u8; 10] = [0xff, 0xfe, 78, 0, 65, 0, 13, 0, 10, 0];

#[derive(Clone)]
struct CreationReceipt {
    handle: usize,
    identity: FileIdentity,
    security: SecurityDescriptor,
    path: PathBuf,
}

struct CreationFixture {
    dir: PathBuf,
    target: PathBuf,
    store: Store,
    n: crate::Snapshot,
    locked: LockedStaged,
    created: CreationReceipt,
    private: SecurityDescriptor,
    private_probe: File,
}

fn generated_directory(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "texzi-t07-creation-{label}-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    eprintln!(
        "T07 generated CreationDefault fixture retained: {}",
        dir.display()
    );
    dir
}

fn directory_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    entries
}

fn assert_non_null_dacl(descriptor: &SecurityDescriptor, protected: bool) {
    let descriptor = descriptor.as_bytes().as_ptr().cast_mut().cast();
    let mut present = 0;
    let mut defaulted = 0;
    let mut dacl = std::ptr::null_mut();
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: read-only OS-produced self-relative descriptor remains live;
    // all output pointers refer to initialized local storage.
    assert_ne!(
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) },
        0
    );
    assert_ne!(present, 0);
    assert!(!dacl.is_null());
    assert_ne!(
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) },
        0
    );
    if protected {
        assert_ne!(control & SE_DACL_PROTECTED, 0);
    }
}

fn creation_fixture(label: &str) -> CreationFixture {
    let dir = generated_directory(label);
    let target = dir.join("saved.txt");
    let mut store = Store::new(HistoryBudget::default());
    let (initial, binding) = LoadedBytes::decode(Arc::from(&b"A\r\n"[..]), Some(Encoding::Utf8))
        .unwrap()
        .attach(&mut store)
        .unwrap();
    let mut profile = initial.body.profile;
    profile.file_format = Some(Format {
        encoding: Encoding::Utf16Le,
        bom: true,
    });
    let formatted = store
        .set_serialization(initial.doc, initial.revision, profile, None)
        .unwrap()
        .snapshot;
    let n = store
        .submit(Transaction {
            doc: initial.doc,
            base: formatted.revision,
            edits: vec![Edit::input(&formatted, 0..0, "N")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    let mut worker = ObservedTarget::inspect(&target)
        .unwrap()
        .prepare_bound(&binding, &mut store)
        .unwrap()
        .begin(&mut store)
        .unwrap();
    let created = Arc::new(Mutex::new(None::<CreationReceipt>));
    let creation_hook = created.clone();
    worker.work.set_creation_hook(move |file, path| {
        assert_eq!(file.metadata()?.len(), 0);
        assert_separate_reader_denied(path);
        let security = SecurityDescriptor::read(file)?;
        assert_non_null_dacl(&security, false);
        *creation_hook.lock().unwrap() = Some(CreationReceipt {
            handle: file.as_raw_handle() as usize,
            identity: windows_identity::identity(file).unwrap(),
            security,
            path: path.to_owned(),
        });
        Ok(())
    });
    let private = Arc::new(Mutex::new(None::<SecurityDescriptor>));
    let private_hook = private.clone();
    let creation_for_private = created.clone();
    worker.work.set_security_hook(move |file| {
        let created = creation_for_private.lock().unwrap();
        let created = created.as_ref().unwrap();
        assert_eq!(file.as_raw_handle() as usize, created.handle);
        assert_eq!(windows_identity::identity(file).unwrap(), created.identity);
        assert_eq!(file.metadata()?.len(), 0);
        assert_separate_reader_denied(&created.path);
        let security = SecurityDescriptor::read(file)?;
        assert_non_null_dacl(&security, true);
        *private_hook.lock().unwrap() = Some(security);
        Ok(())
    });
    let staged = worker.write().unwrap();
    let created = created.lock().unwrap().clone().unwrap();
    let private = private.lock().unwrap().clone().unwrap();
    assert_ne!(private, created.security);
    assert_eq!(staged.temporary_identity(), created.identity);
    assert_eq!(staged.creation_security(), &created.security);
    assert_eq!(staged.temporary_security(), &private);
    assert_eq!(staged.payload(), CAPTURED_UTF16);
    assert_eq!(staged.read_created_bytes().unwrap(), CAPTURED_UTF16);
    assert_eq!(staged.capture().snapshot.token, n.token);
    let locked = staged.lock_temporary().unwrap();
    locked.check_owner(&store).unwrap();
    assert_eq!(locked.creation_security(), &created.security);
    assert_eq!(locked.temporary_security(), &private);
    let private_probe = {
        let file = locked.staged.staged.created_file().unwrap();
        assert_eq!(file.as_raw_handle() as usize, created.handle);
        assert_eq!(windows_identity::identity(&file).unwrap(), created.identity);
        assert_eq!(SecurityDescriptor::read(&file).unwrap(), private);
        duplicate_read_control(&file).unwrap()
    };
    assert!(!target.exists());
    assert_separate_reader_denied(&created.path);
    CreationFixture {
        dir,
        target,
        store,
        n,
        locked,
        created,
        private,
        private_probe,
    }
}

fn newer_edit(store: &mut Store, n: &crate::Snapshot) -> crate::Snapshot {
    store
        .submit(Transaction {
            doc: n.doc,
            base: n.revision,
            edits: vec![Edit::input(n, 0..0, "B")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot
}

#[test]
fn creation_default_publishes_captured_descriptor_and_binds_n_without_reverting_newer_edit() {
    let CreationFixture {
        target,
        mut store,
        n,
        locked,
        created,
        private,
        private_probe,
        ..
    } = creation_fixture("success");
    let capture = locked.capture().clone();
    let (before, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    let ready = locked.prepare_new_creation_default().unwrap();
    assert_eq!(ready.security_policy(), NewFileSecurity::CreationDefault);
    assert_eq!(SecurityDescriptor::read(&private_probe).unwrap(), private);
    assert!(!target.exists());
    assert_separate_reader_denied(&created.path);
    let (prepared, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(prepared.state, before.state);
    assert_eq!(prepared.cursor, before.cursor);
    drop(private_probe);
    let job = ready.permit(&mut store).unwrap();
    assert_eq!(job.security_policy(), NewFileSecurity::CreationDefault);
    let current = newer_edit(&mut store, &n);
    let depth = store.history_depth(n.doc).unwrap();
    // Exercise the unmodified production DACL helper on its original File.
    let published = job.run().unwrap();
    assert_eq!(
        published.security_policy(),
        NewFileSecurity::CreationDefault
    );
    assert_eq!(published.capture().operation, capture.operation);
    assert_eq!(published.identity(), created.identity);
    assert!(!created.path.exists());
    assert_separate_reader_denied(&target);
    let committed = published.accept(&mut store).unwrap();
    assert_eq!(committed.security_policy, NewFileSecurity::CreationDefault);
    assert_eq!(committed.saved_snapshot.token, n.token);
    assert_eq!(committed.saved_snapshot.body.to_utf8(), b"NA\r\n");
    assert_eq!(committed.saved_snapshot.body.profile, n.body.profile);
    assert_eq!(committed.version.identity, created.identity);
    assert_eq!(committed.version.security, created.security);
    assert_eq!(committed.version.bytes.as_ref(), CAPTURED_UTF16);
    assert_eq!(fs::read(&target).unwrap(), CAPTURED_UTF16);
    let actual = ObservedTarget::inspect(&target).unwrap();
    assert_eq!(actual.version().unwrap(), &committed.version);
    assert_eq!(committed.path, actual.resolved_path());
    assert_eq!(committed.binding_epoch, 1);
    let disk = committed.binding.disk.as_ref().unwrap();
    assert_eq!(disk.version, committed.version);
    assert_eq!(disk.path, committed.path);
    assert_eq!(disk.parent, actual.parent_identity());
    assert_eq!(disk.epoch, committed.binding_epoch);
    assert!(disk.anchor.retains(created.identity).unwrap());
    assert_eq!(committed.binding.imported.token, n.token);
    assert_eq!(
        committed.binding.format,
        n.body.profile.file_format.unwrap()
    );
    assert_eq!(committed.binding.original.as_ref(), CAPTURED_UTF16);
    let (after, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(after.state.global.metadata.baseline.token, n.token);
    assert!(after.state.global.metadata.baseline.backed_by_disk);
    assert_eq!(
        after.state.global.metadata.attention.target,
        Some(created.identity.target_key())
    );
    assert_eq!(
        after.state.global.metadata.attention.binding_epoch,
        committed.binding_epoch
    );
    assert_eq!(after.state.global.document.token, current.token);
    assert_eq!(after.state.global.document.body.to_utf8(), b"BNA\r\n");
    assert!(after.state.global.metadata.save.active.is_none());
    assert_eq!(store.history_depth(n.doc).unwrap(), depth);
    assert!(store.dirty(n.doc).unwrap());
    assert_eq!(store.publication_recovery.len(), 0);
}

#[test]
fn creation_default_existing_target_is_rejected_without_mutating_version_entries_or_store() {
    let dir = generated_directory("existing");
    let target = dir.join("existing.txt");
    fs::write(&target, b"existing bytes\r\n").unwrap();
    let observed = ObservedTarget::inspect(&target).unwrap();
    let original = observed.version().unwrap().clone();
    let mut store = Store::new(HistoryBudget::default());
    let (n, binding) = LoadedBytes::decode(Arc::from(&b"memory\r\n"[..]), Some(Encoding::Utf8))
        .unwrap()
        .attach(&mut store)
        .unwrap();
    let locked = observed
        .prepare_bound(&binding, &mut store)
        .unwrap()
        .begin(&mut store)
        .unwrap()
        .write()
        .unwrap()
        .lock_temporary()
        .unwrap();
    let capture = locked.capture().clone();
    let temporary = locked.recovery_temp().to_owned();
    let entries = directory_entries(&dir);
    let depth = store.history_depth(n.doc).unwrap();
    let (before, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    let failure = match locked.prepare_new_creation_default() {
        Ok(_) => panic!("CreationDefault acquired an existing-target publication capability"),
        Err(failure) => failure,
    };
    assert!(matches!(
        &failure.issue,
        PublishIssue::ExistingTargetUnsupported
    ));
    assert_eq!(failure.security_policy(), NewFileSecurity::CreationDefault);
    assert_eq!(failure.capture().operation, capture.operation);
    assert_eq!(failure.read_retained_bytes().unwrap(), b"memory\r\n");
    assert_eq!(directory_entries(&dir), entries);
    assert_eq!(
        ObservedTarget::inspect(&target).unwrap().version().unwrap(),
        &original
    );
    assert_eq!(fs::read(&target).unwrap(), b"existing bytes\r\n");
    assert!(temporary.exists());
    let (rejected, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(rejected.state, before.state);
    assert_eq!(rejected.cursor, before.cursor);
    assert_eq!(store.history_depth(n.doc).unwrap(), depth);
    assert_eq!(
        store.captured_save_stage(&capture),
        Ok(SaveStage::WritingTemp)
    );
    assert!(binding.disk.is_none());
    assert_eq!(binding.imported.token, n.token);
    failure.record_failure(&mut store).unwrap();
    assert_eq!(failure.record_failure(&mut store), Err(Error::InvalidSave));
    let (recorded, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(
        recorded.state.global.metadata.baseline,
        before.state.global.metadata.baseline
    );
    assert_eq!(
        recorded.state.global.metadata.attention.target,
        before.state.global.metadata.attention.target
    );
    assert_eq!(
        recorded.state.global.metadata.attention.binding_epoch,
        before.state.global.metadata.attention.binding_epoch
    );
    assert_eq!(
        recorded.state.global.metadata.history,
        before.state.global.metadata.history
    );
    assert_eq!(store.snapshot(n.doc).unwrap().token, n.token);
    assert_eq!(
        ObservedTarget::inspect(&target).unwrap().version().unwrap(),
        &original
    );
    assert_eq!(directory_entries(&dir), entries);
}

#[test]
fn creation_default_dacl_access_denial_after_real_rename_retains_n_and_original_resources() {
    let CreationFixture {
        dir,
        target,
        mut store,
        n,
        locked,
        created,
        private,
        private_probe,
    } = creation_fixture("dacl-denied");
    let capture = locked.capture().clone();
    let expected_final_path = locked.staged.target_path();
    let ready = locked.prepare_new_creation_default().unwrap();
    assert_eq!(ready.security_policy(), NewFileSecurity::CreationDefault);
    assert_eq!(SecurityDescriptor::read(&private_probe).unwrap(), private);
    drop(private_probe);
    let mut job = ready.permit(&mut store).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let hook_calls = calls.clone();
    let expected = created.clone();
    let expected_private = private.clone();
    job.set_creation_dacl_handle(move |file| {
        // This hook is reached after real rename and final path/ID/parent checks.
        assert_eq!(file.as_raw_handle() as usize, expected.handle);
        assert_eq!(windows_identity::identity(file).unwrap(), expected.identity);
        assert_eq!(
            windows_identity::volume_path(file).unwrap(),
            expected_final_path
        );
        assert!(!expected.path.exists());
        assert_eq!(SecurityDescriptor::read(file)?, expected_private);
        let limited = duplicate_read_control(file)?;
        assert_eq!(SecurityDescriptor::read(&limited)?, expected_private);
        hook_calls.fetch_add(1, Ordering::SeqCst);
        // Real SetSecurityInfo must reject this handle lacking WRITE_DAC;
        // no synthetic Err or replacement of the publication result is used.
        Ok(limited)
    });
    let current = newer_edit(&mut store, &n);
    let depth = store.history_depth(n.doc).unwrap();
    let (before, subscription) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    let failure = match job.run() {
        Ok(_) => panic!("READ_CONTROL-only handle restored a DACL"),
        Err(failure) => failure,
    };
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(failure.security_policy(), NewFileSecurity::CreationDefault);
    assert!(matches!(&failure.issue, PublishIssue::Io(error) if error.raw_os_error() == Some(5)));
    assert_eq!(failure.capture().operation, capture.operation);
    assert_eq!(failure.capture().binding_epoch, capture.binding_epoch);
    assert_eq!(failure.capture().snapshot.token, n.token);
    assert!(Arc::ptr_eq(
        &failure.capture().snapshot.body,
        &capture.snapshot.body
    ));
    let actual_path = failure.current_path().unwrap();
    assert_eq!(actual_path.file_name(), target.file_name());
    assert!(target.exists());
    assert!(!created.path.exists());
    assert_eq!(failure.read_retained_bytes().unwrap(), CAPTURED_UTF16);
    let actual = failure.current_version().unwrap();
    assert_eq!(actual.identity, created.identity);
    assert_eq!(actual.bytes.as_ref(), CAPTURED_UTF16);
    assert_eq!(actual.security, private);
    assert_separate_reader_denied(&target);
    let (worker_returned, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(worker_returned.state, before.state);
    assert_eq!(worker_returned.cursor, before.cursor);
    failure.record_failure(&mut store).unwrap();
    let (after, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(
        after.state.global.metadata.baseline,
        before.state.global.metadata.baseline
    );
    assert_eq!(
        after.state.global.metadata.attention.target,
        before.state.global.metadata.attention.target
    );
    assert_eq!(
        after.state.global.metadata.attention.binding_epoch,
        before.state.global.metadata.attention.binding_epoch
    );
    assert_eq!(
        after.state.global.metadata.history,
        before.state.global.metadata.history
    );
    assert_eq!(after.state.global.document.token, current.token);
    assert_eq!(after.state.global.document.body.to_utf8(), b"BNA\r\n");
    assert_eq!(store.history_depth(n.doc).unwrap(), depth);
    assert!(store.dirty(n.doc).unwrap());
    assert!(after.state.global.metadata.save.active.is_none());
    let terminal = after
        .state
        .global
        .metadata
        .save
        .last_terminal
        .as_ref()
        .unwrap();
    assert_eq!(terminal.operation, capture.operation);
    assert_eq!(terminal.result, crate::SaveResult::OutcomeUnknown);
    let recovery = after
        .state
        .global
        .metadata
        .attention
        .recoveries
        .last()
        .unwrap();
    assert!(recovery.detail.recovery_paths.contains(&actual_path));
    assert_eq!(recovery.detail.os_error, Some(5));
    assert_eq!(
        store
            .recovery_snapshot(n.doc, capture.operation)
            .unwrap()
            .token,
        n.token
    );
    assert!(matches!(
        store.poll_observer(&subscription).unwrap(),
        crate::ObserverPoll::Batch(_)
    ));
    assert_eq!(failure.record_failure(&mut store), Err(Error::InvalidSave));
    assert!(matches!(
        store.poll_observer(&subscription).unwrap(),
        crate::ObserverPoll::Pending
    ));
    drop(failure);
    assert_separate_reader_denied(&target);
    let notices = store.drain_dropped_publications();
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].security_policy(),
        NewFileSecurity::CreationDefault
    );
    assert!(notices[0].terminal_recorded);
    assert!(notices[0].owner_error.is_none());
    assert_eq!(notices[0].capture().operation, capture.operation);
    assert_eq!(notices[0].capture().snapshot.token, n.token);
    assert_eq!(notices[0].read_retained_bytes().unwrap(), CAPTURED_UTF16);
    assert!(matches!(
        store.poll_observer(&subscription).unwrap(),
        crate::ObserverPoll::Pending
    ));
    // A different existing-file key in this parent is still held by Unknown.
    let other_path = dir.join("other-existing.txt");
    fs::write(&other_path, b"other").unwrap();
    let (_, other_binding) = LoadedBytes::decode(Arc::from(&b"other"[..]), Some(Encoding::Utf8))
        .unwrap()
        .attach(&mut store)
        .unwrap();
    let other = ObservedTarget::inspect(&other_path).unwrap();
    assert_ne!(other.reservation_key(), capture.target);
    assert!(matches!(
        other.prepare_bound(&other_binding, &mut store),
        Err(BoundFailure {
            issue: BoundIssue::File(FileError::Document(Error::UnresolvedConflict)),
            ..
        })
    ));
    // Intentionally retain the unresolved recovery; no owner acknowledgement,
    // ACL repair, path-based mutation, deletion or publication retry occurs.
}

#[test]
fn cancelled_creation_default_ready_drops_without_publication_or_relaxing_private_temporary() {
    let CreationFixture {
        target,
        mut store,
        n,
        locked,
        created,
        private,
        private_probe,
        ..
    } = creation_fixture("cancel");
    let capture = locked.capture().clone();
    let ready = locked.prepare_new_creation_default().unwrap();
    assert_eq!(ready.security_policy(), NewFileSecurity::CreationDefault);
    let current = newer_edit(&mut store, &n);
    let depth = store.history_depth(n.doc).unwrap();
    let (before, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    store.cancel_save(&capture).unwrap();
    assert_eq!(SecurityDescriptor::read(&private_probe).unwrap(), private);
    assert_separate_reader_denied(&created.path);
    assert!(!target.exists());
    drop(ready);
    assert_eq!(SecurityDescriptor::read(&private_probe).unwrap(), private);
    drop(private_probe);
    // Inspect only this generated temp after all retained handles have closed.
    let mut file = File::open(&created.path).unwrap();
    let actual = windows_identity::read_version_handle(&mut file).unwrap();
    assert_eq!(actual.identity, created.identity);
    assert_eq!(actual.security, private);
    assert_eq!(actual.bytes.as_ref(), CAPTURED_UTF16);
    assert!(!target.exists());
    assert_eq!(store.publication_recovery.len(), 0);
    let (after, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    assert_eq!(
        after.state.global.metadata.baseline,
        before.state.global.metadata.baseline
    );
    assert_eq!(
        after.state.global.metadata.attention.target,
        before.state.global.metadata.attention.target
    );
    assert_eq!(
        after.state.global.metadata.attention.binding_epoch,
        before.state.global.metadata.attention.binding_epoch
    );
    assert_eq!(after.state.global.document.token, current.token);
    assert_eq!(store.history_depth(n.doc).unwrap(), depth);
    assert!(store.dirty(n.doc).unwrap());
    assert!(after.state.global.metadata.save.active.is_none());
    let terminal = after
        .state
        .global
        .metadata
        .save
        .last_terminal
        .as_ref()
        .unwrap();
    assert_eq!(terminal.operation, capture.operation);
    assert_eq!(terminal.result, crate::SaveResult::Cancelled);
}
