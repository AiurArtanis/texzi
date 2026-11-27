//! Run each ignored case alone with --test-threads=1 --nocapture.
//! Paths are supplied by the operator and opened read-only. The 100 MB case
//! gives the editable in-memory view no destination for saving.
use super::*;
use nsh_base::editor::{ExternalEditKind, ExternalSelections};
use nsh_core::{
    ApproxTextEngine, Bounds, Core, EventKind, EventPayload, MountItem, NodeId, Size,
    desktop::{WindowMode, WindowPlacement, WindowState},
    element::{Environment, Root},
    platform::Os,
};
use std::{collections::HashMap, ffi::c_void, time::Instant};
use texzi_document::files::paged::{FileSource, PAGE_BYTES};

const TWO_GIB: u64 = 2 * 1024 * 1024 * 1024;
const ONE_HUNDRED_MB: u64 = 100_000_000;
const CHOICE: &str = "大文件 · 只读分页 · 2048.0 MiB";
const PENDING: &str = "正在统计…";
const NOTIFY: &str = "重复通知同一正文修订";
const APPEND: &str = "提交山河到正文末尾";

fn fixture(variable: &str, len: u64) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(variable).unwrap_or_else(|| {
        panic!("set {variable} to an existing synthetic {len}-byte UTF-8 text fixture")
    }));
    let metadata = path.metadata().unwrap();
    assert!(metadata.is_file());
    assert_eq!(metadata.len(), len, "wrong length for {variable}");
    path
}

#[component]
fn PagedWorkbench() -> impl IntoElement {
    let path = fixture("TEXZI_ZEN_VOLUME_FIXTURE", TWO_GIB);
    let shell = Shell::new();
    let file = FileSource::open(&path).unwrap();
    let document = shell.open_large(path, Arc::new(file), None, None, None);
    assert!(!document.editable_body_ready());
    shell.run("texzi.view.toggleZen");
    assert!(shell.zen.get());
    render_shell(shell)
}

#[component]
fn EditableFooter() -> impl IntoElement {
    let path = fixture("TEXZI_ZEN_100M_FIXTURE", ONE_HUNDRED_MB);
    let shell = Shell::new();
    let source = NativeView::open_file(&path).unwrap();
    assert_eq!(source.disk_len(), ONE_HUNDRED_MB);
    let doc = shell.add_with_mode(source.clone(), None, Some(false));
    assert!(doc.editable_body_ready() && !doc.displayed_rich());
    shell.run("texzi.view.toggleZen");
    let notify = source.clone();
    // Exercise the production footer independently of large-file editor layout.
    view()
        .size_full()
        .child(footer(shell))
        .child(
            view()
                .label(NOTIFY)
                .child(text(NOTIFY))
                .on_press(move |_: &()| notify.notify()),
        )
        .child(
            view()
                .label(APPEND)
                .child(text(APPEND))
                .on_press(move |_: &()| {
                    let before = source.snapshot().unwrap();
                    let end = before.rope.len_bytes();
                    let inserted = " 山河 ";
                    let caret = end + inserted.len();
                    let committed = source
                        .edit(
                            before.revision,
                            before.view_epoch,
                            vec![(end..end, inserted.into())],
                            ExternalSelections {
                                ranges: vec![(caret, caret)],
                                primary: 0,
                            },
                            ExternalEditKind::Other,
                        )
                        .unwrap();
                    assert_eq!(committed.snapshot.revision, before.revision + 1);
                    assert_eq!(committed.snapshot.rope.len_bytes(), caret);
                    source.notify();
                }),
        )
}

fn counted(label: &str) -> Option<u64> {
    label.strip_suffix(" 字")?.parse().ok()
}

fn byte_range(label: &str) -> Option<(u64, u64)> {
    let (range, total) = label.strip_prefix("字节 [")?.split_once(") / ")?;
    let (start, end) = range.split_once(", ")?;
    let total: u64 = total.strip_suffix(" B")?.parse().ok()?;
    (total == TWO_GIB).then_some((start.parse().ok()?, end.parse().ok()?))
}

fn footer_label(label: &str) -> bool {
    label == CHOICE || label == PENDING || counted(label).is_some() || byte_range(label).is_some()
}

