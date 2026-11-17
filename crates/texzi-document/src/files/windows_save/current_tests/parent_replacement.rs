//! Generated parent-replacement fixture; execution remains a Windows-only step.
use super::*;

fn generated_tree_bytes(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    fn visit(root: &Path, dir: &Path, entries: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            let relative = path.strip_prefix(root).unwrap().to_owned();
            if kind.is_dir() {
                entries.push((relative, None));
                visit(root, &path, entries);
            } else {
                assert!(
                    kind.is_file(),
                    "generated fixture contains a non-regular file"
                );
                entries.push((relative, Some(fs::read(path).unwrap())));
            }
        }
    }
    let mut entries = Vec::new();
    visit(root, root, &mut entries);
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

#[test]
fn replaced_unicode_parent_of_historical_source_is_not_an_independent_save_as() {
    const PRECONDITION: &str =
        "T07_CURRENT_02_PARENT_FIXTURE_PRECONDITION_NOT_MET (not product PASS): ";
    const ORIGINAL: &[u8] = b"A\r\n";
    const REPLACEMENT: &[u8] = b"external uppercase parent object\n";
    let (original, mut store, old_doc, old_binding) = fixture();
    let root = original.parent().unwrap().to_owned();
    let lower_parent = root.join("éParent");
    let upper_parent = root.join("ÉParent");
    let retained_parent = root.join("retained-original-parent");
    let lower = lower_parent.join("source.txt");
    let upper = upper_parent.join("source.txt");
    let retained = retained_parent.join("source.txt");
    store.close_document(old_doc.doc).unwrap();
    drop(old_binding);
    fs::create_dir(&lower_parent)
        .unwrap_or_else(|e| panic!("{PRECONDITION}cannot create lowercase parent: {e}"));
    fs::rename(&original, &lower)
        .unwrap_or_else(|e| panic!("{PRECONDITION}cannot move generated source: {e}"));

    let (n, binding) = {
        let lower_observed = ObservedTarget::inspect(&lower)
            .unwrap_or_else(|e| panic!("{PRECONDITION}cannot inspect lowercase source: {e:?}"));
        {
            let upper_before = ObservedTarget::inspect(&upper).unwrap_or_else(|e| {
                panic!("{PRECONDITION}directory spellings do not resolve equivalently: {e:?}")
            });
            assert!(
                upper_before.version().is_some(),
                "{PRECONDITION}uppercase parent spelling does not find the source"
            );
            assert_eq!(
                upper_before.parent_identity(),
                lower_observed.parent_identity(),
                "{PRECONDITION}éParent/ÉParent resolve to different directories"
            );
            assert_eq!(
                upper_before.version().unwrap().identity,
                lower_observed.version().unwrap().identity,
                "{PRECONDITION}equivalent parent spellings resolve to different files"
            );
        } // Release the case-equivalence observation's no-delete-share parent.
        match lower_observed
            .open_existing(None)
            .unwrap()
            .attach(&mut store)
            .unwrap()
        {
            OpenOutcome::Imported { snapshot, binding } => (snapshot, binding),
            _ => panic!("new lowercase-parent source import expected"),
        }
    }; // The consumed captured-open and every short-lived parent handle are gone.

    let (historical, original_parent_id, original_version, original_epoch) = {
        let disk = binding.disk.as_ref().unwrap();
        (
            disk.path.clone(),
            disk.parent,
            disk.version.clone(),
            disk.epoch,
        )
    };
    assert_eq!(
        historical.file_name(),
        lower.file_name(),
        "{PRECONDITION}historical canonical leaf is not source.txt"
    );
    assert_eq!(
        historical.parent().unwrap().file_name(),
        lower_parent.file_name(),
        "{PRECONDITION}initial canonical parent does not retain lowercase spelling"
    );
    assert_eq!(original_version.bytes.as_ref(), ORIGINAL);
    // FileBinding and Store retain only share-delete source-file attribute
    // anchors here. No ObservedTarget or BoundPrepared survives this point.
    fs::rename(&lower_parent, &retained_parent).unwrap_or_else(|e| {
        panic!("{PRECONDITION}cannot retain the original parent by rename: {e}")
    });
    fs::create_dir(&upper_parent)
        .unwrap_or_else(|e| panic!("{PRECONDITION}cannot create replacement parent: {e}"));
    fs::write(&upper, REPLACEMENT)
        .unwrap_or_else(|e| panic!("{PRECONDITION}cannot create replacement source: {e}"));

    let new = ObservedTarget::inspect(&upper)
        .unwrap_or_else(|e| panic!("{PRECONDITION}cannot inspect replacement source: {e:?}"));
    let new_version = new.version().unwrap_or_else(|| {
        panic!("{PRECONDITION}replacement source is absent after fixture creation")
    });
    let new_parent_id = new.parent_identity();
    assert_ne!(
        new_parent_id, original_parent_id,
        "{PRECONDITION}replacement parent has the original directory ID"
    );
    assert_ne!(
        new_version.identity, original_version.identity,
        "{PRECONDITION}replacement file has the original file ID"
    );
    assert_eq!(new_version.bytes.as_ref(), REPLACEMENT);
    let canonical = new
        .canonical_observed_path()
        .unwrap_or_else(|e| panic!("{PRECONDITION}cannot resolve replacement canonical: {e:?}"))
        .unwrap_or_else(|| panic!("{PRECONDITION}replacement has no canonical path"));
    assert_eq!(
        canonical.file_name(),
        upper.file_name(),
        "{PRECONDITION}replacement canonical leaf is not source.txt"
    );
    assert_eq!(
        canonical.parent().unwrap().file_name(),
        upper_parent.file_name(),
        "{PRECONDITION}replacement canonical parent does not retain uppercase spelling"
    );
    assert_ne!(
        canonical, historical,
        "{PRECONDITION}historical and replacement canonical paths are identical"
    );
    assert_eq!(
        new.resolved_path(),
        canonical,
        "{PRECONDITION}resolved replacement path differs from its canonical path"
    );
    assert_ne!(
        canonical.parent(),
        historical.parent(),
        "{PRECONDITION}historical and replacement canonical parents are identical"
    );
    assert!(
        !canonical
            .to_str()
            .unwrap()
            .eq_ignore_ascii_case(historical.to_str().unwrap()),
        "{PRECONDITION}ASCII folding already matches the historical full path"
    );
    {
        // Resolve the actual stored full path, including its historical parent
        // spelling, rather than inferring equivalence from Unicode case rules.
        let historical_now = ObservedTarget::inspect(&historical).unwrap_or_else(|e| {
            panic!("{PRECONDITION}historical full path cannot resolve replacement: {e:?}")
        });
        assert!(
            historical_now.version().is_some(),
            "{PRECONDITION}historical full path no longer finds a source"
        );
        assert_eq!(
            historical_now.parent_identity(),
            new_parent_id,
            "{PRECONDITION}historical full path resolves to a different parent"
        );
        assert_eq!(
            historical_now.version(),
            new.version(),
            "{PRECONDITION}historical full path resolves to a different file version"
        );
        assert_eq!(
            historical_now
                .canonical_observed_path()
                .unwrap_or_else(|e| {
                    panic!("{PRECONDITION}cannot resolve historical canonical: {e:?}")
                }),
            Some(canonical.clone()),
            "{PRECONDITION}historical full path canonical resolves elsewhere"
        );
        let retained_observed = ObservedTarget::inspect(&retained).unwrap_or_else(|e| {
            panic!("{PRECONDITION}cannot inspect retained original source: {e:?}")
        });
        assert_eq!(
            retained_observed.parent_identity(),
            original_parent_id,
            "{PRECONDITION}retained parent no longer has its original ID"
        );
        assert_eq!(
            retained_observed.version(),
            Some(&original_version),
            "{PRECONDITION}retained file no longer has its original full version"
        );
    }
    assert!(
        binding
            .disk
            .as_ref()
            .unwrap()
            .anchor
            .retains(original_version.identity)
            .unwrap()
    );

    let before = observe(&mut store, n.doc);
    let bytes = generated_tree_bytes(&root);
    let error = failure(new.prepare_save_as(&binding, &mut store));
    assert!(matches!(
        &error.issue,
        BoundIssue::Identity(IdentityError::Changed)
    ));
    unchanged(&mut store, &before, &error);
    assert_eq!(generated_tree_bytes(&root), bytes);
    let after = observe(&mut store, n.doc);
    assert_eq!(
        after.state.global.metadata.baseline,
        before.state.global.metadata.baseline
    );
    assert_eq!(
        after.state.global.metadata.attention.binding_epoch,
        before.state.global.metadata.attention.binding_epoch
    );
    let disk = binding.checked_disk_binding(&store).unwrap();
    assert_eq!(disk.path, historical);
    assert_eq!(disk.parent, original_parent_id);
    assert_eq!(disk.version, original_version);
    assert_eq!(disk.epoch, original_epoch);
    assert!(disk.anchor.retains(original_version.identity).unwrap());
    assert_eq!(fs::read(&retained).unwrap(), ORIGINAL);
    assert_eq!(fs::read(&upper).unwrap(), REPLACEMENT);
    assert_eq!(fs::read(&historical).unwrap(), REPLACEMENT);

    // An unrelated parent remains a legal SaveAs. The failed Current preflight
    // must not consume operation 1 or leave a reservation blocking this owner.
    let independent_parent = root.join("independent-parent");
    fs::create_dir(&independent_parent).unwrap();
    let destination = independent_parent.join("copy.txt");
    let bytes = generated_tree_bytes(&root);
    let copy = ObservedTarget::inspect(&destination)
        .unwrap()
        .prepare_save_as(&binding, &mut store)
        .unwrap();
    assert_eq!(copy.capture().operation, 1);
    assert_eq!(copy.capture().binding_epoch, original_epoch);
    assert_eq!(copy.capture().snapshot.token, n.token);
    assert_eq!(copy.prepared.bytes(), ORIGINAL);
    store.cancel_save(copy.capture()).unwrap();
    drop(copy);
    assert!(!destination.exists());
    assert_eq!(generated_tree_bytes(&root), bytes);
    let after = observe(&mut store, n.doc);
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
    assert_eq!(store.snapshot(n.doc).unwrap().token, n.token);
    assert_eq!(fs::read(retained).unwrap(), ORIGINAL);
    assert_eq!(fs::read(upper).unwrap(), REPLACEMENT);
    assert_eq!(fs::read(historical).unwrap(), REPLACEMENT);
}
