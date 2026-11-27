use std::ops::{ControlFlow, Range};
use texzi_search::{
    Cancelled, Collector, Hit, Literal, MAX_CHECKPOINTS, MAX_SELECTIONS, MAX_VISIBLE, Navigation,
    Options, Rope, ScanEvent, Scope, SearchResults, is_word,
};

const EXACT: Options = Options {
    case_sensitive: true,
    whole_word: false,
};

fn scan(
    text: &Rope,
    literal: &Literal,
    scope: &Scope,
    nav: Navigation,
    viewport: Range<usize>,
    select_cap: Option<usize>,
) -> SearchResults {
    let mut out = Collector::new(text.len_bytes(), nav, viewport, select_cap);
    let iteration = literal.scan_events(
        text,
        0,
        scope,
        || false,
        |event| {
            match event {
                ScanEvent::Match(range) => {
                    let resume = range.end;
                    out.record(range, Some(resume));
                }
                ScanEvent::Progress { resume, .. } => out.progress(resume),
            }
            ControlFlow::<()>::Continue(())
        },
    );
    assert_eq!(iteration, Ok(ControlFlow::Continue(())));
    out.finish()
}

fn hits(source: &str, query: &str, options: Options, scope: &Scope) -> Vec<Range<usize>> {
    let text = Rope::from_str(source);
    let lit = Literal::new(query, options).unwrap();
    let result = scan(
        &text,
        &lit,
        scope,
        Navigation::NextFrom(0),
        0..0,
        Some(MAX_SELECTIONS),
    );
    result.selected.unwrap().unwrap()
}

#[test]
fn unicode_simple_folding_preserves_each_source_byte_range() {
    let text = "K k K S ſ s Σ ς σ I i ı İ ß ẞ SS e é e\u{301} ｋ 🙂";
    for (query, expected) in [
        ("K", vec!["K", "k", "K"]),
        ("ſ", vec!["S", "ſ", "s", "S", "S"]),
        ("ς", vec!["Σ", "ς", "σ"]),
        ("i", vec!["I", "i"]),
        ("ß", vec!["ß", "ẞ"]),
        ("e", vec!["e", "e"]),
        ("é", vec!["é"]),
        ("🙂", vec!["🙂"]),
    ] {
        let found = hits(text, query, Options::default(), &Scope::Document);
        assert_eq!(
            found.into_iter().map(|r| &text[r]).collect::<Vec<_>>(),
            expected,
            "{query}"
        );
    }
    let tje = "\u{1c89}\u{1c8a}";
    assert_eq!(
        hits(tje, "\u{1c8a}", Options::default(), &Scope::Document),
        [0..3, 3..6]
    );
    assert!(is_word('\u{1c89}')); // Cyrillic letter added in Unicode 16.0
    assert!(!is_word('\u{16ea0}')); // still unassigned in this pinned version
    assert!(Literal::new("", EXACT).is_none());
    assert_eq!(
        hits("a\nb\nc", "a\r\nb\rc", EXACT, &Scope::Document),
        [0..5]
    );
}

#[test]
fn f01_documented_partial_and_whole_word_counts() {
    let text = "alpha alphabet Alpha alpha_1\n猫咪 猫咪们 猫\n.* [x]";
    let default = Options::default();
    let word = Options {
        whole_word: true,
        ..default
    };
    assert_eq!(hits(text, "alpha", default, &Scope::Document).len(), 4);
    assert_eq!(hits(text, "alpha", EXACT, &Scope::Document).len(), 3);
    assert_eq!(hits(text, "alpha", word, &Scope::Document).len(), 2);
    assert_eq!(hits(text, "猫", default, &Scope::Document).len(), 3);
    assert_eq!(hits(text, "猫", word, &Scope::Document).len(), 1);
    let punctuation = text.find(".*").unwrap();
    assert_eq!(
        hits(text, ".*", default, &Scope::Document),
        [punctuation..punctuation + 2]
    );
}

