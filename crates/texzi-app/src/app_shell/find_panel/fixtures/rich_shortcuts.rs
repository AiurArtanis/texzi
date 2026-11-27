use super::*;
use nsh_core::{
    ApproxTextEngine, Core, Size,
    element::{Environment, Root},
    platform::Os,
};
use std::time::Instant;

const MARKDOWN: &str = "[first](HIDDEN) and [second](HIDDEN)\n";
thread_local! {
    static ALL: Cell<bool> = const { Cell::new(false) };
    static OUTPUT: RefCell<Option<Rc<NativeView>>> = const { RefCell::new(None) };
}

#[component]
fn RichShortcutFixture() -> impl IntoElement {
    let shell = Shell::new();
    let source = NativeView::fixture(MARKDOWN);
    let doc = shell.add_with_mode(source.clone(), Some("links.md".into()), Some(true));
    assert!(doc.displayed_rich());
    let rendered = scan(&source, Some(&doc.gate), "HIDDEN", None).unwrap();
    assert_eq!(rendered.results.total, 0);
    open(&shell, false);
    shell.find_panel.query.set("HIDDEN".into());
    shell.find_panel.replacement.set("NEW".into());
    let command = if ALL.get() {
        "texzi.find.replaceAll"
    } else {
        "texzi.find.replaceOne"
    };
    shell.run(command);
    assert!(!doc.displayed_rich() && shell.panel.get() == "find");
    OUTPUT.with_borrow_mut(|out| *out = Some(source));
    overlay(shell, doc)
}

#[test]
fn both_replace_commands_from_rich_find_rescan_source_before_publishing() {
    for all in [false, true] {
        ALL.set(all);
        let mut core = Core::new(
            ApproxTextEngine,
            Size {
                width: px(800.0),
                height: px(600.0),
            },
        );
        core.set_environment(Environment {
            os: Os::Windows,
            ..Default::default()
        });
        core.mount_root(&Root::new::<RichShortcutFixture>(|| ()));
        core.run_component_tasks();
        core.commit();
        let source = OUTPUT.take().unwrap();
        let expected = MARKDOWN.replacen("HIDDEN", "NEW", if all { 2 } else { 1 });
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            core.run_component_tasks();
            core.commit();
            let (snapshot, _) = source.search_snapshot().unwrap();
            if snapshot.body.to_utf8() == expected.as_bytes() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "source replacement did not finish"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        let (snapshot, epoch) = source.search_snapshot().unwrap();
        source.history(snapshot.revision.0, epoch, true).unwrap();
        assert_eq!(
            source.search_snapshot().unwrap().0.body.to_utf8(),
            MARKDOWN.as_bytes()
        );
    }
}
