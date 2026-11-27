//! Disk-backed Undo/Redo. Only the immutable load ticket runs off the owning
//! thread. The requesting pane and Store must still accept the same operation.
use super::{NativeView, Owner, doc, err};
use nsh_base::editor::{ExternalDocument, ExternalUpdate};
use nsh_core::prelude::{Owner as UiOwner, Signal, blocking, effect, spawn};
use std::{
    cell::Cell,
    rc::{Rc, Weak},
};

const LOADING: &str = "正在读取撤销或重做历史…";

pub(super) struct Target {
    shell_owner: UiOwner,
    status: Signal<String>,
    active: Box<dyn Fn() -> bool>,
    generation: Cell<u64>,
}

/// The shell can keep a closed tab to reopen it. A live Store or NativeView
/// alone therefore cannot authorize delivery to the original editor pane.
impl NativeView {
    pub(crate) fn bind_history_target(
        &self,
        shell_owner: UiOwner,
        status: Signal<String>,
        active: impl Fn() -> bool + 'static,
    ) {
        let target = Rc::new(Target {
            shell_owner,
            status,
            active: Box::new(active),
            generation: Cell::new(0),
        });
        *self.history_target.borrow_mut() = Some(target.clone());
        effect(move || {
            if !(target.active)() {
                target
                    .generation
                    .set(target.generation.get().wrapping_add(1));
            }
        });
    }

    pub(super) fn request_owned_history(
        self: &Rc<Self>,
        revision: u64,
        epoch: u64,
        undo: bool,
    ) -> Result<Option<ExternalUpdate>, String> {
        // The worker may already have revived its weak-cached root. Reserve
        // the one publication before probing residency again on a repeat key.
        if self.owner.loading_history.get() {
            let target = self.history_target.borrow().clone();
            if target
                .as_ref()
                .is_some_and(|ui| ui.status.try_get().as_deref() == Some(LOADING))
            {
                return Ok(None);
            }
            let message = "已有一次撤销或重做正在读取历史，请稍后重试";
            if let Some(ui) = target.as_ref() {
                ui.status.try_set(message.into());
            }
            return Err(message.into());
        }
        let job = {
            let mut store = self.owner.store.borrow_mut();
            let view = store.view(self.view).map_err(err)?;
            if store.snapshot(view.doc).map_err(err)?.revision.0 != revision {
                return Err(err(doc::Error::StaleRevision));
            }
            match store
                .take_history_step(self.view, epoch, !undo)
                .map_err(err)?
            {
                doc::HistoryStep::Ready(receipt) => {
                    return self.history_update(&store, receipt, undo).map(Some);
                }
                doc::HistoryStep::Cold(job) => job,
            }
        };
        let target = self
            .history_target
            .borrow()
            .clone()
            .filter(|ui| (ui.active)())
            .ok_or("原编辑组已关闭或已切换，请在当前正文重试")?;
        self.owner.loading_history.set(true);
        let pending = Loading(Rc::downgrade(&self.owner));
        let generation = target.generation.get();
        let requester = Rc::downgrade(self);
        target.status.set(LOADING.into());
        let shell = target.shell_owner;
        shell.run(|| {
            spawn(async move {
                let result = blocking(move || job.load()).await;
                // The pane may have closed, changed mode, moved to another group
                // or started composition while this exact root was being read.
                let view = requester.upgrade().filter(|view| {
                    view.history_target
                        .borrow()
                        .as_ref()
                        .is_some_and(|t| Rc::ptr_eq(t, &target))
                        && target.generation.get() == generation
                        && (target.active)()
                });
                let result = match view {
                    Some(view)
                        if view
                            .search_snapshot()
                            .is_ok_and(|(s, e)| s.revision.0 == revision && e == epoch) =>
                    {
                        result
                            .map_err(|_| "读取撤销或重做历史的任务未完成".to_string())
                            .and_then(|job| {
                                job.map_err(|e| format!("无法读取撤销或重做历史（{e:?}）"))
                            })
                            .and_then(|loaded| view.publish_loaded_history(loaded))
                    }
                    changed => {
                        // Discard a stale large Rope and provenance tree on the worker.
                        let _ = blocking(move || drop(result)).await;
                        if changed.is_some() {
                            Err("撤销或重做未应用，正文或选区已变化".into())
                        } else {
                            Ok(())
                        }
                    }
                };
                drop(pending);
                if target.status.try_get().as_deref() == Some(LOADING) {
                    target.status.try_set(result.err().unwrap_or_default());
                }
            })
        });
        Ok(None)
    }

