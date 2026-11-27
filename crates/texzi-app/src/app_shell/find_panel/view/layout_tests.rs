use super::*;
use nsh_core::{
    ApproxTextEngine, Core, EventKind, EventPayload, Measurement, MountItem, NodeId, Size,
    element::{Environment, Root},
    platform::Os,
};
use std::collections::HashMap;

const COUNT: &str = "源码 120000000/190000000";
const FIRST_ROW: [&str; 11] = [
    "显示或收起替换",
    "查找文字",
    COUNT,
    "区分大小写",
    "整词匹配",
    "正则表达式",
    "仅在选区内查找",
    "选中全部匹配",
    "上一个",
    "下一个",
    "关闭文内查找",
];
const SECOND_ROW: [&str; 3] = ["替换为", "替换当前", "全部替换"];

#[component]
fn SplitFindFixture() -> impl IntoElement {
    let shell = Shell::new();
    shell.add(NativeView::fixture("one two"), None);
    let doc = shell.current().unwrap();
    doc.gate.source_mounted();
    open(&shell, false);
    shell
        .find_panel
        .status
        .set("源码 · 120000000/190000000".into());
    view()
        .size_full()
        .flex_row()
        .gap(px(5.0))
        .child(view().flex_1().min_w(px(0.0)))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .relative()
                .child(overlay(shell, doc)),
        )
}

fn settle(core: &mut Core<ApproxTextEngine>, names: &mut HashMap<String, NodeId>) {
    for _ in 0..4 {
        core.run_component_tasks();
        if let Some(batch) = core.commit() {
            for item in batch.items {
                match item {
                    MountItem::ViewProps { id, props } => {
                        if let Some(label) = props.accessibility.label.as_deref() {
                            names.insert(label.into(), id);
                        }
                    }
                    MountItem::Input { id, config } => {
                        names.insert(config.placeholder, id);
                    }
                    MountItem::Text { id, text, .. } => {
                        names.entry(text.text.clone()).or_insert(id);
                    }
                    MountItem::Delete { id } => names.retain(|_, node| *node != id),
                    _ => {}
                }
            }
        }
    }
}

fn center_y(rect: Measurement) -> f32 {
    rect.page.y.0 + rect.frame.size.height.0 * 0.5
}

#[test]
fn find_overlay_wraps_in_160_dip_group_and_preserves_mounted_inputs_and_controls() {
    let size = |width| Size {
        width: px(width),
        height: px(600.0),
    };
    let mut core = Core::new(ApproxTextEngine, size(1125.0));
    let mut env = Environment::default();
    env.os = Os::Windows;
    core.set_environment(env);
    core.mount_root(&Root::new::<SplitFindFixture>(|| ()));
    let mut names = HashMap::new();
    settle(&mut core, &mut names);
    let measured = |core: &Core<ApproxTextEngine>, nodes: &HashMap<String, NodeId>, label: &str| {
        core.measure(nodes[label])
            .unwrap_or_else(|| panic!("{label} must be mounted"))
    };
    let row_y = center_y(measured(&core, &names, "查找文字"));
    for name in FIRST_ROW {
        assert!(
            (center_y(measured(&core, &names, name)) - row_y).abs() <= 1.0,
            "{name}"
        );
    }
    let start = measured(&core, &names, "显示或收起替换");
    let end = measured(&core, &names, "关闭文内查找");
    assert!((end.page.x.0 + end.frame.size.width.0 - start.page.x.0 - 384.0).abs() <= 1.0);
    assert!(!names.contains_key("替换为"));
    assert!(core.component_event(
        names["显示或收起替换"],
        EventKind::Press,
        &EventPayload::None
    ));
    settle(&mut core, &mut names);
    let replace_y = center_y(measured(&core, &names, "替换为"));
    assert!(replace_y > row_y);
    for name in SECOND_ROW {
        assert!(
            (center_y(measured(&core, &names, name)) - replace_y).abs() <= 1.0,
            "{name}"
        );
    }

    let query = names["查找文字"];
    let replacement = names["替换为"];
    core.set_viewport(size(325.0)); // 160 + 5 + 160 DIP
    settle(&mut core, &mut names);
    assert_eq!((names["查找文字"], names["替换为"]), (query, replacement));
    assert!(measured(&core, &names, "查找文字").frame.size.width.0 >= 64.0);
    assert!(measured(&core, &names, "替换为").frame.size.width.0 >= 80.0);
    let mut placed = Vec::new();
    for name in FIRST_ROW.into_iter().chain(SECOND_ROW) {
        let rect = measured(&core, &names, name);
        let (x, y) = (rect.page.x.0, rect.page.y.0);
        let (w, h) = (rect.frame.size.width.0, rect.frame.size.height.0);
        assert!(
            w > 0.0 && h > 0.0 && x >= 165.0 && x + w <= 325.0 && y + h <= 600.0,
            "{name}"
        );
        for &(other, x0, y0, x1, y1) in &placed {
            assert!(
                x + w <= x0 || x1 <= x || y + h <= y0 || y1 <= y,
                "{name} overlaps {other}"
            );
        }
        placed.push((name, x, y, x + w, y + h));
    }
    assert!(
        center_y(measured(&core, &names, "关闭文内查找"))
            > center_y(measured(&core, &names, "查找文字"))
    );
}
