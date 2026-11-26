//! Production shell and source editor under NSH Core with real Windows file IO.
use super::*;
use nsh_core::{
    ApproxTextEngine, Command, Core, EventKind, EventPayload, MountItem, NodeId, PayloadValue,
    ROOT_ID,
    element::{Environment, Root},
    keymap::{Keymap, Keystroke},
    platform::Os,
};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
use texzi_document::{Revision, Snapshot, StateToken};

type Hook = Box<dyn FnOnce(&Snapshot) + Send>;

struct Probe {
    pause: RefCell<Option<Hook>>,
    states: RefCell<Vec<EditorState>>,
    auto_save: Option<(PathBuf, &'static str)>,
    next_file: Option<PathBuf>,
}

#[derive(Clone)]
struct EditorState {
    bytes: Vec<u8>,
    revision: Revision,
    token: StateToken,
    dirty: bool,
    path: Option<PathBuf>,
    notice: String,
}

thread_local! {
    static PROBE: RefCell<Option<Rc<Probe>>> = const { RefCell::new(None) };
}

#[component]
fn SaveOverlapFixture() -> impl IntoElement {
    let probe = PROBE.with(|slot| slot.borrow_mut().take().unwrap());
    let shell = Shell::new();
    if let Some((path, mode)) = &probe.auto_save {
        shell.open_canonical_file(path.clone()).unwrap();
        assert!(
            shell
                .settings
                .apply(settings::AUTO_SAVE, None, Some((*mode).into()))
        );
    }
    if let Some(path) = probe.next_file.clone() {
        let next = shell.clone();
        on_key("f6", move |_| {
            next.open_canonical_file(path.clone()).unwrap()
        });
    }
    if let Some(pause) = probe.pause.borrow_mut().take() {
        shell
            .current()
            .unwrap()
            .source
            .before_next_publication(pause);
    }
    let menu = shell.clone();
    on_key("f8", move |_| menu.run("texzi.file.saveAll"));
    let menu = shell.clone();
    on_key("f9", move |_| menu.run("texzi.file.new"));
    let observed = shell.clone();
    on_key("f7", move |_| {
        *probe.states.borrow_mut() = observed.docs.with(|docs| {
            docs.iter()
                .map(|doc| {
                    doc.source.inspect(|store, view| {
                        let id = store.view(view).unwrap().doc;
                        let snapshot = store.snapshot(id).unwrap();
                        EditorState {
                            bytes: snapshot.body.to_utf8(),
                            revision: snapshot.revision,
                            token: snapshot.token,
                            dirty: doc.source.is_dirty().unwrap(),
                            path: doc.path.get(),
                            notice: observed.notice.get(),
                        }
                    })
                })
                .collect()
        });
    });
    render_shell(shell)
}

#[derive(Default)]
struct Harness {
    keys: Option<Arc<Keymap>>,
    source_input: Option<NodeId>,
    input_count: u32,
    focus: Option<NodeId>,
    commands: Vec<Command>,
}

impl Harness {
    fn drain(&mut self, core: &mut Core<ApproxTextEngine>) {
        for _ in 0..4 {
            core.run_component_tasks();
            if let Some(batch) = core.commit() {
                for item in batch.items {
                    match item {
                        MountItem::Keymap(keys) => self.keys = Some(keys),
                        MountItem::Create {
                            id,
                            kind: nsh_core::ViewKind::Input,
                        } => self.source_input = Some(id),
                        MountItem::Command {
                            id,
                            command: Command::Focus,
                        } => self.focus = Some(id),
                        MountItem::Command { command, .. } => self.commands.push(command),
                        _ => {}
                    }
                }
            }
        }
    }

    fn key(&mut self, core: &mut Core<ApproxTextEngine>, key: &str) {
        let key = Keystroke::parse_with(key, false).unwrap();
        let binding = self
            .keys
            .as_ref()
            .unwrap()
            .find(key, true, false)
            .fired()
            .unwrap();
        assert!(core.component_event(
            ROOT_ID,
            EventKind::KeyCommand,
            &EventPayload::Value(PayloadValue::Number(binding as f64))
        ));
        self.drain(core);
    }

