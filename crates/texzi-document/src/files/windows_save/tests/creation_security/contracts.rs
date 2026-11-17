//! Source-only completion of the new-file policy contract; generated files only.
use super::*;
use crate::files::windows_save::recovery::PublicationRecovery;

struct FrozenOwnerState {
    observed: crate::ObserverSnapshot,
    depth: (usize, usize),
}

fn freeze_owner_state(store: &mut Store, n: &crate::Snapshot) -> FrozenOwnerState {
    let depth = store.history_depth(n.doc).unwrap();
    let (observed, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    FrozenOwnerState { observed, depth }
}

fn assert_same_capture(actual: &SaveCapture, expected: &SaveCapture) {
    assert_eq!(actual.operation, expected.operation);
    assert_eq!(actual.target, expected.target);
    assert_eq!(actual.binding_epoch, expected.binding_epoch);
    assert_eq!(actual.guard, expected.guard);
    assert_eq!(actual.snapshot.doc, expected.snapshot.doc);
    assert_eq!(actual.snapshot.revision, expected.snapshot.revision);
    assert_eq!(actual.snapshot.token, expected.snapshot.token);
    assert!(Arc::ptr_eq(&actual.snapshot.body, &expected.snapshot.body));
}

fn assert_owner_and_group_unchanged(actual: &SecurityDescriptor, expected: &SecurityDescriptor) {
    let (actual_owner, actual_group) = actual.owner_group_for_test().unwrap();
    let (expected_owner, expected_group) = expected.owner_group_for_test().unwrap();
    // Keep owner and group separate without printing either current-account SID.
    assert!(actual_owner == expected_owner, "file owner changed");
    assert!(actual_group == expected_group, "file group changed");
}

fn assert_unknown_retains_state(
    store: &mut Store,
    frozen: &FrozenOwnerState,
    n: &crate::Snapshot,
    current: &crate::Snapshot,
    capture: &SaveCapture,
    actual_path: &Path,
) -> crate::ObserverSnapshot {
    let (after, _) = store
        .observe_and_subscribe(n.doc, ObserverScope::new(vec![]), 8)
        .unwrap();
    let before = &frozen.observed.state.global;
    let actual = &after.state.global;
    assert_eq!(actual.metadata.baseline, before.metadata.baseline);
    assert_eq!(
        actual.metadata.attention.target,
        before.metadata.attention.target
    );
    assert_eq!(
        actual.metadata.attention.binding_epoch,
        before.metadata.attention.binding_epoch
    );
    assert_eq!(actual.metadata.history, before.metadata.history);
    assert_eq!(actual.document.token, current.token);
    assert!(Arc::ptr_eq(&actual.document.body, &current.body));
    assert_eq!(store.history_depth(n.doc).unwrap(), frozen.depth);
    assert!(store.dirty(n.doc).unwrap());
    assert!(actual.metadata.save.active.is_none());
    let terminal = actual.metadata.save.last_terminal.as_ref().unwrap();
    assert_eq!(terminal.operation, capture.operation);
    assert_eq!(terminal.result, crate::SaveResult::OutcomeUnknown);
    assert!(matches!(
        &actual.metadata.attention.save_outcome_unknown,
        crate::Unimplemented::NeedsVerification { operation, target }
            if *operation == capture.operation && target == &capture.target
    ));
    let record = actual.metadata.attention.recoveries.last().unwrap();
    assert_eq!(record.operation, capture.operation);
    assert_eq!(record.token, n.token);
    assert_eq!(record.binding_epoch, capture.binding_epoch);
    assert_eq!(record.detail.outcome, SaveFailureOutcome::OutcomeUnknown);
    assert!(
        record
            .detail
            .recovery_paths
            .iter()
            .any(|path| path == actual_path)
    );
    let retained = store.recovery_snapshot(n.doc, capture.operation).unwrap();
    assert_eq!(retained.token, n.token);
    assert!(Arc::ptr_eq(&retained.body, &n.body));
    after
}

fn assert_parent_scope_held(store: &mut Store, dir: &Path, capture: &SaveCapture, label: &str) {
    let path = dir.join(format!("held-{label}.txt"));
    fs::write(&path, b"other").unwrap();
    let (_, binding) = LoadedBytes::decode(Arc::from(&b"other"[..]), Some(Encoding::Utf8))
        .unwrap()
        .attach(store)
        .unwrap();
    let other = ObservedTarget::inspect(&path).unwrap();
    assert_ne!(other.reservation_key(), capture.target);
    assert!(matches!(
        other.prepare_bound(&binding, store),
        Err(BoundFailure {
            issue: BoundIssue::File(FileError::Document(Error::UnresolvedConflict)),
            ..
        })
    ));
}

fn assert_recovery_contract(
    receipt: &PublicationRecovery,
    policy: NewFileSecurity,
    expected_security: &SecurityDescriptor,
    actual_security: &SecurityDescriptor,
    capture: &SaveCapture,
    created: &CreationReceipt,
    actual_path: &Path,
) {
    assert_eq!(receipt.security_policy(), policy);
    assert!(receipt.expected_security() == expected_security);
    assert_same_capture(receipt.capture(), capture);
    assert!(receipt.terminal_recorded);
    assert!(receipt.owner_error.is_none());
    assert_eq!(receipt.current_path().unwrap(), actual_path);
    assert_eq!(receipt.read_retained_bytes().unwrap(), CAPTURED_UTF16);
    let version = receipt.current_version().unwrap();
    assert_eq!(version.identity, created.identity);
    assert_eq!(version.bytes.as_ref(), CAPTURED_UTF16);
    assert!(&version.security == actual_security);
}

#[test]
fn owner_only_same_creation_fixture_keeps_private_descriptor_and_binds_captured_n() {
    let CreationFixture {
        target,
        mut store,
        n,
        locked,
        created,
        private,
        private_probe,
        ..
    } = creation_fixture("owner-only-control");
    let capture = locked.capture().clone();
    // This control cannot silently pass where creation was already private.
    assert!(created.security != private);
    assert_owner_and_group_unchanged(&private, &created.security);
    let ready = locked.prepare_new_owner_only().unwrap();
    assert_eq!(ready.security_policy(), NewFileSecurity::OwnerOnly);
    assert!(SecurityDescriptor::read(&private_probe).unwrap() == private);
    drop(private_probe);
    let job = ready.permit(&mut store).unwrap();
    assert_eq!(job.security_policy(), NewFileSecurity::OwnerOnly);
    assert_same_capture(job.capture(), &capture);
    let current = newer_edit(&mut store, &n);
    let frozen = freeze_owner_state(&mut store, &n);
    let published = job.run().unwrap();
    assert_eq!(published.security_policy(), NewFileSecurity::OwnerOnly);
    assert_same_capture(published.capture(), &capture);
    assert_eq!(published.identity(), created.identity);
    assert!(!created.path.exists());
    assert_separate_reader_denied(&target);
    let committed = published.accept(&mut store).unwrap();
    assert_eq!(committed.security_policy, NewFileSecurity::OwnerOnly);
    assert_eq!(committed.saved_snapshot.token, n.token);
    assert!(Arc::ptr_eq(&committed.saved_snapshot.body, &n.body));
    assert_eq!(committed.version.identity, created.identity);
    assert_eq!(committed.version.bytes.as_ref(), CAPTURED_UTF16);
    assert!(committed.version.security == private);
    assert!(committed.version.security != created.security);
    assert_owner_and_group_unchanged(&committed.version.security, &created.security);
    let actual = ObservedTarget::inspect(&target).unwrap();
    assert!(actual.version().unwrap() == &committed.version);
    assert_eq!(committed.path, actual.resolved_path());
    let disk = committed.binding.disk.as_ref().unwrap();
    assert!(disk.version == committed.version);
    assert_eq!(disk.path, committed.path);
    assert_eq!(disk.parent, actual.parent_identity());
    assert_eq!(disk.epoch, committed.binding_epoch);
    assert_eq!(committed.binding_epoch, capture.binding_epoch + 1);
    assert!(disk.anchor.retains(created.identity).unwrap());
    assert_eq!(committed.binding.imported.token, n.token);
    assert_eq!(committed.binding.original.as_ref(), CAPTURED_UTF16);
    assert_eq!(
        committed.binding.format,
        n.body.profile.file_format.unwrap()
    );
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
    assert!(Arc::ptr_eq(
        &after.state.global.document.body,
        &current.body
    ));
    assert_eq!(
        after.state.global.metadata.history,
        frozen.observed.state.global.metadata.history
    );
    assert_eq!(store.history_depth(n.doc).unwrap(), frozen.depth);
    assert!(store.dirty(n.doc).unwrap());
    assert!(after.state.global.metadata.save.active.is_none());
    assert_eq!(store.publication_recovery.len(), 0);
}

#[test]
fn creation_default_rename_race_preserves_external_target_and_private_original_temp() {
    use std::io::Write;

    let CreationFixture {
        dir,
        target,
        mut store,
        n,
        locked,
        created,
        private,
        private_probe,
    } = creation_fixture("creation-race");
    let capture = locked.capture().clone();
    let expected_security = created.security.clone();
    let ready = locked.prepare_new_creation_default().unwrap();
    drop(private_probe);
    let mut job = ready.permit(&mut store).unwrap();
    let foreign_version = Arc::new(Mutex::new(None::<FileVersion>));
    let foreign_receipt = foreign_version.clone();
    job.set_before_rename(move |path| {
        let mut foreign = OpenOptions::new().write(true).create_new(true).open(path)?;
        foreign.write_all(b"external complete version\r\n")?;
        foreign.sync_all()?;
        drop(foreign);
        *foreign_receipt.lock().unwrap() = Some(
            ObservedTarget::inspect(path)
                .unwrap()
                .version()
                .unwrap()
                .clone(),
        );
        Ok(())
    });
    let dacl_calls = Arc::new(AtomicUsize::new(0));
    let hook_calls = dacl_calls.clone();
    job.set_creation_dacl_handle(move |file| {
        hook_calls.fetch_add(1, Ordering::SeqCst);
        file.try_clone()
    });
    let current = newer_edit(&mut store, &n);
    let frozen = freeze_owner_state(&mut store, &n);
    let failure = match job.run() {
        Ok(_) => panic!("CreationDefault overwrote the raced target"),
        Err(failure) => failure,
    };
    assert!(matches!(
        &failure.issue,
        PublishIssue::Io(error) if matches!(error.raw_os_error(), Some(80 | 183))
    ));
    assert_eq!(dacl_calls.load(Ordering::SeqCst), 0);
    assert_eq!(failure.security_policy(), NewFileSecurity::CreationDefault);
    assert_same_capture(failure.capture(), &capture);
    assert!(
        ObservedTarget::inspect(&target).unwrap().version().unwrap()
            == foreign_version.lock().unwrap().as_ref().unwrap()
    );
    assert_eq!(fs::read(&target).unwrap(), b"external complete version\r\n");
    let actual_path = failure.current_path().unwrap();
    assert_eq!(actual_path.file_name(), created.path.file_name());
    assert!(created.path.exists());
    let actual = failure.current_version().unwrap();
    assert_eq!(actual.identity, created.identity);
    assert_eq!(actual.bytes.as_ref(), CAPTURED_UTF16);
    assert!(actual.security == private);
    assert!(actual.security != expected_security);
    assert_separate_reader_denied(&created.path);
    failure.record_failure(&mut store).unwrap();
    assert_unknown_retains_state(&mut store, &frozen, &n, &current, &capture, &actual_path);
    assert_parent_scope_held(&mut store, &dir, &capture, "race");
    assert_eq!(failure.record_failure(&mut store), Err(Error::InvalidSave));
    drop(failure);
    let notices = store.drain_dropped_publications();
    assert_eq!(notices.len(), 1);
    assert_recovery_contract(
        &notices[0],
        NewFileSecurity::CreationDefault,
        &expected_security,
        &private,
        &capture,
        &created,
        &actual_path,
    );
    assert!(
        ObservedTarget::inspect(&target).unwrap().version().unwrap()
            == foreign_version.lock().unwrap().as_ref().unwrap()
    );
}

#[test]
fn creation_default_verification_failure_after_restore_keeps_creation_security_and_unknown_n() {
    let CreationFixture {
        dir,
        target,
        mut store,
        n,
        locked,
        created,
        private,
        private_probe,
    } = creation_fixture("creation-post-restore-verification");
    let capture = locked.capture().clone();
    let expected_final_path = locked.staged.target_path();
    let expected_security = created.security.clone();
    assert!(expected_security != private);
    assert_owner_and_group_unchanged(&private, &created.security);
    let ready = locked.prepare_new_creation_default().unwrap();
    drop(private_probe);
    let mut job = ready.permit(&mut store).unwrap();
    let restored_calls = Arc::new(AtomicUsize::new(0));
    let hook_calls = restored_calls.clone();
    let expected = created.clone();
    job.set_post_rename_flush_handle(move |file| {
        // The unmodified production DACL helper already returned success here.
        assert_eq!(file.as_raw_handle() as usize, expected.handle);
        assert_eq!(windows_identity::identity(file).unwrap(), expected.identity);
        assert!(SecurityDescriptor::read(file)? == expected.security);
        hook_calls.fetch_add(1, Ordering::SeqCst);
        file.try_clone()
    });
    // Fault the real final format predicate after restore/flush/read, not the IO result.
    job.set_verification_format(Format {
        encoding: Encoding::Utf8,
        bom: false,
    });
    let current = newer_edit(&mut store, &n);
    let frozen = freeze_owner_state(&mut store, &n);
    let failure = match job.run() {
        Ok(_) => panic!("post-restore format mismatch was accepted"),
        Err(failure) => failure,
    };
    assert_eq!(restored_calls.load(Ordering::SeqCst), 1);
    assert!(matches!(&failure.issue, PublishIssue::VerificationMismatch));
    assert_eq!(failure.security_policy(), NewFileSecurity::CreationDefault);
    assert_same_capture(failure.capture(), &capture);
    let actual_path = failure.current_path().unwrap();
    assert_eq!(actual_path, expected_final_path);
    assert_eq!(actual_path.file_name(), target.file_name());
    assert!(target.exists());
    assert!(!created.path.exists());
    let actual = failure.current_version().unwrap();
    assert_eq!(actual.identity, created.identity);
    assert_eq!(actual.bytes.as_ref(), CAPTURED_UTF16);
    assert!(actual.security == expected_security);
    assert!(actual.security != private);
    assert_owner_and_group_unchanged(&actual.security, &created.security);
    assert_owner_and_group_unchanged(&actual.security, &private);
    assert_eq!(failure.read_retained_bytes().unwrap(), CAPTURED_UTF16);
    assert_separate_reader_denied(&target);
    failure.record_failure(&mut store).unwrap();
    assert_unknown_retains_state(&mut store, &frozen, &n, &current, &capture, &actual_path);
    assert_parent_scope_held(&mut store, &dir, &capture, "post-restore");
    // An error does not roll ACLs back: the retained actual descriptor stays restored.
    assert!(failure.current_version().unwrap().security == expected_security);
    drop(failure);
    let notices = store.drain_dropped_publications();
    assert_eq!(notices.len(), 1);
    assert_recovery_contract(
        &notices[0],
        NewFileSecurity::CreationDefault,
        &expected_security,
        &expected_security,
        &capture,
        &created,
        &actual_path,
    );
}

fn dropped_security_policy_contract(after_rename: bool) {
    for policy in [NewFileSecurity::OwnerOnly, NewFileSecurity::CreationDefault] {
        let label = match (policy, after_rename) {
            (NewFileSecurity::OwnerOnly, false) => "owner-permit-drop",
            (NewFileSecurity::CreationDefault, false) => "creation-permit-drop",
            (NewFileSecurity::OwnerOnly, true) => "owner-result-drop",
            (NewFileSecurity::CreationDefault, true) => "creation-result-drop",
        };
        let CreationFixture {
            dir,
            target,
            mut store,
            n,
            locked,
            created,
            private,
            private_probe,
        } = creation_fixture(label);
        let capture = locked.capture().clone();
        let expected_final_path = locked.staged.target_path();
        // Freeze expected before permit. Actual is observed independently below.
        let expected_security = match policy {
            NewFileSecurity::OwnerOnly => private.clone(),
            NewFileSecurity::CreationDefault => created.security.clone(),
        };
        let ready = match policy {
            NewFileSecurity::OwnerOnly => locked.prepare_new_owner_only().unwrap(),
            NewFileSecurity::CreationDefault => locked.prepare_new_creation_default().unwrap(),
        };
        assert_eq!(ready.security_policy(), policy);
        drop(private_probe);
        let job = ready.permit(&mut store).unwrap();
        assert_eq!(job.security_policy(), policy);
        assert_same_capture(job.capture(), &capture);
        let current = newer_edit(&mut store, &n);
        let frozen = freeze_owner_state(&mut store, &n);
        if after_rename {
            let published = job.run().unwrap();
            assert_eq!(published.security_policy(), policy);
            assert_same_capture(published.capture(), &capture);
            assert_eq!(published.identity(), created.identity);
            drop(published);
        } else {
            drop(job);
        }
        assert_eq!(target.exists(), after_rename);
        assert_eq!(created.path.exists(), !after_rename);
        assert_eq!(
            store.acknowledge_publication_recovery(capture.operation),
            Err(Error::InvalidSave)
        );
        let unrecorded = store.retained_publication_recoveries();
        assert_eq!(unrecorded.len(), 1);
        assert!(!unrecorded[0].terminal_recorded);
        assert!(unrecorded[0].owner_error.is_none());
        assert_eq!(unrecorded[0].security_policy(), policy);
        assert!(unrecorded[0].expected_security() == &expected_security);
        assert_same_capture(unrecorded[0].capture(), &capture);
        let actual_path = unrecorded[0].current_path().unwrap();
        let actual_security = if after_rename {
            &expected_security
        } else {
            &private
        };
        let unrecorded_version = unrecorded[0].current_version().unwrap();
        assert_eq!(unrecorded_version.identity, created.identity);
        assert_eq!(unrecorded_version.bytes.as_ref(), CAPTURED_UTF16);
        assert!(&unrecorded_version.security == actual_security);
        assert_eq!(unrecorded[0].read_retained_bytes().unwrap(), CAPTURED_UTF16);
        let inspected = freeze_owner_state(&mut store, &n);
        assert_eq!(inspected.observed.state, frozen.observed.state);
        assert_eq!(inspected.observed.cursor, frozen.observed.cursor);
        drop(unrecorded); // Inspection cannot record a terminal result or lose resources.
        let notices = store.drain_dropped_publications();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].current_path().unwrap(), actual_path);
        assert_eq!(actual_path.file_name() == target.file_name(), after_rename);
        if after_rename {
            assert_eq!(actual_path, expected_final_path);
        }
        assert_recovery_contract(
            &notices[0],
            policy,
            &expected_security,
            actual_security,
            &capture,
            &created,
            &actual_path,
        );
        if policy == NewFileSecurity::CreationDefault && !after_rename {
            // The intended final descriptor is not evidence that restore happened.
            assert!(notices[0].current_version().unwrap().security != expected_security);
        }
        assert_separate_reader_denied(&actual_path);
        assert_unknown_retains_state(&mut store, &frozen, &n, &current, &capture, &actual_path);
        assert_parent_scope_held(&mut store, &dir, &capture, "before-ack");
        let reconciled = freeze_owner_state(&mut store, &n);
        drop(notices); // Losing the drained receipt must not lose the fixed policy.
        let retained = store.retained_publication_recoveries();
        assert_eq!(retained.len(), 1);
        assert_recovery_contract(
            &retained[0],
            policy,
            &expected_security,
            actual_security,
            &capture,
            &created,
            &actual_path,
        );
        drop(retained);
        let again = store.drain_dropped_publications();
        assert_eq!(again.len(), 1);
        assert_recovery_contract(
            &again[0],
            policy,
            &expected_security,
            actual_security,
            &capture,
            &created,
            &actual_path,
        );
        let before_ack = freeze_owner_state(&mut store, &n);
        assert_eq!(before_ack.observed.state, reconciled.observed.state);
        assert_eq!(before_ack.observed.cursor, reconciled.observed.cursor);
        let version_before_ack = again[0].current_version().unwrap();
        store
            .acknowledge_publication_recovery(capture.operation)
            .unwrap();
        assert!(store.retained_publication_recoveries().is_empty());
        assert_eq!(store.publication_recovery.len(), 0);
        let after_ack =
            assert_unknown_retains_state(&mut store, &frozen, &n, &current, &capture, &actual_path);
        assert_eq!(after_ack.state, before_ack.observed.state);
        assert_eq!(after_ack.cursor, before_ack.observed.cursor);
        assert!(again[0].current_version().unwrap() == version_before_ack);
        assert_recovery_contract(
            &again[0],
            policy,
            &expected_security,
            actual_security,
            &capture,
            &created,
            &actual_path,
        );
        assert_parent_scope_held(&mut store, &dir, &capture, "after-ack");
        assert_separate_reader_denied(&actual_path);
        drop(again);
        // Registry acknowledgement and final handle release do not mutate the ACL.
        let mut file = File::open(&actual_path).unwrap();
        let actual = windows_identity::read_version_handle(&mut file).unwrap();
        assert_eq!(actual.identity, created.identity);
        assert_eq!(actual.bytes.as_ref(), CAPTURED_UTF16);
        assert!(&actual.security == actual_security);
    }
}

