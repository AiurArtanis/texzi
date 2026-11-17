use std::sync::Arc;
use texzi_document::{files::*, *};
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
    assert!(matches!(
        loaded.attach(&mut store),
        Err(FileError::NeedsDecodedImportApi)
    ));
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