    fn publish_loaded_history(&self, loaded: doc::LoadedBulkHistory) -> Result<(), String> {
        let receipt = self
            .owner
            .store
            .borrow_mut()
            .submit_loaded_history(loaded)
            .map_err(|e| match e {
                doc::Error::StaleView | doc::Error::StaleRevision | doc::Error::InvalidView => {
                    "撤销或重做未应用，正文或选区已变化".into()
                }
                e => format!("无法应用撤销或重做历史（{e:?}）"),
            })?;
        self.advance_search_scopes(&receipt);
        self.notify();
        Ok(())
    }

    pub(crate) fn loading_history_status(&self, status: &str) -> bool {
        self.owner.loading_history.get() && status == LOADING
    }
}

struct Loading(Weak<Owner>);

impl Drop for Loading {
    fn drop(&mut self) {
        if let Some(owner) = self.0.upgrade() {
            owner.loading_history.set(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{editor_owner::NativeView, rich_owner::ModeGate};
    use nsh_base::editor::{ExternalEditKind, ExternalSelections};
    use nsh_core::{
        ApproxTextEngine, Core, Op, Size,
        element::{Root, RustCall, RustOp, ToJs},
        prelude::*,
    };
    use std::{
        cell::RefCell,
        sync::{Arc, atomic::AtomicBool},
        time::{Duration, Instant},
    };

    #[component]
    fn TestWindow() -> impl IntoElement {
        view()
    }

    // Run through the actual Core dispatcher so background work and signal
    // notifications have the same owning thread as an editor command.
    fn on_ui<R: 'static>(core: &mut Core<ApproxTextEngine>, f: impl FnOnce() -> R + 'static) -> R {
        let result = Rc::new(RefCell::new(None));
        let out = result.clone();
        let call = RefCell::new(Some(f));
        core.apply(Op::Rust(RustOp {
            call: 1,
            run: RustCall::Op(Box::new(move |_| {
                *out.borrow_mut() = Some(call.borrow_mut().take().unwrap()());
                Ok(Box::new(()) as Box<dyn ToJs>)
            })),
        }))
        .unwrap();
        result.take().unwrap()
    }

    struct Fixture {
        writer: Rc<NativeView>,
        other: Rc<NativeView>,
        gate: Rc<ModeGate>,
        active: Signal<bool>,
    }

    fn setup() -> (tokio::runtime::Runtime, Core<ApproxTextEngine>, Rc<Fixture>) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut core = Core::new(
            ApproxTextEngine,
            Size {
                width: px(800.),
                height: px(600.),
            },
        );
        core.set_task_runtime(runtime.handle().clone());
        core.mount_root(&Root::new::<TestWindow>(|| ()));
        let f = on_ui(&mut core, || {
            let mut store = doc::Store::new(doc::HistoryBudget::default());
            store
                .set_observer_limits(doc::ObserverLimits {
                    journal_batches: 0,
                    ..Default::default()
                })
                .unwrap();
            // One opaque Markdown block also exercises the real rich owner's
            // projection without millions of inline formatting operations.
            let original = format!("<!--{}-->\n\nbegin\n", "a".repeat(6 * 1024 * 1024));
            let snapshot = store
                .open_utf8(&original, doc::Profile::default(), false)
                .unwrap();
            let writer = NativeView::from_store(store, snapshot, Default::default()).unwrap();
            let other = writer.sibling_source().unwrap();
            let gate = ModeGate::new(other.clone(), other.sibling().unwrap());
            let active = signal(true);
            gate.bind_history_pane(UiOwner::current(), move || active.get());
            let (snapshot, epoch) = writer.search_snapshot().unwrap();
            let len = snapshot.body.len_bytes();
            let mut plan =
                doc::PreparedBulkBuilder::new(snapshot, Arc::new(AtomicBool::new(false)));
            plan.replace(0..len, [doc::BulkPart::Literal("after\n")])
                .unwrap();
            writer
                .submit_find_replace(epoch, plan.finish().unwrap())
                .unwrap();
            assert!(!writer.owner.loading_history.get());
            Rc::new(Fixture {
                writer,
                other,
                gate,
                active,
            })
        });
        (runtime, core, f)
    }

    fn complete(core: &mut Core<ApproxTextEngine>, f: &Fixture) {
        let until = Instant::now() + Duration::from_secs(10);
        while f.writer.owner.loading_history.get() {
            core.run_component_tasks();
            core.commit();
            assert!(
                Instant::now() < until,
                "background history failed to settle"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        core.run_component_tasks();
        core.commit();
    }

    #[test]
    fn cold_history_source_key_and_rich_menu_publish_shared_revisions_once() {
        let (_runtime, mut core, f) = setup();
        let origin = f.other.snapshot().unwrap().revision;
        let key = f.clone();
        on_ui(&mut core, move || {
            let s = key.other.snapshot().unwrap();
            // Call the same trait object entry that NSH Buffer uses for Ctrl+Z.
            let binding = key.other.buffer();
            assert!(
                binding
                    .0
                    .clone()
                    .request_history(s.revision, s.view_epoch, true)
                    .unwrap()
                    .is_none()
            );
            assert!(
                binding
                    .0
                    .request_history(s.revision, s.view_epoch, true)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(key.writer.snapshot().unwrap().revision, origin);
            assert_eq!(key.gate.status.get(), LOADING);
        });
        complete(&mut core, &f);
        assert_eq!(f.other.snapshot().unwrap().revision, origin + 1);
        assert!(f.writer.snapshot().unwrap().rope.len_bytes() > 6 * 1024 * 1024);
        let menu = f.clone();
        on_ui(&mut core, move || {
            let source = menu.other.snapshot().unwrap();
            let in_paragraph = source.rope.len_bytes() - 3;
            menu.other
                .select(
                    source.revision,
                    source.view_epoch,
                    ExternalSelections {
                        primary: 0,
                        ranges: vec![(in_paragraph, in_paragraph)],
                    },
                )
                .unwrap();
            menu.gate.to_rich();
            assert!(menu.gate.rich_mode.get(), "{}", menu.gate.status.get());
            menu.gate.shell_history(false).unwrap();
            assert!(menu.writer.owner.loading_history.get());
            assert_eq!(menu.other.snapshot().unwrap().revision, origin + 1);
        });
        complete(&mut core, &f);
        let observed = f.clone();
        on_ui(&mut core, move || {
            let s = observed.writer.snapshot().unwrap();
            assert_eq!(s.revision, origin + 2);
            assert_eq!(s.rope.slice(..), "after\n");
            assert!(observed.gate.status.get().is_empty());
            assert_eq!(
                observed
                    .gate
                    .session()
                    .state()
                    .doc()
                    .child(0)
                    .text_content(),
                "after"
            );
        });
    }

    #[test]
    fn closed_pane_and_sibling_edits_discard_cold_undo_without_consuming_it() {
        let (_runtime, mut core, f) = setup();
        let origin = f.writer.snapshot().unwrap().revision;
        let close = f.clone();
        on_ui(&mut core, move || {
            close.gate.shell_history(true).unwrap();
            close.active.set(false);
        });
        complete(&mut core, &f);
        assert_eq!(f.writer.snapshot().unwrap().revision, origin);
        let edit = f.clone();
        on_ui(&mut core, move || {
            edit.active.set(true);
            edit.gate.shell_history(true).unwrap();
            let s = edit.writer.snapshot().unwrap();
            edit.writer
                .edit(
                    s.revision,
                    s.view_epoch,
                    vec![(0..0, "!".into())],
                    ExternalSelections {
                        primary: 0,
                        ranges: vec![(1, 1)],
                    },
                    ExternalEditKind::Typing,
                )
                .unwrap();
            edit.writer.notify();
        });
        complete(&mut core, &f);
        let verified = f.clone();
        on_ui(&mut core, move || {
            let s = verified.writer.snapshot().unwrap();
            assert_eq!(s.revision, origin + 1);
            assert_eq!(s.rope.slice(..), "!after\n");
            assert!(verified.gate.status.get().contains("未应用"));
            assert_eq!(verified.other.can_history(), (true, false));
            // The intervening conventional step is still synchronous.
            verified.gate.shell_history(true).unwrap();
            assert!(!verified.writer.owner.loading_history.get());
            assert_eq!(
                verified.writer.snapshot().unwrap().rope.slice(..),
                "after\n"
            );
        });
    }
}