#[test]
fn whole_word_uses_original_neighbors_and_unicode_categories() {
    let word = Options {
        case_sensitive: false,
        whole_word: true,
    };
    let text = "Alpha alphabet ALPHA2 _alpha alpha αalpha alpha\u{203f}x alpha\u{301} 中猫 猫1 猫";
    let words = hits(text, "alpha", word, &Scope::Document);
    assert_eq!(
        words.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(),
        ["Alpha", "alpha"]
    );
    let cat_before_digit = text.find("猫1").unwrap();
    assert_eq!(
        hits(text, "猫", word, &Scope::Document),
        [
            cat_before_digit..cat_before_digit + 3,
            text.len() - 3..text.len()
        ]
    );
    let in_word = text.find("alphabet").unwrap();
    assert!(
        hits(
            text,
            "alpha",
            word,
            &Scope::selections([in_word..in_word + 5])
        )
        .is_empty()
    );
    assert_eq!(hits("xa a a", "a a", word, &Scope::Document), [3..6]);
    assert!(hits("🙂a", "🙂", word, &Scope::Document).is_empty());
    assert_eq!(hits("e\u{301}", "e", EXACT, &Scope::Document), [0..1]);
    assert!(hits("e\u{301}", "e", word, &Scope::Document).is_empty());
    assert!(is_word('\u{203f}'));
    assert!(is_word('\u{301}'));
    assert!(is_word('Ⅶ'));
    assert!(!is_word('\u{200d}'));
    let long_word = "甲乙丙".repeat(400);
    let rope = Rope::from_str(&long_word);
    let first_chunk = rope.chunks().next().unwrap();
    assert!(first_chunk.len() < rope.len_bytes());
    assert!(hits(&long_word, first_chunk, word, &Scope::Document).is_empty());
}

#[test]
fn whole_word_uses_the_editors_grapheme_and_script_runs_across_rope_leaves() {
    let whole = Options {
        case_sensitive: true,
        whole_word: true,
    };
    let text = "猫alpha猫咪カ한a‿b1e\u{301}";
    for (word, bytes) in [
        ("猫", 0..3),
        ("alpha", 3..8),
        ("猫咪カ한", 8..20),
        ("a‿b1e\u{301}", 20..29),
    ] {
        assert_eq!(hits(text, word, whole, &Scope::Document), [bytes], "{word}");
    }
    for part in ["猫咪", "カ", "한", "b1", "e", "\u{301}"] {
        assert!(
            hits(text, part, whole, &Scope::Document).is_empty(),
            "{part}"
        );
    }
    // The mark belongs to the Han grapheme on its left, including at a
    // replay checkpoint after a cluster larger than one Rope leaf.
    let cluster = format!("猫{}", "\u{301}".repeat(3000));
    let subject = format!("{cluster}alpha猫");
    let rope = Rope::from_str(&subject);
    let word = Literal::new("alpha", whole).unwrap();
    let mut found = Vec::new();
    let _ = word
        .scan(
            &rope,
            cluster.len(),
            &Scope::Document,
            || false,
            |r| {
                found.push(r);
                ControlFlow::<()>::Continue(())
            },
        )
        .unwrap();
    assert_eq!(found, [cluster.len()..cluster.len() + 5]);
    assert!(hits(&subject, "猫\u{301}", whole, &Scope::Document).is_empty());
    let mut polls = 0;
    let stopped = word.scan(
        &rope,
        cluster.len(),
        &Scope::Document,
        || {
            polls += 1;
            polls == 4
        },
        |_| ControlFlow::<()>::Continue(()),
    );
    assert_eq!(stopped, Err(Cancelled));
}

