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
    frames: HashMap<NodeId, Bounds<Points>>,
    content: HashMap<NodeId, Size<Points>>,
    requests: Vec<(NodeId, u32, u64, f32)>,
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
                        MountItem::Frame { id, frame } => {
                            self.frames.insert(id, frame);
                        }
                        MountItem::ScrollContent { id, size } => {
                            self.content.insert(id, size);
                        }
                        MountItem::ViewProps { id, props }
                            if props.test_id.as_deref() == Some("texzi.rich.thumb-target") =>
                        {
                            self.receiver = Some(id)
                        }
                        MountItem::Text { id, text, .. } => {
                            if let Some(n) = text.text.strip_prefix("row-") {
                                self.rows.insert(
                                    id,
                                    n.split_ascii_whitespace().next().unwrap().parse().unwrap(),
                                );
                            }
                        }
                        MountItem::Delete { id } => {
                            self.rows.remove(&id);
                        }
                        MountItem::Command {
                            id,
                            command: Command::Native { name, args },
                        } if Some(id) == self.scroll && name.as_ref() == "texziApplyRichThumb" => {
                            self.requests.push((
                                args[0].as_u64().unwrap() as NodeId,
                                args[1].as_u64().unwrap() as u32,
                                args[2].as_u64().unwrap(),
                                args[3].as_f64().unwrap() as f32,
                            ));
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

    fn row_top(&self, mut id: NodeId, scroller: NodeId) -> f32 {
        let mut y = 0.0;
        while id != scroller {
            y += self.frames.get(&id).map_or(0.0, |frame| frame.origin.y.0);
            id = self.parents[&id];
        }
        y
    }
}

#[component]
fn MixedThumbFixture() -> impl IntoElement {
    let blocks = (1..=4097)
        .map(|n| {
            let extra = if n <= 64 {
                " wrapped text".repeat(80)
            } else {
                String::new()
            };
            json!({"type":"paragraph","content":[{"type":"text","text":format!("row-{n}{extra}")}]})
        })
        .collect::<Vec<_>>();
    nsh_base::rich_editor::view::rich_editor()
        .content(json!({"type":"doc","content":blocks}))
        .classes(crate::app::rich_classes())
        .size_full()
}

fn measured_mixed_page(fraction: f64) -> (Mounted, NodeId, f32) {
    let mut core = Core::new(ApproxTextEngine, Size::new(px(800.0), px(600.0)));
    let mut environment = Environment::default();
    environment.os = Os::Windows;
    core.set_environment(environment);
    core.mount_root(&Root::new::<MixedThumbFixture>(|| ()));
    let mut mounted = Mounted::default();
    mounted.flush(&mut core);
    let outer = mounted.parents[&mounted.receiver.unwrap()];
    mounted.scroll = Some(outer);
    let initial_max = mounted.content[&outer].height.0 - mounted.frames[&outer].size.height.0;
    mounted.event(&mut core, "thumbBegin", json!([7]));
    mounted.event(&mut core, "thumbTarget", json!([7, fraction]));
    mounted.flush(&mut core);
    for task in core.take_main_thread_tasks() {
        task.run();
    }
    mounted.flush(&mut core);
    (mounted, outer, initial_max)
}

#[test]
fn a_short_final_block_keeps_the_clamped_viewport_covered_after_heights_change() {
    let (mounted, outer, initial_max) = measured_mixed_page(1.0);
    assert_eq!(mounted.requests.last().unwrap().3, 1.0);
    let height = mounted.frames[&outer].size.height.0;
    let bottom = mounted.content[&outer].height.0;
    let top = bottom - height;
    let mut visible: Vec<_> = mounted
        .rows
        .iter()
        .filter_map(|(&id, &n)| {
            let y = mounted.row_top(id, outer);
            (y >= top && y < bottom).then_some((n, y))
        })
        .collect();
    visible.sort_by_key(|&(row, _)| row);
    eprintln!(
        "clamped thumb initial={initial_max} max={top} viewport={height} visible={visible:?}"
    );
    assert!(
        initial_max > top + height,
        "the measured short blocks must shrink the estimated content"
    );
    assert!(
        visible.iter().any(|&(_, y)| y < top + height * 0.25),
        "top quarter of the clamped page is empty"
    );
    assert!(
        visible.iter().any(|&(_, y)| y > top + height * 0.75),
        "bottom quarter of the clamped page is empty"
    );
    assert!(visible.windows(2).all(|rows| rows[0].0 + 1 == rows[1].0));
    assert!(mounted.contains(4097));
}

