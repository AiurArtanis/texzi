//! Memory-only binding reconstruction tests, not native publication/IO evidence.
use super::*;
use crate::{BoundaryPolicy, Edit, HistoryBudget, Store, Transaction};
use std::sync::Arc;

fn edited(store: &mut Store, before: &Snapshot, range: Range<usize>, text: &str) -> Snapshot {
    store
        .submit(Transaction {
            doc: before.doc,
            base: before.revision,
            edits: vec![Edit::input(before, range, text)],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot
}

fn memory_binding(snapshot: &Snapshot, raw: Vec<u8>, format: Format) -> FileBinding {
    FileBinding {
        imported: snapshot.clone(),
        original: raw.into(),
        format,
        #[cfg(windows)]
        disk: None,
    }
}

#[test]
fn edited_baseline_with_saved_none_spans_still_maps_original_alias_and_eols() {
    let format = Format {
        encoding: Encoding::Gb18030WebEncodingRs0844,
        bom: false,
    };
    let original = vec![0xa3, 0xa0, b'A', b'\r', b'\n', b'B'];
    let mut store = Store::new(HistoryBudget::default());
    let (initial, binding) = super::super::LoadedBytes::decode_exact(original.into(), format)
        .unwrap()
        .attach(&mut store)
        .unwrap();
    let n = edited(&mut store, &initial, 3..4, "中X");
    let n_raw = binding.encode_snapshot(&n, format).unwrap();
    assert_eq!(&*n_raw, &[0xa3, 0xa0, 0xd6, 0xd0, b'X', b'\r', b'\n', b'B']);
    assert!(n.body.origins().iter().any(|(_, origin)| origin.is_none()));
    let rebound = memory_binding(&n, n_raw.to_vec(), format);
    let current = edited(&mut store, &n, 8..9, "Z");
    assert_eq!(
        &*rebound.encode_snapshot(&current, format).unwrap(),
        &[0xa3, 0xa0, 0xd6, 0xd0, b'X', b'\r', b'\n', b'Z']
    );
    let restored = store.undo(n.doc).unwrap().snapshot;
    assert!(Arc::ptr_eq(&restored.body, &n.body));
    assert_eq!(
        &*rebound.encode_snapshot(&restored, format).unwrap(),
        &*n_raw
    );
}

#[test]
fn canonical_rebound_bytes_do_not_authorize_an_older_unmapped_origin() {
    let format = Format {
        encoding: Encoding::Gb18030WebEncodingRs0844,
        bom: false,
    };
    let mut store = Store::new(HistoryBudget::default());
    let (initial, binding) =
        super::super::LoadedBytes::decode_exact(vec![0xa3, 0xa0, b'A'].into(), format)
            .unwrap()
            .attach(&mut store)
            .unwrap();
    let n = edited(&mut store, &initial, 0..3, "");
    let n_raw = binding.encode_snapshot(&n, format).unwrap();
    assert_eq!(&*n_raw, b"A");
    let rebound = memory_binding(&n, n_raw.to_vec(), format);
    let older = store.undo(n.doc).unwrap().snapshot;
    assert_eq!(older.token, initial.token);
    assert_eq!(older.body.source().to_string(), "\u{3000}A");
    assert!(matches!(
        rebound.encode_snapshot(&older, format),
        Err(FileError::NeedsLegacyByteMap)
    ));
    // The original binding still has that exact source's bytes. This does not
    // simulate a saved baseline, disk receipt, epoch change or native publish.
    assert_eq!(
        &*binding.encode_snapshot(&older, format).unwrap(),
        &[0xa3, 0xa0, b'A']
    );
    let redone = store.redo(n.doc).unwrap().snapshot;
    assert_eq!(&*rebound.encode_snapshot(&redone, format).unwrap(), b"A");
}

fn gb18030_alias_with_inserted_none_newlines() -> (Store, Snapshot, FileBinding, Snapshot) {
    let format = Format {
        encoding: Encoding::Gb18030WebEncodingRs0844,
        bom: false,
    };
    // A3 A0 is the imported U+3000 alias. The original EOLs are CRLF, LF, CR.
    // Logical UTF-8 offsets: U+3000 0..3, A 3..4, LF 4..5, B 5..6,
    // LF 6..7, C 7..8, LF 8..9, D 9..10.
    let original = vec![
        0xa3, 0xa0, b'A', b'\r', b'\n', b'B', b'\n', b'C', b'\r', b'D',
    ];
    let mut store = Store::new(HistoryBudget::default());
    let (initial, binding) = super::super::LoadedBytes::decode_exact(original.into(), format)
        .unwrap()
        .attach(&mut store)
        .unwrap();
    assert_eq!(initial.body.source().to_string(), "\u{3000}A\nB\nC\nD");
    assert_eq!(initial.body.eols(), vec![Eol::CrLf, Eol::Lf, Eol::Cr]);

    let mut insert = Edit::input(&initial, 6..6, "中\r文\nX\r\n");
    assert_eq!(insert.text, "中\n文\nX\n");
    // Freeze the plan explicitly, independent of dominant-EOL selection.
    insert.eols = vec![Eol::Cr, Eol::Lf, Eol::CrLf];
    assert_eq!(
        insert.text.bytes().filter(|&b| b == b'\n').count(),
        insert.eols.len()
    );
    let inserted = store
        .submit(Transaction {
            doc: initial.doc,
            base: initial.revision,
            edits: vec![insert],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    assert_eq!(
        inserted.body.source().to_string(),
        "\u{3000}A\nB中\n文\nX\n\nC\nD"
    );
    assert_eq!(
        inserted.body.eols(),
        vec![Eol::CrLf, Eol::Cr, Eol::Lf, Eol::CrLf, Eol::Lf, Eol::Cr]
    );
    let none_ranges: Vec<_> = inserted
        .body
        .origins()
        .into_iter()
        .filter_map(|(range, origin)| origin.is_none().then_some(range))
        .collect();
    assert_eq!(none_ranges, vec![6..16]);
    assert_eq!(
        inserted.body.source().byte_slice(6..16).to_string(),
        "中\n文\nX\n"
    );
    (store, initial, binding, inserted)
}

#[test]
fn inserted_none_newlines_preserve_alias_and_all_mixed_eols_through_undo_redo() {
    let (mut store, initial, binding, inserted) = gb18030_alias_with_inserted_none_newlines();
    let format = binding.format();
    // Handwritten bytes: inserted 中=D6 D0, 文=CE C4; new EOLs=CR, LF, CRLF.
    // The original alias and the old CRLF/LF/CR remain in their original order.
    let expected = [
        0xa3, 0xa0, b'A', b'\r', b'\n', b'B', 0xd6, 0xd0, b'\r', 0xce, 0xc4, b'\n', b'X', b'\r',
        b'\n', b'\n', b'C', b'\r', b'D',
    ];
    assert_ne!(inserted.token, initial.token);
    assert!(!Arc::ptr_eq(&inserted.body, &initial.body));
    assert_eq!(
        &*binding.encode_snapshot(&inserted, format).unwrap(),
        &expected
    );

    let restored = store.undo(initial.doc).unwrap().snapshot;
    assert_eq!(restored.token, initial.token);
    assert!(Arc::ptr_eq(&restored.body, &initial.body));
    assert_eq!(restored.body.eols(), vec![Eol::CrLf, Eol::Lf, Eol::Cr]);
    assert_eq!(
        &*binding.encode_snapshot(&restored, format).unwrap(),
        &[
            0xa3, 0xa0, b'A', b'\r', b'\n', b'B', b'\n', b'C', b'\r', b'D'
        ]
    );
    let redone = store.redo(initial.doc).unwrap().snapshot;
    assert_eq!(redone.token, inserted.token);
    assert!(Arc::ptr_eq(&redone.body, &inserted.body));
    assert_eq!(
        redone.body.eols(),
        vec![Eol::CrLf, Eol::Cr, Eol::Lf, Eol::CrLf, Eol::Lf, Eol::Cr]
    );
    assert_eq!(
        &*binding.encode_snapshot(&redone, format).unwrap(),
        &expected
    );
}

#[test]
fn memory_rebound_with_none_newlines_and_multibyte_text_maps_a_later_none_edit() {
    let (mut store, _initial, binding, inserted) = gb18030_alias_with_inserted_none_newlines();
    let format = binding.format();
    let inserted_raw = binding.encode_snapshot(&inserted, format).unwrap();
    assert_eq!(
        &*inserted_raw,
        &[
            0xa3, 0xa0, b'A', b'\r', b'\n', b'B', 0xd6, 0xd0, b'\r', 0xce, 0xc4, b'\n', b'X',
            b'\r', b'\n', b'\n', b'C', b'\r', b'D',
        ]
    );
    // This reconstructs only an in-memory binding, with the inserted None span
    // containing multibyte text and all three EOL kinds. No save/epoch/IO occurs.
    let rebound = memory_binding(&inserted, inserted_raw.to_vec(), format);
    // Within None 6..16: 中 6..9, LF 9..10, 文 10..13, LF 13..14,
    // X 14..15, LF 15..16. Replace 文 with 中Y, retaining every newline.
    let current = edited(&mut store, &inserted, 10..13, "中Y");
    assert_ne!(current.token, inserted.token);
    assert!(!Arc::ptr_eq(&current.body, &inserted.body));
    assert_eq!(
        current.body.source().to_string(),
        "\u{3000}A\nB中\n中Y\nX\n\nC\nD"
    );
    assert_eq!(
        current.body.eols(),
        vec![Eol::CrLf, Eol::Cr, Eol::Lf, Eol::CrLf, Eol::Lf, Eol::Cr]
    );
    let expected = [
        0xa3, 0xa0, b'A', b'\r', b'\n', b'B', 0xd6, 0xd0, b'\r', 0xd6, 0xd0, b'Y', b'\n', b'X',
        b'\r', b'\n', b'\n', b'C', b'\r', b'D',
    ];
    assert_eq!(
        &*rebound.encode_snapshot(&current, format).unwrap(),
        &expected
    );
    let restored = store.undo(inserted.doc).unwrap().snapshot;
    assert!(Arc::ptr_eq(&restored.body, &inserted.body));
    assert_eq!(
        &*rebound.encode_snapshot(&restored, format).unwrap(),
        &*inserted_raw
    );
    let redone = store.redo(inserted.doc).unwrap().snapshot;
    assert!(Arc::ptr_eq(&redone.body, &current.body));
    assert_eq!(
        &*rebound.encode_snapshot(&redone, format).unwrap(),
        &expected
    );
}

#[test]
fn same_format_gbk_preserves_single_byte_euro_and_double_byte_origin_on_adjacent_edits() {
    let format = Format {
        encoding: Encoding::GbkWebEncodingRs0844,
        bom: false,
    };
    // Pinned encoding_rs 0.8.44 uses Gb18030Decoder for GBK; gb18030.rs
    // explicitly decodes 80 as U+20AC. These bytes are handwritten, not encode
    // output or an independent GBK-standard oracle. 中 is the original D6 D0.
    // Logical offsets: Euro 0..3, A 3..4, 中 4..7, B 7..8, LF 8..9, C 9..10.
    let original = vec![0x80, b'A', 0xd6, 0xd0, b'B', b'\r', b'\n', b'C'];
    let mut store = Store::new(HistoryBudget::default());
    let (initial, binding) = super::super::LoadedBytes::decode_exact(original.into(), format)
        .unwrap()
        .attach(&mut store)
        .unwrap();
    assert_eq!(initial.body.source().to_string(), "\u{20ac}A中B\nC");
    assert_eq!(initial.body.eols(), vec![Eol::CrLf]);
    let left = edited(&mut store, &initial, 3..4, "XY");
    assert_ne!(left.token, initial.token);
    assert!(!Arc::ptr_eq(&left.body, &initial.body));
    assert_eq!(
        &*binding.encode_snapshot(&left, format).unwrap(),
        &[0x80, b'X', b'Y', 0xd6, 0xd0, b'B', b'\r', b'\n', b'C']
    );
    // The first edit shifts original B to 8..9, immediately after 中 5..8.
    let current = edited(&mut store, &left, 8..9, "Z");
    assert_eq!(current.body.source().to_string(), "\u{20ac}XY中Z\nC");
    assert_eq!(current.body.eols(), vec![Eol::CrLf]);
    assert_eq!(
        &*binding.encode_snapshot(&current, format).unwrap(),
        &[0x80, b'X', b'Y', 0xd6, 0xd0, b'Z', b'\r', b'\n', b'C']
    );
}
