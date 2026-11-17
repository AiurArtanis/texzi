use std::sync::Arc;
use texzi_document::{files::*, *};
mod legacy_spans;
fn format(encoding: Encoding, bom: bool) -> Format {
    Format { encoding, bom }
}
#[test]
fn strict_unicode_bom_eol_and_byte_exact_baseline() {
    for e in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
        for bom in [false, true] {
            let f = format(e, bom);
            let bytes = encode("甲😀\r\na\nb\r", f).unwrap();
            let loaded = LoadedBytes::decode(Arc::from(bytes.clone()), Some(e)).unwrap();
            assert_eq!(loaded.text(), "甲😀\r\na\nb\r");
            assert_eq!(loaded.format(), f);
            let mut store = Store::new(HistoryBudget::default());
            let (s, binding) = loaded.attach(&mut store).unwrap();
            assert_eq!(s.body.eols(), vec![Eol::CrLf, Eol::Lf, Eol::Cr]);
            assert_eq!(&*binding.encode_snapshot(&s, f).unwrap(), bytes);
            assert!(!store.backed_by_disk(s.doc).unwrap());
        }
    }
}
#[test]
fn malformed_input_and_bom_conflicts_reject_without_replacement() {
    assert_eq!(
        LoadedBytes::decode(Arc::from([0xff, 0xfe, 0, 0]), Some(Encoding::Utf16Le))
            .unwrap()
            .text(),
        "\0"
    );
    for (bytes, e) in [
        (vec![0xc0, 0xaf], Encoding::Utf8),
        (vec![0, 0xd8], Encoding::Utf16Le),
        (vec![0xdc, 0], Encoding::Utf16Be),
        (vec![1], Encoding::Utf16Le),
    ] {
        assert!(LoadedBytes::decode(bytes.into(), Some(e)).is_err());
    }
    assert!(matches!(
        LoadedBytes::decode(Arc::from([0xff, 0xfe, 65, 0]), Some(Encoding::Utf8)),
        Err(FileError::BomMismatch)
    ));
    assert!(matches!(
        LoadedBytes::decode(Arc::from([0xff]), None),
        Err(FileError::EncodingChoiceRequired)
    ));
    let loaded =
        LoadedBytes::decode(Arc::from([0xef, 0xbb, 0xbf, 0xef, 0xbb, 0xbf, 65]), None).unwrap();
    assert_eq!(loaded.text(), "\u{feff}A");
    let mut store = Store::new(HistoryBudget::default());
    let (s, binding) = loaded.attach(&mut store).unwrap();
    assert_eq!(s.body.source().to_string(), "\u{feff}A");
    assert_eq!(
        &*binding.encode_snapshot(&s, binding.format()).unwrap(),
        &[0xef, 0xbb, 0xbf, 0xef, 0xbb, 0xbf, 65]
    );
}
#[test]
fn handwritten_feff_goldens_do_not_strip_content_and_undo_recovers_original() {
    let cases: Vec<(Format, Vec<u8>, &str)> = vec![
        (
            format(Encoding::Utf8, true),
            vec![0xef, 0xbb, 0xbf, 0xef, 0xbb, 0xbf, 65],
            "\u{feff}A",
        ),
        (
            format(Encoding::Utf16Le, true),
            vec![0xff, 0xfe, 0xff, 0xfe, 65, 0],
            "\u{feff}A",
        ),
        (
            format(Encoding::Utf16Be, true),
            vec![0xfe, 0xff, 0xfe, 0xff, 0, 65],
            "\u{feff}A",
        ),
        (
            format(Encoding::Utf8, false),
            vec![0xef, 0xbb, 0xbf, 65],
            "\u{feff}A",
        ),
        (
            format(Encoding::Utf16Le, true),
            vec![0xff, 0xfe, 0xff, 0xfe, 0xff, 0xfe, 65, 0],
            "\u{feff}\u{feff}A",
        ),
        (format(Encoding::Utf8, false), vec![], ""),
        (format(Encoding::Utf16Be, true), vec![0xfe, 0xff], ""),
    ];
    for (f, raw, text) in cases {
        let loaded = LoadedBytes::decode_exact(raw.clone().into(), f).unwrap();
        assert_eq!(loaded.text(), text);
        let mut store = Store::new(HistoryBudget::default());
        let (s, binding) = loaded.attach(&mut store).unwrap();
        assert_eq!(s.body.source().to_string(), text);
        assert_eq!(&*binding.encode_snapshot(&s, f).unwrap(), raw);
        if let Some(at) = text.find('A') {
            let next = store
                .submit(Transaction {
                    doc: s.doc,
                    base: s.revision,
                    edits: vec![Edit::input(&s, at..at + 1, "B")],
                    boundary: BoundaryPolicy::Grapheme,
                    cancelled: false,
                })
                .unwrap()
                .snapshot;
            let changed = binding.encode_snapshot(&next, f).unwrap();
            let decoded = LoadedBytes::decode_exact(changed, f).unwrap();
            assert_eq!(decoded.text(), text.replace('A', "B"));
            let restored = store.undo(s.doc).unwrap().snapshot;
            assert_eq!(restored.token, s.token);
            assert_eq!(&*binding.encode_snapshot(&restored, f).unwrap(), raw);
        }
    }
}
#[test]
fn format_history_and_memory_only_receipt_keep_newer_format_dirty() {
    let raw = vec![0xff, 0xfe, 65, 0];
    let loaded = LoadedBytes::decode(raw.clone().into(), None).unwrap();
    let f = loaded.format();
    let mut store = Store::new(HistoryBudget::default());
    let (s, b) = loaded.attach(&mut store).unwrap();
    let prepared = b
        .prepare(
            &mut store,
            TargetKey("memory-fixture".into()),
            String::new(),
            f,
        )
        .unwrap();
    let mut profile = s.body.profile;
    profile.file_format = Some(format(Encoding::Utf16Be, true));
    let changed = store
        .set_serialization(s.doc, s.revision, profile, None)
        .unwrap()
        .snapshot;
    assert_ne!(changed.token, s.token);
    assert_eq!(
        &*b.encode_snapshot(&changed, profile.file_format.unwrap())
            .unwrap(),
        &[0xfe, 0xff, 0, 65]
    );
    // Simulated Store receipt only; no file is written or claimed verified here.
    super::complete(&mut store, prepared.capture());
    assert!(store.dirty(s.doc).unwrap());
    assert_eq!(store.snapshot(s.doc).unwrap().body.profile, profile);
    assert_eq!(
        store.accept_verified_save(prepared.capture()),
        Err(Error::InvalidSave)
    );
    let restored = store.undo(s.doc).unwrap().snapshot;
    assert_eq!(restored.token, s.token);
    assert!(!store.dirty(s.doc).unwrap());
    assert_eq!(&*b.encode_snapshot(&restored, f).unwrap(), raw);
    let noop = store
        .set_serialization(s.doc, restored.revision, restored.body.profile, None)
        .unwrap();
    assert!(!noop.changed);
    assert_eq!(noop.snapshot.token, restored.token);
    let redo = store.redo(s.doc).unwrap().snapshot;
    assert_eq!(redo.body.profile, profile);
    assert!(store.dirty(s.doc).unwrap());
    let mut without_bom = profile;
    without_bom.file_format = Some(format(Encoding::Utf16Be, false));
    let bom_change = store
        .set_serialization(s.doc, redo.revision, without_bom, None)
        .unwrap()
        .snapshot;
    assert_ne!(bom_change.token, redo.token);
    assert_eq!(bom_change.body.source().to_string(), "A");
    assert_eq!(
        &*b.encode_snapshot(&bom_change, without_bom.file_format.unwrap())
            .unwrap(),
        &[0, 65]
    );
    let undo_bom = store.undo(s.doc).unwrap().snapshot;
    assert_eq!(undo_bom.token, redo.token);
    assert_eq!(undo_bom.body.profile, profile);
}
#[test]
fn unencodable_format_is_undoable_and_does_not_reserve_save_target() {
    let loaded = LoadedBytes::decode_exact(
        vec![0xf0, 0x9f, 0x98, 0x80].into(),
        format(Encoding::Utf8, false),
    )
    .unwrap();
    let mut store = Store::new(HistoryBudget::default());
    let (s, b) = loaded.attach(&mut store).unwrap();
    let mut profile = s.body.profile;
    let f = format(Encoding::Windows1252, false);
    profile.file_format = Some(f);
    let bad = store
        .set_serialization(s.doc, s.revision, profile, None)
        .unwrap()
        .snapshot;
    assert_ne!(bad.token, s.token);
    assert!(
        b.prepare(
            &mut store,
            TargetKey("bad-encoding".into()),
            String::new(),
            f
        )
        .is_err()
    );
    assert!(store.dirty(s.doc).unwrap());
    let c = store
        .begin_save(s.doc, TargetKey("not-reserved".into()), String::new())
        .unwrap();
    store.cancel_save(&c).unwrap();
    let restored = store.undo(s.doc).unwrap().snapshot;
    assert_eq!(restored.token, s.token);
    assert!(!store.dirty(s.doc).unwrap());
}
#[test]
fn noncanonical_legacy_bytes_preserve_untouched_alias_after_edit_and_undo() {
    let f = format(Encoding::Gb18030WebEncodingRs0844, false);
    let raw = vec![0xa3, 0xa0, 65];
    let loaded = LoadedBytes::decode_exact(raw.clone().into(), f).unwrap();
    assert_eq!(loaded.text(), "\u{3000}A");
    let mut store = Store::new(HistoryBudget::default());
    let (s, b) = loaded.attach(&mut store).unwrap();
    assert_eq!(&*b.encode_snapshot(&s, f).unwrap(), raw);
    let changed = store
        .submit(Transaction {
            doc: s.doc,
            base: s.revision,
            edits: vec![Edit::input(&s, 3..4, "B")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    assert_eq!(&*b.encode_snapshot(&changed, f).unwrap(), &[0xa3, 0xa0, 66]);
    let restored = store.undo(s.doc).unwrap().snapshot;
    assert_eq!(&*b.encode_snapshot(&restored, f).unwrap(), raw);
    let redone = store.redo(s.doc).unwrap().snapshot;
    assert_eq!(&*b.encode_snapshot(&redone, f).unwrap(), &[0xa3, 0xa0, 66]);
}
#[test]
fn legacy_choices_are_explicit_and_lossy_output_is_rejected() {
    let f = format(Encoding::GbkWebEncodingRs0844, false);
    assert_eq!(encode("中文", f).unwrap(), vec![0xd6, 0xd0, 0xce, 0xc4]);
    assert!(LoadedBytes::decode(Arc::from([0x81, 0x30, 0x81, 0x30]), Some(f.encoding)).is_err());
    assert!(encode("😀", f).is_err());
    assert_eq!(
        encode("€", format(Encoding::Windows1252, false)).unwrap(),
        vec![0x80]
    );
    assert!(LoadedBytes::decode(Arc::from([0x81]), Some(Encoding::Windows1252)).is_err());
    for encoding in [Encoding::Gbk936, Encoding::Gb18030_2022] {
        assert!(matches!(
            LoadedBytes::decode(Arc::from([65]), Some(encoding)),
            Err(FileError::UnsupportedStandard)
        ));
    }
}
#[test]
fn captured_payload_does_not_follow_later_edits_or_clear_dirty() {
    let raw = vec![0xff, 0xfe, 65, 0, 13, 0, 10, 0];
    let loaded = LoadedBytes::decode(raw.clone().into(), None).unwrap();
    let f = loaded.format();
    let mut store = Store::new(HistoryBudget::default());
    let (s, binding) = loaded.attach(&mut store).unwrap();
    let prepared = binding
        .prepare(
            &mut store,
            TargetKey("fixture-only-target".into()),
            "fixture-guard".into(),
            f,
        )
        .unwrap();
    store
        .submit(Transaction {
            doc: s.doc,
            base: s.revision,
            edits: vec![Edit::input(&s, 0..0, "B")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap();
    assert_eq!(prepared.bytes(), raw);
    assert_eq!(prepared.capture().snapshot.token, s.token);
    assert!(store.dirty(s.doc).unwrap());
    assert_eq!(
        store.accept_verified_save(prepared.capture()),
        Err(Error::InvalidSave)
    );
    store.cancel_save(prepared.capture()).unwrap();
    let restored = store.undo(s.doc).unwrap().snapshot;
    assert_eq!(restored.token, s.token);
    assert_eq!(&*binding.encode_snapshot(&restored, f).unwrap(), raw);
    assert!(!store.dirty(s.doc).unwrap());
}
