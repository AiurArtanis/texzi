//! Native editing checkpoint. File IO and rich-text integration follow separately.

struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
        }
    }

    fn flush(&self) {}
}

static LOGGER: StderrLogger = StderrLogger;

#[cfg(windows)]
mod editor_owner;
#[cfg(windows)]
mod rich_owner;
#[cfg(windows)]
mod rich_projection;
#[cfg(windows)]
mod startup_fault;

#[cfg(windows)]
mod app {
    use nsh_core::prelude::*;
    use nsh_platform_windows as platform;

    fn rich_classes() -> nsh_base::rich_editor::view::RichEditorClasses {
        use std::collections::BTreeMap;
        let mut classes = nsh_base::rich_editor::view::RichEditorClasses::default();
        classes.content = Some("p-4 text-base".into());
        classes.nodes = Some(BTreeMap::from([
            ("paragraph".into(), "mb-3".into()),
            ("heading".into(), "font-bold mt-4 mb-3".into()),
            ("heading[level=1]".into(), "text-3xl".into()),
            ("heading[level=2]".into(), "text-2xl".into()),
            ("heading[level=3]".into(), "text-xl".into()),
            ("heading[level=4]".into(), "text-lg".into()),
            ("codeBlock".into(), "font-mono bg-muted p-3 mb-3".into()),
            (
                "blockquote".into(),
                "border-l-2 border-border pl-4 my-3".into(),
            ),
            ("bulletList".into(), "pl-6 my-2".into()),
            ("orderedList".into(), "pl-6 my-2".into()),
            ("tableCell".into(), "border border-border px-3 py-2".into()),
            (
                "tableHeader".into(),
                "font-bold bg-muted border border-border px-3 py-2".into(),
            ),
        ]));
        classes.marks = Some(BTreeMap::from([
            ("bold".into(), "font-bold".into()),
            ("italic".into(), "italic".into()),
            ("strike".into(), "line-through".into()),
            ("link".into(), "underline".into()),
            ("code".into(), "font-mono bg-muted".into()),
        ]));
        classes.task_checkbox =
            Some("w-4 h-4 border border-foreground checked:bg-foreground".into());
        classes
    }

    #[component]
    pub fn Window() -> impl IntoElement {
        nsh_core::element::set_tokens(nsh_base::tokens_of(Some(r##"{"cssVars":{"light":{"foreground":"#000000","background":"#ffffff","muted":"#f5f5f5","muted-foreground":"#525252","border":"#d4d4d4","accent":"#e5e5e5","accent-foreground":"#000000","input":"#d4d4d4"},"dark":{"foreground":"#000000","background":"#ffffff","muted":"#f5f5f5","muted-foreground":"#525252","border":"#d4d4d4","accent":"#e5e5e5","accent-foreground":"#000000","input":"#d4d4d4"}}}"##)).expect("fixed content palette"));
        let source = crate::editor_owner::NativeView::new();
        let rich = source.sibling().expect("independent rich view");
        provide_context(source.buffer());
        let gate = crate::rich_owner::ModeGate::new(source, rich);
        let rich_mode = gate.rich_mode;
        let status = gate.status;
        let source_request = gate.clone();
        let rich_request = gate.clone();
        let source_busy = gate.clone();
        let rich_busy = gate.clone();
        let render_rich = gate.clone();
        let render_source = gate.clone();
        view()
            .flex_1()
            .bg(white())
            .text_color(black())
            .child(
                view()
                    .flex_row()
                    .gap(px(16.0))
                    .p(px(8.0))
                    .child(
                        nsh_base::button()
                            .child(text("源码"))
                            .disabled(move || source_busy.busy())
                            .on_press(move |_: &()| source_request.to_source()),
                    )
                    .child(
                        nsh_base::button()
                            .child(text("富文本"))
                            .disabled(move || rich_busy.busy())
                            .on_press(move |_: &()| rich_request.to_rich()),
                    ),
            )
            .child(text(move || status.get()).text_color(black()))
            .child(
                when(rich_mode, move || {
                    nsh_base::rich_editor::view::rich_editor()
                        .editor(render_rich.session())
                        .classes(rich_classes())
                        .auto_focus(true)
                        .size_full()
                })
                .otherwise(move || {
                    render_source.source_mounted();
                    nsh_base::editor::editor()
                        .auto_focus(true)
                        .language("markdown")
                        .size_full()
                }),
            )
    }

    pub fn run() -> std::process::ExitCode {
        platform::run_app(
            platform::AppSource::Rust(platform::Root::of::<Window>()),
            platform::AppOptions {
                title: "texzi — 原生编辑".to_owned(),
                app_id: "dev.texzi.tx00".to_owned(),
                size: (960, 640),
                ..Default::default()
            },
        )
    }
}

fn main() -> std::process::ExitCode {
    if let Err(error) = log::set_logger(&LOGGER) {
        eprintln!("texzi: cannot initialize diagnostics: {error}");
        return std::process::ExitCode::FAILURE;
    }
    log::set_max_level(log::LevelFilter::Info);
    #[cfg(windows)]
    {
        if std::env::args_os().any(|arg| arg == "--tx00-fail-window-create") {
            if let Err(error) = startup_fault::arm() {
                log::error!("Cannot arm TX00 startup fixture (not an NSH failure result): {error}");
                return std::process::ExitCode::FAILURE;
            }
        }
        app::run()
    }
    #[cfg(not(windows))]
    {
        eprintln!("texzi TX-00 requires Windows x64 MSVC.");
        std::process::ExitCode::FAILURE
    }
}
