//! Exercise the actual rich editor mount batches while withholding the
//! Windows mount fence and native offset receipt independently.
use std::collections::{HashMap, HashSet};

use nsh_core::{
    ApproxTextEngine, Command, Core, EventKind, EventPayload, MountItem, NodeId, ViewKind,
    element::{Environment, JsEvent, Root},
    platform::Os,
    prelude::*,
};
use serde_json::{Value, json};

#[component]
fn ThumbFixture() -> impl IntoElement {
    let blocks = (1..=4096)
        .map(|n| json!({"type":"paragraph","content":[{"type":"text","text":format!("row-{n}")}]}))
        .collect::<Vec<_>>();
    nsh_base::rich_editor::view::rich_editor()
        .content(json!({"type":"doc","content":blocks}))
        .classes(crate::app::rich_classes())
        .size_full()
}

#[derive(Default)]
struct Mounted {
    receiver: Option<NodeId>,
    scroll: Option<NodeId>,
    scrolls: HashSet<NodeId>,
    parents: HashMap<NodeId, NodeId>,
    rows: HashMap<NodeId, usize>,
    offsets: Vec<f32>,
}

impl Mounted {
    fn flush(&mut self, core: &mut Core<ApproxTextEngine>) {
        for _ in 0..8 {
            core.run_component_tasks();
            if let Some(batch) = core.commit() {
                for item in batch.items {
                    match item {
                        MountItem::Create {
                            id,
                            kind: ViewKind::Scroll,
                        } => {
                            self.scrolls.insert(id);
                        }
                        MountItem::Insert { parent, child, .. } => {
                            self.parents.insert(child, parent);
                        }
                        MountItem::ViewProps { id, props }
                            if props.test_id.as_deref() == Some("texzi.rich.thumb-target") =>
                        {
                            self.receiver = Some(id)
                        }
                        MountItem::Text { id, text, .. } => {
                            if let Some(n) = text.text.strip_prefix("row-") {
                                self.rows.insert(id, n.parse().unwrap());
                            }
                        }
                        MountItem::Delete { id } => {
                            self.rows.remove(&id);
                        }
                        MountItem::Command {
                            id,
                            command: Command::ScrollTo { y, .. },
                        } if Some(id) == self.scroll => {
                            self.offsets.push(y.0);
                        }
                        _ => {}
                    }
                }
            }
            let mut events = Vec::new();
            core.take_events(&mut events);
            for event in events {
                match event {
                    JsEvent::Event(id, kind, payload) => {
                        core.component_event(id, kind, &payload);
                    }
                    JsEvent::LayoutListener {
                        host,
                        id,
                        listener,
                        frame,
                    } => core.component_layout(host, id, &listener, frame),
                    _ => {}
                }
            }
        }
    }

    fn event(&self, core: &mut Core<ApproxTextEngine>, name: &str, data: Value) {
        assert!(core.component_event(
            self.receiver.unwrap(),
            EventKind::NativeEvent,
            &EventPayload::Native {
                name: name.into(),
                data
            }
        ));
    }

    fn contains(&self, row: usize) -> bool {
        self.rows.values().any(|&n| n == row)
    }
}

#[test]
fn a_fast_thumb_keeps_both_pages_until_their_mount_and_scroll_are_acknowledged() {
    let mut core = Core::new(ApproxTextEngine, Size::new(px(800.0), px(600.0)));
    let mut environment = Environment::default();
    environment.os = Os::Windows;
    core.set_environment(environment);
    core.mount_root(&Root::new::<ThumbFixture>(|| ()));
    let mut mounted = Mounted::default();
    mounted.flush(&mut core);
    let outer = mounted.parents[&mounted.receiver.unwrap()];
    assert!(mounted.scrolls.contains(&outer));
    mounted.scroll = Some(outer);
    assert!(mounted.contains(1) && !mounted.contains(4096));
    mounted.offsets.clear();

    mounted.event(&mut core, "thumbTarget", json!(1.0));
    mounted.flush(&mut core);
    assert!(
        mounted.contains(1) && mounted.contains(4096),
        "mounted {} rows from {:?} to {:?}, viewport {:?}",
        mounted.rows.len(),
        mounted.rows.values().min(),
        mounted.rows.values().max(),
        core.scroll_listener(mounted.scroll.unwrap())
    );
    assert!(!mounted.contains(2048) && mounted.rows.len() < 512);
    assert!(
        mounted.offsets.is_empty(),
        "the platform has not applied the target mount"
    );

    mounted.event(&mut core, "thumbTarget", json!(0.5));
    mounted.event(&mut core, "thumbTarget", json!(0.25));
    for task in core.take_main_thread_tasks() {
        task.run();
    }
    mounted.flush(&mut core);
    let [to] = mounted.offsets[..] else {
        panic!("expected one mounted target: {:?}", mounted.offsets)
    };
    assert!(to > 10_000.0 && mounted.contains(1) && mounted.contains(4096));
    mounted.event(&mut core, "thumbApplied", json!([to, to]));
    mounted.flush(&mut core);
    assert!(!mounted.contains(1) && !mounted.contains(2048) && mounted.contains(4096));
    assert!(
        mounted.rows.values().any(|&n| (900..1150).contains(&n)),
        "latest queued target"
    );
    assert!(mounted.rows.len() < 512);
}
