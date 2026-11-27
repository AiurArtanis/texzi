use BulkPart::{Literal, Original};
use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use texzi_document::{
    files::{Encoding, Format, LoadedBytes, encode},
    *,
};

fn open(st: &mut Store, s: &Snapshot, at: usize, affinity: Affinity) -> ViewState {
    st.open_view(
        s.doc,
        ModeTag::Source,
        SelectionSet::caret(s, at),
        Anchor {
            position: SourceByte::at(s, at),
            affinity,
        },
        17,
    )
    .unwrap()
}
fn builder(s: &Snapshot) -> PreparedBulkBuilder {
    PreparedBulkBuilder::new(s.clone(), Arc::new(AtomicBool::new(false)))
}

const EURO_BYTES: [(Encoding, &[u8], &[u8]); 2] = [
    (Encoding::GbkWebEncodingRs0844, &[0xa2, 0xe3], &[0x80]),
    (Encoding::Gb18030WebEncodingRs0844, &[0x80], &[0xa2, 0xe3]),
];

#[test]
fn disk_backed_undo_and_redo_keep_legacy_aliases_after_the_import_binding_is_gone() {
    use texzi_document::files::encode_body;
    let format = Format {
        encoding: Encoding::Gb18030WebEncodingRs0844,
        bom: false,
    };
    let n = 4 * 1024 * 1024;
    let raw: Arc<[u8]> = [b"\x80".as_slice(), &vec![b'A'; n], b"\x80"]
        .concat()
        .into();
    let mut st = Store::new(HistoryBudget::default());
    st.set_observer_limits(ObserverLimits {
        journal_batches: 0,
        ..Default::default()
    })
    .unwrap();
    let s = st
        .open_loaded_with_line_limit(
            LoadedBytes::decode(raw.clone(), Some(format.encoding)).unwrap(),
            MAX_EDIT_LINES,
            || false,
        )
        .unwrap();
    let v = open(&mut st, &s, 0, Affinity::Before);
    let mut p = builder(&s);
    p.replace(3..3 + n, [Original(n + 3..n + 6), Literal(&"B".repeat(n))])
        .unwrap();
    drop(st.submit_bulk(v.id, v.epoch, p.finish().unwrap()).unwrap());
    drop(s);
    for redo in [false, true] {
        let HistoryStep::Cold(job) = st
            .take_history_step(v.id, st.view(v.id).unwrap().epoch, redo)
            .unwrap()
        else {
            panic!("history root should have been evicted")
        };
        let change = st.submit_loaded_history(job.load().unwrap()).unwrap();
        let bytes = encode_body(&change.snapshot.body, format).unwrap();
        if redo {
            assert_eq!(&bytes[..2], &[0x80, 0x80]);
            assert_eq!(&bytes[2..2 + n], vec![b'B'; n]);
            assert_eq!(&bytes[2 + n..], &[0x80]);
        } else {
            assert_eq!(bytes, raw.as_ref());
        }
    }
}

#[cfg(windows)]
#[test]
fn editing_ascii_next_to_a_legacy_alias_saves_its_original_bytes_and_undo_restores_the_file() {
    use std::{
        fs,
        hash::{BuildHasher, RandomState},
    };
    use texzi_document::files::{encode_body, save};

    let dir = std::env::temp_dir().join(format!(
        "texzi-gb-alias-{:016x}",
        RandomState::new().hash_one(std::process::id())
    ));
    fs::create_dir(&dir).unwrap();
    let path = dir.join("fixture.txt");
    for (encoding, alternate, canonical) in EURO_BYTES {
        let format = Format {
            encoding,
            bom: false,
        };
        assert_eq!(encode("€", format).unwrap(), canonical);
        let raw: Arc<[u8]> = [alternate, b"A"].concat().into();
        let expected = [alternate, b"B"].concat();
        fs::write(&path, &raw).unwrap();
        let mut st = Store::new(HistoryBudget::default());
        let (s, file) = LoadedBytes::decode(raw.clone(), Some(encoding))
            .unwrap()
            .attach(&mut st)
            .unwrap();
        assert_eq!(s.body.source().slice(..), "€A");
        let v = open(&mut st, &s, 0, Affinity::Before);
        let mut p = builder(&s);
        p.replace(3..4, [Literal("B")]).unwrap();
        let changed = st
            .submit_bulk(v.id, v.epoch, p.finish().unwrap())
            .unwrap()
            .snapshot;
        let output = encode_body(&changed.body, format).unwrap();
        assert_eq!(output, expected);
        assert_eq!(
            file.encode_snapshot(&changed, format).unwrap().as_ref(),
            expected
        );
        save::write(&path, &output, Some(&raw)).unwrap();
        assert_eq!(fs::read(&path).unwrap(), expected);
        let undo = st.undo(s.doc).unwrap().snapshot;
        let restored = encode_body(&undo.body, format).unwrap();
        save::write(&path, &restored, Some(&expected)).unwrap();
        assert_eq!(fs::read(&path).unwrap(), raw.as_ref());
    }
    fs::remove_file(path).unwrap();
    fs::remove_dir(dir).unwrap();
}

