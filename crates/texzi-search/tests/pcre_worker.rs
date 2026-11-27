//! Harness-free so each search starts this same real executable in worker mode.
#![cfg_attr(windows, windows_subsystem = "windows")]
use std::{
    env,
    ops::{ControlFlow, Range},
    process::{self, Command},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use texzi_search::{
    Literal, Navigation, Options, Rope, Scope, SearchResults,
    pcre::{self, Budget, Piece, RegexError, RegexQuery, RegexSubject, VisitError},
};

const CASE: &str = "TEXZI_PCRE_WORKER_FIXTURE";
static LIVE: AtomicBool = AtomicBool::new(false);

fn frozen_versions() {
    use pcre2_sys::{PCRE2_CONFIG_UNICODE_VERSION, PCRE2_CONFIG_VERSION, pcre2_config_8};
    let value = |which| {
        let mut buffer = [0u8; 128];
        let count = unsafe { pcre2_config_8(which, buffer.as_mut_ptr().cast()) };
        assert!(count > 0 && count as usize <= buffer.len());
        std::str::from_utf8(&buffer[..count as usize - 1])
            .unwrap()
            .to_string()
    };
    assert!(value(PCRE2_CONFIG_VERSION).starts_with("10.46 "));
    assert_eq!(value(PCRE2_CONFIG_UNICODE_VERSION), "16.0.0");
}

fn casefold_parity() {
    let rope = Rope::from_str("K k K S ſ s Σ ς σ I i ı İ ß ẞ SS \u{1c89}\u{1c8a}");
    let subject = RegexSubject::stage(&rope, &Budget::new(&LIVE)).unwrap();
    for query in ["k", "s", "I", "ß", "ς", "\u{1c89}"] {
        let options = Options {
            case_sensitive: false,
            whole_word: false,
        };
        let mut expected = Vec::new();
        let _ = Literal::new(query, options)
            .unwrap()
            .scan(
                &rope,
                0,
                &Scope::Document,
                || false,
                |range| {
                    expected.push(range);
                    ControlFlow::<()>::Continue(())
                },
            )
            .unwrap();
        let actual = search(
            &subject,
            RegexQuery {
                pattern: query,
                scope: &Scope::Document,
                options,
            },
        );
        assert_eq!(actual.visible, expected, "case folding of {query}");
    }
}

fn single_atom_differential() {
    let all_ascii: String = (0..128u8).map(char::from).collect();
    let ascii = all_ascii.repeat(3);
    let scope = Scope::selections([3..177, 191..ascii.len()]);
    for text in [ascii.clone(), format!("{ascii}猫K")] {
        let subject = stage(&text);
        for (pat, whole, case_sensitive) in [
            (".", false, true),
            ("[a-z]", false, false),
            ("[^A-Z]", true, true),
            (r"\s", false, true),
            (r"\D", true, false),
            ("K", false, false),
        ] {
            let options = Options {
                whole_word: whole,
                case_sensitive,
            };
            let fast = RegexQuery {
                pattern: pat,
                options,
                scope: &scope,
            };
            // The equivalent noncapturing group always goes through native matching.
            let grouped = format!("(?:{pat})");
            let native = RegexQuery {
                pattern: &grouped,
                ..fast
            };
            assert_eq!(search(&subject, fast), search(&subject, native));
            let replaced = |query| {
                let mut result = Vec::new();
                subject
                    .replacements(query, "$0$&", None, &Budget::new(&LIVE), |range, parts| {
                        assert_eq!(
                            parts.pieces().collect::<Vec<_>>(),
                            [Piece::Source(range.clone()), Piece::Source(range.clone())]
                        );
                        result.push(range);
                        Ok::<_, ()>(())
                    })
                    .unwrap();
                result
            };
            assert_eq!(replaced(fast), replaced(native));
        }
    }
}

fn q<'a>(pattern: &'a str, scope: &'a Scope) -> RegexQuery<'a> {
    RegexQuery {
        pattern,
        options: Options {
            case_sensitive: true,
            whole_word: false,
        },
        scope,
    }
}

fn stage(text: &str) -> RegexSubject {
    RegexSubject::stage(&Rope::from_str(text), &Budget::new(&LIVE)).unwrap()
}

fn search(subject: &RegexSubject, query: RegexQuery<'_>) -> SearchResults {
    let mut first = None;
    let result = subject
        .search(
            query,
            Navigation::NextFrom(0),
            0..usize::MAX,
            Some(64),
            &Budget::new(&LIVE),
            |hit| first = Some(hit),
        )
        .unwrap();
    assert_eq!(first, result.first.as_ref().map(|hit| hit.range.clone()));
    result
}

fn semantics() {
    let any = Scope::Document;
    let subject = stage("one\ntwo\n");
    assert_eq!(search(&subject, q(r"(?=o)", &any)).visible, [0..0, 6..6]);
    assert_eq!(search(&subject, q(r"o(?!n)", &any)).visible, [6..7]);
    let cleared = search(&subject, q("", &any));
    assert_eq!(cleared.total, 0);
    assert!(
        subject
            .viewport(q("", &any), cleared.viewport(0..8), &Budget::new(&LIVE))
            .unwrap()
            .is_empty()
    );
    assert_eq!(search(&subject, q(r"^one.*two$", &any)).total, 0);
    assert_eq!(search(&subject, q(r"(?s)^one.*two$", &any)).visible, [0..7]);
    assert_eq!(
        search(&stage("😀o"), q(r"(?=.)|\z", &any)).visible,
        [0..0, 4..4, 5..5]
    );
    assert_eq!(search(&stage(""), q(r"\z", &any)).visible, [0..0]);
    assert_eq!(search(&stage("a\0b"), q("\0", &any)).visible, [1..2]);
    let result = subject.search(
        q("(", &any),
        Navigation::NextFrom(0),
        0..10,
        None,
        &Budget::new(&LIVE),
        |_| {},
    );
    assert!(matches!(result, Err(RegexError::Syntax { byte: 1, .. })));
    assert!(matches!(
        subject.search(
            q(r"\C", &any),
            Navigation::NextFrom(0),
            0..10,
            None,
            &Budget::new(&LIVE),
            |_| {}
        ),
        Err(RegexError::Syntax { .. })
    ));

    let scope = Scope::selections([1..3]);
    let subject = stage("XabY");
    assert_eq!(search(&subject, q("(?<=X)ab(?=Y)", &scope)).visible, [1..3]);
    assert_eq!(search(&subject, q("^ab$", &scope)).total, 0);
    let subject = stage("xa a a");
    let mut req = q("a a", &any);
    req.options.whole_word = true;
    assert_eq!(search(&subject, req).visible, [3..6]);
    let mut req = q("猫", &any);
    req.options.whole_word = true;
    assert_eq!(search(&stage("猫咪 猫 e\u{301} e‿猫"), req).total, 1);
}

fn replacement() {
    let text = "a axy!";
    let subject = stage(text);
    let any = Scope::Document;
    let pattern = r"(?<name>a)(x)?(?=(?<later>y)?)";
    let template = "$0|$&|$1|${name}|$$|$2|${later}\\n";
    let mut output = String::new();
    let mut end = 0;
    let count = subject
        .replacements(
            q(pattern, &any),
            template,
            None,
            &Budget::new(&LIVE),
            |hit, replacement| {
                output.push_str(&text[end..hit.start]);
                for piece in replacement.pieces() {
                    match piece {
                        Piece::Text(s) => output.push_str(s),
                        Piece::Source(r) => output.push_str(&text[r]),
                    }
                }
                end = hit.end;
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    output.push_str(&text[end..]);
    assert_eq!(count, 2);
    assert_eq!(output, "a|a|a|a|$||\\n ax|ax|a|a|$|x|y\\ny!");
    for template in ["$3", "${missing}"] {
        let error = subject.replacements(
            q(r"ZZZ", &any),
            template,
            None,
            &Budget::new(&LIVE),
            |_, _| -> Result<(), ()> { panic!("unknown capture reached visitor") },
        );
        assert!(matches!(
            error,
            Err(VisitError::Search(RegexError::Template { byte: 0, .. }))
        ));
    }
    let mut seen = Vec::new();
    assert_eq!(
        subject
            .replacements(
                q("a", &any),
                "$&",
                Some(2..3),
                &Budget::new(&LIVE),
                |hit, _| {
                    seen.push(hit);
                    Ok::<_, ()>(())
                }
            )
            .unwrap(),
        1
    );
    assert_eq!(seen, [2..3]);
    let mut named = Vec::new();
    subject
        .replacements(
            q("(?J)(?:(?<n>a)|(?<n>x))", &any),
            "${n}",
            None,
            &Budget::new(&LIVE),
            |hit, replacement| {
                assert_eq!(
                    replacement.pieces().collect::<Vec<_>>(),
                    [Piece::Source(hit.clone())]
                );
                named.push(hit);
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    assert_eq!(named, [0..1, 2..3, 3..4]);
}

fn bounded() {
    let any = Scope::Document;
    let tail = 256 * 1024;
    let content = format!("BEGIN{}END", ".".repeat(tail));
    let subject = stage(&content);
    let spans = search(&subject, q(r"(?s)BEGIN.+?END", &any));
    assert_eq!(spans.visible, [0..content.len()]);
    let scoped = Scope::selections([tail..tail + 2]);
    assert_eq!(search(&subject, q(r"\.", &scoped)).total, 2);
    let result = search(&subject, q(r"\.", &any));
    assert_eq!(result.total, tail as u64);
    let initial = subject.worker_stats().unwrap();
    assert_eq!(initial.subject_bytes_read, content.len() as u64);
    assert_eq!(initial.pattern_compilations, 1);
    assert!(result.visible.len() <= texzi_search::MAX_VISIBLE);
    assert!(result.index.points().len() < 10);
    let range = tail - 5..tail + 2;
    assert_eq!(
        subject
            .viewport(
                q(r"\.", &any),
                result.viewport(range.clone()),
                &Budget::new(&LIVE)
            )
            .unwrap(),
        range.clone().map(|i| i..i + 1).collect::<Vec<Range<_>>>()
    );
    let (hit, wrapped) = subject
        .navigate(
            q(r"\.", &any),
            result.navigation(Navigation::PreviousFrom(tail)),
            &Budget::new(&LIVE),
        )
        .unwrap();
    assert_eq!(hit.unwrap().range, tail - 1..tail);
    assert!(!wrapped);
    let success = subject
        .replacements(
            q(r"\.", &any),
            "$0",
            Some(tail - 1..tail),
            &Budget::new(&LIVE),
            |_, _| Ok::<_, ()>(()),
        )
        .unwrap();
    assert_eq!(success, 1);
    let reused = subject.worker_stats().unwrap();
    assert_eq!(
        (
            reused.pid,
            reused.subject_bytes_read,
            reused.pattern_compilations
        ),
        (initial.pid, initial.subject_bytes_read, 1)
    );
    assert_eq!(
        reused.completed_operations,
        initial.completed_operations + 3
    );
    let more: Result<u64, VisitError<u8>> =
        subject.replacements(q(r"\.", &any), "X", None, &Budget::new(&LIVE), |_, _| {
            Err(42)
        });
    assert!(matches!(more, Err(VisitError::Visitor(42))));
    assert_eq!(search(&subject, q("END", &any)).total, 1);
    subject.retire();
    assert!(subject.worker_stats().is_none());
    assert_eq!(search(&subject, q("END", &any)).total, 1);
}

fn cancellation() {
    let cancel = AtomicBool::new(false);
    let subject = stage(&"abcdef ".repeat(500_000));
    let mut first_at = None;
    let outcome = subject.search(
        q("(?=.)", &Scope::Document),
        Navigation::NextFrom(0),
        0..10,
        None,
        &Budget::new(&cancel),
        |_| {
            first_at = Some(Instant::now());
            cancel.store(true, Ordering::Release);
        },
    );
    assert_eq!(outcome.unwrap_err(), RegexError::Cancelled);
    assert!(first_at.unwrap().elapsed() < Duration::from_millis(250));
    assert_eq!(search(&subject, q("missing", &Scope::Document)).total, 0);
    let subject = stage(&format!("{}!", "a".repeat(256)));
    let result = subject.search(
        q("(*NO_START_OPT)(*NO_AUTO_POSSESS)^(a+)+$", &Scope::Document),
        Navigation::NextFrom(0),
        0..100,
        None,
        &Budget::new(&LIVE),
        |_| {},
    );
    assert!(matches!(result, Err(RegexError::EngineLimit { .. })));
}

fn deadline() {
    let subject = stage("abc");
    let begin = Instant::now();
    let result = subject.search(
        q(".", &Scope::Document),
        Navigation::NextFrom(0),
        0..3,
        None,
        &Budget::new(&LIVE),
        |_| {},
    );
    assert_eq!(result.unwrap_err(), RegexError::Timeout);
    assert!(begin.elapsed() < Duration::from_secs(4)); // an un-killed child sleeps 30 s
}

fn check_cleanup(mode: &str, expected_exit: i32) {
    let dir = tempfile::tempdir().unwrap();
    let mut process = Command::new(env::current_exe().unwrap());
    process
        .arg(mode)
        .env("TMP", dir.path())
        .env("TEMP", dir.path())
        .env("TMPDIR", dir.path());
    if mode == "deadline" {
        process.env(CASE, "1");
    }
    assert_eq!(process.status().unwrap().code(), Some(expected_exit));
    let until = Instant::now() + Duration::from_secs(1);
    while dir.path().read_dir().unwrap().next().is_some() {
        assert!(Instant::now() < until, "subject file remained after {mode}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn main() {
    let mode = env::args().nth(1);
    if mode.as_deref() == Some(pcre::WORKER_FLAG) {
        if env::var_os(CASE).is_some() {
            std::thread::sleep(Duration::from_secs(30));
        }
        process::exit(pcre::worker_main());
    }
    if mode.as_deref() == Some("deadline") {
        deadline();
        return;
    }
    #[cfg(windows)]
    if mode.as_deref() == Some("exit-with-worker") {
        let subject = stage(&"abcdef".repeat(200_000));
        let _ = subject.search(
            q("(?=.)", &Scope::Document),
            Navigation::NextFrom(0),
            0..20,
            None,
            &Budget::new(&LIVE),
            |_| process::exit(42),
        );
        panic!("the worker did not publish its first hit");
    }
    frozen_versions();
    casefold_parity();
    single_atom_differential();
    semantics();
    replacement();
    bounded();
    cancellation();
    check_cleanup("deadline", 0);
    #[cfg(windows)]
    check_cleanup("exit-with-worker", 42);
}
