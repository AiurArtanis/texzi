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

    #[component]
    pub fn Window() -> impl IntoElement {
        nsh_core::element::set_tokens(nsh_base::tokens_of(Some(r##"{"cssVars":{"light":{"foreground":"#000000","background":"#ffffff","muted":"#f5f5f5","muted-foreground":"#525252","border":"#d4d4d4","accent":"#e5e5e5","accent-foreground":"#000000","input":"#d4d4d4"},"dark":{"foreground":"#000000","background":"#ffffff","muted":"#f5f5f5","muted-foreground":"#525252","border":"#d4d4d4","accent":"#e5e5e5","accent-foreground":"#000000","input":"#d4d4d4"}}}"##)).expect("fixed content palette"));
        let source = crate::editor_owner::NativeView::new();
        let rich = source.sibling().expect("independent rich view");
        provide_context(source.buffer());
        let rich_mode = signal(false);
        let status = signal(String::new());
        let active: std::rc::Rc<
            std::cell::RefCell<Option<std::rc::Rc<crate::rich_owner::RichOwner>>>,
        > = std::rc::Rc::new(std::cell::RefCell::new(None));
        let source_owner = source.clone();
        let rich_source_owner = source.clone();
        let source_mode_active = active.clone();
        let source_active = active.clone();
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
                            .on_press(move |_: &()| {
                                use nsh_base::editor::ExternalDocument;
                                if source_mode_active
                                    .borrow()
                                    .as_ref()
                                    .is_some_and(|o| o.session().composing())
                                {
                                    status.set("请先完成或取消中文组合，再切换模式".into());
                                    return;
                                }
                                source_owner.fence();
                                source_owner.trace_state("mode_source");
                                rich_mode.set(false);
                            }),
                    )
                    .child(
                        nsh_base::button()
                            .child(text("富文本"))
                            .on_press(move |_: &()| {
                                use nsh_base::editor::ExternalDocument;
                                rich_source_owner.fence();
                                rich_source_owner.trace_state("mode_rich");
                                rich_mode.set(true)
                            }),
                    ),
            )
            .child(text(move || status.get()).text_color(black()))
            .child(
                when(rich_mode, move || {
                    match crate::rich_owner::RichOwner::new(rich.clone(), status) {
                        Ok(owner) => {
                            status.set(String::new());
                            let session = owner.session();
                            *active.borrow_mut() = Some(owner);
                            nsh_base::rich_editor::view::rich_editor()
                                .editor(session)
                                .auto_focus(true)
                                .size_full()
                                .into_element()
                        }
                        Err(e) => {
                            status.set(e);
                            view().into_element()
                        }
                    }
                })
                .otherwise(move || {
                    source_active.borrow_mut().take();
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