#[test]
fn an_identical_looking_legacy_capture_copies_its_wire_spelling_and_literal_text_uses_the_codec() {
    use texzi_document::files::encode_body;
    for (encoding, alternate, canonical) in EURO_BYTES {
        let format = Format {
            encoding,
            bom: false,
        };
        let raw: Arc<[u8]> = [b"x\r\n".as_slice(), alternate, b"|", canonical, b"\rz"]
            .concat()
            .into();
        let mut st = Store::new(HistoryBudget::default());
        let s = st
            .open_loaded_with_line_limit(
                LoadedBytes::decode(raw.clone(), Some(encoding)).unwrap(),
                MAX_EDIT_LINES,
                || false,
            )
            .unwrap();
        assert_eq!(s.body.source().slice(..), "x\n€|€\nz");
        let v = open(&mut st, &s, 0, Affinity::Before);
        let mut p = builder(&s);
        p.replace(6..9, [Original(2..5)]).unwrap();
        let copy = st.submit_bulk(v.id, v.epoch, p.finish().unwrap()).unwrap();
        assert!(copy.changed);
        assert_eq!(copy.snapshot.body.source(), s.body.source());
        assert_eq!(
            encode_body(&copy.snapshot.body, format).unwrap(),
            [b"x\r\n".as_slice(), alternate, b"|", alternate, b"\rz"].concat()
        );
        assert_eq!(
            encode_body(&st.undo(s.doc).unwrap().snapshot.body, format).unwrap(),
            raw.as_ref()
        );
        let s = st.snapshot(s.doc).unwrap();
        let mut p = builder(&s);
        p.replace(0..1, [Original(2..5), Literal("€"), Original(2..5)])
            .unwrap();
        let change = st
            .submit_bulk(v.id, st.view(v.id).unwrap().epoch, p.finish().unwrap())
            .unwrap();
        assert_eq!(
            encode_body(&change.snapshot.body, format).unwrap(),
            [
                alternate, canonical, alternate, b"\r\n", alternate, b"|", canonical, b"\rz"
            ]
            .concat()
        );
        let utf8 = Format {
            encoding: Encoding::Utf8,
            bom: true,
        };
        assert_eq!(
            encode_body(&change.snapshot.body, utf8).unwrap(),
            encode("€€€\r\n€|€\rz", utf8).unwrap()
        );
        let switched = st
            .set_serialization(
                s.doc,
                change.snapshot.revision,
                change.snapshot.body.profile,
                Some(Eol::Lf),
            )
            .unwrap()
            .snapshot;
        assert_eq!(
            encode_body(&switched.body, format).unwrap(),
            [
                alternate, canonical, alternate, b"\n", alternate, b"|", canonical, b"\nz"
            ]
            .concat()
        );
    }
}
#[test]
fn source_captures_reorder_and_preserve_mixed_eols_and_exact_undo_from_sibling() {
    let mut st = Store::new(HistoryBudget::default());
    let format = Format {
        encoding: Encoding::Utf16Le,
        bom: true,
    };
    let raw: Arc<[u8]> = encode("a\r\nb\nc\rz", format).unwrap().into();
    let (s, file) = LoadedBytes::decode(raw.clone(), Some(format.encoding))
        .unwrap()
        .attach(&mut st)
        .unwrap();
    let a = open(&mut st, &s, 0, Affinity::Before);
    let b = open(&mut st, &s, s.body.len_bytes(), Affinity::After);
    let (_, sub) = st
        .observe_and_subscribe(s.doc, ObserverScope::new(vec![a.id, b.id]), 4)
        .unwrap();
    let mut plan = builder(&s);
    plan.replace(0..3, [Original(3..6), Literal("\r\n"), Original(2..3)])
        .unwrap();
    let changed = st
        .submit_bulk(a.id, a.epoch, plan.finish().unwrap())
        .unwrap();
    assert!(changed.is_bulk() && changed.forward.is_empty() && changed.inverse.is_empty());
    assert_eq!(
        file.encode_snapshot(&changed.snapshot, format)
            .unwrap()
            .as_ref(),
        encode("\nc\r\r\nb\nc\rz", format).unwrap()
    );
    for (logical, origin) in changed.snapshot.body.origins() {
        if let Some(span) = origin {
            let part = Body::import_decoded(&span.source[span.raw], Profile::default());
            assert_eq!(
                part.source().slice(..),
                changed.snapshot.body.source().byte_slice(logical)
            );
        } else {
            assert_eq!(logical, 3..4);
        }
    }
    assert_eq!(st.history_depth(s.doc).unwrap(), (1, 0));
    let ObserverPoll::Batch(batch) = st.poll_observer(&sub).unwrap() else {
        panic!("missing bulk batch")
    };
    assert_eq!(
        batch
            .events
            .iter()
            .filter(|v| matches!(v,ObserverEvent::Content(c) if c.bulk.is_some()))
            .count(),
        1
    );
    assert_eq!(
        batch
            .events
            .iter()
            .filter(|v| matches!(v, ObserverEvent::Global(_)))
            .count(),
        1
    );
    let b = st.view(b.id).unwrap();
    assert_eq!(
        b.selection.selections[0].range().start,
        changed.snapshot.body.len_bytes()
    );
    let undo = st.undo_view(b.id, b.epoch).unwrap();
    assert_eq!(undo.history_group, changed.history_group);
    assert_eq!(undo.snapshot.token, s.token);
    assert!(Arc::ptr_eq(&undo.snapshot.body, &s.body));
    assert!(Arc::ptr_eq(
        &file.encode_snapshot(&undo.snapshot, format).unwrap(),
        &raw
    ));
    assert_eq!(
        st.view(b.id).unwrap().selection.selections[0].range().start,
        s.body.len_bytes()
    );
    assert!(!st.dirty(s.doc).unwrap());
    assert!(
        st.redo_view(a.id, st.view(a.id).unwrap().epoch)
            .unwrap()
            .is_bulk()
    );
}
#[test]
fn exact_scalar_changes_snap_only_view_endpoints_and_accept_two_hits_in_one_grapheme() {
    let mut st = Store::new(HistoryBudget::default());
    let s = st
        .open_utf8("e\u{301}Xz", Profile::default(), false)
        .unwrap();
    let a = open(&mut st, &s, 0, Affinity::Before);
    let b = open(&mut st, &s, 3, Affinity::Before);
    assert_eq!(s.snap_source_offset(1, Affinity::Before).unwrap(), 0);
    assert_eq!(s.snap_source_offset(1, Affinity::After).unwrap(), 3);
    let mut p = builder(&s);
    p.replace(0..1, [Literal("i")]).unwrap();
    p.replace(1..3, [Literal("\u{300}")]).unwrap();
    p.replace(3..4, [Literal("\u{301}")]).unwrap();
    let c = st.submit_bulk(a.id, a.epoch, p.finish().unwrap()).unwrap();
    assert_eq!(c.snapshot.body.source().slice(..), "i\u{300}\u{301}z");
    assert_eq!(st.view(b.id).unwrap().scroll.position.offset, 0);
    assert_eq!(st.view(b.id).unwrap().selection.selections[0].range(), 5..5);
    assert!(
        st.view(b.id)
            .unwrap()
            .mapping_snaps
            .iter()
            .any(|v| v.requested == 3 && v.actual == 0)
    );
    let new = open(&mut st, &c.snapshot, 6, Affinity::After);
    st.undo_view(new.id, new.epoch).unwrap();
    assert_eq!(
        st.view(new.id).unwrap().selection.selections[0].range(),
        5..5
    );
    assert_eq!(
        st.snapshot(s.doc).unwrap().body.source().slice(..),
        "e\u{301}Xz"
    );
}
#[test]
fn cancelled_stale_or_partially_invalid_preparation_never_publishes() {
    let mut st = Store::new(HistoryBudget::default());
    let s = st.open_utf8("eé", Profile::default(), false).unwrap();
    let v = open(&mut st, &s, 0, Affinity::Before);
    let mut p = builder(&s);
    p.replace(0..1, [Literal("new")]).unwrap();
    assert_eq!(
        p.replace(1..2, [Literal("bad")]),
        Err(Error::InvalidBoundary)
    );
    assert!(matches!(p.finish(), Err(Error::InvalidBoundary)));
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut p = PreparedBulkBuilder::new(s.clone(), cancelled.clone());
    p.replace(0..1, [Literal("new")]).unwrap();
    let ready = p.finish().unwrap();
    cancelled.store(true, Ordering::Release);
    assert!(matches!(
        st.submit_bulk(v.id, v.epoch, ready),
        Err(Error::Cancelled)
    ));
    let make = || {
        let mut p = builder(&s);
        p.replace(0..1, [Literal("new")]).unwrap();
        p.finish().unwrap()
    };
    st.set_selection(v.id, v.epoch, SelectionSet::caret(&s, 1))
        .unwrap();
    assert!(matches!(
        st.submit_bulk(v.id, v.epoch, make()),
        Err(Error::StaleView)
    ));
    let v = st.view(v.id).unwrap();
    let rich = st.set_mode_tag(v.id, v.epoch, ModeTag::Rich).unwrap();
    assert!(matches!(
        st.submit_bulk(rich.id, rich.epoch, make()),
        Err(Error::InvalidView)
    ));
    assert_eq!(st.history_depth(s.doc).unwrap(), (0, 0));
    assert!(!st.dirty(s.doc).unwrap());
    let v = st.set_mode_tag(v.id, rich.epoch, ModeTag::Source).unwrap();
    st.submit_bulk(v.id, v.epoch, make()).unwrap();
    assert!(matches!(
        st.submit_bulk(v.id, st.view(v.id).unwrap().epoch, make()),
        Err(Error::StaleRevision)
    ));
    assert_eq!(st.history_depth(s.doc).unwrap(), (1, 0));
}
#[test]
fn zero_or_byte_identical_with_same_eol_does_not_spend_history() {
    let mut st = Store::new(HistoryBudget {
        document: 0,
        process: 0,
    });
    let s = st.open_utf8("aa\r\n", Profile::default(), false).unwrap();
    let v = open(&mut st, &s, 0, Affinity::Before);
    let mut p = builder(&s);
    p.replace(0..1, []).unwrap();
    p.replace(2..2, [Literal("a")]).unwrap();
    assert!(
        !st.submit_bulk(v.id, v.epoch, p.finish().unwrap())
            .unwrap()
            .changed
    );
    let mut p = builder(&s);
    p.replace(1..3, [Original(1..3)]).unwrap();
    assert!(
        !st.submit_bulk(v.id, v.epoch, p.finish().unwrap())
            .unwrap()
            .changed
    );
    assert!(
        !st.submit_bulk(v.id, v.epoch, builder(&s).finish().unwrap())
            .unwrap()
            .changed
    );
    let mut p = builder(&s);
    p.replace(0..1, [Literal("b")]).unwrap();
    assert!(matches!(
        st.submit_bulk(v.id, v.epoch, p.finish().unwrap()),
        Err(Error::HistoryBudget)
    ));
    assert_eq!(st.history_charge(), 0);
    assert_eq!(st.snapshot(s.doc).unwrap().token, s.token);
}

