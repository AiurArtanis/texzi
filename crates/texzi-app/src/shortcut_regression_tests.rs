//! Headless NSH Core/keymap coverage of the production shell shortcut helper.
//! The Input's focus event and the platform's `editing` flag are supplied here;
//! this does not exercise Win32 key delivery, native menus or a visible dialog.
use super::{CHORDS, SHELL_SHORTCUTS, bind_chord_continuations, bind_shell_shortcuts};
use crate::{editor_owner::NativeView, rich_owner::ModeGate};
use nsh_base::{
    editor::{ExternalDocument, ExternalEditKind, ExternalSelections},
    rich_editor::{
        state::{EditorState, GapCursor, KeyEvent, PluginSpec, Transaction},
        view::EditorSession,
    },
};
use nsh_core::{
    ApproxTextEngine, Command, Core, EventKind, EventPayload, MountItem, NodeId, Op, PayloadValue,
    ROOT_ID, Size, ViewKind,
    desktop::DesktopCommand,
    element::{Environment, Root, RustCall, RustOp, ToJs, input, view},
    file_dialog::{FileRequest, OpenDialog, OpenDialogKind},
    geometry::px,
    keymap::{KeyBinding, KeyMatch, Keymap, Keystroke, When},
    platform::Os,
    prelude::*,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

struct Probe {
    install_shell: bool,
    calls: RefCell<Vec<String>>,
    resets: Cell<usize>,
    focus_events: Cell<usize>,
    input_changes: Cell<usize>,
    input_text: RefCell<String>,
    cancellations: Cell<usize>,
    answers: Cell<usize>,
    zen: Cell<bool>,
    zen_armed: Cell<bool>,
    source: RefCell<Option<Rc<NativeView>>>,
    rich_session: RefCell<Option<EditorSession>>,
}

#[component]
fn SavePickerFixture() -> impl IntoElement {
    super::view_shell()
}

#[component]
fn SettingsRoutingFixture() -> impl IntoElement {
    let shell = super::Shell::new();
    let original = NativeView::fixture("abc");
    let original_path = std::path::PathBuf::from(r"C:\texzi-test\existing.txt");
    shell.add(original.clone(), Some(original_path.clone()));
    let original_id = shell.active.get();
    let before = original.snapshot().unwrap();
    original
        .edit(
            before.revision,
            before.view_epoch,
            vec![(3..3, "!".into())],
            ExternalSelections {
                ranges: vec![(1, 3)],
                primary: 0,
            },
            ExternalEditKind::Typing,
        )
        .unwrap();
    let after = original.snapshot().unwrap();
    shell.split_editor(false);
    shell.show_settings();
    shell.run("texzi.group.joinAll");
    shell.run("texzi.find.open");
    assert!(shell.split.get().is_some());
    assert!(shell.second_active.get());
    assert!(shell.panel.get().is_empty());
    shell.dismiss_settings(true);
    assert!(shell.second_active.get());
    shell.show_settings();
    shell.open_canonical_file(original_path).unwrap();
    assert!(!shell.settings_active());
    assert_eq!(shell.current().unwrap().id, original_id);
    assert!(original.is_dirty().unwrap());
    let returned = original.snapshot().unwrap();
    assert_eq!(returned.revision, after.revision);
    assert_eq!(returned.selections, after.selections);
    shell.show_settings();
    shell.focus_group(2);
    assert!(!shell.settings_active());
    assert!(shell.second_active.get());
    shell.show_settings();
    shell.run("texzi.tab.previous");
    assert_eq!(shell.current().unwrap().id, original_id);
    shell.close_tab(1);
    shell.show_settings();
    shell.run("texzi.tab.reopenClosed");
    assert_eq!(shell.current().unwrap().id, 1);
    assert!(
        shell
            .settings
            .apply(super::settings::MARKDOWN_VIEW, None, Some("source".into()))
    );
    shell.add(
        NativeView::fixture("# Source"),
        Some(r"C:\texzi-test\source.md".into()),
    );
    let source = shell.current().unwrap();
    assert!(!source.displayed_rich());
    let mid_sized_markdown = std::path::Path::new("notes.md");
    assert!(!shell.initial_markdown_rich(Some(mid_sized_markdown), super::LARGE_BYTES, None));
    assert!(shell.initial_markdown_rich(Some(mid_sized_markdown), super::LARGE_BYTES, Some(true)));
    assert!(!shell.initial_markdown_rich(
        Some(mid_sized_markdown),
        super::large_file::SOURCE_ONLY_BYTES + 1,
        Some(true)
    ));
    assert!(
        shell
            .settings
            .apply(super::settings::MARKDOWN_VIEW, None, Some("wysiwyg".into()))
    );
    assert!(!source.displayed_rich());
    assert!(!shell.initial_markdown_rich(
        Some(mid_sized_markdown),
        super::LARGE_BYTES,
        Some(false)
    ));
    shell.add(
        NativeView::fixture("# Rich"),
        Some(r"C:\texzi-test\rich.md".into()),
    );
    assert!(shell.current().unwrap().displayed_rich());

    shell.add(
        NativeView::fixture("let n = 4;"),
        Some(r"C:\texzi-test\example.rs".into()),
    );
    let code = shell.current().unwrap();
    assert!(!shell.source_wrap(&code));
    assert!(shell.source_wrap(&source));
    assert!(
        shell
            .settings
            .apply(super::settings::WRAP_CODE, None, Some("on".into()))
    );
    assert!(shell.source_wrap(&code));
    code.wrap.set(Some(false));
    assert!(!shell.source_wrap(&code));
    assert_eq!(shell.tab_size(&code), 4);
    code.tab_size.set(Some(8));
    assert_eq!(shell.tab_size(&code), 8);
    assert_eq!(shell.tab_size(&source), 4);
    code.tab_size.set(None);
    assert_eq!(shell.tab_size(&code), 4);
    super::render_shell(shell)
}

#[test]
fn document_navigation_leaves_settings_without_mutating_retained_editors() {
    let mut core = Core::new(
        ApproxTextEngine,
        Size {
            width: px(800.0),
            height: px(600.0),
        },
    );
    let mut environment = Environment::default();
    environment.os = Os::Windows;
    core.set_environment(environment);
    core.mount_root(&Root::new::<SettingsRoutingFixture>(|| ()));
    for _ in 0..4 {
        core.run_component_tasks();
        core.commit();
    }
}

#[test]
fn activity_settings_opens_one_closable_tab_at_the_bottom() {
    let mut core = Core::new(
        ApproxTextEngine,
        Size {
            width: px(800.0),
            height: px(600.0),
        },
    );
    let mut environment = Environment::default();
    environment.os = Os::Windows;
    core.set_environment(environment);
    core.mount_root(&Root::new::<SavePickerFixture>(|| ()));
    let controls = |core: &mut Core<ApproxTextEngine>| {
        let mut found = Vec::new();
        for _ in 0..4 {
            core.run_component_tasks();
            if let Some(batch) = core.commit() {
                for item in batch.items {
                    if let MountItem::ViewProps { id, props } = item
                        && let Some(label) = props.accessibility.label.clone()
                    {
                        found.push((id, label));
                    }
                }
            }
        }
        found
    };
    let initial = controls(&mut core);
    let gear = initial
        .iter()
        .find(|(_, label)| label.as_ref() == "设置")
        .unwrap()
        .0;
    let frame = core.measure(gear).unwrap();
    assert!(frame.page.x.0 < 44.0 && frame.page.y.0 > 500.0);
    assert!(core.component_event(gear, EventKind::Press, &EventPayload::None));
    let opened = controls(&mut core);
    let label = opened
        .iter()
        .filter(|(_, label)| label.as_ref() == "设置标签")
        .collect::<Vec<_>>();
    assert_eq!(label.len(), 1);
    let tab = label[0].0;
    let close = opened
        .iter()
        .find(|(_, label)| label.as_ref() == "关闭设置标签")
        .unwrap()
        .0;
    assert!(core.component_event(gear, EventKind::Press, &EventPayload::None));
    assert!(
        controls(&mut core)
            .iter()
            .all(|(_, label)| label.as_ref() != "设置标签")
    );
    assert!(core.measure(tab).is_some());
    assert!(core.component_event(close, EventKind::Press, &EventPayload::None));
    controls(&mut core);
    assert!(core.measure(tab).is_none());
}

#[test]
fn saving_new_files_opens_native_picker_and_can_cancel_without_blocking_save_as() {
    let mut core = Core::new(
        ApproxTextEngine,
        Size {
            width: px(960.0),
            height: px(640.0),
        },
    );
    let mut environment = Environment::default();
    environment.os = Os::Windows;
    environment.file_requests = true;
    core.set_environment(environment);
    core.mount_root(&Root::new::<SavePickerFixture>(|| ()));
    let drain = |core: &mut Core<ApproxTextEngine>| {
        let mut keymap = None;
        let mut commands = Vec::new();
        for _ in 0..4 {
            core.run_component_tasks();
            if let Some(batch) = core.commit() {
                for item in batch.items {
                    match item {
                        MountItem::Keymap(keys) => keymap = Some(keys),
                        MountItem::Command { command, .. } => commands.push(command),
                        _ => {}
                    }
                }
            }
        }
        (keymap, commands)
    };
    let keys = drain(&mut core).0.expect("Shell registers save shortcuts");
    let open_picker = |core: &mut Core<ApproxTextEngine>, key, title| {
        let stroke = Keystroke::parse_with(key, false).unwrap();
        let binding = keys.find(stroke, true, false).fired().unwrap();
        assert!(core.component_event(
            ROOT_ID,
            EventKind::KeyCommand,
            &EventPayload::Value(PayloadValue::Number(binding as f64)),
        ));
        let (_, commands) = drain(core);
        assert!(!commands.iter().any(|cmd| matches!(cmd, Command::Dialog(_))));
        let (id, actual_title) = commands
            .into_iter()
            .find_map(|cmd| match cmd {
                Command::FileRequest { id, request } => match *request {
                    FileRequest::SaveDialog(dialog) => Some((id, dialog.title)),
                    _ => None,
                },
                _ => None,
            })
            .expect("save opens the native picker");
        assert_eq!(actual_title.as_deref(), Some(title));
        id
    };
    let cancel_picker = |core: &mut Core<ApproxTextEngine>, id| {
        assert!(core.component_event(
            ROOT_ID,
            EventKind::FileRequest,
            &EventPayload::file_answer(id, Vec::new()),
        ));
        drain(core);
    };
    let save = open_picker(&mut core, "ctrl+s", "保存新文件");
    cancel_picker(&mut core, save);
    let save_as = open_picker(&mut core, "ctrl+shift+s", "另存为新文件");
    cancel_picker(&mut core, save_as);
}

impl Probe {
    fn new(install_shell: bool) -> Rc<Self> {
        Rc::new(Self {
            install_shell,
            calls: RefCell::new(Vec::new()),
            resets: Cell::new(0),
            focus_events: Cell::new(0),
            input_changes: Cell::new(0),
            input_text: RefCell::new("保留输入框正文".into()),
            cancellations: Cell::new(0),
            answers: Cell::new(0),
            zen: Cell::new(false),
            zen_armed: Cell::new(false),
            source: RefCell::new(None),
            rich_session: RefCell::new(None),
        })
    }

    fn observe_zen(self: &Rc<Self>, shell: Rc<super::Shell>) {
        let probe = self.clone();
        effect(move || {
            probe.zen.set(shell.zen.get());
            probe.zen_armed.set(shell.zen_exit.armed.get());
        });
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
    bind_fixture_shortcuts(probe.clone());
    let focused = probe.clone();
    let changed = probe.clone();
    view().child(
        input()
            .value("保留输入框正文".to_owned())
            .auto_focus(true)
            .on_focus(move |_: &()| focused.focus_events.set(focused.focus_events.get() + 1))
            .on_change(move |value: &str| {
                changed.input_changes.set(changed.input_changes.get() + 1);
                *changed.input_text.borrow_mut() = value.into();
            }),
    )
}

#[component]
fn FocusedSourceShortcutFixture() -> impl IntoElement {
    let probe = PROBE.with(|p| p.borrow().as_ref().unwrap().clone());
    bind_fixture_shortcuts(probe.clone());
    provide_context(NativeView::fixture("first\nsecond").buffer());
    nsh_base::editor::editor()
        .auto_focus(true)
        .on_focus(move |_: &()| probe.focus_events.set(probe.focus_events.get() + 1))
}

#[component]
fn FocusedZenFixture() -> impl IntoElement {
    let probe = PROBE.with(|p| p.borrow().as_ref().unwrap().clone());
    let shell = super::Shell::new();
    let source = NativeView::fixture("first\nsecond");
    shell.add(source.clone(), None);
    let doc = shell.current().unwrap();
    doc.gate.source_mounted();
    provide_context(source.buffer());
    *probe.source.borrow_mut() = Some(source);
    probe.observe_zen(shell);
    nsh_base::editor::editor()
        .component_ref(&doc.reference)
        .auto_focus(true)
        .on_focus(move |_: &()| probe.focus_events.set(probe.focus_events.get() + 1))
}

#[component]
fn FocusedRichZenFixture() -> impl IntoElement {
    let probe = PROBE.with(|p| p.borrow().as_ref().unwrap().clone());
    let shell = super::Shell::new();
    shell.add(NativeView::fixture("***\n\nEditable rich text."), None);
    let doc = shell.current().unwrap();
    doc.gate.to_rich();
    assert!(doc.gate.rich_mode.get(), "{}", doc.gate.status.get());
    let session = doc.gate.session();
    *probe.rich_session.borrow_mut() = Some(session.clone());
    probe.observe_zen(shell.clone());
    let document = doc.id;
    view()
        .relative()
        .size_full()
        .child(
            nsh_base::rich_editor::view::rich_editor()
                .editor(session)
                .component_ref(&doc.rich_reference)
                .application_shortcuts(crate::native_menu::application_shortcuts())
                .auto_focus(true)
                .on_focus(move |_: &()| probe.focus_events.set(probe.focus_events.get() + 1))
                .on_unhandled_escape({
                    let shell = shell.clone();
                    move |_: &()| shell.handle_rich_escape(document, false)
                })
                .size_full(),
        )
        .child(super::format_button(shell, doc, false))
}

fn bind_fixture_shortcuts(probe: Rc<Probe>) {
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

        // Use the production continuation helper in a real dynamic owner.
        // The full shell's timeout, modal and IME handling are not replaced
        // or claimed here.
        let prefix = signal(false);
        on_key(super::CHORD_PREFIX, move |_| prefix.set(true));
        let run = probe.clone();
        effect(move || {
            if !prefix.get() {
                return;
            }
            let run = run.clone();
            bind_chord_continuations(Rc::new(move |target| {
                // Shell.run likewise moves service creation and its await to
                // the long-lived shell owner before this prefix is disposed.
                owner.run(|| match target {
                    Some(id) => run.run(id),
                    None => run.cancellations.set(run.cancellations.get() + 1),
                });
                prefix.set(false);
            }));
        });
    }
}

struct Harness {
    core: Core<ApproxTextEngine>,
    probe: Rc<Probe>,
    keys: Option<Arc<Keymap>>,
    input: Option<NodeId>,
    requests: Vec<(u32, OpenDialogKind, bool)>,
    format_button: Option<NodeId>,
    popup_requests: usize,
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Release test-owned documents during ordinary stack cleanup. Leaving
        // a Gate in RICH_PROBE until TLS destruction outlives the RichOwner
        // registry it uses in Drop. Never hold a RefMut while releasing it.
        let rich = RICH_PROBE.with(|slot| slot.borrow_mut().take());
        let plain = PROBE.with(|slot| slot.borrow_mut().take());
        drop(rich);
        drop(plain);
    }
}

