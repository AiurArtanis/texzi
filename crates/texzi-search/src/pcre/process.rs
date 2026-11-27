use std::{
    io::{self, BufReader},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        Arc, Mutex, MutexGuard, TryLockError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use super::{Budget, RegexError, VisitError, WORKER_FLAG, wire};
use crate::Rope;

#[cfg(windows)]
mod platform {
    use std::{
        mem,
        os::windows::{
            io::{AsRawHandle, FromRawHandle, OwnedHandle},
            process::CommandExt,
        },
        ptr,
    };
    use windows_sys::Win32::System::{JobObjects::*, Threading::CREATE_NO_WINDOW};

    use super::*;

    pub fn command(cmd: &mut Command) {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    pub type Job = OwnedHandle;

    pub fn job(child: &Child) -> io::Result<Job> {
        // The child blocks reading the request until assignment succeeds. Closing the
        // last Job handle also kills it if the GUI process is terminated unexpectedly.
        unsafe {
            let raw = CreateJobObjectW(ptr::null(), ptr::null());
            if raw.is_null() {
                return Err(io::Error::last_os_error());
            }
            let handle = OwnedHandle::from_raw_handle(raw);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = mem::zeroed();
            limits.BasicLimitInformation.LimitFlags =
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
            limits.ProcessMemoryLimit = 512 * 1024 * 1024;
            if SetInformationJobObject(
                raw,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(raw, child.as_raw_handle()) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(handle)
        }
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    pub fn command(_: &mut Command) {}
    pub type Job = ();
    pub fn job(_: &Child) -> io::Result<Job> {
        Ok(())
    }
}

struct ChildGuard {
    child: Child,
    job: Option<platform::Job>,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Session {
    guard: ChildGuard,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    generation: u64,
    stats: Option<super::WorkerStats>,
}

impl Session {
    fn spawn(executable: &Path, generation: u64) -> Result<Self, RegexError> {
        let mut cmd = Command::new(executable);
        cmd.arg(WORKER_FLAG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        platform::command(&mut cmd);
        let mut guard = ChildGuard {
            child: cmd.spawn().map_err(|_| RegexError::Launch)?,
            job: None,
        };
        guard.job = Some(platform::job(&guard.child).map_err(|_| RegexError::Launch)?);
        let stdin = guard.child.stdin.take().ok_or(RegexError::Worker)?;
        let stdout = guard.child.stdout.take().ok_or(RegexError::Worker)?;
        Ok(Self {
            guard,
            stdin,
            reader: BufReader::with_capacity(wire::BATCH_BYTES, stdout),
            generation,
            stats: None,
        })
    }

    fn call<T, E>(
        &mut self,
        payload: &[u8],
        subject: Option<&Rope>,
        budget: &Budget<'_>,
        generation: &AtomicU64,
        viewport: bool,
        receive: impl FnOnce(&mut BufReader<ChildStdout>) -> Result<T, VisitError<E>>,
    ) -> Result<T, VisitError<E>> {
        let complete = AtomicBool::new(false);
        let ticket = self.generation;
        let child = &mut self.guard.child;
        let pid = child.id();
        thread::scope(|scope| {
            let watcher = thread::Builder::new()
                .name("texzi-regex-watch".into())
                .spawn_scoped(scope, || {
                    loop {
                        if complete.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        let reason = if generation.load(Ordering::Acquire) != ticket
                            || (!viewport && budget.cancel.load(Ordering::Acquire))
                        {
                            Some(RegexError::Cancelled)
                        } else if std::time::Instant::now() >= budget.deadline {
                            Some(RegexError::Timeout)
                        } else {
                            None
                        };
                        if let Some(reason) = reason {
                            let _ = child.kill();
                            return Err(reason);
                        }
                        thread::park_timeout(Duration::from_millis(3));
                    }
                })
                .map_err(|_| RegexError::Launch)?;
            let result = (|| {
                wire::write_blob(&mut self.stdin, payload)?;
                if let Some(rope) = subject {
                    let mut ready = Vec::new();
                    if wire::next_frame(&mut self.reader, &mut ready)? != wire::SUBJECT_READY {
                        return Err(RegexError::Worker.into());
                    }
                    // The watchdog is already live. Killing this child closes the
                    // only pipe reader and releases even a blocked write/flush.
                    wire::write_subject(&mut self.stdin, rope).map_err(|_| RegexError::Staging)?;
                }
                let value = receive(&mut self.reader)?;
                let mut buffer = Vec::new();
                if wire::next_frame(&mut self.reader, &mut buffer)? != wire::STATS {
                    return Err(RegexError::Worker.into());
                }
                let stats: super::WorkerStats = wire::parse(&buffer)?;
                if stats.pid != pid {
                    return Err(RegexError::Worker.into());
                }
                self.stats = Some(stats);
                Ok(value)
            })();
            complete.store(true, Ordering::Release);
            watcher.thread().unpark();
            watcher.join().map_err(|_| RegexError::Worker)??;
            result
        })
    }
}

#[derive(Default)]
struct Slot {
    worker: Option<Session>,
    reaper: Option<thread::JoinHandle<()>>,
}

impl Slot {
    fn stop_before(&mut self, generation: u64) {
        if self
            .worker
            .as_ref()
            .is_none_or(|worker| worker.generation >= generation)
        {
            return;
        }
        if let Some(mut session) = self.worker.take() {
            // TerminateProcess/Kill is asynchronous. The next user of this subject
            // joins its reaper before starting another full private subject allocation.
            let _ = session.guard.child.kill();
            self.reaper = thread::Builder::new()
                .name("texzi-regex-reap".into())
                .spawn(move || drop(session))
                .ok();
        }
    }
}

#[derive(Default)]
pub(super) struct Sessions {
    identity: Mutex<Option<(String, crate::Options, crate::Scope)>>,
    generation: AtomicU64,
    viewport: AtomicU64,
    slot: Arc<Mutex<Slot>>,
    pending_retirement: Arc<AtomicU64>,
}

impl Sessions {
    pub fn stats(&self) -> Option<super::WorkerStats> {
        let slot = self.slot.try_lock().ok()?;
        let session = slot.worker.as_ref()?;
        if session.generation == self.generation.load(Ordering::Acquire) {
            session.stats
        } else {
            None
        }
    }

    pub fn retire(&self) {
        let next = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if let Ok(mut slot) = self.slot.try_lock() {
            slot.stop_before(next);
        } else if self.pending_retirement.fetch_max(next, Ordering::AcqRel) == 0 {
            // The in-flight caller can already have read its final response. Recheck
            // after it drops the lock so an idle retired child cannot miss the signal.
            let slot = Arc::clone(&self.slot);
            let pending = Arc::clone(&self.pending_retirement);
            let cleaner = thread::Builder::new()
                .name("texzi-regex-retire".into())
                .spawn(move || {
                    if let Ok(mut slot) = slot.lock() {
                        slot.stop_before(pending.swap(0, Ordering::AcqRel));
                    }
                });
            if cleaner.is_err() {
                self.pending_retirement.store(0, Ordering::Release);
            }
        }
    }

    fn reserve(
        &self,
        request: &wire::Request,
        budget: &Budget<'_>,
    ) -> Result<(u64, Option<u64>), RegexError> {
        let mut key = self.identity.lock().map_err(|_| RegexError::Worker)?;
        budget.check()?;
        if !key.as_ref().is_some_and(|(pat, options, scope)| {
            pat == &request.pattern && *options == request.options && scope == &request.scope
        }) {
            self.generation.fetch_add(1, Ordering::AcqRel);
            *key = Some((
                request.pattern.clone(),
                request.options,
                request.scope.clone(),
            ));
        }
        let viewport = matches!(request.operation, wire::Operation::Viewport(_))
            .then(|| self.viewport.fetch_add(1, Ordering::AcqRel) + 1);
        Ok((self.generation.load(Ordering::Acquire), viewport))
    }

    fn ticket(&self, generation: u64, viewport: Option<u64>) -> Result<(), RegexError> {
        if self.generation.load(Ordering::Acquire) != generation
            || viewport.is_some_and(|n| self.viewport.load(Ordering::Acquire) != n)
        {
            Err(RegexError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn lock_for(
        &self,
        generation: u64,
        view: Option<u64>,
        budget: &Budget<'_>,
    ) -> Result<MutexGuard<'_, Slot>, RegexError> {
        loop {
            budget.check()?;
            self.ticket(generation, view)?;
            match self.slot.try_lock() {
                Ok(slot) => return Ok(slot),
                Err(TryLockError::Poisoned(_)) => return Err(RegexError::Worker),
                Err(TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(2)),
            }
        }
    }

    pub fn exchange<T, E>(
        &self,
        executable: &Path,
        rope: &Rope,
        request: &wire::Request,
        budget: &Budget<'_>,
        receive: impl FnOnce(
            &mut BufReader<ChildStdout>,
            &dyn Fn() -> Result<(), RegexError>,
        ) -> Result<T, VisitError<E>>,
    ) -> Result<T, VisitError<E>> {
        let (generation, view) = self.reserve(request, budget)?;
        let mut slot = self.lock_for(generation, view, budget)?;
        if let Some(reaper) = slot.reaper.take() {
            reaper.join().map_err(|_| RegexError::Worker)?;
        }
        if slot
            .worker
            .as_ref()
            .is_some_and(|s| s.generation != generation)
        {
            drop(slot.worker.take());
        }
        self.ticket(generation, view)?;
        budget.check()?;
        let initial = slot.worker.is_none();
        let payload = if initial {
            wire::json(request)?
        } else {
            wire::json(&request.operation)?
        };
        if payload.len() > wire::CONTROL_BYTES {
            return Err(RegexError::Resource.into());
        }
        if initial {
            slot.worker = Some(Session::spawn(executable, generation)?);
        }
        let worker = slot.worker.as_mut().ok_or(RegexError::Worker)?;
        let active = || {
            self.ticket(generation, view)?;
            budget.check()
        };
        let result = worker.call(
            &payload,
            initial.then_some(rope),
            budget,
            &self.generation,
            view.is_some(),
            |reader| receive(reader, &active),
        );
        if result.is_err()
            || self.generation.load(Ordering::Acquire) != generation
            || std::time::Instant::now() >= budget.deadline
            || (view.is_none() && budget.cancel.load(Ordering::Acquire))
        {
            // Includes cancellation, malformed/truncated responses, callback failure and timeout.
            // Reap synchronously on the background caller before publishing an outcome.
            drop(slot.worker.take());
        }
        let value = result?;
        self.ticket(generation, view)?;
        budget.check()?;
        Ok(value)
    }
}

impl Drop for Sessions {
    fn drop(&mut self) {
        self.retire();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Navigation, Options, Scope};

    #[test]
    fn closing_the_only_pipe_reader_releases_an_inflight_subject_write() {
        use std::io::{Read, Write};
        use std::sync::atomic::AtomicUsize;

        struct Probe<'a> {
            pipe: io::PipeWriter,
            entered: &'a AtomicUsize,
            returned: &'a AtomicUsize,
        }
        impl Write for Probe<'_> {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.entered.fetch_add(1, Ordering::Release);
                let result = self.pipe.write(bytes);
                self.returned.fetch_add(1, Ordering::Release);
                result
            }
            fn flush(&mut self) -> io::Result<()> {
                self.pipe.flush()
            }
        }

        let (mut reader, pipe) = io::pipe().unwrap();
        let entered = AtomicUsize::new(0);
        let returned = AtomicUsize::new(0);
        let rope = Rope::from_str(&"x".repeat(8 * 1024 * 1024));
        thread::scope(|scope| {
            let mut probe = Probe {
                pipe,
                entered: &entered,
                returned: &returned,
            };
            let write = scope.spawn(move || wire::write_subject(&mut probe, &rope));
            let mut first = [0u8];
            reader.read_exact(&mut first).unwrap();
            assert_eq!(first[0], b'x');
            // The reader is paused and a real OS write has entered without returning.
            thread::sleep(Duration::from_millis(25));
            assert!(entered.load(Ordering::Acquire) > returned.load(Ordering::Acquire));
            drop(reader);
            assert!(write.join().unwrap().is_err());
            assert_eq!(
                entered.load(Ordering::Acquire),
                returned.load(Ordering::Acquire)
            );
        });
    }

    #[test]
    fn missing_worker_is_a_launch_error_before_any_subject_transfer() {
        let rope = Rope::from_str("unchanged");
        let folder = tempfile::tempdir().unwrap();
        let missing = folder.path().join("absent-worker");
        let request = wire::Request {
            subject_len: 9,
            pattern: "x".into(),
            options: Options::default(),
            scope: Scope::Document,
            operation: wire::Operation::Search {
                navigation: Navigation::NextFrom(0),
                viewport: 0..9,
                select_cap: None,
            },
        };
        let cancel = AtomicBool::new(false);
        let result: Result<(), VisitError<()>> = Sessions::default().exchange(
            &missing,
            &rope,
            &request,
            &Budget::new(&cancel),
            |_, _| panic!("an absent worker cannot send results"),
        );
        assert!(matches!(
            result,
            Err(VisitError::Search(RegexError::Launch))
        ));
    }

    #[test]
    fn newer_viewports_coalesce_without_retiring_the_query() {
        use crate::{Collector, Navigation, Options, Scope};
        let summary = Collector::new(100, Navigation::NextFrom(0), 0..10, None).finish();
        let sessions = Sessions::default();
        let cancel = AtomicBool::new(false);
        let budget = Budget::new(&cancel);
        let mut request = wire::Request {
            subject_len: 100,
            pattern: "x".into(),
            options: Options::default(),
            scope: Scope::Document,
            operation: wire::Operation::Viewport(summary.viewport(0..10)),
        };
        let first = sessions.reserve(&request, &budget).unwrap();
        request.operation = wire::Operation::Viewport(summary.viewport(50..60));
        let second = sessions.reserve(&request, &budget).unwrap();
        assert_eq!(first.0, second.0);
        assert_eq!(
            sessions.ticket(first.0, first.1),
            Err(RegexError::Cancelled)
        );
        sessions.ticket(second.0, second.1).unwrap();
        sessions.retire();
        assert_eq!(
            sessions.ticket(second.0, second.1),
            Err(RegexError::Cancelled)
        );
    }
}
