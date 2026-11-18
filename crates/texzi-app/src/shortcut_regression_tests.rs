//! Headless NSH Core/keymap coverage of the production shell shortcut helper.
//! The Input's focus event and the platform's `editing` flag are supplied here;
//! this does not exercise Win32 key delivery, native menus or a visible dialog.
use super::{CHORDS, SHELL_SHORTCUTS, bind_shell_shortcuts};
use nsh_core::{
    ApproxTextEngine, Command, Core, EventKind, EventPayload, MountItem, NodeId, PayloadValue,
    ROOT_ID, Size, ViewKind,
    element::{Environment, Root, input, view},
    file_dialog::{FileRequest, OpenDialog, OpenDialogKind},
    geometry::px,
    keymap::{KeyMatch, Keymap, Keystroke},
    platform::Os,
    prelude::*,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

struct Probe {
    install_shell: bool,
    calls: RefCell<Vec<String>>,
    resets: Cell<usize>,
    focus_events: Cell<usize>,
    input_changes: Cell<usize>,
    answers: Cell<usize>,
}

impl Probe {
    fn new(install_shell: bool) -> Rc<Self> {
        Rc::new(Self {
            install_shell,
            calls: RefCell::new(Vec::new()),
            resets: Cell::new(0),
            focus_events: Cell::new(0),
            input_changes: Cell::new(0),
            answers: Cell::new(0),
        })
    }

    fn run(self: &Rc<Self>, id: &str) {
        self.calls.borrow_mut().push(id.into());
        let dialog = match id {
            "texzi.file.open" => OpenDialog::files().multiple(true),
            "texzi.folder.open" => OpenDialog::folders(),
            _ => return,
        };
        // Core emits a FileRequest mount command. No platform backend consumes
        // it in this fixture, so no OS dialog or filesystem access occurs.
        let answer = pick_paths(dialog);
        let probe = self.clone();
        spawn(async move {
            assert_eq!(answer.await.unwrap(), None);
            probe.answers.set(probe.answers.get() + 1);
        });
    }
}

thread_local! {
    static PROBE: RefCell<Option<Rc<Probe>>> = const { RefCell::new(None) };
}

#[component]
fn ShortcutFixture() -> impl IntoElement {
    let probe = PROBE.with(|p| p.borrow().as_ref().unwrap().clone());
    let owner = Owner::current();
    let armed = signal(true);
    let reset = probe.clone();
    // Use the actual early, pass-through Zen observers, not a synthetic
    // consumed shortcut that could make the old negative control misleading.
    crate::zen_state::bind_reset_on_other_keys(
        armed,
        Rc::new(move || reset.resets.set(reset.resets.get() + 1)),
    );

    if probe.install_shell {
        let run = probe.clone();
        bind_shell_shortcuts(Rc::new(move |id| owner.run(|| run.run(id))));

        // Reproduce the existing chord's dynamic-owner registration boundary.
        // Its target comes from the production CHORDS table. The full shell's
        // timeout, modal and IME handling are not replaced or claimed here.
        let prefix = signal(false);
        on_key("mod+k", move |_| prefix.set(true));
        let run = probe.clone();
        effect(move || {
            if !prefix.get() {
                return;
            }
            let target = CHORDS
                .iter()
                .find(|(key, _)| *key == "mod+o")
                .map(|(_, id)| *id)
                .expect("the existing Ctrl+K Ctrl+O command remains defined");
            let run = run.clone();
            on_key("mod+o", move |_| {
                // Shell.run likewise moves service creation and its await to
                // the long-lived shell owner before this prefix is disposed.
                owner.run(|| run.run(target));
                prefix.set(false);
            });
        });
    }

    let focused = probe.clone();
    let changed = probe.clone();
    view().child(
        input()
            .value("保留输入框正文".to_owned())
            .auto_focus(true)
            .on_focus(move |_: &()| focused.focus_events.set(focused.focus_events.get() + 1))
            .on_change(move |_: &str| changed.input_changes.set(changed.input_changes.get() + 1)),
    )
}

struct Harness {
    core: Core<ApproxTextEngine>,
    probe: Rc<Probe>,
    keys: Option<Arc<Keymap>>,
    input: Option<NodeId>,
    requests: Vec<(u32, OpenDialogKind, bool)>,
}

impl Harness {
    fn new(install_shell: bool) -> Self {
        let probe = Probe::new(install_shell);
        PROBE.with(|p| *p.borrow_mut() = Some(probe.clone()));
        let mut core = Core::new(
            ApproxTextEngine,
            Size {
                width: px(800.0),
                height: px(600.0),
            },
        );
        let mut environment = Environment::default();
        environment.os = Os::Windows;
        environment.file_requests = true;
        core.set_environment(environment);
        core.mount_root(&Root::new::<ShortcutFixture>(|| ()));
        let mut fixture = Self {
            core,
            probe,
            keys: None,
            input: None,
            requests: Vec::new(),
        };
        fixture.flush();
        let input = fixture.input.expect("a real NSH Input was mounted");
        assert!(
            fixture
                .core
                .component_event(input, EventKind::Focus, &EventPayload::None)
        );
        fixture.flush();
        assert_eq!(fixture.probe.focus_events.get(), 1);
        fixture
    }

    fn flush(&mut self) {
        for _ in 0..4 {
            self.core.run_component_tasks();
            if let Some(batch) = self.core.commit() {
                for item in batch.items {
                    match item {
                        MountItem::Keymap(keys) => self.keys = Some(keys),
                        MountItem::Create {
                            id,
                            kind: ViewKind::Input,
                        } => self.input = Some(id),
                        MountItem::Command {
                            command: Command::FileRequest { id, request },
                            ..
                        } => {
                            let FileRequest::OpenDialog(dialog) = *request else {
                                panic!("shortcut fixture must only emit an OpenDialog request");
                            };
                            self.requests.push((id, dialog.kind, dialog.multiple));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn matched(&self, text: &str, repeat: bool) -> KeyMatch {
        let stroke = Keystroke::parse_with(text, false).unwrap();
        self.keys
            .as_ref()
            .expect("Core emitted its actual registered keymap")
            .find(stroke, true, repeat)
    }

    fn press(&mut self, text: &str, repeat: bool) -> KeyMatch {
        let matched = self.matched(text, repeat);
        if let Some(binding) = matched.fired() {
            assert!(self.core.component_event(
                ROOT_ID,
                EventKind::KeyCommand,
                &EventPayload::Value(PayloadValue::Number(binding as f64)),
            ));
            self.flush();
        }
        matched
    }

    fn cancel_request(&mut self, id: u32) {
        assert!(self.core.component_event(
            ROOT_ID,
            EventKind::FileRequest,
            &EventPayload::file_answer(id, Vec::new()),
        ));
        self.flush();
    }
}

#[test]
fn actual_shell_ctrl_o_fires_once_in_input_and_held_repeat_emits_no_second_open_request() {
    assert_eq!(
        SHELL_SHORTCUTS
            .iter()
            .filter(|(key, id)| *key == "mod+o" && *id == "texzi.file.open")
            .count(),
        1,
    );
    let mut fixture = Harness::new(true);
    let matched = fixture.press("ctrl+o", false);
    assert!(matches!(matched, KeyMatch::Fire(_)));
    assert!(
        matched.consumed(),
        "the platform must stop native-menu fallback"
    );
    assert_eq!(fixture.probe.calls.borrow().as_slice(), ["texzi.file.open"]);
    assert_eq!(
        fixture.probe.resets.get(),
        0,
        "the early observer did not mask Open"
    );
    assert_eq!(fixture.requests.len(), 1);
    let (request, kind, multiple) = fixture.requests[0];
    assert_eq!(kind, OpenDialogKind::Files);
    assert!(multiple);

    assert_eq!(fixture.press("ctrl+o", true), KeyMatch::Held);
    assert_eq!(fixture.probe.calls.borrow().len(), 1);
    assert_eq!(
        fixture.requests.len(),
        1,
        "a held key did not issue another request"
    );
    assert_eq!(fixture.probe.input_changes.get(), 0);
    fixture.cancel_request(request);
    assert_eq!(fixture.probe.answers.get(), 1);
}

#[test]
fn observer_only_old_ctrl_o_path_notifies_without_consuming_or_opening() {
    let mut fixture = Harness::new(false);
    for repeat in [false, true] {
        let matched = fixture.press("ctrl+o", repeat);
        assert!(matches!(matched, KeyMatch::Notify(_)));
        assert!(
            !matched.consumed(),
            "Notify must leave native menu handling available"
        );
    }
    assert_eq!(fixture.probe.resets.get(), 2);
    assert!(fixture.probe.calls.borrow().is_empty());
    assert!(fixture.requests.is_empty());
    assert_eq!(fixture.probe.input_changes.get(), 0);
}

#[test]
fn dynamic_ctrl_k_ctrl_o_folder_binding_precedes_file_open_then_cleans_up() {
    let mut fixture = Harness::new(true);
    let original = fixture.matched("ctrl+o", false);
    assert!(matches!(original, KeyMatch::Fire(_)));
    assert!(matches!(fixture.press("ctrl+k", false), KeyMatch::Fire(_)));
    let chord = fixture.matched("ctrl+o", false);
    assert!(matches!(chord, KeyMatch::Fire(_)));
    assert_ne!(
        chord, original,
        "the newer dynamic chord owns the second key"
    );
    assert_eq!(fixture.press("ctrl+o", false), chord);
    assert_eq!(
        fixture.probe.calls.borrow().as_slice(),
        ["texzi.folder.open"]
    );
    assert_eq!(fixture.requests.len(), 1);
    let folder_request = fixture.requests[0];
    assert_eq!(folder_request.1, OpenDialogKind::Folders);
    assert!(!folder_request.2);
    fixture.cancel_request(folder_request.0);

    assert_eq!(fixture.matched("ctrl+o", false), original);
    assert_eq!(fixture.press("ctrl+o", false), original);
    assert_eq!(
        fixture.probe.calls.borrow().as_slice(),
        ["texzi.folder.open", "texzi.file.open"],
    );
    assert_eq!(fixture.requests.len(), 2);
    let file_request = fixture.requests[1];
    assert_eq!(file_request.1, OpenDialogKind::Files);
    assert!(file_request.2);
    fixture.cancel_request(file_request.0);
    assert_eq!(fixture.probe.answers.get(), 2);
    assert_eq!(fixture.probe.input_changes.get(), 0);
}