#[test]
fn a_released_half_track_grab_keeps_its_fraction_after_measuring_shorter_blocks() {
    let (mounted, outer, initial_max) = measured_mixed_page(0.5);
    let height = mounted.frames[&outer].size.height.0;
    let max = mounted.content[&outer].height.0 - height;
    let fraction = mounted.requests.last().unwrap().3;
    assert_eq!(fraction, 0.5);
    assert!(initial_max - max > height);
    let top = max * fraction;
    let mut rows: Vec<_> = mounted
        .rows
        .iter()
        .filter_map(|(&id, &n)| {
            let y = mounted.row_top(id, outer) - top;
            (0.0 <= y && y < height).then_some((n, y))
        })
        .collect();
    rows.sort_by_key(|&(n, _)| n);
    eprintln!("half track initial={initial_max} max={max} actual={top} visible={rows:?}");
    assert!(rows.first().is_some_and(|&(_, y)| y < height * 0.25));
    assert!(rows.last().is_some_and(|&(_, y)| y > height * 0.75));
    assert!(rows.windows(2).all(|pair| pair[0].0 + 1 == pair[1].0));
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
    mounted.requests.clear();

    let receiver = mounted.receiver.unwrap();
    mounted.event(&mut core, "thumbBegin", json!([21]));
    mounted.event(&mut core, "thumbTarget", json!([21, 1.0]));
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
        mounted.requests.is_empty(),
        "the platform has not applied the target mount"
    );

    mounted.event(&mut core, "thumbTarget", json!([21, 0.5]));
    mounted.event(&mut core, "thumbTarget", json!([21, 0.25]));
    for task in core.take_main_thread_tasks() {
        task.run();
    }
    mounted.flush(&mut core);
    let [(node, 21, ticket, fraction)] = mounted.requests[..] else {
        panic!("expected one mounted target: {:?}", mounted.requests)
    };
    assert_eq!(node, receiver);
    assert_eq!(fraction, 1.0);
    let to = mounted.content[&outer].height.0 - mounted.frames[&outer].size.height.0;
    assert!(to > 10_000.0 && mounted.contains(1) && mounted.contains(4096));
    mounted.event(&mut core, "thumbApplied", json!([20, ticket, to]));
    mounted.event(&mut core, "thumbApplied", json!([21, ticket + 1, to]));
    mounted.flush(&mut core);
    assert!(mounted.contains(1) && mounted.contains(4096));
    mounted.event(&mut core, "thumbApplied", json!([21, ticket, to]));
    mounted.flush(&mut core);
    assert!(!mounted.contains(1) && !mounted.contains(2048) && mounted.contains(4096));
    assert!(
        mounted.rows.values().any(|&n| (900..1150).contains(&n)),
        "latest queued target"
    );
    assert!(mounted.rows.len() < 512);

    // The quarter-page request is waiting for its mount. A reflow must
    // discard that destination even when its earlier fence runs later.
    core.set_viewport(Size::new(px(520.0), px(420.0)));
    mounted.flush(&mut core);
    assert_ne!(mounted.receiver, Some(receiver));
    for task in core.take_main_thread_tasks() {
        task.run();
    }
    mounted.flush(&mut core);
    assert_eq!(mounted.requests.len(), 1, "stale layout issued a command");
    assert!(!mounted.rows.values().any(|&n| (900..1150).contains(&n)));

    mounted.event(&mut core, "thumbBegin", json!([22]));
    mounted.event(&mut core, "thumbTarget", json!([22, 0.5]));
    mounted.flush(&mut core);
    mounted.event(&mut core, "thumbCancel", json!([22]));
    mounted.event(&mut core, "thumbBegin", json!([23]));
    mounted.event(&mut core, "thumbTarget", json!([23, 0.25]));
    for task in core.take_main_thread_tasks() {
        task.run();
    }
    mounted.flush(&mut core);
    assert_eq!(mounted.requests.len(), 2);
    let (new_node, new_grab, _, _) = mounted.requests[1];
    assert_eq!(Some(new_node), mounted.receiver);
    assert_eq!(new_grab, 23);
}
