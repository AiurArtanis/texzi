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
