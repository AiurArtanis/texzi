//! Generated-file Windows fixtures only. No user documents or ACL changes.
use super::*;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
static NEXT: AtomicU64 = AtomicU64::new(1);
fn fixture(raw: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "texzi-captured-open-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    let path = dir.join("source.txt");
    fs::write(&path, raw).unwrap();
    eprintln!(
        "T07 generated captured-open fixture retained: {}",
        dir.display()
    );
    path
}
#[test]
fn captured_open_preserves_exact_bom_text_eols_and_legacy_original_bytes() {
    let cases: &[(Encoding, bool, &[u8], &str)] = &[
        (
            Encoding::Utf8,
            true,
            b"\xef\xbb\xbf\xef\xbb\xbfA\r\nB\n",
            "\u{feff}A\r\nB\n",
        ),
        (
            Encoding::Utf16Le,
            true,
            &[255, 254, 255, 254, 65, 0, 13, 0, 10, 0, 66, 0, 10, 0],
            "\u{feff}A\r\nB\n",
        ),
        (
            Encoding::Utf16Be,
            true,
            &[254, 255, 254, 255, 0, 65, 0, 13, 0, 10, 0, 66, 0, 10],
            "\u{feff}A\r\nB\n",
        ),
        (
            Encoding::Windows1252,
            false,
            &[128, 65, 13, 10, 66, 10],
            "€A\r\nB\n",
        ),
    ];
    for &(encoding, bom, raw, text) in cases {
        let path = fixture(raw);
        let format = Format { encoding, bom };
        let observed = ObservedTarget::inspect(&path).unwrap();
        let version = observed.version().unwrap().clone();
        let mut capture = observed.open_existing_exact(format).unwrap();
        assert_eq!(capture.version(), &version);
        assert_eq!(capture.loaded().format(), format);
        assert_eq!(capture.loaded().text(), text);
        assert_eq!(capture.loaded().original(), raw);
        assert_eq!(capture.version().bytes.as_ref(), raw);
        capture.recheck().unwrap();
        drop(capture);
        assert_eq!(fs::read(path).unwrap(), raw);
    }
}
#[test]
fn captured_open_allows_reader_but_blocks_normal_writer_and_rename_until_drop() {
    let raw = b"original\r\n";
    let path = fixture(raw);
    let capture = ObservedTarget::inspect(&path)
        .unwrap()
        .open_existing(None)
        .unwrap();
    assert_eq!(fs::read(&path).unwrap(), raw);
    let writer = OpenOptions::new()
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(&path);
    assert_eq!(writer.unwrap_err().raw_os_error(), Some(32));
    let moved = path.with_file_name("moved.txt");
    assert_eq!(
        fs::rename(&path, &moved).unwrap_err().raw_os_error(),
        Some(32)
    );
    assert!(!moved.exists());
    assert_eq!(capture.loaded().original(), raw);
    drop(capture);
    fs::rename(&path, &moved).unwrap();
    assert_eq!(fs::read(moved).unwrap(), raw);
}
#[test]
fn already_open_writer_prevents_capture_without_share_fallback() {
    let raw = b"not changed";
    let path = fixture(raw);
    let observed = ObservedTarget::inspect(&path).unwrap();
    let writer = OpenOptions::new()
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(&path)
        .unwrap();
    let failure = match observed.open_existing(None) {
        Ok(_) => panic!("conflicting writer unexpectedly accepted"),
        Err(failure) => failure,
    };
    assert!(
        matches!(&failure.issue, OpenIssue::Identity(IdentityError::Io(e)) if e.raw_os_error()==Some(32))
    );
    drop(writer);
    // A fresh observation and capture can succeed after the fixture writer closes.
    assert_eq!(
        ObservedTarget::inspect(&path)
            .unwrap()
            .open_existing(None)
            .unwrap()
            .loaded()
            .original(),
        raw
    );
    assert_eq!(fs::read(path).unwrap(), raw);
}
#[test]
fn bytes_changed_after_observation_are_rejected_instead_of_silently_imported() {
    let path = fixture(b"before");
    let observed = ObservedTarget::inspect(&path).unwrap();
    fs::write(&path, b"after!").unwrap();
    let failure = match observed.open_existing(None) {
        Ok(_) => panic!("changed file accepted"),
        Err(failure) => failure,
    };
    assert!(matches!(
        &failure.issue,
        OpenIssue::Identity(IdentityError::Changed)
    ));
    assert_eq!(fs::read(path).unwrap(), b"after!");
}
#[test]
fn same_bytes_replacement_with_new_file_id_is_rejected() {
    let raw = b"same bytes";
    let path = fixture(raw);
    let observed = ObservedTarget::inspect(&path).unwrap();
    let old_id = observed.version().unwrap().identity;
    let backup = path.with_file_name("original-retained.txt");
    fs::rename(&path, &backup).unwrap();
    fs::write(&path, raw).unwrap();
    assert_ne!(
        ObservedTarget::inspect(&path)
            .unwrap()
            .version()
            .unwrap()
            .identity,
        old_id
    );
    let failure = match observed.open_existing(None) {
        Ok(_) => panic!("replacement identity accepted"),
        Err(failure) => failure,
    };
    assert!(matches!(
        &failure.issue,
        OpenIssue::Identity(IdentityError::Changed)
    ));
    assert_eq!(fs::read(path).unwrap(), raw);
    assert_eq!(fs::read(backup).unwrap(), raw);
}
#[test]
fn missing_invalid_and_unsupported_standard_do_not_create_or_rewrite_files() {
    let path = fixture(&[255]);
    let invalid = ObservedTarget::inspect(&path)
        .unwrap()
        .open_existing_exact(Format {
            encoding: Encoding::Utf8,
            bom: false,
        });
    assert!(matches!(
        invalid,
        Err(OpenFailure {
            issue: OpenIssue::Decode(FileError::InvalidBytes),
            ..
        })
    ));
    for encoding in [Encoding::Gbk936, Encoding::Gb18030_2022] {
        assert!(matches!(
            ObservedTarget::inspect(&path)
                .unwrap()
                .open_existing(Some(encoding)),
            Err(OpenFailure {
                issue: OpenIssue::Decode(FileError::UnsupportedStandard),
                ..
            })
        ));
    }
    let absent = path.with_file_name("absent.txt");
    assert!(matches!(
        ObservedTarget::inspect(&absent)
            .unwrap()
            .open_existing(None),
        Err(OpenFailure {
            issue: OpenIssue::Missing,
            ..
        })
    ));
    assert!(!absent.exists());
    assert_eq!(fs::read(path).unwrap(), [255]);
}