#[cfg(windows)]
#[test]
fn disk_save_undo_and_external_conflict_keep_exact_bytes_across_codecs() {
    use std::{
        fs,
        hash::{BuildHasher, RandomState},
    };
    let dir = std::env::temp_dir().join(format!(
        "texzi-bulk-save-{:016x}",
        RandomState::new().hash_one(std::process::id())
    ));
    fs::create_dir(&dir).unwrap();
    let target = dir.join("target.txt");
    let neighbor = dir.join("neighbor.txt");
    fs::write(&neighbor, b"neighbor").unwrap();
    for (encoding, bom) in [
        (Encoding::Utf8, false),
        (Encoding::Utf8, true),
        (Encoding::Utf16Le, true),
        (Encoding::Utf16Be, true),
        (Encoding::Gb18030WebEncodingRs0844, false),
    ] {
        let format = Format { encoding, bom };
        let raw: Arc<[u8]> = encode("甲\r\n猫\n乙\r", format).unwrap().into();
        fs::write(&target, &raw).unwrap();
        let mut store = Store::new(HistoryBudget::default());
        let (s, file) = LoadedBytes::decode(raw.clone(), Some(encoding))
            .unwrap()
            .attach(&mut store)
            .unwrap();
        let a = open(&mut store, &s, 0, Affinity::Before);
        let b = open(&mut store, &s, 4, Affinity::After);
        let mut p = builder(&s);
        p.replace(4..7, [Original(8..11), Literal("-"), Original(0..3)])
            .unwrap();
        let edit = store
            .submit_bulk(a.id, a.epoch, p.finish().unwrap())
            .unwrap();
        let bytes = file.encode_snapshot(&edit.snapshot, format).unwrap();
        texzi_document::files::save::write(&target, &bytes, Some(&raw)).unwrap();
        assert_eq!(
            fs::read(&target).unwrap(),
            encode("甲\r\n乙-甲\n乙\r", format).unwrap()
        );
        let undo = store
            .undo_view(b.id, store.view(b.id).unwrap().epoch)
            .unwrap();
        let restored = file.encode_snapshot(&undo.snapshot, format).unwrap();
        texzi_document::files::save::write(&target, &restored, Some(&bytes)).unwrap();
        assert_eq!(fs::read(&target).unwrap(), raw.as_ref());
        fs::write(&target, b"external").unwrap();
        assert!(texzi_document::files::save::write(&target, &bytes, Some(&raw)).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"external");
        assert_eq!(fs::read(&neighbor).unwrap(), b"neighbor");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
    }
    fs::remove_file(target).unwrap();
    fs::remove_file(neighbor).unwrap();
    fs::remove_dir(dir).unwrap();
}

