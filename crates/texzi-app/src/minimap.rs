use super::{divider, frame, secondary_foreground, selected_surface};
use crate::editor_owner::NativeView;
use nsh_core::{PointerEvents, prelude::*};
use std::rc::Rc;
use texzi_document::Body;

const MAX_BANDS: usize = 80;
const MAX_PREFIX: usize = 72;

#[derive(Clone, PartialEq)]
struct Miniature {
    lines: usize,
    bands: Vec<(u8, u8)>,
}

fn sample(body: &Body) -> Miniature {
    let lines = body.logical_line_count();
    let count = lines.min(MAX_BANDS);
    let bands = (0..count)
        .map(|band| {
            let line = band * lines / count;
            let range = body.logical_line_bytes(line).unwrap();
            let prefix = body.source().byte_slice(range);
            let mut indentation = 0;
            let mut non_space_end = 0;
            let mut seen_text = false;
            for (index, ch) in prefix.chars().take(MAX_PREFIX).enumerate() {
                if ch == '\n' {
                    break;
                }
                if !ch.is_whitespace() {
                    seen_text = true;
                    non_space_end = index + 1;
                } else if !seen_text {
                    indentation += 1;
                }
            }
            (
                indentation as u8,
                non_space_end.saturating_sub(indentation) as u8,
            )
        })
        .collect();
    Miniature { lines, bands }
}

pub(super) fn render(source: Rc<NativeView>, viewport: Signal<(u32, u32)>) -> impl IntoElement {
    let image = computed(move || source.current_body().map(|body| sample(&body)));
    let extent = computed(move || {
        let Some(lines) = image.with(|image| image.as_ref().map(|image| image.lines)) else {
            return (0.0, 0.0);
        };
        let (first, last) = viewport.get();
        let first = (first as usize).min(lines - 1);
        let last = (last as usize + 1).clamp(first + 1, lines);
        (
            first as f32 / lines as f32,
            (last - first) as f32 / lines as f32,
        )
    });
    view()
        .relative()
        .w(px(64.0))
        .flex_shrink_0()
        .h_full()
        .border_l(px(1.0))
        .border_color(divider)
        .bg(frame)
        .pointer_events(PointerEvents::None)
        .child(
            view()
                .absolute()
                .left_0()
                .right_0()
                .top(move || relative(extent.get().0))
                .h(move || relative(extent.get().1))
                .min_h(px(3.0))
                .bg(selected_surface),
        )
        .child(dynamic(move || {
            let image = image.get()?;
            let count = image.bands.len();
            Some(
                view()
                    .absolute()
                    .inset_0()
                    .children(image.bands.into_iter().enumerate().map(
                        move |(index, (indentation, length))| {
                            view()
                                .absolute()
                                .top(relative(index as f32 / count as f32))
                                .left(px(4.0 + f32::from(indentation) * 0.75))
                                .w(px(f32::from(length) * 0.75))
                                .h(px(1.5))
                                .bg(secondary_foreground)
                                .opacity(0.48)
                        },
                    )),
            )
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_base::editor::{Editor, EditorApi, EditorRange};
    use nsh_core::{
        ApproxTextEngine, Core, EventKind, EventPayload, MountItem, PayloadValue, ROOT_ID, Size,
        element::Root,
        geometry::point,
        keymap::{Keymap, Keystroke},
    };
    use std::{cell::Cell, sync::Arc};

    #[test]
    fn sample_preserves_indentation_and_uses_only_bounded_line_prefixes() {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        let repeated = " xyz\n".repeat(3_000);
        let source = NativeView::fixture(&format!("    fn main() {{\u{2028}}}\n{repeated}"));
        let image = sample(&source.current_body().unwrap());
        assert_eq!(image.lines, 3002);
        assert_eq!(image.bands.len(), MAX_BANDS);
        assert_eq!(image.bands[0].0, 4);
        assert_eq!(image.bands[1], (1, 3));

        let single = NativeView::fixture(&"x".repeat(5_000));
        let image = sample(&single.current_body().unwrap());
        assert_eq!(image.bands, vec![(0, MAX_PREFIX as u8)]);
    }

    thread_local! {
        static REFLOW: Cell<Option<(EditorRange, f32)>> = const { Cell::new(None) };
    }

    #[component]
    fn ReflowFixture() -> impl IntoElement {
        let minimap_shown = signal(false);
        let editor = component_ref::<Editor>();
        let line = "0123456789".repeat(36);
        let body = (0..40)
            .map(|n| format!("{n:02} {line}\n"))
            .collect::<String>();
        on_key("f6", move |_| {
            editor
                .api::<EditorApi>()
                .unwrap()
                .set_scroll_offset(point(px(0.0), px(950.0)));
        });
        on_key("f7", move |_| {
            let api = editor.api::<EditorApi>().unwrap();
            REFLOW.with(|slot| slot.set(Some((api.visible_range(), api.scroll_offset().y.0))));
        });
        on_key("f8", move |_| minimap_shown.set(true));
        view()
            .size_full()
            .flex_row()
            .child(
                nsh_base::editor::editor()
                    .component_ref(&editor)
                    .default_value(Some(body.into()))
                    .soft_wrap(true)
                    .anchor_wrap_reflow(true)
                    .font_size(px(16.0))
                    .line_height(px(24.0))
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full(),
            )
            .child(when(
                move || minimap_shown.get(),
                || view().w(px(64.0)).flex_shrink_0().h_full(),
            ))
    }

    #[test]
    fn narrower_minimap_layout_holds_the_top_logical_line() {
        let mut core = Core::new(
            ApproxTextEngine,
            Size {
                width: px(620.0),
                height: px(480.0),
            },
        );
        core.mount_root(&Root::new::<ReflowFixture>(|| ()));
        let mut keys: Option<Arc<Keymap>> = None;
        for _ in 0..10 {
            core.run_component_tasks();
            if let Some(batch) = core.commit() {
                for item in batch.items {
                    if let MountItem::Keymap(keymap) = item {
                        keys = Some(keymap);
                    }
                }
            }
        }
        let keys = keys.unwrap();
        let press = |core: &mut Core<ApproxTextEngine>, key| {
            let key = Keystroke::parse_with(key, false).unwrap();
            let command = keys.find(key, true, false).fired().unwrap();
            assert!(core.component_event(
                ROOT_ID,
                EventKind::KeyCommand,
                &EventPayload::Value(PayloadValue::Number(command as f64))
            ));
            for _ in 0..10 {
                core.run_component_tasks();
                let _ = core.commit();
            }
        };
        press(&mut core, "f6");
        press(&mut core, "f7");
        let (before, old_y) = REFLOW.with(Cell::take).unwrap();
        assert!(
            before.start.line > 0,
            "the fixture must be below the first folded display rows"
        );
        press(&mut core, "f8");
        press(&mut core, "f7");
        let (after, new_y) = REFLOW.with(Cell::take).unwrap();
        assert_ne!(
            old_y, new_y,
            "the fixture must actually rewrap above the viewport"
        );
        assert_eq!(before.start.line, after.start.line);
    }
}
