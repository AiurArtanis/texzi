use super::*;
use crate::editor_owner::NativeView;
use nsh_core::{
    ApproxTextEngine, Bounds, Core, EventKind, EventPayload, MountItem, NodeId, PayloadValue,
    ROOT_ID, Size,
    desktop::{WindowMode, WindowPlacement, WindowState},
    element::{Environment, Root},
    keymap::{Keymap, Keystroke},
    platform::Os,
};
use std::time::Instant;

struct Observed {
    source: Rc<NativeView>,
    composing: Cell<bool>,
    rich: Cell<bool>,
    second: Cell<bool>,
}

thread_local! {
    static PROBE: RefCell<Option<Rc<Observed>>> = const { RefCell::new(None) };
}

#[component]
fn CountingWorkbench() -> impl IntoElement {
    let shell = Shell::new();
    let blank = shell.current().unwrap();
    let linked = shell.add_with_mode(
        NativeView::fixture("[湖](https://hidden.example/a/b/c)"),
        Some("zen-count.md".into()),
        Some(true),
    );
    assert!(linked.displayed_rich());
    shell.activate_primary(blank.id);
    let probe = Rc::new(Observed {
        source: blank.source.clone(),
        composing: Cell::new(false),
        rich: Cell::new(false),
        second: Cell::new(false),
    });
    PROBE.with(|slot| *slot.borrow_mut() = Some(probe.clone()));
    let observed = shell.clone();
    effect(move || {
        probe.composing.set(probe.source.is_composing());
        probe.second.set(observed.second_active.get());
        probe
            .rich
            .set(observed.current().is_some_and(|doc| doc.displayed_rich()));
    });
    render_shell(shell)
}

#[derive(Default)]
struct Screen {
    keys: Option<Arc<Keymap>>,
    editor: Option<NodeId>,
    footer: Option<(NodeId, String)>,
    published: Vec<String>,
}