fn reference(map: &[(Range<usize>, usize)], pos: usize, a: Affinity) -> usize {
    let mut shift = 0isize;
    for (r, len) in map {
        if r.start > pos {
            break;
        }
        if pos < r.end {
            return r.start.checked_add_signed(shift).unwrap()
                + if a == Affinity::After { *len } else { 0 };
        }
        if pos == r.start && a == Affinity::Before {
            continue;
        }
        shift += *len as isize - r.len() as isize;
    }
    pos.checked_add_signed(shift).unwrap()
}
#[test]
fn both_compact_map_directions_match_elementary_edits_at_all_boundaries() {
    let mut st = Store::new(HistoryBudget::default());
    let s = st
        .open_utf8(&"x".repeat(180), Profile::default(), false)
        .unwrap();
    let a = open(&mut st, &s, 0, Affinity::Before);
    let edits = [
        (0..1, ""),
        (1..2, ""),
        (2..5, "XX"),
        (5..7, "XX"),
        (8..8, "123"),
        (9..9, "123"),
        (10..10, "123"),
        (15..16, ""),
        (17..18, ""),
        (19..20, ""),
        (22..23, "Z"),
        (24..25, "Z"),
        (26..27, "Z"),
        (90..100, "+"),
        (180..180, "END"),
    ];
    let mut f = Vec::new();
    let mut inv = Vec::new();
    let mut shift = 0isize;
    let mut p = builder(&s);
    for (r, text) in edits {
        p.replace(r.clone(), [Literal(text)]).unwrap();
        let start = r.start.checked_add_signed(shift).unwrap();
        inv.push((start..start + text.len(), r.len()));
        shift += text.len() as isize - r.len() as isize;
        f.push((r, text.len()));
    }
    let c = st.submit_bulk(a.id, a.epoch, p.finish().unwrap()).unwrap();
    let u = st.undo(s.doc).unwrap();
    for (rec, map) in [(&c, &f), (&u, &inv)] {
        for pos in 0..=rec.previous_body.len_bytes() {
            for aff in [Affinity::Before, Affinity::After] {
                assert_eq!(
                    rec.bulk.as_ref().unwrap().map_offset(pos, aff),
                    reference(map, pos, aff),
                    "at {pos}, affinity {aff:?}"
                );
            }
        }
    }
}
#[test]
fn dense_equal_shapes_are_one_run_and_irregular_edits_spill_and_stay_reversible() {
    let mut st = Store::new(HistoryBudget::default());
    let s = st
        .open_utf8(&"x".repeat(1_000_000), Profile::default(), false)
        .unwrap();
    let v = open(&mut st, &s, 0, Affinity::Before);
    let mut p = builder(&s);
    for i in 0..s.body.len_bytes() {
        p.replace(i..i + 1, [Literal("y")]).unwrap();
    }
    let c = st.submit_bulk(v.id, v.epoch, p.finish().unwrap()).unwrap();
    assert_eq!(c.bulk.as_ref().unwrap().run_count(), 1);
    assert_eq!(c.bulk.as_ref().unwrap().spill_bytes(), 0);
    let s = st.undo(s.doc).unwrap().snapshot;
    let mut p = builder(&s);
    let mut expected = String::new();
    let mut old = 0;
    for i in 0..16_000 {
        let len = 1 + i % 3;
        let gap = i % 2;
        expected.push_str(&"x".repeat(gap));
        old += gap;
        let text = if i % 2 == 0 { "AA" } else { "BBB" };
        p.replace(old..old + len, [Literal(text)]).unwrap();
        old += len;
        expected.push_str(text);
    }
    expected.push_str(&"x".repeat(s.body.len_bytes() - old));
    let r = st
        .submit_bulk(v.id, st.view(v.id).unwrap().epoch, p.finish().unwrap())
        .unwrap();
    assert!(r.bulk.as_ref().unwrap().spill_bytes() > 128 * 1024);
    assert_eq!(r.snapshot.body.source().slice(..), expected.as_str());
    assert_eq!(
        st.undo(s.doc).unwrap().snapshot.body.source(),
        s.body.source()
    );
}