#[derive(Default)]
struct Screen {
    actions: HashMap<String, NodeId>,
    footer: Option<(NodeId, String)>,
    text: HashMap<NodeId, String>,
    publications: Vec<String>,
}

impl Screen {
    fn pump(&mut self, core: &mut Core<ApproxTextEngine>) {
        for _ in 0..3 {
            core.run_component_tasks();
            if let Some(batch) = core.commit() {
                for item in batch.items {
                    match item {
                        MountItem::ViewProps { id, props } => {
                            let Some(label) = props.accessibility.label.as_deref() else {
                                continue;
                            };
                            if matches!(label, "只读分页查看" | "末页" | NOTIFY | APPEND) {
                                self.actions.insert(label.to_owned(), id);
                            }
                            // The inspector's byte-range text also changes on each
                            // page. Only the Zen footer has this focusable Text label.
                            if props.accessibility.role == AccessibilityRole::Text
                                && props.focusable == Some(true)
                                && footer_label(label)
                            {
                                self.footer = Some((id, label.to_owned()));
                                self.publications.push(label.to_owned());
                            }
                        }
                        MountItem::Text { id, text, .. } if footer_label(text.text.as_str()) => {
                            self.text.insert(id, text.text.as_str().to_owned());
                        }
                        // Core sends an accessibility-only delta when this same
                        // mounted Text changes its label on the next revision/page.
                        MountItem::AccessibilityLabel { id, label }
                            if self.footer.as_ref().is_some_and(|(node, _)| *node == id) =>
                        {
                            let label = label.as_deref().unwrap_or_default().to_owned();
                            self.publications.push(label.clone());
                            self.footer = Some((id, label));
                        }
                        MountItem::Delete { id } => {
                            self.actions.retain(|_, node| *node != id);
                            self.text.remove(&id);
                            if self.footer.as_ref().is_some_and(|(node, _)| *node == id) {
                                self.footer = None;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn press(&mut self, core: &mut Core<ApproxTextEngine>, label: &str) {
        let id = self.actions[label];
        assert!(
            core.component_event(id, EventKind::Press, &EventPayload::None),
            "{label}"
        );
    }

    fn wait(
        &mut self,
        core: &mut Core<ApproxTextEngine>,
        matches: impl Fn(&str) -> bool,
    ) -> String {
        // A diagnostic watchdog only. The duration is not a performance target.
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            self.pump(core);
            if let Some((id, label)) = &self.footer
                && matches(label)
                && self.text.get(id) == Some(label)
                && core
                    .measure(*id)
                    .is_some_and(|m| m.frame.size.height.0 > 0.0)
            {
                return label.clone();
            }
            assert!(
                Instant::now() < deadline,
                "footer labels {:?}, last {:?}, text {:?}, height {:?}, actions {:?}",
                self.publications,
                self.footer,
                self.text,
                self.footer
                    .as_ref()
                    .and_then(|(id, _)| core.measure(*id))
                    .map(|m| m.frame.size.height),
                self.actions.keys().collect::<Vec<_>>(),
            );
            std::thread::sleep(Duration::from_millis(4));
        }
    }

    fn settle(&mut self, core: &mut Core<ApproxTextEngine>) {
        // Includes the worker debounce and the footer result poll so a late
        // count appearing in place of the chosen file/page is observable.
        let until = Instant::now() + Duration::from_millis(250);
        while Instant::now() < until {
            self.pump(core);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn environment(runtime: &tokio::runtime::Runtime) -> Core<ApproxTextEngine> {
    let size = Size::new(px(1100.0), px(760.0));
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
    let mut core = Core::new(ApproxTextEngine, size);
    core.set_task_runtime(runtime.handle().clone());
    core.set_environment(env);
    core
}

fn process_read_bytes() -> u64 {
    #[repr(C)]
    #[derive(Default)]
    struct IoCounters {
        read_operations: u64,
        write_operations: u64,
        other_operations: u64,
        read_bytes: u64,
        write_bytes: u64,
        other_bytes: u64,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn GetProcessIoCounters(process: *mut c_void, counters: *mut IoCounters) -> i32;
    }
    let mut counters = IoCounters::default();
    // IO_COUNTERS has six ULONGLONGs. The kernel fills our live, writable
    // struct for the current process; the returned pseudo-handle is not owned.
    let ok = unsafe { GetProcessIoCounters(GetCurrentProcess(), &mut counters) };
    assert_ne!(ok, 0, "{}", std::io::Error::last_os_error());
    counters.read_bytes
}

#[test]
#[ignore = "requires TEXZI_ZEN_VOLUME_FIXTURE and an isolated Windows test process"]
fn zen_2g_choice_and_pages_show_byte_status_without_scanning() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let mut core = environment(&runtime);
    let mut screen = Screen::default();
    let initial = process_read_bytes();
    core.mount_root(&Root::new::<PagedWorkbench>(|| ()));
    screen.wait(&mut core, |label| label == CHOICE);
    screen.settle(&mut core);
    let at_choice = process_read_bytes();
    screen.press(&mut core, "只读分页查看");
    let first = screen.wait(&mut core, |label| {
        byte_range(label).is_some_and(|(start, end)| {
            start <= 3 && end > start && end - start <= PAGE_BYTES as u64
        })
    });
    let at_first = process_read_bytes();
    screen.press(&mut core, "末页");
    let last = screen.wait(&mut core, |label| {
        byte_range(label).is_some_and(|(start, end)| {
            end == TWO_GIB && end > start && end - start <= PAGE_BYTES as u64
        })
    });
    screen.settle(&mut core);
    let final_read = process_read_bytes();
    assert!(
        screen
            .publications
            .iter()
            .all(|s| s == CHOICE || byte_range(s).is_some()),
        "an editable count appeared for the read-only tab: {:?}",
        screen.publications
    );
    let (_, first_end) = byte_range(&first).unwrap();
    let (last_start, _) = byte_range(&last).unwrap();
    let choice_reads = at_choice - initial;
    let first_reads = at_first - at_choice;
    let tail_reads = final_read - at_first;
    eprintln!(
        "ZEN_VOLUME bytes={TWO_GIB} choice_read={choice_reads} first_read={first_reads} tail_read={tail_reads} first={first:?} tail={last:?} footer_events={:?}",
        screen.publications
    );
    assert!(
        first_reads >= first_end && tail_reads >= TWO_GIB - last_start,
        "page reads were not observed"
    );
    assert!(
        final_read - initial < 16 * 1024 * 1024,
        "read-only Zen exceeded the isolated process I/O allowance"
    );
}

#[test]
#[ignore = "requires TEXZI_ZEN_100M_FIXTURE and an isolated Windows test process"]
fn zen_100m_same_revision_notifications_and_latest_edit() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let mut core = environment(&runtime);
    let mut screen = Screen::default();
    core.mount_root(&Root::new::<EditableFooter>(|| ()));
    let original = screen.wait(&mut core, |s| counted(s).is_some());
    let count = counted(&original).unwrap();
    assert!(count > 0, "provide nonempty synthetic prose");
    let settled = screen.publications.len();
    for _ in 0..4 {
        screen.press(&mut core, NOTIFY);
        screen.pump(&mut core);
    }
    screen.settle(&mut core);
    assert!(
        screen.publications[settled..]
            .iter()
            .all(|s| s == &original),
        "a same-revision notification restarted the footer"
    );
    screen.press(&mut core, APPEND);
    screen.wait(&mut core, |s| s == PENDING);
    screen.press(&mut core, APPEND);
    screen.pump(&mut core);
    let latest = screen.publications.len();
    screen.press(&mut core, APPEND);
    let completed = screen.wait(&mut core, |s| counted(s) == Some(count + 6));
    screen.settle(&mut core);
    assert!(
        screen.publications[latest..]
            .iter()
            .all(|s| s == PENDING || s == &completed),
        "an obsolete revision reached the footer: {:?}",
        &screen.publications[latest..]
    );
    eprintln!(
        "ZEN_VOLUME bytes={ONE_HUNDRED_MB} same_revision_notifications=4 appended_revisions=3 baseline={count} final={} footer_events={:?}",
        count + 6,
        screen.publications
    );
}
