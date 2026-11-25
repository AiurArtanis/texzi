//! Bounded, cancellable root-file discovery for Quick Open. This module reads
//! directory entries and metadata only; opening/decoding a chosen file belongs
//! to the shell's existing document path.
use std::{
    fs::{self, Metadata},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ScanLimits {
    pub max_entries: usize,
    pub max_files: usize,
    pub max_errors: usize,
}
impl Default for ScanLimits {
    fn default() -> Self {
        // Share the current 2000-node enumeration bound. Folder search's
        // 200 matching-text-lines limit is not a file-candidate unit. These
        // bounds are not measured latency guarantees or an opened-file cap.
        Self {
            max_entries: 2000,
            max_files: 2000,
            max_errors: 20,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub path: PathBuf,
    pub display: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ScanReport {
    pub files: Vec<Candidate>,
    /// Metadata attempts, including the root and failed read_dir entries.
    pub scanned: usize,
    /// Observed links, special files and failures; not an estimate of unseen files.
    pub skipped: usize,
    pub limited: bool,
    pub cancelled: bool,
    pub errors: Vec<String>,
    pub errors_omitted: usize,
}

fn linked(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}
fn error(
    report: &mut ScanReport,
    limits: ScanLimits,
    path: &Path,
    message: impl std::fmt::Display,
) {
    if report.errors.len() < limits.max_errors {
        report.errors.push(format!("{}：{message}", path.display()));
    } else {
        report.errors_omitted = report.errors_omitted.saturating_add(1);
    }
}
fn relative_display(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Enumerate only the requested root. Directory links and Windows reparse
/// points are skipped before traversal; no file body is opened or decoded.
/// The shell owns job/root invalidation and decides whether to publish this
/// partial report. Complete and partial candidate lists are sorted; a limited
/// subset is not advertised as the globally first filenames in a larger tree.
pub(crate) fn scan(root: PathBuf, limits: ScanLimits, cancel: Arc<AtomicBool>) -> ScanReport {
    let mut report = ScanReport::default();
    if cancel.load(Ordering::Relaxed) {
        report.cancelled = true;
        return report;
    }
    if limits.max_entries == 0 || limits.max_files == 0 {
        report.limited = true;
        return report;
    }
    let mut pending = vec![(root.clone(), true)];
    while let Some((path, is_root)) = pending.pop() {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        if report.scanned >= limits.max_entries || report.files.len() >= limits.max_files {
            report.limited = true;
            break;
        }
        report.scanned += 1;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(reason) => {
                report.skipped += 1;
                error(&mut report, limits, &path, reason);
                continue;
            }
        };
        if linked(&metadata) {
            report.skipped += 1;
            if is_root {
                error(
                    &mut report,
                    limits,
                    &path,
                    "快速打开根目录不能是链接或重解析点",
                );
            }
            continue;
        }
        if is_root && !metadata.is_dir() {
            report.skipped += 1;
            error(&mut report, limits, &path, "快速打开根路径不是目录");
            continue;
        }
        if metadata.is_dir() {
            let mut entries = match fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(reason) => {
                    report.skipped += 1;
                    error(&mut report, limits, &path, reason);
                    continue;
                }
            };
            let mut children = Vec::new();
            loop {
                if cancel.load(Ordering::Relaxed) {
                    report.cancelled = true;
                    break;
                }
                // Reserve a slot for every queued child, rather than reading
                // an unbounded directory into memory before checking a limit.
                let reserved = report
                    .scanned
                    .saturating_add(pending.len())
                    .saturating_add(children.len());
                let Some(entry) = entries.next() else {
                    break;
                };
                if reserved >= limits.max_entries {
                    report.limited = true;
                    break;
                }
                match entry {
                    Ok(entry) => children.push(entry.path()),
                    Err(reason) => {
                        report.scanned += 1;
                        report.skipped += 1;
                        error(&mut report, limits, &path, reason);
                    }
                }
            }
            if report.cancelled {
                break;
            }
            children.sort();
            pending.extend(children.into_iter().rev().map(|child| (child, false)));
        } else if metadata.is_file() {
            let relative = match path.strip_prefix(&root) {
                Ok(relative) => relative,
                Err(reason) => {
                    report.skipped += 1;
                    error(&mut report, limits, &path, reason);
                    continue;
                }
            };
            report.files.push(Candidate {
                display: relative_display(relative),
                path,
            });
        } else {
            report.skipped += 1;
            error(&mut report, limits, &path, "不是普通文件或目录");
        }
    }
    report.cancelled |= cancel.load(Ordering::Relaxed);
    report.files.sort_by(|left, right| {
        left.display
            .cmp(&right.display)
            .then_with(|| left.path.cmp(&right.path))
    });
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);
    struct Fixture {
        root: PathBuf,
        temp: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = fs::canonicalize(std::env::temp_dir()).unwrap();
            let root = temp.join(format!(
                "texzi-quick-open-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            Self {
                root: fs::canonicalize(root).unwrap(),
                temp,
            }
        }
        fn file(&self, relative: &str, contents: &[u8]) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            // Cleanup can only target this test's resolved direct Temp child.
            if self.root.is_absolute()
                && self.root.parent() == Some(self.temp.as_path())
                && self
                    .root
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("texzi-quick-open-"))
                && fs::symlink_metadata(&self.root)
                    .is_ok_and(|metadata| metadata.is_dir() && !linked(&metadata))
            {
                if let Err(reason) = fs::remove_dir_all(&self.root) {
                    eprintln!(
                        "Quick Open fixture cleanup failed: {}: {reason}",
                        self.root.display()
                    );
                }
            }
        }
    }
    fn not_cancelled() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[test]
    fn quick_open_metadata_scan_returns_nested_relative_files_in_stable_order() {
        let fixture = Fixture::new();
        fixture.file("Z.txt", b"z");
        fixture.file("a.md", b"a");
        fixture.file("nested/b.rs", b"b");
        fixture.file("nested/深层/猫.txt", b"cat");
        fixture.file("z.bin", &[0, 0xff, 0x81]);
        let report = scan(fixture.root.clone(), ScanLimits::default(), not_cancelled());
        assert_eq!(
            report
                .files
                .iter()
                .map(|file| file.display.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Z.txt",
                "a.md",
                "nested/b.rs",
                "nested/深层/猫.txt",
                "z.bin"
            ]
        );
        assert_eq!(report.scanned, 8);
        assert_eq!(report.skipped, 0);
        assert!(!report.limited && !report.cancelled);
        assert!(report.errors.is_empty());
        for file in &report.files {
            assert!(file.path.starts_with(&fixture.root));
            assert!(file.path.is_file());
        }
        assert_eq!(
            fs::read(fixture.root.join("z.bin")).unwrap(),
            [0, 0xff, 0x81],
            "binary candidates are not filtered or rewritten as text"
        );
        assert_eq!(
            scan(fixture.root.clone(), ScanLimits::default(), not_cancelled()),
            report
        );
    }

    #[test]
    fn quick_open_scan_reports_pre_cancel_and_both_budgets_without_claiming_complete_results() {
        let fixture = Fixture::new();
        for name in ["c.txt", "a.txt", "b.txt"] {
            fixture.file(name, b"fixture");
        }
        let cancelled = scan(
            fixture.root.clone(),
            ScanLimits::default(),
            Arc::new(AtomicBool::new(true)),
        );
        assert!(cancelled.cancelled);
        assert_eq!(cancelled.scanned, 0);
        assert!(cancelled.files.is_empty());
        let one_entry = scan(
            fixture.root.clone(),
            ScanLimits {
                max_entries: 1,
                ..ScanLimits::default()
            },
            not_cancelled(),
        );
        assert_eq!(one_entry.scanned, 1);
        assert!(one_entry.files.is_empty());
        assert!(one_entry.limited);
        assert!(!one_entry.cancelled);
        let two_files = scan(
            fixture.root.clone(),
            ScanLimits {
                max_files: 2,
                ..ScanLimits::default()
            },
            not_cancelled(),
        );
        assert_eq!(two_files.files.len(), 2);
        assert_eq!(two_files.scanned, 3);
        assert!(two_files.limited);
        assert!(
            two_files
                .files
                .windows(2)
                .all(|pair| pair[0].display <= pair[1].display)
        );
        let zero = scan(
            fixture.root.clone(),
            ScanLimits {
                max_entries: 0,
                ..ScanLimits::default()
            },
            not_cancelled(),
        );
        assert!(zero.limited);
        assert_eq!(zero.scanned, 0);
        assert!(zero.files.is_empty());
        let empty = Fixture::new();
        let exact = scan(
            empty.root.clone(),
            ScanLimits {
                max_entries: 1,
                ..ScanLimits::default()
            },
            not_cancelled(),
        );
        assert!(!exact.limited && !exact.cancelled);
        assert_eq!(exact.scanned, 1);
        assert!(exact.files.is_empty());
    }

    #[test]
    fn quick_open_invalid_roots_report_errors_and_do_not_return_a_root_file_as_a_candidate() {
        let fixture = Fixture::new();
        fixture.file("only.txt", b"kept");
        let missing = scan(
            fixture.root.join("missing"),
            ScanLimits::default(),
            not_cancelled(),
        );
        assert!(missing.files.is_empty());
        assert_eq!(missing.scanned, 1);
        assert_eq!(missing.skipped, 1);
        assert_eq!(missing.errors.len(), 1);
        assert!(missing.errors[0].contains("missing"));
        assert!(!missing.limited && !missing.cancelled);
        let file = scan(
            fixture.root.join("only.txt"),
            ScanLimits::default(),
            not_cancelled(),
        );
        assert!(file.files.is_empty());
        assert_eq!(file.skipped, 1);
        assert!(file.errors[0].contains("不是目录"));
        let omitted = scan(
            fixture.root.join("missing"),
            ScanLimits {
                max_errors: 0,
                ..ScanLimits::default()
            },
            not_cancelled(),
        );
        assert!(omitted.errors.is_empty());
        assert_eq!(omitted.errors_omitted, 1);
        assert_eq!(omitted.skipped, 1);
        assert_eq!(fs::read(fixture.root.join("only.txt")).unwrap(), b"kept");
    }
}
