use super::counter::Counter;
use nsh_base::rich_editor::model::Node;
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const DEBOUNCE: Duration = Duration::from_millis(60);

pub(super) enum Input {
    Source(Arc<texzi_document::Body>),
    Rich(Node),
}

impl Input {
    fn count(&self, mut cancelled: impl FnMut() -> bool) -> Option<u64> {
        let mut counter = Counter::default();
        match self {
            Self::Source(body) => {
                for chunk in body.source().chunks() {
                    if !counter.push(chunk, &mut cancelled) {
                        return None;
                    }
                }
            }
            Self::Rich(root) => {
                // Keep a child iterator per ancestor, even for very wide documents.
                // Marked adjacent text nodes share the same counter state. Links
                // and images never contribute their hidden URL or attributes.
                let mut stack = vec![(std::slice::from_ref(root).iter(), false)];
                while let Some((children, block)) = stack.last_mut() {
                    if cancelled() {
                        return None;
                    }
                    let Some(node) = children.next() else {
                        if *block {
                            counter.boundary();
                        }
                        stack.pop();
                        continue;
                    };
                    if let Some(text) = node.text() {
                        if !counter.push(text, &mut cancelled) {
                            return None;
                        }
                    } else if node.is_leaf() {
                        // hardBreak, images and horizontal rules separate text.
                        counter.boundary();
                    } else {
                        let block = node.is_block();
                        if block {
                            counter.boundary();
                        }
                        stack.push((node.content().iter(), block));
                    }
                }
            }
        }
        (!cancelled()).then(|| counter.total())
    }
}

struct Pending {
    generation: u64,
    after: Instant,
    input: Input,
}

#[derive(Default)]
struct Queue {
    active: bool,
    running: bool,
    pending: Option<Pending>,
    result: Option<(u64, u64)>,
    retry_spawn_after: Option<Instant>,
}

#[derive(Default)]
struct Shared {
    generation: AtomicU64,
    queue: Mutex<Queue>,
    wake: Condvar,
}

impl Shared {
    fn start_if_needed(self: &Arc<Self>, q: &mut Queue) {
        if q.running
            || q.pending.is_none()
            || q.retry_spawn_after.is_some_and(|at| Instant::now() < at)
        {
            return;
        }
        let worker = self.clone();
        if thread::Builder::new()
            .name("texzi-zen-count".into())
            .spawn(move || worker.run())
            .is_ok()
        {
            q.running = true;
            q.retry_spawn_after = None;
        } else {
            // Keep the real input. A later poll can retry thread creation.
            q.retry_spawn_after = Some(Instant::now() + Duration::from_secs(1));
        }
    }

    fn run(&self) {
        loop {
            let job = {
                let mut q = self.queue.lock().unwrap();
                loop {
                    if !q.active {
                        q.running = false;
                        return;
                    }
                    if let Some(pending) = &q.pending {
                        let wait = pending.after.saturating_duration_since(Instant::now());
                        if wait.is_zero() {
                            break q.pending.take().unwrap();
                        }
                        q = self.wake.wait_timeout(q, wait).unwrap().0;
                    } else {
                        q = self.wake.wait(q).unwrap();
                    }
                }
            };
            let generation = job.generation;
            let current = || self.generation.load(Ordering::Acquire) == generation;
            let result = job.input.count(|| !current());
            drop(job);
            if let Some(total) = result {
                let mut q = self.queue.lock().unwrap();
                // submit/cancel also change the generation under this lock.
                if current() {
                    q.result = Some((generation, total));
                }
            }
        }
    }
}

#[derive(Default)]
pub(super) struct Jobs {
    shared: Arc<Shared>,
}

impl Jobs {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn submit(&self, input: Input) -> u64 {
        let mut q = self.shared.queue.lock().unwrap();
        let generation = self
            .shared
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        let old = q.pending.replace(Pending {
            generation,
            after: Instant::now() + DEBOUNCE,
            input,
        });
        q.active = true;
        q.result = None;
        self.shared.start_if_needed(&mut q);
        drop(q);
        self.shared.wake.notify_one();
        drop(old);
        generation
    }

    pub(super) fn cancel(&self) {
        let mut q = self.shared.queue.lock().unwrap();
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        q.active = false;
        q.result = None;
        let old = q.pending.take();
        drop(q);
        self.shared.wake.notify_one();
        drop(old);
    }

    pub(super) fn take(&self, generation: u64) -> Option<u64> {
        let mut q = self.shared.queue.lock().unwrap();
        if self.shared.generation.load(Ordering::Acquire) != generation {
            return None;
        }
        self.shared.start_if_needed(&mut q);
        q.result
            .take_if(|(id, _)| *id == generation)
            .map(|(_, total)| total)
    }
}

impl Drop for Jobs {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(text: &str) -> Input {
        Input::Source(Arc::new(texzi_document::Body::import_utf8(
            text,
            Default::default(),
        )))
    }

    #[test]
    fn rich_count_joins_marks_and_splits_breaks_blocks_and_cells() {
        let manager = crate::rich_projection::manager();
        let node = manager.schema().node_from_json(&serde_json::json!({
            "type":"doc", "content":[
                {"type":"paragraph","content":[
                    {"type":"text","text":"al"},
                    {"type":"text","text":"pha","marks":[{"type":"bold"}]},
                    {"type":"hardBreak"},
                    {"type":"text","text":"link","marks":[{"type":"link","attrs":{"href":"https://hidden.example/URL_ONLY"}}]}
                ]},
                {"type":"table","content":[{"type":"tableRow","content":[
                    {"type":"tableCell","content":[{"type":"paragraph","content":[{"type":"text","text":"left"}]}]},
                    {"type":"tableCell","content":[{"type":"paragraph","content":[{"type":"text","text":"right"}]}]}
                ]}]}
            ]
        })).unwrap();
        assert_eq!(Input::Rich(node).count(|| false), Some(4));
    }

    #[test]
    fn superseded_results_cannot_replace_or_consume_the_latest_count() {
        let jobs = Jobs::new();
        let old = jobs.submit(source("old"));
        let newest = jobs.submit(source("山河 alpha"));
        let until = Instant::now() + Duration::from_secs(5);
        let total = loop {
            assert_eq!(jobs.take(old), None);
            if let Some(total) = jobs.take(newest) {
                break total;
            }
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(total, 3);
        assert_eq!(jobs.take(newest), None);
        let cancelled = jobs.submit(source("obsolete"));
        jobs.cancel();
        while jobs.shared.queue.lock().unwrap().running {
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(jobs.take(cancelled), None);
    }
}