#[test]
fn scope_merges_contact_reversal_and_never_restarts_global_enumeration_at_selection_edges() {
    let scope = Scope::selections([
        12..16,
        Range { start: 8, end: 2 },
        1..1,
        6..14,
        16..18,
        30..32,
    ]);
    assert_eq!(scope.ranges().unwrap(), &[2..18, 30..32]);
    assert!(scope.contains(&(2..18)));
    assert!(scope.contains(&(18..18)));
    assert!(!scope.contains(&(17..31)));
    assert!(!Scope::selections([3..3]).contains(&(3..3)));
    assert!(hits("aaaaa", "aa", EXACT, &Scope::selections([1..3])).is_empty());
    let text = "a".repeat(10_001);
    assert_eq!(
        hits(&text, "a", EXACT, &Scope::selections([10_000..10_001])),
        [10_000..10_001]
    );
}

#[test]
fn long_literal_crosses_rope_chunks_and_retains_raw_length_after_folding() {
    let text = format!("{}光ÉK{}", "泡".repeat(720), "🙂".repeat(720));
    let rope = Rope::from_str(&text);
    assert!(rope.chunks().count() >= 4);
    for (middle, options) in [("光ÉK", EXACT), ("光ék", Options::default())] {
        let query = format!("{}{middle}{}", "泡".repeat(370), "🙂".repeat(330));
        let found = hits(&text, &query, options, &Scope::Document);
        assert_eq!(found, [1050..2160 + "光ÉK".len() + 330 * 4]);
    }
    let code = "🧑‍🔬 🧑‍🔬";
    let found = hits(code, "\u{200d}", EXACT, &Scope::Document);
    assert_eq!(found, [4..7, 16..19]);
}

#[test]
fn cancellation_reaches_absent_dense_and_partial_prefix_scans() {
    let text = Rope::from_str(&"a".repeat(128 * 1024));
    for options in [EXACT, Options::default()] {
        for needle in ["absent", "a", "aaaaaab"] {
            let lit = Literal::new(needle, options).unwrap();
            let mut polls = 0;
            let mut count = 0;
            let result = lit.scan(
                &text,
                0,
                &Scope::Document,
                || {
                    polls += 1;
                    polls == 4
                },
                |_| {
                    count += 1;
                    ControlFlow::<()>::Continue(())
                },
            );
            assert_eq!(result, Err(Cancelled));
            assert_eq!(polls, 4);
            assert!(count < 4096);
        }
    }
    let lit = Literal::new("a", EXACT).unwrap();
    assert_eq!(
        lit.scan(
            &text,
            0,
            &Scope::Document,
            || false,
            |r| ControlFlow::Break(r)
        ),
        Ok(ControlFlow::Break(0..1))
    );
    assert!(Literal::try_new(&"a".repeat(50_000), EXACT, || true).is_err());
}

#[test]
fn checkpoint_navigation_and_visible_replay_use_global_ordinals_in_dense_text() {
    let text = Rope::from_str(&"x".repeat(300_000));
    let lit = Literal::new("x", EXACT).unwrap();
    let scope = Scope::selections([100..300_000]);
    let results = scan(
        &text,
        &lit,
        &scope,
        Navigation::NextFrom(280_000),
        0..300_000,
        Some(3),
    );
    assert_eq!(results.total, 299_900);
    assert_eq!(
        results.current,
        Some(Hit {
            ordinal: 279_901,
            range: 280_000..280_001
        })
    );
    assert!(results.visible.len() <= MAX_VISIBLE);
    assert!(results.visible_sampled);
    assert!(results.visible.last().unwrap().start > 290_000);
    assert_eq!(
        results
            .selected
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap_err()
            .limit,
        3
    );
    assert!((3..=MAX_CHECKPOINTS).contains(&results.index.points().len()));
    assert!(results.index.before_byte(280_000).byte > 200_000);

    for (nav, expected, wrapped) in [
        (Navigation::PreviousFrom(280_000), 279_900, false),
        (Navigation::NextFrom(280_000), 279_901, false),
        (Navigation::PreviousBefore(279_901), 279_900, false),
        (Navigation::NextAfter(279_901), 279_902, false),
        (Navigation::PreviousBefore(1), 299_900, true),
        (Navigation::NextAfter(299_900), 1, true),
        (Navigation::PreviousFrom(0), 299_900, true),
        (Navigation::NextFrom(300_000), 1, true),
    ] {
        let mut replay = results.navigation(nav);
        if !replay.done() {
            assert!(replay.start() > 200_000);
            let _ = lit
                .scan(
                    &text,
                    replay.start(),
                    &scope,
                    || false,
                    |m| replay.record(m),
                )
                .unwrap();
        }
        let (hit, did_wrap) = replay.finish();
        assert_eq!((hit.unwrap().ordinal, did_wrap), (expected, wrapped));
    }
    let mut window = results.viewport(280_000..280_003);
    assert!(window.start() > 200_000);
    let end = lit.scan_events(
        &text,
        window.start(),
        &scope,
        || false,
        |event| match event {
            ScanEvent::Match(m) => window.record(m),
            ScanEvent::Progress { resume, .. } => window.progress(resume),
        },
    );
    assert_eq!(end, Ok(ControlFlow::Break(())));
    assert_eq!(
        window.finish(),
        [280_000..280_001, 280_001..280_002, 280_002..280_003]
    );
}

