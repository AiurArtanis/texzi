//! Native editing checkpoint. File IO and rich-text integration follow separately.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

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
mod app_shell;
#[cfg(windows)]
mod native_menu;
#[cfg(all(test, windows))]
mod syntax_matrix_tests;

#[cfg(windows)]
mod editor_owner;
#[cfg(all(test, windows))]
mod list_structural_tests;
#[cfg(windows)]
mod raw_block_controls;
#[cfg(all(test, windows))]
mod raw_block_delete_tests;
#[cfg(windows)]
mod rich_controls;
#[cfg(windows)]
mod rich_images;
#[cfg(windows)]
mod rich_owner;
#[cfg(windows)]
mod rich_projection;
#[cfg(windows)]
mod startup_fault;
#[cfg(windows)]
mod table_controls;
#[cfg(all(test, windows))]
mod table_controls_tests;

#[cfg(windows)]
mod app {
    use nsh_core::prelude::*;
    use nsh_platform_windows as platform;

    pub(crate) fn rich_classes() -> nsh_base::rich_editor::view::RichEditorClasses {
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
        nsh_core::element::set_tokens(nsh_base::tokens_of(Some(r##"{"cssVars":{"light":{"foreground":"#20242b","background":"#ffffff","muted":"#f3f4f6","muted-foreground":"#616975","border":"#dee2e8","accent":"#edf1f6","accent-foreground":"#20242b","input":"#dee2e8"},"dark":{"foreground":"#e5e9f0","background":"#191d23","muted":"#20242b","muted-foreground":"#a7b0bd","border":"#3b4350","accent":"#303845","accent-foreground":"#e5e9f0","input":"#3b4350"}}}"##)).expect("application palette"));
        crate::app_shell::view_shell()
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
