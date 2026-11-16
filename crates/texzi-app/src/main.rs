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
mod startup_fault;

#[cfg(windows)]
mod app {
    use nsh_core::prelude::*;
    use nsh_platform_windows as platform;

    #[component]
    pub fn Window() -> impl IntoElement {
        nsh_core::element::set_tokens(nsh_base::tokens_of(Some(r##"{"cssVars":{"light":{"foreground":"#000000","background":"#ffffff"},"dark":{"foreground":"#000000","background":"#ffffff"}}}"##)).expect("fixed content palette"));
        provide_context(crate::editor_owner::NativeView::initial());
        view().flex_1().bg(white()).child(
            nsh_base::editor::editor()
                .auto_focus(true)
                .language("markdown")
                .size_full(),
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