    fn change_input(&mut self, core: &mut Core<ApproxTextEngine>, text: &str) {
        self.input_count += 1;
        assert!(core.component_event(
            self.source_input.unwrap(),
            EventKind::ChangeText,
            &EventPayload::Text {
                text: text.into(),
                count: self.input_count,
            }
        ));
        self.drain(core);
    }

    fn sample(&mut self, core: &mut Core<ApproxTextEngine>, probe: &Probe) -> EditorState {
        self.samples(core, probe).pop().unwrap()
    }

    fn samples(&mut self, core: &mut Core<ApproxTextEngine>, probe: &Probe) -> Vec<EditorState> {
        self.key(core, "f7");
        probe.states.take()
    }

    fn wait_for_notice(
        &mut self,
        core: &mut Core<ApproxTextEngine>,
        probe: &Probe,
        expected: &str,
    ) -> Vec<EditorState> {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            self.drain(core);
            let states = self.samples(core, probe);
            let notice = &states.last().unwrap().notice;
            if notice == expected {
                return states;
            }
            assert!(
                Instant::now() < deadline,
                "expected {expected}; got {notice}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn picker(&mut self, core: &mut Core<ApproxTextEngine>, title: &str) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            self.drain(core);
            for command in self.commands.drain(..) {
                if let Command::FileRequest { id, request } = command
                    && let nsh_core::file_dialog::FileRequest::SaveDialog(dialog) = *request
                {
                    assert_eq!(dialog.title.as_deref(), Some(title));
                    return id;
                }
            }
            assert!(Instant::now() < deadline, "missing picker: {title}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn open_shell(probe: &Rc<Probe>) -> (tokio::runtime::Runtime, Core<ApproxTextEngine>, Harness) {
    PROBE.with(|slot| *slot.borrow_mut() = Some(probe.clone()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let mut core = Core::new(
        ApproxTextEngine,
        Size {
            width: px(960.0),
            height: px(640.0),
        },
    );
    core.set_task_runtime(runtime.handle().clone());
    let mut environment = Environment::default();
    environment.os = Os::Windows;
    environment.file_requests = true;
    core.set_environment(environment);
    core.mount_root(&Root::new::<SaveOverlapFixture>(|| ()));
    let mut harness = Harness::default();
    harness.drain(&mut core);
    assert!(core.component_event(
        harness.source_input.unwrap(),
        EventKind::Focus,
        &EventPayload::None
    ));
    (runtime, core, harness)
}

#[test]
fn selector_cancel_save_all_partial_then_save_as_new_target() {
    let root = std::env::temp_dir().join(format!(
        "texzi-save-all-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let first = root.join("first.txt");
    let second = root.join("second.txt");
    let probe = Rc::new(Probe {
        pause: RefCell::new(None),
        states: RefCell::new(Vec::new()),
        auto_save: None,
        next_file: None,
    });
    let (_runtime, mut core, mut harness) = open_shell(&probe);
    let answer = |core: &mut Core<ApproxTextEngine>, id, paths| {
        assert!(core.component_event(
            ROOT_ID,
            EventKind::FileRequest,
            &EventPayload::file_answer(id, paths),
        ));
    };
    harness.change_input(&mut core, "first");
    harness.key(&mut core, "ctrl+s");
    let canceled = harness.picker(&mut core, "保存新文件");
    harness.focus = None;
    answer(&mut core, canceled, vec![]);
    let canceled_state = harness
        .wait_for_notice(&mut core, &probe, "已取消保存")
        .pop()
        .unwrap();
    assert!(canceled_state.dirty && canceled_state.path.is_none());
    assert_eq!(harness.focus, harness.source_input);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);

    harness.key(&mut core, "f9");
    harness.change_input(&mut core, "second");
    harness.key(&mut core, "f8");
    let request = harness.picker(&mut core, "保存新文件");
    answer(&mut core, request, vec![first.clone()]);
    let request = harness.picker(&mut core, "保存新文件");
    assert_eq!(std::fs::read(&first).unwrap(), b"first");
    harness.focus = None;
    answer(&mut core, request, vec![]);
    let states = harness.wait_for_notice(&mut core, &probe, "已保存 1 个文档，已取消剩余保存");
    assert_eq!(states.len(), 2);
    assert_eq!(states[0].path.as_ref(), Some(&first));
    assert!(!states[0].dirty);
    assert!(states[1].dirty && states[1].path.is_none());
    assert_eq!(states[1].bytes, b"second");
    assert_eq!(harness.focus, harness.source_input);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);

    harness.key(&mut core, "ctrl+shift+s");
    let request = harness.picker(&mut core, "另存为新文件");
    answer(&mut core, request, vec![second.clone()]);
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let state = harness.sample(&mut core, &probe);
        if state.path.as_ref() == Some(&second) {
            assert!(!state.dirty);
            break;
        }
        assert!(Instant::now() < deadline, "Save As: {}", state.notice);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(std::fs::read(&first).unwrap(), b"first");
    assert_eq!(std::fs::read(&second).unwrap(), b"second");
    drop(core);
    std::fs::remove_file(first).unwrap();
    std::fs::remove_file(second).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn source_input_during_real_menu_save_keeps_newer_revision_dirty() {
    let root = std::env::temp_dir().join(format!(
        "texzi-app-save-overlap-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let destination = root.join("overlap.txt");
    let (started_tx, started_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let ui = std::thread::current().id();
    let probe = Rc::new(Probe {
        pause: RefCell::new(Some(Box::new(move |capture: &Snapshot| {
            assert_ne!(ui, std::thread::current().id());
            started_tx.send(capture.clone()).unwrap();
            resume_rx.recv_timeout(Duration::from_secs(15)).unwrap();
        }))),
        states: RefCell::new(Vec::new()),
        auto_save: None,
        next_file: None,
    });
    let (_runtime, mut core, mut harness) = open_shell(&probe);
    harness.change_input(&mut core, "N");
    let n = harness.sample(&mut core, &probe);
    assert_eq!(n.bytes, b"N");
    assert!(n.dirty);
    harness.key(&mut core, "ctrl+s");
    let picker = harness
        .commands
        .drain(..)
        .find_map(|command| match command {
            Command::FileRequest { id, request }
                if matches!(*request, nsh_core::file_dialog::FileRequest::SaveDialog(_)) =>
            {
                Some(id)
            }
            _ => None,
        })
        .unwrap();
    harness.focus = None;
    assert!(core.component_event(
        ROOT_ID,
        EventKind::FileRequest,
        &EventPayload::file_answer(picker, vec![destination.clone()])
    ));
    let deadline = Instant::now() + Duration::from_secs(12);
    let capture = loop {
        harness.drain(&mut core);
        if let Ok(capture) = started_rx.try_recv() {
            break capture;
        }
        assert!(
            Instant::now() < deadline,
            "publication worker did not reach the test barrier"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!((capture.revision, capture.token), (n.revision, n.token));
    assert!(!destination.exists());
    assert_eq!(harness.focus, harness.source_input);
    harness.change_input(&mut core, "N+1");
    let newer = harness.sample(&mut core, &probe);
    assert_eq!(newer.bytes, b"N+1");
    assert!(newer.dirty && newer.path.is_none());
    assert_eq!(newer.revision.0, n.revision.0 + 1);
    assert_ne!(newer.token, n.token);
    resume_tx.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    let accepted = loop {
        let state = harness.sample(&mut core, &probe);
        if state.path.as_ref() == Some(&destination) {
            break state;
        }
        assert!(
            Instant::now() < deadline,
            "save receipt was not accepted: {}",
            state.notice
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(std::fs::read(&destination).unwrap(), b"N");
    assert_eq!(accepted.bytes, b"N+1");
    assert_eq!(
        (accepted.revision, accepted.token),
        (newer.revision, newer.token)
    );
    assert!(accepted.dirty && accepted.notice.contains("仍有较新的未保存修改"));
    eprintln!(
        "save overlap: captured={:?}, newer={:?}, accepted={:?}, disk=N, dirty={}",
        n.revision, newer.revision, accepted.revision, accepted.dirty
    );
    drop(core);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn named_auto_save_uses_the_configured_trigger_and_protects_external_changes() {
    for mode in ["afterDelay", "onFocusChange"] {
        let root =
            std::env::temp_dir().join(format!("texzi-auto-save-{}-{mode}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("named.txt");
        std::fs::write(&path, "original").unwrap();
        let probe = Rc::new(Probe {
            pause: RefCell::new(None),
            states: RefCell::new(Vec::new()),
            auto_save: Some((path.clone(), mode)),
            next_file: None,
        });
        let (_runtime, mut core, mut harness) = open_shell(&probe);
        harness.change_input(&mut core, "!");
        let local = harness.sample(&mut core, &probe).bytes;
        assert_eq!(local, b"!original");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        let blur = |core: &mut Core<ApproxTextEngine>, harness: &mut Harness| {
            if mode == "onFocusChange" {
                core.component_event(
                    harness.source_input.unwrap(),
                    EventKind::Blur,
                    &EventPayload::None,
                );
                harness.drain(core);
            }
        };
        blur(&mut core, &mut harness);
        let saved = harness
            .wait_for_notice(&mut core, &probe, "已保存 named.txt")
            .pop()
            .unwrap();
        assert!(!saved.dirty);
        assert_eq!(std::fs::read(&path).unwrap(), local);

        std::fs::write(&path, "external").unwrap();
        core.component_event(
            harness.source_input.unwrap(),
            EventKind::Focus,
            &EventPayload::None,
        );
        harness.change_input(&mut core, "?");
        blur(&mut core, &mut harness);
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            harness.drain(&mut core);
            let actual = harness.sample(&mut core, &probe);
            if actual.notice.starts_with("已暂停 named.txt 的自动保存") {
                assert!(actual.dirty);
                break;
            }
            assert!(Instant::now() < deadline, "{}", actual.notice);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}

#[test]
fn delayed_auto_save_survives_other_tabs_and_an_edit_during_publication() {
    let root = std::env::temp_dir().join(format!("texzi-auto-save-overlap-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let first = root.join("first.txt");
    let second = root.join("second.txt");
    std::fs::write(&first, "A").unwrap();
    std::fs::write(&second, "B").unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let probe = Rc::new(Probe {
        pause: RefCell::new(Some(Box::new(move |capture: &Snapshot| {
            started_tx.send(capture.body.to_utf8()).unwrap();
            resume_rx.recv_timeout(Duration::from_secs(12)).unwrap();
        }))),
        states: RefCell::new(Vec::new()),
        auto_save: Some((first.clone(), "afterDelay")),
        next_file: Some(second.clone()),
    });
    let (_runtime, mut core, mut harness) = open_shell(&probe);
    harness.change_input(&mut core, "1");
    let first_revision = harness.sample(&mut core, &probe).bytes;
    assert_eq!(first_revision, b"1A");
    harness.key(&mut core, "f6");
    core.component_event(
        harness.source_input.unwrap(),
        EventKind::Focus,
        &EventPayload::None,
    );
    harness.change_input(&mut core, "2");
    let second_revision = harness.sample(&mut core, &probe).bytes;
    assert_eq!(second_revision, b"2B");
    let deadline = Instant::now() + Duration::from_secs(9);
    loop {
        harness.drain(&mut core);
        if let Ok(bytes) = started_rx.try_recv() {
            assert_eq!(bytes, first_revision);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the second edit must not cancel A's timer"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.key(&mut core, "ctrl+pageup");
    core.component_event(
        harness.source_input.unwrap(),
        EventKind::Focus,
        &EventPayload::None,
    );
    harness.change_input(&mut core, "3");
    let new_first_revision = harness
        .samples(&mut core, &probe)
        .into_iter()
        .find(|state| state.path.as_ref() == Some(&first))
        .unwrap()
        .bytes;
    assert_ne!(first_revision, new_first_revision);
    assert_eq!(std::fs::read(&first).unwrap(), b"A");
    resume_tx.send(()).unwrap();

    let deadline = Instant::now() + Duration::from_secs(9);
    loop {
        harness.drain(&mut core);
        let states = harness.samples(&mut core, &probe);
        let a = states
            .iter()
            .find(|state| state.path.as_ref() == Some(&first))
            .unwrap();
        let b = states
            .iter()
            .find(|state| state.path.as_ref() == Some(&second))
            .unwrap();
        if !a.dirty && !b.dirty {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "pending automatic saves must complete and catch up"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(std::fs::read(&first).unwrap(), new_first_revision);
    assert_eq!(std::fs::read(&second).unwrap(), second_revision);
    std::fs::remove_file(first).unwrap();
    std::fs::remove_file(second).unwrap();
    std::fs::remove_dir(root).unwrap();
}
