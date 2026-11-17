use super::*;

// Source fixtures only: NOT_RUN when authored; no filesystem save is exercised.
// Handwritten byte expectations are independent of texzi's encoder. The pinned
// encoding_rs 0.8.44 src/gb18030.rs fixtures explicitly decode A3 A0 and A1 A1 as
// U+3000 and encode U+3000 as A1 A1 (GB18030 and GBK).

fn replace(store: &mut Store, s: &Snapshot, range: std::ops::Range<usize>, text: &str) -> Snapshot {
    store
        .submit(Transaction {
            doc: s.doc,
            base: s.revision,
            edits: vec![Edit::input(s, range, text)],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot
}

#[test]
fn handwritten_alias_and_mixed_eols_survive_ascii_edit_undo_redo() {
    let f = format(Encoding::Gb18030WebEncodingRs0844, false);
    let raw: &[u8] = b"\xa3\xa0A\r\nB\nC\rD";
    let loaded = LoadedBytes::decode_exact(Arc::from(raw), f).unwrap();
    assert_eq!(loaded.text(), "\u{3000}A\r\nB\nC\rD");
    let mut store = Store::new(HistoryBudget::default());
    let (s, binding) = loaded.attach(&mut store).unwrap();
    assert_eq!(s.body.source().to_string(), "\u{3000}A\nB\nC\nD");
    assert_eq!(s.body.eols(), vec![Eol::CrLf, Eol::Lf, Eol::Cr]);

    // SourceByte offsets address normalized UTF-8, not the original byte stream.
    let changed = replace(&mut store, &s, 5..6, "X");
    assert_eq!(changed.body.source().to_string(), "\u{3000}A\nX\nC\nD");
    assert_eq!(changed.body.eols(), vec![Eol::CrLf, Eol::Lf, Eol::Cr]);
    assert_eq!(
        &*binding.encode_snapshot(&changed, f).unwrap(),
        b"\xa3\xa0A\r\nX\nC\rD"
    );
    assert_eq!(s.body.source().to_string(), "\u{3000}A\nB\nC\nD");
    assert_eq!(s.body.eols(), vec![Eol::CrLf, Eol::Lf, Eol::Cr]);
    assert_eq!(&*binding.encode_snapshot(&s, f).unwrap(), raw);

    let undo = store.undo(s.doc).unwrap().snapshot;
    assert_eq!(undo.token, s.token);
    assert!(Arc::ptr_eq(&undo.body, &s.body));
    assert_eq!(&*binding.encode_snapshot(&undo, f).unwrap(), raw);
    let redo = store.redo(s.doc).unwrap().snapshot;
    assert_eq!(redo.token, changed.token);
    assert!(Arc::ptr_eq(&redo.body, &changed.body));
    assert_eq!(
        &*binding.encode_snapshot(&redo, f).unwrap(),
        b"\xa3\xa0A\r\nX\nC\rD"
    );
    assert!(!store.backed_by_disk(s.doc).unwrap());
}

#[test]
fn handwritten_four_byte_scalar_stays_intact_beside_an_ascii_edit() {
    // Pinned encoding_rs gb18030.rs golden: 94 39 DA 33 -> U+1F4A9.
    let f = format(Encoding::Gb18030WebEncodingRs0844, false);
    let raw: &[u8] = b"\xa3\xa0\x94\x39\xda\x33A\r\n";
    let loaded = LoadedBytes::decode_exact(Arc::from(raw), f).unwrap();
    assert_eq!(loaded.text(), "\u{3000}\u{1f4a9}A\r\n");
    let mut store = Store::new(HistoryBudget::default());
    let (s, binding) = loaded.attach(&mut store).unwrap();
    let changed = replace(&mut store, &s, 7..8, "B");
    assert_eq!(
        &*binding.encode_snapshot(&changed, f).unwrap(),
        b"\xa3\xa0\x94\x39\xda\x33B\r\n"
    );
    assert_eq!(&*binding.encode_snapshot(&s, f).unwrap(), raw);
}

#[test]
fn replacing_alias_encodes_new_spaces_canonically_and_keeps_untouched_alias() {
    let f = format(Encoding::Gb18030WebEncodingRs0844, false);
    let raw: &[u8] = b"\xa3\xa0A\xa3\xa0";
    let loaded = LoadedBytes::decode_exact(Arc::from(raw), f).unwrap();
    let mut store = Store::new(HistoryBudget::default());
    let (s, binding) = loaded.attach(&mut store).unwrap();

    // Replacing one space with two is an actual edit; identical text is a no-op.
    let changed = replace(&mut store, &s, 0..3, "\u{3000}\u{3000}");
    assert_eq!(
        changed.body.source().to_string(),
        "\u{3000}\u{3000}A\u{3000}"
    );
    assert_eq!(
        &*binding.encode_snapshot(&changed, f).unwrap(),
        b"\xa1\xa1\xa1\xa1A\xa3\xa0"
    );
    assert_eq!(&*binding.encode_snapshot(&s, f).unwrap(), raw);
    let undo = store.undo(s.doc).unwrap().snapshot;
    assert_eq!(&*binding.encode_snapshot(&undo, f).unwrap(), raw);
}

#[test]
fn multiple_length_changing_edits_keep_aliases_on_both_sides() {
    let f = format(Encoding::Gb18030WebEncodingRs0844, false);
    let raw: &[u8] = b"L\xa3\xa0M\r\nN\xa3\xa0R\nS\r";
    let loaded = LoadedBytes::decode_exact(Arc::from(raw), f).unwrap();
    let mut store = Store::new(HistoryBudget::default());
    let (s, binding) = loaded.attach(&mut store).unwrap();
    assert_eq!(s.body.source().to_string(), "L\u{3000}M\nN\u{3000}R\nS\n");
    let changed = store
        .submit(Transaction {
            doc: s.doc,
            base: s.revision,
            edits: vec![
                Edit::input(&s, 0..1, "LEFT"),
                Edit::input(&s, 4..5, "middle"),
                Edit::input(&s, 10..11, "RIGHT"),
            ],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    assert_eq!(
        changed.body.source().to_string(),
        "LEFT\u{3000}middle\nN\u{3000}RIGHT\nS\n"
    );
    assert_eq!(
        &*binding.encode_snapshot(&changed, f).unwrap(),
        b"LEFT\xa3\xa0middle\r\nN\xa3\xa0RIGHT\nS\r"
    );
    assert_eq!(changed.body.eols(), vec![Eol::CrLf, Eol::Lf, Eol::Cr]);
    assert_eq!(&*binding.encode_snapshot(&s, f).unwrap(), raw);
}

#[test]
fn format_changes_encode_content_and_never_reuse_old_alias_bytes() {
    let original_format = format(Encoding::Gb18030WebEncodingRs0844, false);
    let raw: &[u8] = b"\xa3\xa0A\r\nB\nC\r";
    for (target_format, expected) in [
        (
            format(Encoding::GbkWebEncodingRs0844, false),
            b"\xa1\xa1A\r\nB\nC\r".as_slice(),
        ),
        (
            format(Encoding::Utf8, true),
            b"\xef\xbb\xbf\xe3\x80\x80A\r\nB\nC\r".as_slice(),
        ),
    ] {
        let loaded = LoadedBytes::decode_exact(Arc::from(raw), original_format).unwrap();
        let mut store = Store::new(HistoryBudget::default());
        let (s, binding) = loaded.attach(&mut store).unwrap();
        let mut profile = s.body.profile;
        profile.file_format = Some(target_format);
        let changed = store
            .set_serialization(s.doc, s.revision, profile, None)
            .unwrap()
            .snapshot;
        assert_eq!(
            &*binding.encode_snapshot(&changed, target_format).unwrap(),
            expected
        );
        assert_eq!(changed.body.eols(), vec![Eol::CrLf, Eol::Lf, Eol::Cr]);
        assert_eq!(&*binding.encode_snapshot(&s, original_format).unwrap(), raw);
        let undo = store.undo(s.doc).unwrap().snapshot;
        assert_eq!(undo.token, s.token);
        assert_eq!(
            &*binding.encode_snapshot(&undo, original_format).unwrap(),
            raw
        );
    }

    // Neither strict standard may borrow the explicitly selected Web codec/map.
    for encoding in [Encoding::Gbk936, Encoding::Gb18030_2022] {
        let loaded = LoadedBytes::decode_exact(Arc::from(raw), original_format).unwrap();
        let mut store = Store::new(HistoryBudget::default());
        let (s, binding) = loaded.attach(&mut store).unwrap();
        let target_format = format(encoding, false);
        let mut profile = s.body.profile;
        profile.file_format = Some(target_format);
        let changed = store
            .set_serialization(s.doc, s.revision, profile, None)
            .unwrap()
            .snapshot;
        assert!(matches!(
            binding.encode_snapshot(&changed, target_format),
            Err(FileError::UnsupportedStandard)
        ));
    }
}

#[test]
fn unencodable_new_fragment_does_not_reserve_target_or_skip_operation() {
    let f = format(Encoding::GbkWebEncodingRs0844, false);
    let raw: &[u8] = b"\xa3\xa0A";
    let loaded = LoadedBytes::decode_exact(Arc::from(raw), f).unwrap();
    let mut store = Store::new(HistoryBudget::default());
    let (s, binding) = loaded.attach(&mut store).unwrap();
    let target = TargetKey("legacy-map-memory-fixture".into());

    // These captures exercise only in-memory reservation, not a disk save.
    let first = binding
        .prepare(&mut store, target.clone(), String::new(), f)
        .unwrap();
    let operation = first.capture().operation;
    store.cancel_save(first.capture()).unwrap();
    let changed = replace(&mut store, &s, 3..4, "😀");
    assert!(matches!(
        binding.prepare(&mut store, target.clone(), String::new(), f),
        Err(FileError::Unrepresentable)
    ));
    let current = store.snapshot(s.doc).unwrap();
    assert_eq!(current.token, changed.token);
    assert!(Arc::ptr_eq(&current.body, &changed.body));
    assert!(store.dirty(s.doc).unwrap());

    let undo = store.undo(s.doc).unwrap().snapshot;
    assert_eq!(undo.token, s.token);
    let next = binding
        .prepare(&mut store, target.clone(), String::new(), f)
        .unwrap();
    assert_eq!(next.capture().operation, operation + 1);
    assert_eq!(next.capture().target, target);
    assert_eq!(next.capture().snapshot.token, undo.token);
    assert_eq!(next.bytes(), raw);
    store.cancel_save(next.capture()).unwrap();
    assert!(!store.backed_by_disk(s.doc).unwrap());
}