#[test]
fn sparse_checkpoints_replay_a_long_crossing_match_and_survive_unmatched_gaps() {
    let needle = format!("{}z", "A".repeat(100_000));
    let raw = format!("{}{needle}{}", "x".repeat(100_000), "x".repeat(200_000));
    let text = Rope::from_str(&raw);
    for options in [EXACT, Options::default()] {
        let lit = Literal::new(&needle, options).unwrap();
        let out = scan(
            &text,
            &lit,
            &Scope::Document,
            Navigation::NextFrom(0),
            0..1,
            None,
        );
        assert_eq!(out.total, 1);
        assert!(out.selected.is_none());
        let mut window = out.viewport(180_000..181_000);
        assert!(window.start() <= 100_000);
        let _ = lit
            .scan_events(
                &text,
                window.start(),
                &Scope::Document,
                || false,
                |event| match event {
                    ScanEvent::Match(m) => window.record(m),
                    ScanEvent::Progress { resume, .. } => window.progress(resume),
                },
            )
            .unwrap();
        assert_eq!(window.finish(), [100_000..200_001]);
        assert!(out.index.before_byte(390_000).byte > 300_000);
    }
}

#[test]
fn empty_matches_at_a_scalar_and_eof_keep_independent_navigation() {
    let entries = [(0..0, Some(4)), (4..4, Some(5)), (5..5, None)];
    let collect = |nav, band| {
        let mut c = Collector::new(5, nav, band, Some(3));
        for (range, next) in &entries {
            assert_eq!(c.record(range.clone(), *next), range.start == 0);
        }
        c.finish()
    };
    let all = collect(Navigation::NextFrom(0), 0..5);
    assert_eq!(all.visible, [0..0, 4..4, 5..5]);
    assert_eq!(all.selected.unwrap().unwrap().len(), 3);
    for (nav, ordinal, wrap) in [
        (Navigation::NextAfter(1), 2, false),
        (Navigation::NextAfter(3), 1, true),
        (Navigation::PreviousFrom(4), 2, false),
        (Navigation::PreviousBefore(2), 1, false),
        (Navigation::PreviousBefore(1), 3, true),
    ] {
        let s = collect(nav, 5..5);
        assert_eq!(s.visible, [5..5]);
        assert_eq!(s.current.as_ref().unwrap().ordinal, ordinal);
        assert_eq!(s.wrapped, wrap);
        let mut nav = s.navigation(nav);
        for (range, _) in &entries {
            if nav.done() || nav.record(range.clone()).is_break() {
                break;
            }
        }
        assert_eq!(nav.finish(), (s.current, wrap));
    }
}
