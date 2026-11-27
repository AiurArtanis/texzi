//! Numeric, synthetic-only checks of the real parent pipe and process lifetime.
use super::*;
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    thread,
};

pub(super) const SIGNALS: &str = "TEXZI_PCRE_TEST_SIGNALS";
const RECEIPTS: &str = "TEXZI_PCRE_TEST_TRANSFER_RECEIPTS";

fn signals() -> PathBuf {
    PathBuf::from(env::var_os(SIGNALS).unwrap())
}

fn wait_for<T>(mut ready: impl FnMut() -> Option<T>) -> T {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(value) = ready() {
            return value;
        }
        assert!(Instant::now() < until, "timed out waiting for the fixture");
        thread::sleep(Duration::from_millis(1));
    }
}

fn signal_pid(dir: &Path) {
    fs::write(dir.join("pid-new"), process::id().to_string()).unwrap();
    fs::rename(dir.join("pid-new"), dir.join("pid")).unwrap();
}

fn signalled_pid(dir: &Path) -> u32 {
    wait_for(|| fs::read_to_string(dir.join("pid")).ok()?.parse().ok())
}

#[cfg(windows)]
pub(super) struct WorkerHandle(std::os::windows::io::OwnedHandle);

#[cfg(windows)]
pub(super) fn observe_signalled_pid(dir: &Path) -> WorkerHandle {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};
    let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, signalled_pid(dir)) };
    assert!(!raw.is_null(), "the isolated worker must still be alive");
    WorkerHandle(unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(raw) })
}

#[cfg(windows)]
impl WorkerHandle {
    pub(super) fn assert_exited(&self, timeout_ms: u32) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{
            Foundation::WAIT_OBJECT_0, System::Threading::WaitForSingleObject,
        };
        assert_eq!(
            unsafe { WaitForSingleObject(self.0.as_raw_handle(), timeout_ms) },
            WAIT_OBJECT_0
        );
    }
}

pub(super) fn fixture_before_worker() {
    match env::var(CASE).ok().as_deref() {
        Some("deadline") => thread::sleep(Duration::from_secs(30)),
        #[cfg(windows)]
        Some("exit-with-worker") => {
            let dir = signals();
            signal_pid(&dir);
            wait_for(|| dir.join("go").exists().then_some(()));
        }
        Some("paused-reader" | "paused-timeout") => {
            let dir = signals();
            if let Err(error) = fs::File::create_new(dir.join("first")) {
                assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
                return; // the follow-up query uses the real interpreter
            }
            let mut input = io::stdin().lock();
            let mut length = [0; 4];
            input.read_exact(&mut length).unwrap();
            let length = u32::from_le_bytes(length);
            assert!(length < 65_536);
            io::copy(&mut input.by_ref().take(length.into()), &mut io::sink()).unwrap();
            // Test peer accepts the production request, acknowledges its subject,
            // reads a prefix, then holds the only reader without consuming more.
            let mut out = io::stdout().lock();
            out.write_all(&[7, 0, 0, 0, 0]).unwrap();
            out.flush().unwrap();
            let mut first = [0u8];
            input.read_exact(&mut first).unwrap();
            assert_eq!(first[0], b'x');
            signal_pid(&dir);
            thread::sleep(Duration::from_secs(30));
        }
        _ => {}
    }
}

pub(super) fn paused_transfer(deadline: bool) {
    let dir = signals();
    let raw = format!("{}END", "x".repeat(8 * 1024 * 1024));
    let subject = stage(&raw);
    let serial = Mutex::new(());
    let cancel = AtomicBool::new(false);
    let start = Instant::now();
    thread::scope(|threads| {
        let first = threads.spawn(|| {
            let _guard = serial.lock().unwrap();
            let outcome = subject.search(
                q("x", &Scope::Document),
                Navigation::NextFrom(0),
                0..30,
                None,
                &Budget::new(&cancel),
                |_| panic!("stalled transfer published a match"),
            );
            (outcome, Instant::now())
        });
        let old_pid = signalled_pid(&dir);
        #[cfg(windows)]
        let old_process = observe_signalled_pid(&dir);
        thread::sleep(Duration::from_millis(30));
        assert!(!first.is_finished());
        let signalled = Instant::now();
        if !deadline {
            cancel.store(true, Ordering::Release);
        }
        // This models the application's search-job mutex: the follow-up is queued
        // immediately while the first search still owns it.
        let next = threads.spawn(|| {
            let _guard = serial.lock().unwrap();
            let result = search(&subject, q("END$", &Scope::Document));
            assert_eq!(result.total, 1);
            (subject.worker_stats().unwrap(), Instant::now())
        });
        let (outcome, stopped) = first.join().unwrap();
        #[cfg(windows)]
        old_process.assert_exited(0);
        let (stats, next_done) = next.join().unwrap();
        assert_eq!(stats.subject_bytes_read, raw.len() as u64);
        assert_eq!(stats.pattern_compilations, 1);
        assert_ne!(old_pid, stats.pid);
        if deadline {
            assert_eq!(outcome.unwrap_err(), RegexError::Timeout);
            assert!(stopped.duration_since(start) < Duration::from_secs(3));
        } else {
            assert_eq!(outcome.unwrap_err(), RegexError::Cancelled);
            assert!(stopped.duration_since(signalled) < Duration::from_millis(250));
        }
        if let Some(path) = env::var_os(RECEIPTS) {
            let row = serde_json::json!({
                "outcome": if deadline { "timeout" } else { "cancelled" },
                "parent": process::id(), "stalled_worker": old_pid, "next_worker": stats.pid,
                "subject_bytes": raw.len(), "next_subject_bytes_read": stats.subject_bytes_read,
                "from_start_us": stopped.duration_since(start).as_micros(),
                "signal_to_return_us": stopped.duration_since(signalled).as_micros(),
                "from_return_to_next_complete_us": next_done.duration_since(stopped).as_micros(),
            });
            writeln!(
                OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(path)
                    .unwrap(),
                "{row}"
            )
            .unwrap();
        }
    });
}
