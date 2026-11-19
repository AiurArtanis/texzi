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
use texzi_document::{Revision, SaveCapture, StateToken};

type Hook = Box<dyn FnOnce(&SaveCapture) + Send>;

struct Probe {
    pause: RefCell<Option<Hook>>,
    state: RefCell<Option<EditorState>>,
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
    let doc = shell.current().unwrap();
    doc.source
        .before_next_publication(probe.pause.borrow_mut().take().unwrap());
    let observed = shell.clone();
    on_key("f7", move |_| {
        let (bytes, revision, token, dirty) = doc.source.inspect(|store, view| {
            let id = store.view(view).unwrap().doc;
            let snapshot = store.snapshot(id).unwrap();
            (
                snapshot.body.to_utf8(),
                snapshot.revision,
                snapshot.token,
                store.dirty(id).unwrap(),
            )
        });
        *probe.state.borrow_mut() = Some(EditorState {
            bytes,
            revision,
            token,
            dirty,
            path: doc.path.get(),
            notice: observed.notice.get(),
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
        self.key(core, "f7");
        probe.state.borrow_mut().take().unwrap()
    }
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
        pause: RefCell::new(Some(Box::new(move |capture: &SaveCapture| {
            assert_ne!(ui, std::thread::current().id());
            started_tx.send(capture.snapshot.clone()).unwrap();
            resume_rx.recv_timeout(Duration::from_secs(15)).unwrap();
        }))),
        state: RefCell::new(None),
    });
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
