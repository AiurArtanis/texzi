//! Real NSH Core/dialog/task ownership regression. No platform window is used.
use super::await_shell_modal;
use nsh_core::{
    ApproxTextEngine, Command, Core, EventKind, EventPayload, MountItem, NodeId, Op, PayloadValue,
    ROOT_ID, Size,
    element::{Root, RustCall, RustOp, ToJs, view},
    geometry::px,
    prelude::*,
};
use std::{
    cell::{Cell, RefCell},
    future::Future,
    pin::Pin,
    rc::{Rc, Weak},
    task::{Context, Poll},
};

struct Probe {
    stable: bool,
    shell: Cell<Owner>,
    panel: Cell<Option<Signal<bool>>>,
    modal: Rc<Cell<bool>>,
    transient_disposed: Cell<bool>,
    show_under_shell: Cell<bool>,
    done_under_shell: Cell<bool>,
    done: Cell<usize>,
    drops: Rc<Cell<usize>>,
    polls: Rc<Cell<usize>>,
    lease: RefCell<Option<Weak<()>>>,
}
impl Probe {
    fn new(stable: bool) -> Rc<Self> {
        Rc::new(Self {
            stable,
            shell: Cell::new(Owner::detached()),
            panel: Cell::new(None),
            modal: Rc::new(Cell::new(false)),
            transient_disposed: Cell::new(false),
            show_under_shell: Cell::new(false),
            done_under_shell: Cell::new(false),
            done: Cell::new(0),
            drops: Rc::new(Cell::new(0)),
            polls: Rc::new(Cell::new(0)),
            lease: RefCell::new(None),
        })
    }
}
thread_local! { static PROBE: RefCell<Option<Rc<Probe>>> = const { RefCell::new(None) }; }

