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
}