impl Screen {
    fn pump(&mut self, core: &mut Core<ApproxTextEngine>) {
        for _ in 0..4 {
            core.run_component_tasks();
            if let Some(batch) = core.commit() {
                for item in batch.items {
                    match item {
                        MountItem::Keymap(keys) => self.keys = Some(keys),
                        MountItem::Input { id, config } if config.multiline => {
                            self.editor = Some(id)
                        }
                        MountItem::Text { id, text, .. } => {
                            let value = text.text.as_str();
                            if value == "正在统计…"
                                || value
                                    .strip_suffix(" 字")
                                    .is_some_and(|n| n.parse::<u64>().is_ok())
                            {
                                self.published.push(value.into());
                                self.footer = Some((id, value.into()));
                            }
                        }
                        MountItem::Delete { id }
                            if self.footer.as_ref().is_some_and(|(n, _)| *n == id) =>
                        {
                            self.footer = None
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn key(&mut self, core: &mut Core<ApproxTextEngine>, name: &str) {
        let stroke = Keystroke::parse_with(name, false).unwrap();
        let binding = self
            .keys
            .as_ref()
            .unwrap()
            .find(stroke, true, false)
            .fired()
            .unwrap_or_else(|| panic!("no focused binding for {name}"));
        assert!(core.component_event(
            ROOT_ID,
            EventKind::KeyCommand,
            &EventPayload::Value(PayloadValue::Number(binding as f64))
        ));
        self.pump(core);
    }

    fn wait_for(&mut self, core: &mut Core<ApproxTextEngine>, value: &str) {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            self.pump(core);
            if self.footer.as_ref().is_some_and(|(id, s)| {
                s == value
                    && core
                        .measure(*id)
                        .is_some_and(|m| m.frame.size.height.0 > 0.0)
            }) {
                return;
            }
            let now = Instant::now();
            assert!(now < deadline, "expected {value}, saw {:?}", self.published);
            // Follow Core's actual timer. Only the mounted result settles the assertion.
            let wake = core.next_task_timer().unwrap_or(deadline).min(deadline);
            std::thread::park_timeout(wake.saturating_duration_since(now));
        }
    }
}

#[test]
fn zen_footer_tracks_committed_text_undo_latest_revision_and_visible_rich_link() {
    let size = Size::new(px(960.0), px(640.0));
    let bounds = Bounds {
        origin: point(px(0.0), px(0.0)),
        size,
    };
    let mut window = WindowState::new(bounds, WindowPlacement::new(WindowMode::Normal, bounds));
    window.focused = true;
    window.visible = true;
    let mut env = Environment::default();
    env.os = Os::Windows;
    env.window = size;
    env.window_state = Some(window);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let mut core = Core::new(ApproxTextEngine, size);
    core.set_task_runtime(runtime.handle().clone());
    core.set_environment(env);
    core.mount_root(&Root::new::<CountingWorkbench>(|| ()));
    let mut screen = Screen::default();
    screen.pump(&mut core);
    let probe = PROBE.with(|slot| slot.borrow_mut().take().unwrap());
    assert!(screen.footer.is_none());
    let sink = screen.editor.expect("mounted source editor input");
    assert!(core.component_event(sink, EventKind::Focus, &EventPayload::None));
    screen.pump(&mut core);
    screen.key(&mut core, "ctrl+k");
    screen.key(&mut core, "z");
    screen.wait_for(&mut core, "0 字");

    let compose = |core: &mut Core<ApproxTextEngine>, composing: bool, text: &str| {
        assert!(core.component_event(
            sink,
            EventKind::CompositionChange,
            &EventPayload::Map(vec![
                ("composing", PayloadValue::Bool(composing)),
                ("text", PayloadValue::Str(text.into()))
            ])
        ));
    };
    let change = |core: &mut Core<ApproxTextEngine>, count, text: &str| {
        assert!(core.component_event(
            sink,
            EventKind::ChangeText,
            &EventPayload::Text {
                count,
                text: text.into()
            }
        ));
    };
    let empty = probe.source.snapshot().unwrap().revision;
    let preedit = screen.published.len();
    compose(&mut core, true, "shanhe");
    change(&mut core, 1, "山河");
    screen.pump(&mut core);
    assert!(probe.composing.get());
    let held = probe.source.snapshot().unwrap();
    assert_eq!(held.revision, empty);
    assert_eq!(held.rope.len_bytes(), 0);
    assert!(screen.published[preedit..].iter().all(|s| s == "0 字"));
    screen.wait_for(&mut core, "0 字");

    compose(&mut core, false, "");
    screen.wait_for(&mut core, "2 字");
    assert!(!probe.composing.get());
    let committed = probe.source.snapshot().unwrap();
    assert!(committed.revision > empty);
    assert_eq!(committed.rope.to_string(), "山河");
    screen.key(&mut core, "ctrl+z");
    screen.wait_for(&mut core, "0 字");
    assert_eq!(probe.source.snapshot().unwrap().rope.len_bytes(), 0);

    change(&mut core, 2, "一二三");
    screen.pump(&mut core);
    let after_new_edit = screen.published.len();
    change(&mut core, 3, "一二三四");
    screen.wait_for(&mut core, "4 字");
    assert_eq!(
        probe.source.snapshot().unwrap().rope.to_string(),
        "一二三四"
    );
    assert!(
        screen.published[after_new_edit..]
            .iter()
            .all(|s| s == "正在统计…" || s == "4 字")
    );

    screen.key(&mut core, "ctrl+pagedown");
    screen.wait_for(&mut core, "1 字");
    assert!(probe.rich.get()); // only the displayed 湖 counts
    screen.key(&mut core, "ctrl+pageup");
    screen.wait_for(&mut core, "4 字");
    assert!(!probe.rich.get());

    screen.key(&mut core, "ctrl+\\");
    assert!(probe.second.get());
    screen.key(&mut core, "ctrl+1");
    assert!(!probe.second.get());
    screen.key(&mut core, "ctrl+pagedown");
    screen.wait_for(&mut core, "1 字");
    assert!(!probe.second.get() && probe.rich.get());
    screen.key(&mut core, "ctrl+2");
    screen.wait_for(&mut core, "4 字");
    assert!(probe.second.get() && !probe.rich.get());
}