struct TrackedAnswer<F> {
    inner: Pin<Box<F>>,
    drops: Rc<Cell<usize>>,
    polls: Rc<Cell<usize>>,
    _lease: Rc<()>,
}
impl<F: Future> Future for TrackedAnswer<F> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.polls.set(this.polls.get() + 1);
        this.inner.as_mut().poll(cx)
    }
}
impl<F> Drop for TrackedAnswer<F> {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

#[component]
fn TaskFixture() -> impl IntoElement {
    let probe = PROBE.with(|p| p.borrow().as_ref().unwrap().clone());
    let owner = Owner::current();
    let panel = signal(true);
    probe.shell.set(owner);
    probe.panel.set(Some(panel));
    view().child(when(panel, move || {
        let probe = probe.clone();
        view().on_press(move |_: &()| {
            // The palette row closes its own transient branch before explain.
            let transient = Owner::current();
            panel.set(false);
            transient.dispose();
            probe.transient_disposed.set(!transient.is_live());
            let shown = probe.clone();
            let show = move || {
                shown
                    .show_under_shell
                    .set(format!("{:?}", Owner::current()) == format!("{owner:?}"));
                let lease = Rc::new(());
                *shown.lease.borrow_mut() = Some(Rc::downgrade(&lease));
                let answer = nsh_base::alert_dialog()
                    .title("owner regression")
                    .description("unavailable command")
                    .action("ok", "知道了")
                    .close_on_escape(true)
                    .show();
                TrackedAnswer {
                    inner: Box::pin(answer),
                    drops: shown.drops.clone(),
                    polls: shown.polls.clone(),
                    _lease: lease,
                }
            };
            let completed = probe.clone();
            let done = move |result: Result<nsh_base::AlertDialogResult, String>| {
                assert!(result.is_ok_and(|answer| answer.action.as_deref() == Some("ok")));
                assert!(
                    !completed.modal.get(),
                    "production helper clears modal before focus/completion"
                );
                completed
                    .done_under_shell
                    .set(format!("{:?}", Owner::current()) == format!("{owner:?}"));
                completed.done.set(completed.done.get() + 1);
            };
            if probe.stable {
                await_shell_modal(&owner, probe.modal.clone(), show, done);
            } else {
                // Negative control: the original dead-listener scope cancels
                // NSH's real spawn. This must not masquerade as a passing fix.
                probe.modal.set(true);
                let answer = show();
                let modal = probe.modal.clone();
                spawn(async move {
                    let result = answer.await;
                    modal.set(false);
                    done(result);
                });
            }
        })
    }))
}

fn turns(core: &mut Core<ApproxTextEngine>) -> Vec<MountItem> {
    let mut items = Vec::new();
    for _ in 0..4 {
        core.run_component_tasks();
        if let Some(batch) = core.commit() {
            items.extend(batch.items);
        }
    }
    items
}
fn setup(stable: bool) -> (Core<ApproxTextEngine>, Rc<Probe>, NodeId) {
    let probe = Probe::new(stable);
    PROBE.with(|p| *p.borrow_mut() = Some(probe.clone()));
    let mut core = Core::new(
        ApproxTextEngine,
        Size {
            width: px(800.0),
            height: px(600.0),
        },
    );
    core.mount_root(&Root::new::<TaskFixture>(|| ()));
    let button = turns(&mut core)
        .iter()
        .find_map(|item| match item {
            MountItem::Events { id, events } if events.contains(EventKind::Press) => Some(*id),
            _ => None,
        })
        .expect("the transient palette row has a real NSH Press listener");
    (core, probe, button)
}
fn module(core: &mut Core<ApproxTextEngine>, call: u32, f: impl FnOnce() + 'static) {
    let run = RefCell::new(Some(f));
    core.apply(Op::Rust(RustOp {
        call,
        run: RustCall::Op(Box::new(move |_| {
            run.borrow_mut().take().unwrap()();
            Ok(Box::new(()) as Box<dyn ToJs>)
        })),
    }))
    .unwrap();
}
fn answer(core: &mut Core<ApproxTextEngine>, id: u32) -> bool {
    let handled = core.component_event(
        ROOT_ID,
        EventKind::Dialog,
        &EventPayload::Map(vec![
            ("id", PayloadValue::Number(id as f64)),
            ("index", PayloadValue::Number(0.0)),
        ]),
    );
    turns(core);
    handled
}

#[test]
fn shell_owned_dialog_survives_closed_palette_and_repeated_real_answers_clear_modal() {
    let (mut core, probe, mut button) = setup(true);
    let mut previous = None;
    for cycle in 1..=2 {
        assert!(core.component_event(button, EventKind::Press, &EventPayload::None));
        let id = turns(&mut core)
            .iter()
            .find_map(|item| match item {
                MountItem::Command {
                    command: Command::Dialog(dialog),
                    ..
                } => Some(dialog.id),
                _ => None,
            })
            .expect("actual AlertDialog has a Core answer id");
        assert!(probe.transient_disposed.get());
        assert!(probe.show_under_shell.get());
        assert!(probe.modal.get());
        assert_eq!(probe.done.get(), cycle - 1);
        if let Some(old) = previous {
            assert!(!answer(&mut core, old));
            assert!(probe.modal.get());
        }
        assert!(answer(&mut core, id));
        assert!(!probe.modal.get());
        assert_eq!(probe.done.get(), cycle);
        assert!(probe.done_under_shell.get());
        assert_eq!(probe.drops.get(), cycle);
        assert!(probe.lease.borrow().as_ref().unwrap().upgrade().is_none());
        previous = Some(id);
        if cycle == 1 {
            let reopen = probe.clone();
            module(&mut core, 1, move || {
                reopen
                    .shell
                    .get()
                    .run(|| reopen.panel.get().unwrap().set(true))
            });
            button = turns(&mut core)
                .iter()
                .find_map(|item| match item {
                    MountItem::Events { id, events } if events.contains(EventKind::Press) => {
                        Some(*id)
                    }
                    _ => None,
                })
                .expect("a new palette branch can be opened after the answer");
        }
    }
}

#[test]
fn dead_listener_negative_control_keeps_modal_stuck_even_after_real_dialog_answer() {
    let (mut core, probe, button) = setup(false);
    assert!(core.component_event(button, EventKind::Press, &EventPayload::None));
    let id = turns(&mut core)
        .iter()
        .find_map(|item| match item {
            MountItem::Command {
                command: Command::Dialog(dialog),
                ..
            } => Some(dialog.id),
            _ => None,
        })
        .expect("the old path can still show a dialog");
    assert!(probe.transient_disposed.get());
    assert!(!probe.show_under_shell.get());
    assert!(answer(&mut core, id));
    assert_eq!(probe.done.get(), 0, "dead-owner NSH task was cancelled");
    assert!(
        probe.modal.get(),
        "the original stuck-modal failure is actually reproduced"
    );
    assert_eq!(probe.drops.get(), 1);
    assert!(probe.lease.borrow().as_ref().unwrap().upgrade().is_none());
}

#[test]
fn disposing_shell_cancels_real_pending_dialog_task_and_releases_captured_future() {
    let (mut core, probe, button) = setup(true);
    assert!(core.component_event(button, EventKind::Press, &EventPayload::None));
    let id = turns(&mut core)
        .iter()
        .find_map(|item| match item {
            MountItem::Command {
                command: Command::Dialog(dialog),
                ..
            } => Some(dialog.id),
            _ => None,
        })
        .unwrap();
    assert!(probe.modal.get());
    assert!(
        probe.polls.get() > 0,
        "the actual NSH executor polled the pending answer"
    );
    assert_eq!(probe.drops.get(), 0);
    let close = probe.clone();
    module(&mut core, 1, move || close.shell.get().dispose());
    turns(&mut core);
    assert_eq!(probe.drops.get(), 1);
    assert!(probe.lease.borrow().as_ref().unwrap().upgrade().is_none());
    answer(&mut core, id); // a late platform response must not resurrect the task
    assert_eq!(probe.done.get(), 0);
    assert_eq!(probe.drops.get(), 1);
}
