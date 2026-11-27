//! Count the committed text in the active Zen editor.
mod counter;
#[cfg(test)]
mod fixtures;
mod jobs;
#[cfg(test)]
mod volume_tests;

use super::*;
use jobs::{Input, Jobs};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Target {
    tab: u64,
    second: bool,
    revision: u64,
    rich: bool,
}

impl Target {
    fn current_in(self, shell: &Shell) -> bool {
        shell.zen.get()
            && shell.second_active.get() == self.second
            && shell.current().is_some_and(|doc| {
                doc.id == self.tab
                    && doc.editable_body_ready()
                    && doc.displayed_rich() == self.rich
                    && doc
                        .source
                        .body_snapshot()
                        .is_some_and(|s| s.revision.0 == self.revision)
            })
    }
}

pub(super) fn footer(shell: Rc<Shell>) -> impl IntoElement {
    let jobs = Rc::new(Jobs::new());
    let label = signal(String::new());
    let pending = signal(None::<(Target, u64)>);
    let requested = Cell::new(None::<Target>);
    let observe = shell.clone();
    let queue = jobs.clone();
    effect(move || {
        let reset = |value: String| {
            if requested.take().is_some() {
                queue.cancel();
            }
            pending.set(None);
            label.set(value);
        };
        let Some(doc) = observe.current() else {
            return reset(String::new());
        };
        if let Some(large) = &doc.large {
            return reset(
                large
                    .visible_byte_position()
                    .unwrap_or_else(|| large.status()),
            );
        }
        let Some(snapshot) = doc.source.body_snapshot() else {
            return reset(String::new());
        };
        let target = Target {
            tab: doc.id,
            second: observe.second_active.get(),
            revision: snapshot.revision.0,
            rich: doc.displayed_rich(),
        };
        if requested.get() == Some(target) {
            return;
        }
        if snapshot.body.len_bytes() == 0 {
            reset("0 字".into());
            requested.set(Some(target));
            return;
        }
        let input = if target.rich {
            // A sibling can publish a Store revision before its rich projection
            // is ready. The projection's revision signal wakes this observer.
            let Some(root) = doc.gate.visible_text_root(target.revision) else {
                return reset("正在统计…".into());
            };
            Input::Rich(root)
        } else {
            Input::Source(snapshot.body)
        };
        requested.set(Some(target));
        label.set("正在统计…".into());
        pending.set(Some((target, queue.submit(input))));
    });
    let results = jobs.clone();
    effect(move || {
        if let Some((target, generation)) = pending.get() {
            let results = results.clone();
            let shell = shell.clone();
            let timer = set_interval(Duration::from_millis(32), move || {
                if let Some(count) = results.take(generation)
                    && target.current_in(&shell)
                {
                    label.set(format!("{count} 字"));
                    pending.set(None);
                }
            });
            cleanup(move || timer.cancel());
        }
    });
    cleanup(move || jobs.cancel());
    view()
        .w_full()
        .items_end()
        .bg(|| color("paper"))
        .pr(px(12.0))
        .py(px(2.0))
        .child(when(move || !label.with(String::is_empty), move || {
            text(move || label.get())
                .label(move || label.get())
                .accessible(true)
                .role(AccessibilityRole::Text)
                .focusable(true)
                .tabular_nums()
                .text_size(px(font_size("meta")))
                .text_color(|| color("muted-readable"))
                .app_class("focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring")
        }))
}
