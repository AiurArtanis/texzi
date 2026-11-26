use super::{Document, SaveError, Shell, settings};
use nsh_core::prelude::*;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

const EDIT_DELAY: Duration = Duration::from_millis(1000);
const WAIT_FOR_SAVE: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Delay,
    Blur,
}

#[derive(PartialEq, Eq)]
struct Target {
    mode: Mode,
    path: PathBuf,
    revision: u64,
}

pub(super) struct State {
    blurred: Signal<bool>,
    paused: Signal<bool>,
    generation: Cell<u64>,
    pending: RefCell<Option<Target>>,
}

impl State {
    pub(super) fn new() -> Self {
        Self {
            blurred: signal(false),
            paused: signal(false),
            generation: Cell::new(0),
            pending: RefCell::new(None),
        }
    }

    pub(super) fn focus_changed(&self, focused: bool) {
        self.blurred.set(!focused);
    }

    pub(super) fn after_explicit_save(&self) {
        self.blurred.set(false);
        self.paused.set(false);
    }

    fn change_target(&self, target: Option<Target>) -> Option<(u64, Mode)> {
        let mut pending = self.pending.borrow_mut();
        if *pending == target {
            return None;
        }
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let schedule = target.as_ref().map(|target| (generation, target.mode));
        *pending = target;
        schedule
    }
}

fn mode(shell: &Shell) -> Option<Mode> {
    match shell.preference(settings::AUTO_SAVE).as_str() {
        Some("afterDelay") => Some(Mode::Delay),
        Some("onFocusChange") => Some(Mode::Blur),
        _ => None,
    }
}

fn target(shell: &Shell, doc: &Document, mode: Option<Mode>) -> Option<Target> {
    let state = &doc.auto_save;
    if mode != Some(Mode::Blur) {
        state.blurred.set(false);
    }
    let mode = mode?;
    let path = doc.path.get()?;
    if state.paused.get() || !doc.source.has_disk_baseline() {
        return None;
    }
    if mode == Mode::Blur && !state.blurred.get() {
        return None;
    }
    if doc.gate.busy()
        || shell.split.with(|split| {
            split
                .as_ref()
                .is_some_and(|other| other.id == doc.id && other.gate.busy())
        })
    {
        return None;
    }
    let revision = match doc.source.dirty_revision() {
        Some(revision) => revision,
        None => {
            state.blurred.set(false);
            return None;
        }
    };
    Some(Target {
        mode,
        path,
        revision,
    })
}

pub(super) fn install(shell: &Rc<Shell>) {
    let shell = Rc::downgrade(shell);
    effect(move || {
        let Some(shell) = shell.upgrade() else { return };
        let mode = mode(&shell);
        for doc in shell.docs.get() {
            let target = target(&shell, &doc, mode);
            if let Some((generation, when)) = doc.auto_save.change_target(target) {
                let delay = if when == Mode::Delay {
                    EDIT_DELAY
                } else {
                    Duration::ZERO
                };
                schedule(&shell, &doc, generation, delay);
            }
        }
    });
}

fn schedule(shell: &Rc<Shell>, doc: &Document, generation: u64, delay: Duration) {
    let shell = Rc::downgrade(shell);
    let id = doc.id;
    let state = Rc::downgrade(&doc.auto_save);
    set_timeout(delay, move || {
        let (Some(shell), Some(state)) = (shell.upgrade(), state.upgrade()) else {
            return;
        };
        if state.generation.get() != generation {
            return;
        }
        let Some(doc) = shell
            .docs
            .with(|docs| docs.iter().find(|doc| doc.id == id).cloned())
        else {
            return;
        };
        let current = target(&shell, &doc, mode(&shell));
        if current.as_ref() != state.pending.borrow().as_ref() {
            return;
        }
        let Some(Target { path, .. }) = current else {
            return;
        };
        if shell.modal.get() || shell.auto_saving.get().is_some() {
            schedule(&shell, &doc, generation, WAIT_FOR_SAVE);
            return;
        }
        shell.auto_saving.set(Some(id));
        spawn(async move {
            let saved = doc.source.save_file(path, true, false).await;
            match saved {
                Ok(()) => {
                    if !doc.source.is_dirty().unwrap_or(true) {
                        state.blurred.set(false);
                    }
                    shell.show_saved(&doc);
                }
                Err(error) => {
                    state.paused.set(true);
                    state.blurred.set(false);
                    let info = match error {
                        SaveError::Encoding => {
                            "当前编码无法保存部分字符，请手动保存并检查编码。".into()
                        }
                        SaveError::Other(info) => info,
                    };
                    shell
                        .notice
                        .set(format!("已暂停 {} 的自动保存 · {info}", doc.name()));
                }
            }
            shell.auto_saving.set(None);
        });
    });
}