impl Harness {
    fn new(install_shell: bool) -> Self {
        Self::with_root(install_shell, Root::new::<ShortcutFixture>(|| ()))
    }

    fn with_root(install_shell: bool, root: Root) -> Self {
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
        core.mount_root(&root);
        let mut fixture = Self {
            core,
            probe,
            keys: None,
            input: None,
            requests: Vec::new(),
            format_button: None,
            popup_requests: 0,
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

    fn focus_window(&mut self) {
        use nsh_core::{
            desktop::{WindowMode, WindowPlacement, WindowState},
            geometry::{Bounds, point},
        };

        let bounds = Bounds {
            origin: point(px(0.0), px(0.0)),
            size: Size {
                width: px(800.0),
                height: px(600.0),
            },
        };
        let mut window = WindowState::new(bounds, WindowPlacement::new(WindowMode::Normal, bounds));
        window.focused = true;
        window.visible = true;
        let mut environment = Environment::default();
        environment.os = Os::Windows;
        environment.file_requests = true;
        environment.window_state = Some(window);
        self.core.set_environment(environment);
        self.flush();
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
                        MountItem::ViewProps { id, props }
                            if props.accessibility.label.as_deref() == Some("格式命令") =>
                        {
                            self.format_button = Some(id)
                        }
                        MountItem::Command {
                            command: Command::Desktop(command),
                            ..
                        } if matches!(*command, DesktopCommand::PopupMenu { .. }) => {
                            self.popup_requests += 1;
                        }
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
fn real_source_f3_and_shift_f3_route_once_to_shell_and_yield_to_pending_chords() {
    let focused_editor = |install_shell| {
        Harness::with_root(
            install_shell,
            Root::new::<FocusedSourceShortcutFixture>(|| ()),
        )
    };
    {
        let mut editor_only = focused_editor(false);
        for key in ["f3", "shift+f3"] {
            // The focused Editor installs its own repeatable find commands.
            assert!(matches!(editor_only.press(key, false), KeyMatch::Fire(_)));
            assert!(matches!(editor_only.press(key, true), KeyMatch::Fire(_)));
        }
        assert!(editor_only.probe.calls.borrow().is_empty());
    }

    let mut fixture = focused_editor(true);
    for key in ["f3", "shift+f3"] {
        assert!(matches!(fixture.press(key, false), KeyMatch::Fire(_)));
        assert_eq!(fixture.press(key, true), KeyMatch::Held);
    }
    assert_eq!(
        fixture.probe.calls.borrow().as_slice(),
        ["texzi.find.next", "texzi.find.previous"]
    );

    for key in ["f3", "shift+f3"] {
        assert!(fixture.press("ctrl+k", false).consumed());
        assert!(fixture.press(key, false).consumed());
    }
    assert_eq!(fixture.probe.cancellations.get(), 2);
    assert!(fixture.press("ctrl+k", false).consumed());
    assert!(fixture.press("s", false).consumed());
    assert!(fixture.press("shift+f3", false).consumed());
    assert!(fixture.press("f3", false).consumed());
    assert_eq!(
        fixture.probe.calls.borrow().as_slice(),
        [
            "texzi.find.next",
            "texzi.find.previous",
            "texzi.file.saveAll",
            "texzi.find.previous",
            "texzi.find.next",
        ]
    );
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

struct RichDocument {
    source: Rc<NativeView>,
    rich: Rc<NativeView>,
    gate: Rc<ModeGate>,
}

struct RichProbe {
    reserve_application_keys: bool,
    document: RefCell<Option<Rc<RichDocument>>>,
}

thread_local! {
    static RICH_PROBE: RefCell<Option<Rc<RichProbe>>> = const { RefCell::new(None) };
}

#[component]
fn RichShortcutFixture() -> impl IntoElement {
    let probe = PROBE.with(|p| p.borrow().as_ref().unwrap().clone());
    let rich_probe = RICH_PROBE.with(|p| p.borrow().as_ref().unwrap().clone());
    bind_fixture_shortcuts(probe.clone());
    let source = NativeView::fixture("正文 &amp; \\*保留\\*\r\n\r\n<keep>literal</keep>\r\n");
    let rich = source.sibling().unwrap();
    let gate = ModeGate::new(source.clone(), rich.clone());
    gate.to_rich();
    assert!(gate.rich_mode.get(), "{}", gate.status.get());
    let session = gate.session();
    *rich_probe.document.borrow_mut() = Some(Rc::new(RichDocument { source, rich, gate }));
    let shortcuts = if rich_probe.reserve_application_keys {
        crate::native_menu::application_shortcuts()
    } else {
        Vec::new()
    };
    view().size_full().child(
        nsh_base::rich_editor::view::rich_editor()
            .editor(session)
            .application_shortcuts(shortcuts)
            .on_focus(move |_: &()| probe.focus_events.set(probe.focus_events.get() + 1))
            .size_full(),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RichAudit {
    source: String,
    rich: String,
    doc: serde_json::Value,
    selection: serde_json::Value,
    stored_marks: Vec<serde_json::Value>,
    status: String,
}

fn view_audit(view: &NativeView) -> String {
    let native = view.snapshot().unwrap();
    view.inspect(|store, id| {
        let document = store.view(id).unwrap().doc;
        let snapshot = store.snapshot(document).unwrap();
        format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            document,
            snapshot.revision,
            snapshot.token,
            snapshot.body.to_utf8(),
            snapshot.body.profile,
            snapshot.body.eols(),
            snapshot.body.origins(),
            store.history_depth(document).unwrap(),
            store.dirty(document).unwrap(),
            native.selections,
        )
    })
}

impl Harness {
    fn rich(reserve_application_keys: bool) -> (Self, Rc<RichDocument>, KeyMatch) {
        let probe = Probe::new(true);
        PROBE.with(|p| *p.borrow_mut() = Some(probe.clone()));
        let rich_probe = Rc::new(RichProbe {
            reserve_application_keys,
            document: RefCell::new(None),
        });
        RICH_PROBE.with(|p| *p.borrow_mut() = Some(rich_probe.clone()));
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
        core.mount_root(&Root::new::<RichShortcutFixture>(|| ()));
        let mut fixture = Self {
            core,
            probe,
            keys: None,
            input: None,
            requests: Vec::new(),
            format_button: None,
            popup_requests: 0,
        };
        fixture.flush();
        let root_open = fixture.matched("ctrl+o", false);
        assert!(matches!(root_open, KeyMatch::Fire(_)));
        let document = rich_probe.document.borrow().as_ref().unwrap().clone();
        fixture.rich_focus(true);
        assert_eq!(fixture.probe.focus_events.get(), 1);
        (fixture, document, root_open)
    }

    fn rich_focus(&mut self, focused: bool) {
        let sink = self
            .input
            .expect("the real RichEditor sink Input was mounted");
        assert!(self.core.component_event(
            sink,
            if focused {
                EventKind::Focus
            } else {
                EventKind::Blur
            },
            &EventPayload::None,
        ));
        self.flush();
    }

    fn inspect<T: 'static>(&mut self, read: impl FnOnce() -> T + 'static) -> T {
        let result = Rc::new(RefCell::new(None));
        let output = result.clone();
        let read = RefCell::new(Some(read));
        self.core
            .apply(Op::Rust(RustOp {
                call: 81_000,
                run: RustCall::Op(Box::new(move |_| {
                    *output.borrow_mut() = Some(read.borrow_mut().take().unwrap()());
                    Ok(Box::new(()) as Box<dyn ToJs>)
                })),
            }))
            .unwrap();
        self.flush();
        let value = result.borrow_mut().take().unwrap();
        value
    }

    fn rich_audit(&mut self, document: &Rc<RichDocument>) -> RichAudit {
        let document = document.clone();
        self.inspect(move || {
            let state = document.gate.session().state();
            RichAudit {
                source: view_audit(&document.source),
                rich: view_audit(&document.rich),
                doc: state.doc().to_json(),
                selection: state.selection().to_json(),
                stored_marks: state
                    .stored_marks()
                    .unwrap_or_default()
                    .iter()
                    .map(|mark| mark.to_json())
                    .collect(),
                status: document.gate.status.get(),
            }
        })
    }
}

#[test]
fn real_rich_without_reserved_keys_masks_root_ctrl_o_and_blur_removes_the_mask() {
    let (mut fixture, document, root_open) = Harness::rich(false);
    let before = fixture.rich_audit(&document);
    let rich_open = fixture.matched("ctrl+o", false);
    assert!(matches!(rich_open, KeyMatch::Fire(_)));
    assert_ne!(
        rich_open, root_open,
        "the actual focused Rich view registered a newer key"
    );
    assert_eq!(fixture.press("ctrl+o", false), rich_open);
    assert!(fixture.probe.calls.borrow().is_empty());
    assert!(fixture.requests.is_empty());
    assert_eq!(fixture.rich_audit(&document), before);

    fixture.rich_focus(false);
    assert_eq!(fixture.matched("ctrl+o", false), root_open);
    assert_eq!(fixture.press("ctrl+o", false), root_open);
    assert_eq!(fixture.probe.calls.borrow().as_slice(), ["texzi.file.open"]);
    assert_eq!(fixture.requests.len(), 1);
    fixture.cancel_request(fixture.requests[0].0);
    assert_eq!(fixture.probe.answers.get(), 1);
    assert_eq!(fixture.rich_audit(&document), before);
}

#[test]
fn real_rich_reserved_ctrl_o_keeps_root_open_chord_priority_and_refocus_without_document_writes() {
    let (mut fixture, document, root_open) = Harness::rich(true);
    let before = fixture.rich_audit(&document);
    assert!(matches!(fixture.press("ctrl+e", false), KeyMatch::Fire(_)));
    assert_eq!(
        fixture.probe.calls.borrow().as_slice(),
        ["texzi.navigation.quickOpen"],
    );
    assert!(fixture.requests.is_empty());
    assert_eq!(fixture.rich_audit(&document), before);
    // The host list uses normalized application shortcuts while the view's
    // Windows table uses ctrl+o: this tests their real matching equivalence.
    assert_eq!(fixture.matched("ctrl+o", false), root_open);
    assert_eq!(fixture.press("ctrl+o", false), root_open);
    assert_eq!(fixture.press("ctrl+o", true), KeyMatch::Held);
    assert_eq!(fixture.requests.len(), 1);
    assert_eq!(fixture.requests[0].1, OpenDialogKind::Files);
    fixture.cancel_request(fixture.requests[0].0);
    assert_eq!(fixture.rich_audit(&document), before);

    assert!(matches!(fixture.press("ctrl+k", false), KeyMatch::Fire(_)));
    let folder = fixture.matched("ctrl+o", false);
    assert!(matches!(folder, KeyMatch::Fire(_)));
    assert_ne!(folder, root_open);
    assert_eq!(fixture.press("ctrl+o", false), folder);
    assert_eq!(fixture.requests.len(), 2);
    assert_eq!(fixture.requests[1].1, OpenDialogKind::Folders);
    fixture.cancel_request(fixture.requests[1].0);
    assert_eq!(fixture.matched("ctrl+o", false), root_open);
    assert_eq!(fixture.rich_audit(&document), before);

    fixture.rich_focus(false);
    assert_eq!(fixture.matched("ctrl+o", false), root_open);
    fixture.rich_focus(true);
    assert_eq!(fixture.matched("ctrl+o", false), root_open);
    assert_eq!(fixture.press("ctrl+o", false), root_open);
    assert_eq!(fixture.requests.len(), 3);
    assert_eq!(fixture.requests[2].1, OpenDialogKind::Files);
    fixture.cancel_request(fixture.requests[2].0);
    assert_eq!(
        fixture.probe.calls.borrow().as_slice(),
        [
            "texzi.navigation.quickOpen",
            "texzi.file.open",
            "texzi.folder.open",
            "texzi.file.open",
        ],
    );
    assert_eq!(fixture.probe.answers.get(), 3);
    assert_eq!(fixture.rich_audit(&document), before);
}

#[test]
fn real_rich_ctrl_b_still_toggles_stored_bold_marks_without_source_or_history_writes() {
    let (mut fixture, document, root_open) = Harness::rich(true);
    let before = fixture.rich_audit(&document);
    assert!(before.stored_marks.is_empty());
    let bold = fixture.matched("ctrl+b", false);
    assert!(
        matches!(bold, KeyMatch::Fire(_)),
        "Ctrl+B must reach the real Rich keymap"
    );
    assert_ne!(bold, root_open);
    assert_eq!(fixture.press("ctrl+b", false), bold);
    let marked = fixture.rich_audit(&document);
    assert!(
        marked
            .stored_marks
            .iter()
            .any(|mark| mark["type"] == "bold")
    );
    assert_eq!(marked.source, before.source);
    assert_eq!(marked.rich, before.rich);
    assert_eq!(marked.doc, before.doc);
    assert_eq!(marked.selection, before.selection);
    assert_eq!(marked.status, before.status);
    assert!(fixture.probe.calls.borrow().is_empty());
    assert!(fixture.requests.is_empty());

    assert_eq!(fixture.press("ctrl+b", false), bold);
    assert_eq!(fixture.rich_audit(&document), before);
}

#[test]
fn zen_double_escape_reaches_the_shell_with_a_focused_source_input() {
    let mut fixture = Harness::with_root(false, Root::new::<FocusedZenFixture>(|| ()));
    fixture.focus_window();

    // Keep the core's matching in text-focus mode throughout, as on Win32
    // before its fallback can blur the input. The old Auto binding misses it.
    assert_eq!(fixture.matched("escape", false), KeyMatch::None);
    assert!(fixture.press("ctrl+k", false).consumed());
    assert!(fixture.press("z", false).consumed());
    assert!(fixture.probe.zen.get());
    assert!(matches!(
        fixture.press("escape", false),
        KeyMatch::Notify(_)
    ));
    assert!(fixture.probe.zen_armed.get());
    fixture.press("escape", true);
    assert!(fixture.probe.zen.get());
    assert!(matches!(
        fixture.press("escape", false),
        KeyMatch::Notify(_)
    ));
    assert!(!fixture.probe.zen.get());
    assert!(!fixture.probe.zen_armed.get());
    assert_eq!(fixture.matched("escape", false), KeyMatch::None);

    let source = fixture.probe.source.borrow().as_ref().unwrap().clone();
    let selections = || source.snapshot().unwrap().selections.ranges;
    for (key, expected) in [
        ("ctrl+a", vec![(0, 12)]),
        ("ctrl+alt+down", vec![(0, 0), (6, 6)]),
    ] {
        assert!(fixture.press("ctrl+home", false).consumed());
        assert!(fixture.press(key, false).consumed());
        assert_eq!(selections(), expected);
        let collapse = fixture.matched("escape", false);
        assert!(matches!(collapse, KeyMatch::Fire(_)));
        assert!(fixture.press("ctrl+k", false).consumed());
        assert!(fixture.press("z", false).consumed());
        assert_eq!(fixture.press("escape", false), collapse);
        assert!(fixture.probe.zen.get());
        assert!(!fixture.probe.zen_armed.get());
        assert!(matches!(selections().as_slice(), [(a, h)] if a == h));
        assert!(matches!(
            fixture.press("escape", false),
            KeyMatch::Notify(_)
        ));
        assert!(fixture.probe.zen_armed.get());
        assert!(matches!(
            fixture.press("escape", false),
            KeyMatch::Notify(_)
        ));
        assert!(!fixture.probe.zen.get());
    }

    assert!(fixture.press("ctrl+k", false).consumed());
    assert!(fixture.press("z", false).consumed());
    assert!(fixture.press("ctrl+f", false).consumed());
    assert_eq!(fixture.press("escape", false), KeyMatch::None);
    assert!(fixture.probe.zen.get());
    assert!(!fixture.probe.zen_armed.get());
}

struct EscapePlugin(Arc<AtomicBool>);

impl PluginSpec for EscapePlugin {
    fn handle_key_down(
        &self,
        _: &EditorState,
        event: &KeyEvent,
        _: &mut dyn FnMut(Transaction),
    ) -> bool {
        event.key == "Escape" && self.0.load(Ordering::Relaxed)
    }
}

#[test]
fn zen_rich_escape_defers_to_the_editor_before_counting_an_empty_text_caret() {
    let mut fixture = Harness::with_root(false, Root::new::<FocusedRichZenFixture>(|| ()));
    fixture.focus_window();
    let session = fixture
        .probe
        .rich_session
        .borrow()
        .as_ref()
        .unwrap()
        .clone();
    let state = session.state();
    let mut plugins = state.plugins().to_vec();
    let plugin_active = Arc::new(AtomicBool::new(false));
    plugins.push(Arc::new(EscapePlugin(plugin_active.clone())));
    session.update_state(state.reconfigure(plugins).unwrap());
    fixture.flush();

    let escape = fixture.press("escape", false);
    assert!(matches!(escape, KeyMatch::Fire(_)));
    assert!(!fixture.probe.zen_armed.get());
    assert!(fixture.press("ctrl+k", false).consumed());
    assert!(fixture.press("z", false).consumed());
    assert!(fixture.probe.zen.get());

    assert!(fixture.press("ctrl+a", false).consumed());
    assert!(!session.state().selection().empty());
    assert_eq!(fixture.press("escape", false), escape);
    assert!(session.state().selection().cursor().is_some());
    assert!(!fixture.probe.zen_armed.get());

    plugin_active.store(true, Ordering::Relaxed);
    assert_eq!(fixture.press("escape", false), escape);
    assert!(!fixture.probe.zen_armed.get());
    plugin_active.store(false, Ordering::Relaxed);

    assert!(fixture.press("ctrl+f", false).consumed());
    assert_eq!(fixture.press("escape", false), escape);
    assert!(!fixture.probe.zen_armed.get());

    assert_eq!(fixture.press("escape", false), escape);
    assert!(fixture.probe.zen_armed.get());
    let before_composition = session.state();
    for composing in [true, false] {
        assert!(fixture.core.component_event(
            fixture.input.unwrap(),
            EventKind::CompositionChange,
            &EventPayload::Map(vec![
                ("composing", PayloadValue::Bool(composing)),
                (
                    "text",
                    PayloadValue::Str(if composing {
                        "中".into()
                    } else {
                        String::new()
                    })
                ),
            ]),
        ));
        fixture.flush();
        assert_eq!(session.composing(), composing);
        assert!(!fixture.probe.zen_armed.get());
        if composing {
            assert_eq!(fixture.press("escape", false), escape);
            assert!(!fixture.probe.zen_armed.get());
        }
    }
    let after_composition = session.state();
    assert_eq!(after_composition.revision(), before_composition.revision());
    assert!(
        after_composition
            .selection()
            .eq(before_composition.selection())
    );

    assert_eq!(fixture.press("escape", false), escape);
    assert!(fixture.probe.zen.get());
    assert!(fixture.probe.zen_armed.get());
    fixture.press("escape", true);
    assert!(fixture.probe.zen.get());
    assert_eq!(fixture.press("escape", false), escape);
    assert!(!fixture.probe.zen.get());
    assert!(!fixture.probe.zen_armed.get());

    let state = session.state();
    let before_rule = state.doc().resolve(0).unwrap();
    assert!(GapCursor::valid(&before_rule));
    let mut transaction = state.tr();
    transaction.set_selection(GapCursor::create(before_rule));
    session.update_state(state.apply(transaction).unwrap());
    fixture.flush();
    assert!(session.state().selection().is_gap_cursor());
    assert!(fixture.press("ctrl+k", false).consumed());
    assert!(fixture.press("z", false).consumed());
    assert_eq!(fixture.press("escape", false), escape);
    assert!(fixture.probe.zen_armed.get());
    assert_eq!(fixture.press("escape", false), escape);
    assert!(!fixture.probe.zen.get());
    assert!(session.state().selection().is_gap_cursor());
}

#[test]
fn native_format_popup_breaks_the_zen_escape_sequence() {
    let mut fixture = Harness::with_root(false, Root::new::<FocusedRichZenFixture>(|| ()));
    fixture.focus_window();
    assert!(fixture.press("ctrl+k", false).consumed());
    assert!(fixture.press("z", false).consumed());
    let escape = fixture.press("escape", false);
    assert!(fixture.probe.zen_armed.get());

    assert!(fixture.core.component_event(
        fixture.format_button.expect("mounted format button"),
        EventKind::Press,
        &EventPayload::None,
    ));
    fixture.flush();
    assert_eq!(fixture.popup_requests, 1);
    assert!(fixture.probe.zen.get());
    assert!(!fixture.probe.zen_armed.get());

    // Win32 sends only the close notification. Core settles the popup and
    // returns false so that a JS observer could still see the notification.
    assert!(!fixture.core.component_event(
        ROOT_ID,
        EventKind::Menu,
        &EventPayload::Map(vec![
            ("id", PayloadValue::Number(0.0)),
            ("closed", PayloadValue::Bool(true)),
        ]),
    ));
    fixture.flush();
    assert_eq!(fixture.press("escape", false), escape);
    assert!(fixture.probe.zen.get());
    assert!(fixture.probe.zen_armed.get());
    assert_eq!(fixture.press("escape", false), escape);
    assert!(!fixture.probe.zen.get());
}

#[test]
fn repeated_chord_prefix_reaches_the_prefix_handler_and_preserves_save_all() {
    let mut fixture = Harness::new(true);
    let prefix = fixture.press("ctrl+k", false);
    assert!(matches!(prefix, KeyMatch::Fire(_)));
    assert_eq!(fixture.press("ctrl+k", false), prefix);
    assert!(fixture.press("s", false).consumed());
    assert_eq!(fixture.probe.cancellations.get(), 0);
    assert_eq!(
        fixture.probe.calls.borrow().as_slice(),
        ["texzi.file.saveAll"],
    );

    assert_eq!(fixture.press("ctrl+k", false), prefix);
    assert!(fixture.press("s", false).consumed());
    assert_eq!(
        fixture.probe.calls.borrow().as_slice(),
        ["texzi.file.saveAll", "texzi.file.saveAll"],
    );

    assert_eq!(fixture.press("ctrl+k", false), prefix);
    assert!(fixture.press("ctrl+s", false).consumed());
    assert_eq!(fixture.probe.cancellations.get(), 1);
    assert_eq!(fixture.probe.calls.borrow().len(), 2);
    assert_eq!(fixture.probe.input_changes.get(), 0);
}

#[test]
fn production_plain_chord_continuations_fire_in_input_and_rich_and_cleanup_without_typing() {
    // API-level negative control for the old default-When binding. This is not
    // a claim that the old Windows executable was driven by this test.
    let z = Keystroke::parse_with("z", false).unwrap();
    let old_auto = KeyBinding::new(70_001, z, When::Auto, false);
    assert!(!old_auto.in_text);
    assert_eq!(
        Keymap::new(vec![old_auto]).find(z, true, false),
        KeyMatch::None,
    );
    assert!(CHORDS.contains(&("z", "texzi.view.toggleZen")));
    assert!(!CHORDS.iter().any(|(stroke, _)| *stroke == "q"));

    for rich in [false, true] {
        let (mut fixture, document, root_open) = if rich {
            let (fixture, document, root_open) = Harness::rich(true);
            (fixture, Some(document), root_open)
        } else {
            let fixture = Harness::new(true);
            let root_open = fixture.matched("ctrl+o", false);
            (fixture, None, root_open)
        };
        let before = document
            .as_ref()
            .map(|document| fixture.rich_audit(document));
        let input_before = fixture.probe.input_text.borrow().clone();
        assert_eq!(fixture.matched("ctrl+o", false), root_open);

        assert!(matches!(fixture.press("ctrl+k", false), KeyMatch::Fire(_)));
        assert_eq!(fixture.matched("z", true), KeyMatch::Held);
        let known = fixture.press("z", false);
        assert!(matches!(known, KeyMatch::Fire(_)), "rich={rich}: {known:?}");
        assert!(known.consumed());
        assert_eq!(
            fixture.probe.calls.borrow().as_slice(),
            ["texzi.view.toggleZen"],
        );
        assert_eq!(fixture.probe.cancellations.get(), 0);
        assert_eq!(fixture.matched("ctrl+o", false), root_open);
        assert!(matches!(fixture.matched("z", false), KeyMatch::Notify(_)));
        if let Some(document) = document.as_ref() {
            assert_eq!(fixture.rich_audit(document), *before.as_ref().unwrap());
        }

        assert!(matches!(fixture.press("ctrl+k", false), KeyMatch::Fire(_)));
        let unknown = fixture.press("q", false);
        assert!(
            matches!(unknown, KeyMatch::Fire(_)),
            "rich={rich}: {unknown:?}",
        );
        assert!(unknown.consumed());
        assert_eq!(fixture.probe.cancellations.get(), 1);
        assert_eq!(
            fixture.probe.calls.borrow().as_slice(),
            ["texzi.view.toggleZen"],
            "the unknown continuation must only cancel",
        );
        assert_eq!(fixture.matched("ctrl+o", false), root_open);
        assert!(matches!(fixture.matched("q", false), KeyMatch::Notify(_)));
        assert!(fixture.requests.is_empty());
        assert_eq!(fixture.probe.input_changes.get(), 0);
        assert_eq!(*fixture.probe.input_text.borrow(), input_before);
        if let Some(document) = document.as_ref() {
            assert_eq!(fixture.rich_audit(document), *before.as_ref().unwrap());
        }
    }
}