#[test]
fn measured_regular_replacement_commits_one_root_and_sibling_undo_recovers_all_bytes() {
    // Candidate measurement may explicitly select 10/100 MiB and a denser
    // interval. Every run checks the entire candidate without a second String.
    let mib = match std::env::var("TEXZI_BULK_PROBE_MIB").as_deref() {
        Ok("10") => 10,
        Ok("100") => 100,
        _ => 1,
    };
    let stride = match std::env::var("TEXZI_BULK_PROBE_STRIDE").as_deref() {
        Ok("1") => 1,
        Ok("64") => 64,
        _ => 16,
    };
    let size = mib * 1024 * 1024;
    let mut st = Store::new(HistoryBudget::default());
    let s = st
        .open_utf8(&"a".repeat(size), Profile::default(), false)
        .unwrap();
    let a = open(&mut st, &s, 0, Affinity::Before);
    let b = open(&mut st, &s, size, Affinity::After);
    let start = std::time::Instant::now();
    let mut plan = builder(&s);
    for at in (0..size).step_by(stride) {
        plan.replace(at..at + 1, [Literal("b")]).unwrap();
    }
    let ready = plan.finish().unwrap();
    let prepared = start.elapsed();
    let start = std::time::Instant::now();
    let wrote = st.submit_bulk(a.id, a.epoch, ready).unwrap();
    let committed = start.elapsed();
    assert_eq!(wrote.bulk.as_ref().unwrap().run_count(), 1);
    let mut at = 0;
    for byte in wrote.snapshot.body.source().bytes() {
        assert_eq!(byte, if at % stride == 0 { b'b' } else { b'a' });
        at += 1;
    }
    assert_eq!(at, size);
    let start = std::time::Instant::now();
    let undo = st.undo_view(b.id, st.view(b.id).unwrap().epoch).unwrap();
    let undone = start.elapsed();
    assert_eq!(undo.snapshot.token, s.token);
    assert!(Arc::ptr_eq(&undo.snapshot.body, &s.body));
    assert_eq!(
        st.view(b.id).unwrap().selection.selections[0].range(),
        size..size
    );
    let (backing, peak) = bulk_history_disk_usage();
    println!(
        "bulk_probe bytes={size} stride={stride} hits={} prepare_ms={:.3} commit_ms={:.3} undo_ms={:.3} history_charge={} disk_current={backing} disk_peak={peak}",
        size / stride,
        prepared.as_secs_f64() * 1000.0,
        committed.as_secs_f64() * 1000.0,
        undone.as_secs_f64() * 1000.0,
        st.history_charge()
    );
}
#[test]
fn four_mib_equal_rewrite_cold_undo_and_redo_recover_bytes_and_provenance() {
    let mut st = Store::new(HistoryBudget::default());
    st.set_observer_limits(ObserverLimits {
        journal_batches: 0,
        ..Default::default()
    })
    .unwrap();
    let n = 4 * 1024 * 1024;
    let s = st
        .open_utf8(&"A".repeat(n), Profile::default(), false)
        .unwrap();
    let old = s.token;
    let v = open(&mut st, &s, n, Affinity::After);
    let sibling = open(&mut st, &s, 0, Affinity::Before);
    let mut p = builder(&s);
    p.replace(0..n, [Literal(&"B".repeat(n))]).unwrap();
    let c = st.submit_bulk(v.id, v.epoch, p.finish().unwrap()).unwrap();
    let next = c.snapshot.token;
    assert_eq!(c.snapshot.body.source().byte(n - 1), b'B');
    assert!(st.history_charge() < HistoryBudget::default().document);
    drop(s);
    drop(c);
    let b = st.view(sibling.id).unwrap();
    let HistoryStep::Cold(job) = st.take_history_step(b.id, b.epoch, false).unwrap() else {
        panic!("old root is on disk")
    };
    let loaded = std::thread::spawn(move || job.load().unwrap())
        .join()
        .unwrap();
    let undo = st.submit_loaded_history(loaded).unwrap();
    assert_eq!(undo.snapshot.token, old);
    assert_eq!(undo.snapshot.body.to_utf8(), vec![b'A'; n]);
    let original = undo.snapshot.body.origins();
    assert_eq!(original[0].1.as_ref().unwrap().raw.start, 0);
    assert_eq!(original.last().unwrap().1.as_ref().unwrap().raw.end, n);
    drop(undo);
    let b = st.view(sibling.id).unwrap();
    let HistoryStep::Cold(job) = st.take_history_step(b.id, b.epoch, true).unwrap() else {
        panic!("new root is on disk")
    };
    let loaded = job.load().unwrap();
    let s = st.snapshot(b.doc).unwrap();
    st.set_selection(b.id, b.epoch, SelectionSet::caret(&s, 3))
        .unwrap();
    assert!(matches!(
        st.submit_loaded_history(loaded),
        Err(Error::StaleView)
    ));
    let b = st.view(sibling.id).unwrap();
    let r = st.redo_view(b.id, b.epoch).unwrap();
    assert_eq!(r.snapshot.token, next);
    assert_eq!(r.snapshot.body.to_utf8(), vec![b'B'; n]);
}