#[test]
fn dropped_permitted_worker_retains_both_security_policies_and_private_actual_version() {
    dropped_security_policy_contract(false);
}

#[test]
fn dropped_successful_result_retains_both_security_policies_and_final_actual_version() {
    dropped_security_policy_contract(true);
}

#[test]
fn creation_default_test_only_payload_tamper_is_rejected_before_any_dacl_restore() {
    use std::io::{Seek, SeekFrom, Write};

    const TAMPERED_UTF16: [u8; 10] = [0xff, 0xfe, 88, 0, 65, 0, 13, 0, 10, 0];
    let CreationFixture {
        dir,
        target,
        mut store,
        n,
        locked,
        created,
        private,
        private_probe,
    } = creation_fixture("creation-pre-restore-payload-fault");
    let capture = locked.capture().clone();
    let expected_final_path = locked.staged.target_path();
    let expected_security = created.security.clone();
    let tamper_file = {
        let file = locked.staged.staged.created_file().unwrap();
        assert_eq!(file.as_raw_handle() as usize, created.handle);
        file.try_clone().unwrap()
    };
    let ready = locked.prepare_new_creation_default().unwrap();
    drop(private_probe);
    let mut job = ready.permit(&mut store).unwrap();
    let expected_private = private.clone();
    job.set_before_rename(move |_| {
        // Deliberate in-process test fault after run's last pre-rename recheck.
        // FILE_SHARE_NONE does not expose this duplicate to a public path writer.
        let mut file = &tamper_file;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&TAMPERED_UTF16)?;
        file.set_len(TAMPERED_UTF16.len() as u64)?;
        file.sync_all()?;
        assert!(SecurityDescriptor::read(&tamper_file)? == expected_private);
        Ok(())
    });
    let dacl_calls = Arc::new(AtomicUsize::new(0));
    let hook_calls = dacl_calls.clone();
    job.set_creation_dacl_handle(move |file| {
        hook_calls.fetch_add(1, Ordering::SeqCst);
        file.try_clone()
    });
    let current = newer_edit(&mut store, &n);
    let frozen = freeze_owner_state(&mut store, &n);
    let failure = match job.run() {
        Ok(_) => panic!("tampered original payload reached CreationDefault publication"),
        Err(failure) => failure,
    };
    assert!(matches!(&failure.issue, PublishIssue::VerificationMismatch));
    assert_eq!(dacl_calls.load(Ordering::SeqCst), 0);
    assert_eq!(failure.security_policy(), NewFileSecurity::CreationDefault);
    assert_same_capture(failure.capture(), &capture);
    assert_eq!(failure.capture().snapshot.body.to_utf8(), b"NA\r\n");
    let actual_path = failure.current_path().unwrap();
    assert_eq!(actual_path, expected_final_path);
    assert_eq!(actual_path.file_name(), target.file_name());
    assert!(target.exists());
    assert!(!created.path.exists());
    let actual = failure.current_version().unwrap();
    assert_eq!(actual.identity, created.identity);
    assert_eq!(actual.bytes.as_ref(), TAMPERED_UTF16);
    assert!(actual.security == private);
    assert!(actual.security != expected_security);
    assert_eq!(failure.read_retained_bytes().unwrap(), TAMPERED_UTF16);
    assert_separate_reader_denied(&target);
    failure.record_failure(&mut store).unwrap();
    assert_unknown_retains_state(&mut store, &frozen, &n, &current, &capture, &actual_path);
    assert_parent_scope_held(&mut store, &dir, &capture, "payload-fault");
    assert!(failure.current_version().unwrap().security == private);
}
