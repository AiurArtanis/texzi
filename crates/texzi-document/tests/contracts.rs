use texzi_document::*;
mod binding_epochs;
mod file_codecs;
mod file_staging;
mod native_plans;
mod observers;
mod save_failure_ack;
mod save_failures;
mod save_scopes;
mod views;
#[cfg(windows)]
mod windows_bound_staging;
#[cfg(windows)]
mod windows_identity;

fn store(raw: &str) -> (Store, Snapshot) {
    let mut store = Store::new(HistoryBudget::default());
    let s = store
        .open_utf8(
            raw,
            Profile {
                default_eol: Eol::Lf,
                ..Profile::default()
            },
            false,
        )
        .unwrap();
    (store, s)
}
fn submit(store: &mut Store, s: &Snapshot, r: std::ops::Range<usize>, text: &str) -> Receipt {
    store
        .submit(Transaction {
            doc: s.doc,
            base: s.revision,
            edits: vec![Edit::input(s, r, text)],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
}
fn complete(store: &mut Store, c: &SaveCapture) {
    store.advance_save(c, SaveStage::WritingTemp).unwrap();
    store.advance_save(c, SaveStage::ReadyToReplace).unwrap();
    store.commit_permit(c).unwrap();
    assert_eq!(store.cancel_save(c), Err(Error::TooLate));
    store.advance_save(c, SaveStage::Verifying).unwrap();
    store.accept_verified_save(c).unwrap();
}

#[test]
fn frozen_unicode_coordinates() {
    assert_eq!(unicode_segmentation::UNICODE_VERSION, (17, 0, 0));
    let (_, s) = store(include_str!("fixtures/unicode.txt").trim_end_matches('\n'));
    let i = GraphemeIndex::new(&s);
    assert_eq!(
        (s.body.len_bytes(), s.body.len_utf16(), i.len()),
        (32, 17, 6)
    );
    for (g, b, u) in [
        (0, 0, 0),
        (1, 1, 1),
        (2, 5, 3),
        (3, 8, 5),
        (4, 23, 12),
        (5, 31, 16),
        (6, 32, 17),
    ] {
        let p = SourceByte::at(&s, b);
        assert_eq!(s.byte_to_utf16(p).unwrap().offset, u);
        assert_eq!(s.utf16_to_byte(Utf16Offset::at(&s, u)).unwrap(), p);
        assert_eq!(i.to_byte(GraphemeOffset::at(&s, g)).unwrap(), p);
        assert_eq!(i.from_byte(p).unwrap().offset, g);
    }
    assert_eq!(
        s.byte_to_utf16(SourceByte::at(&s, 2)),
        Err(Error::InvalidBoundary)
    );
    assert_eq!(
        s.utf16_to_byte(Utf16Offset::at(&s, 2)),
        Err(Error::InvalidBoundary)
    );
    assert_eq!(
        i.from_byte(SourceByte::at(&s, 6)),
        Err(Error::InsideGrapheme)
    );
    let (_, foreign) = store("abc");
    assert_eq!(
        s.byte_to_utf16(SourceByte::at(&foreign, 0)),
        Err(Error::InvalidDoc)
    );
}

#[test]
fn chunk_seams_and_unbounded_grapheme_context() {
    use unicode_segmentation::UnicodeSegmentation;
    for raw in [
        format!("{}{}X", "a".repeat(1023), "\u{301}".repeat(2500)),
        format!("a{}Z", "🇨".repeat(2001)),
        format!("{}👩🏽‍💻{}", "x".repeat(1022), "\u{301}".repeat(1200)),
        "क्\u{200d}ष".repeat(800),
    ] {
        let (_, s) = store(&raw);
        let index = GraphemeIndex::new(&s);
        let boundaries: Vec<_> = raw
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(raw.len()))
            .collect();
        assert_eq!(index.len() + 1, boundaries.len());
        for (g, &b) in boundaries.iter().enumerate() {
            assert_eq!(index.to_byte(GraphemeOffset::at(&s, g)).unwrap().offset, b);
        }
        for b in raw
            .char_indices()
            .map(|(b, _)| b)
            .chain(std::iter::once(raw.len()))
        {
            assert_eq!(
                s.is_grapheme_boundary(SourceByte::at(&s, b)).unwrap(),
                boundaries.binary_search(&b).is_ok(),
                "byte {b}"
            );
        }
    }
}

#[test]
fn eol_provenance_snapshot_and_undo() {
    let raw = "\u{feff}one\r\ntwo\nthree\rfour";
    let (mut st, s) = store(raw);
    assert_eq!(s.body.to_utf8(), raw.as_bytes());
    assert_eq!(s.body.dominant_eol(), Eol::CrLf);
    let r = submit(&mut st, &s, 5..6, "中\r\n");
    let n = r.snapshot;
    assert_eq!(s.body.to_utf8(), raw.as_bytes());
    assert_eq!(n.body.source().to_string(), "one\nt中\no\nthree\nfour");
    assert_eq!(n.body.eols(), vec![Eol::CrLf, Eol::CrLf, Eol::Lf, Eol::Cr]);
    for (logical, origin) in n.body.origins() {
        if let Some(o) = origin {
            let b = Body::import_utf8(&o.source[o.raw.clone()], Profile::default());
            assert_eq!(
                b.source().to_string(),
                n.body.source().byte_slice(logical).to_string()
            );
        }
    }
    let u = st.undo(s.doc).unwrap();
    assert_eq!(u.snapshot.body.to_utf8(), raw.as_bytes());
    assert_eq!(u.snapshot.token, s.token);
    assert_eq!(
        u.snapshot
            .body
            .origins()
            .iter()
            .filter(|(_, o)| o.is_none())
            .count(),
        0
    );
    assert!(u.snapshot.revision.0 > n.revision.0);
}

#[test]
fn invalid_transaction_has_no_partial_effects() {
    let (mut st, s) = store("a😀e\u{301}b\n");
    let good = Edit::input(&s, 0..1, "OK");
    let bad = Edit::input(&s, 2..3, "BAD");
    for (edits, policy, expected) in [
        (
            vec![good.clone(), bad],
            BoundaryPolicy::Scalar,
            Error::InvalidBoundary,
        ),
        (
            vec![good.clone(), Edit::input(&s, 0..1, "x")],
            BoundaryPolicy::Grapheme,
            Error::InvalidOverlap,
        ),
        (
            vec![Edit::input(&s, 6..8, "")],
            BoundaryPolicy::Grapheme,
            Error::InsideGrapheme,
        ),
    ] {
        assert_eq!(
            st.submit(Transaction {
                doc: s.doc,
                base: s.revision,
                edits,
                boundary: policy,
                cancelled: false
            })
            .unwrap_err(),
            expected
        );
        assert_eq!(st.snapshot(s.doc).unwrap().token, s.token);
        assert_eq!(st.history_depth(s.doc).unwrap(), (0, 0));
        assert!(!st.dirty(s.doc).unwrap());
    }
    let mut bad_eol = good.clone();
    bad_eol.eols = vec![Eol::Cr];
    assert_eq!(
        st.submit(Transaction {
            doc: s.doc,
            base: s.revision,
            edits: vec![good, bad_eol],
            boundary: BoundaryPolicy::Scalar,
            cancelled: true
        })
        .unwrap_err(),
        Error::Cancelled
    );
    let mut bad_eol = Edit::input(&s, 0..1, "x");
    bad_eol.eols = vec![Eol::Cr];
    assert_eq!(
        st.submit(Transaction {
            doc: s.doc,
            base: s.revision,
            edits: vec![bad_eol],
            boundary: BoundaryPolicy::Scalar,
            cancelled: false
        })
        .unwrap_err(),
        Error::InvalidEolPlan
    );
    assert_eq!(st.snapshot(s.doc).unwrap().body.to_utf8(), s.body.to_utf8());
}

#[test]
fn multi_edit_forward_inverse_and_independent_anchors() {
    let (mut st, s) = store("abc\ndef\nghi");
    let receipt = st
        .submit(Transaction {
            doc: s.doc,
            base: s.revision,
            edits: vec![Edit::input(&s, 1..2, "长\n"), Edit::input(&s, 8..11, "X")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap();
    assert_eq!(receipt.snapshot.body.source().to_string(), "a长\nc\ndef\nX");
    assert_eq!(
        receipt
            .map_anchor(SourceByte::at(&s, 1), Affinity::Before)
            .unwrap()
            .offset,
        1
    );
    assert_eq!(
        receipt
            .map_anchor(SourceByte::at(&s, 1), Affinity::After)
            .unwrap()
            .offset,
        5
    );
    assert_eq!(
        receipt
            .map_anchor(SourceByte::at(&s, 7), Affinity::After)
            .unwrap()
            .offset,
        10
    );
    let undo = st.undo(s.doc).unwrap();
    assert_eq!(undo.snapshot.body.to_utf8(), s.body.to_utf8());
    assert_eq!(
        undo.forward
            .iter()
            .map(|d| d.range.clone())
            .collect::<Vec<_>>(),
        vec![1..5, 11..12]
    );
    assert_eq!(
        st.redo(s.doc).unwrap().snapshot.body.to_utf8(),
        receipt.snapshot.body.to_utf8()
    );
}

#[test]
fn adjacent_patch_endpoints_respect_half_open_ranges() {
    let (mut st, s) = store("abcdef");
    let r = st
        .submit(Transaction {
            doc: s.doc,
            base: s.revision,
            edits: vec![Edit::input(&s, 1..3, "X"), Edit::input(&s, 3..3, "YZ")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap();
    assert_eq!(r.snapshot.body.source().to_string(), "aXYZdef");
    let p = SourceByte::at(&s, 3);
    assert_eq!(r.map_anchor(p, Affinity::Before).unwrap().offset, 2);
    assert_eq!(r.map_anchor(p, Affinity::After).unwrap().offset, 4);
    assert_eq!(
        r.map_anchor(SourceByte::at(&s, 6), Affinity::Before)
            .unwrap()
            .offset,
        7
    );
    assert_eq!(
        st.undo(s.doc).unwrap().snapshot.body.to_utf8(),
        s.body.to_utf8()
    );
}

#[test]
fn saved_token_during_edit_and_undo_aba() {
    let (mut st, s) = store("abc");
    assert!(!st.backed_by_disk(s.doc).unwrap());
    let n = submit(&mut st, &s, 3..3, "!").snapshot;
    let save = st
        .begin_save(
            s.doc,
            TargetKey("generated-fixture-key".into()),
            "fingerprint1".into(),
        )
        .unwrap();
    let n1 = submit(&mut st, &n, 4..4, "?").snapshot;
    complete(&mut st, &save);
    assert!(st.dirty(s.doc).unwrap());
    assert_eq!(st.snapshot(s.doc).unwrap().token, n1.token);
    assert_eq!(st.accept_verified_save(&save), Err(Error::InvalidSave));
    let u = st.undo(s.doc).unwrap().snapshot;
    assert_eq!(u.token, n.token);
    assert!(!st.dirty(s.doc).unwrap());
    assert_eq!(
        u.byte_to_utf16(SourceByte::at(&n, 0)),
        Err(Error::StaleRevision)
    );
    assert!(st.redo(s.doc).is_ok());
    assert!(st.dirty(s.doc).unwrap());
}

#[test]
fn save_leases_cancellation_and_forged_receipts() {
    let (mut st, s) = store("abc");
    let other = st.open_utf8("xyz", Profile::default(), false).unwrap();
    let c = st
        .begin_save(s.doc, TargetKey("same-object".into()), "guard".into())
        .unwrap();
    assert_eq!(
        st.begin_save(other.doc, c.target.clone(), "guard".into())
            .unwrap_err(),
        Error::Busy
    );
    let mut forged = c.clone();
    forged.guard = "wrong".into();
    assert_eq!(st.cancel_save(&forged), Err(Error::InvalidSave));
    st.cancel_save(&c).unwrap();
    assert_eq!(st.commit_permit(&c), Err(Error::InvalidSave));
    assert!(!st.backed_by_disk(s.doc).unwrap());
    let other = st
        .begin_save(other.doc, c.target.clone(), "guard".into())
        .unwrap();
    complete(&mut st, &other);
}

#[test]
fn metadata_noop_budget_and_redo_branch() {
    let (mut st, s) = store("x\r\ny\n");
    let mut profile = s.body.profile;
    profile.utf8_bom = true;
    let n = st
        .set_serialization(s.doc, s.revision, profile, Some(Eol::Cr))
        .unwrap();
    assert!(n.changed);
    assert_eq!(n.snapshot.body.to_utf8(), b"\xef\xbb\xbfx\ry\r");
    let u = st.undo(s.doc).unwrap().snapshot;
    assert_eq!(u.body.to_utf8(), s.body.to_utf8());
    let n = submit(&mut st, &u, 0..1, "z").snapshot;
    assert_eq!(st.redo(s.doc).unwrap_err(), Error::NoHistory);
    assert!(!submit(&mut st, &n, 0..1, "z").changed);
    let mut bounded = Store::new(HistoryBudget {
        document: 1,
        process: 1,
    });
    let s = bounded.open_utf8("abc", Profile::default(), false).unwrap();
    assert_eq!(
        bounded
            .submit(Transaction {
                doc: s.doc,
                base: s.revision,
                edits: vec![Edit::input(&s, 0..1, "X")],
                boundary: BoundaryPolicy::Grapheme,
                cancelled: false
            })
            .unwrap_err(),
        Error::HistoryBudget
    );
    assert_eq!(bounded.snapshot(s.doc).unwrap().token, s.token);
    assert_eq!(bounded.history_charge(), 0);
}

#[test]
fn inserted_bom_is_text_and_scalar_patches_are_explicit() {
    let (mut st, s) = store("e\u{301}");
    let n = submit(&mut st, &s, 0..0, "\u{feff}").snapshot;
    assert_eq!(n.body.source().to_string(), "\u{feff}e\u{301}");
    assert!(!n.body.profile.utf8_bom);
    let r = st
        .submit(Transaction {
            doc: n.doc,
            base: n.revision,
            edits: vec![Edit::input(&n, 4..6, "")],
            boundary: BoundaryPolicy::Scalar,
            cancelled: false,
        })
        .unwrap();
    assert_eq!(r.snapshot.body.source().to_string(), "\u{feff}e");
}

fn random_edit_property(seed: u64, count: usize) {
    use unicode_segmentation::UnicodeSegmentation;
    let (mut st, initial) = store("α😀e\u{301}\nEND");
    let mut s = initial.clone();
    let mut expected = s.body.source().to_string();
    let mut rng = seed;
    let mut sentinel = SourceByte::at(&s, expected.len() - 3);
    let mut batch = 0;
    for step in 0..count {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let boundaries: Vec<_> = expected[..expected.len() - 3]
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(expected.len() - 3))
            .collect();
        let a = (rng as usize) % boundaries.len();
        let b = a + ((rng >> 32) as usize) % (boundaries.len() - a);
        let mut text = ["中", "👩🏽‍💻", "e\u{301}", "\n", "", "🇨🇳", "a"][(rng as usize) % 7];
        if &expected[boundaries[a]..boundaries[b]] == text {
            text = "z"; // Every counted step must commit an actual content change.
        }
        let r = submit(&mut st, &s, boundaries[a]..boundaries[b], text);
        expected.replace_range(boundaries[a]..boundaries[b], text);
        sentinel = r.map_anchor(sentinel, Affinity::After).unwrap();
        s = r.snapshot;
        assert_eq!(
            s.body.source().to_string(),
            expected,
            "seed {seed} step {step}"
        );
        assert_eq!(s.body.to_utf8(), expected.as_bytes());
        assert_eq!(
            sentinel.offset,
            expected.len() - 3,
            "sentinel drift seed {seed} step {step}"
        );
        assert_eq!(
            s.byte_to_utf16(sentinel).unwrap().offset,
            expected[..sentinel.offset].encode_utf16().count()
        );
        if r.changed {
            batch += 1;
        }
        if step % 32 == 31 || step + 1 == count {
            let n = st.history_depth(s.doc).unwrap().0;
            assert_eq!(n, batch);
            for _ in 0..n {
                st.undo(s.doc).unwrap();
            }
            s = st.snapshot(s.doc).unwrap();
            assert_eq!(s.body.to_utf8(), initial.body.to_utf8());
            assert_eq!(s.token, initial.token);
            assert!(!st.dirty(s.doc).unwrap());
            for _ in 0..n {
                st.redo(s.doc).unwrap();
            }
            assert_eq!(
                st.snapshot(s.doc).unwrap().body.to_utf8(),
                expected.as_bytes()
            );
            for _ in 0..n {
                st.undo(s.doc).unwrap();
            }
            s = st.snapshot(s.doc).unwrap();
            expected = s.body.source().to_string();
            sentinel = SourceByte::at(&s, expected.len() - 3);
            batch = 0;
        }
    }
    println!(
        "property seed={seed} edits={count} final_revision={} history_charge={} sentinel={}",
        s.revision.0,
        st.history_charge(),
        sentinel.offset
    );
}
#[test]
fn randomized_edits_undo_redo() {
    for seed in [1, 0x20261116, 0xdeadbeef] {
        random_edit_property(seed, 4096);
    }
}

#[test]
#[ignore = "explicit million-edit component run, not UI latency/performance acceptance"]
fn million_edit_position_stability() {
    random_edit_property(0x20261116, 1_000_000);
}

#[test]
fn large_body_snapshot_shares_untouched_chunks() {
    let raw = "abcd中😀\n".repeat(800_000);
    let (mut st, s) = store(&raw);
    let old_ptr = s.body.source().chunk_at_byte(0).0.as_ptr();
    let end = s.body.len_bytes();
    let n = submit(&mut st, &s, end..end, "!").snapshot;
    assert_eq!(n.body.source().chunk_at_byte(0).0.as_ptr(), old_ptr);
    assert_eq!(s.body.len_bytes(), raw.len());
    assert_eq!(n.body.len_bytes(), raw.len() + 1);
    assert_eq!(
        s.body.source().byte_slice(0..12).to_string(),
        n.body.source().byte_slice(0..12).to_string()
    );
    println!(
        "shared unchanged chunk at body_bytes={} history_charge={}",
        raw.len(),
        st.history_charge()
    );
}
