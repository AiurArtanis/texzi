use nsh_core::{
    Color,
    desktop::{Backdrop, WindowAttributes},
    element::{Tokens, set_tokens, tokens},
    prelude::*,
};
use serde_json::Value;
use std::sync::LazyLock;

static DESIGN: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/origami-tokens.json"))
        .expect("Origami design tokens")
});

const ALIASES: &[(&str, &str)] = &[
    ("foreground", "ink-1"),
    ("background", "paper"),
    ("muted", "paper-subtle"),
    ("muted-foreground", "muted-readable"),
    ("border", "seam-strong"),
    ("input", "seam-strong"),
    ("accent", "pill-hover"),
    ("accent-foreground", "ink-1"),
    ("sidebar-accent", "pill-active"),
    ("sidebar-accent-foreground", "ink-1"),
    ("primary", "iris"),
    ("primary-foreground", "on-primary"),
    ("secondary", "paper-subtle"),
    ("secondary-foreground", "ink-2"),
    ("ring", "iris"),
    ("editor-syntax-keyword", "syn-kw"),
    ("editor-syntax-title", "syn-fn"),
    ("editor-syntax-string", "syn-str"),
    ("editor-syntax-macro", "syn-mac"),
    ("editor-syntax-comment", "comment-readable"),
    ("editor-syntax-number", "number-readable"),
    ("editor-syntax-attribute", "warning-fg"),
    ("editor-syntax-built-in", "success-fg"),
];

static PALETTE: LazyLock<Tokens> = LazyLock::new(|| {
    let mut palette = nsh_base::tokens_of(None).expect("NSH defaults");
    let light = DESIGN["themes"]["light"].as_object().expect("light theme");
    let dark = DESIGN["themes"]["dark"].as_object().expect("dark theme");
    for (name, light_value) in light {
        let light_value = light_value.as_str().expect("light token value");
        let dark_value = dark[name].as_str().expect("dark token value");
        if let (Some(light), Some(dark)) = (Color::parse(light_value), Color::parse(dark_value)) {
            palette.set_color(name, AdaptiveColor::new(light, dark));
        }
    }

    let shell = AdaptiveColor::new(
        parse(&DESIGN["fallback"]["light"]["shell"]),
        parse(&DESIGN["fallback"]["dark"]["shell"]),
    );
    palette.set_color("shell-solid", shell);
    palette.set_color("chrome-edge", required(&palette, "seam-strong"));
    palette.set_number("workbench-border", 1.0);
    palette.set_color("control-face", AdaptiveColor::TRANSPARENT);
    palette.set_color("control-text", required(&palette, "ink-1"));
    palette.set_color("control-disabled", required(&palette, "muted-readable"));
    palette.set_color("active-paper-edge", required(&palette, "iris-border"));
    for (index, name) in ["ribbon-start", "ribbon-middle", "ribbon-end"]
        .into_iter()
        .enumerate()
    {
        let stop = |data: &Value| {
            let gradient = data["z-ribbon"].as_str().expect("ribbon gradient");
            let value = gradient.split('#').nth(index + 1).expect("ribbon stop");
            rgb(u32::from_str_radix(&value[..6], 16).expect("ribbon RGB"))
        };
        palette.set_color(
            name,
            AdaptiveColor::new(
                stop(&DESIGN["themes"]["light"]),
                stop(&DESIGN["themes"]["dark"]),
            ),
        );
    }
    for (alias, name) in ALIASES {
        palette.set_color(alias, required(&palette, name));
    }
    palette.set_font(
        "font-sans",
        DESIGN["fonts"]["ui"].as_str().expect("UI font stack"),
    );
    palette
});

fn parse(value: &Value) -> Color {
    Color::parse(value.as_str().expect("Origami color string")).expect("Origami color")
}

fn required(palette: &Tokens, name: &str) -> AdaptiveColor {
    palette
        .color(name)
        .unwrap_or_else(|| panic!("missing Origami color {name}"))
}

pub(super) fn metric(name: &str) -> f32 {
    let value = match name {
        "modeWidth" => &DESIGN["metrics"]["modeHit"][0],
        "modeHeight" => &DESIGN["metrics"]["modeHit"][1],
        _ => &DESIGN["metrics"][name],
    };
    value.as_f64().expect("Origami metric") as f32
}

pub(super) fn font_size(name: &str) -> f32 {
    DESIGN["fonts"]["sizeDip"][name]
        .as_f64()
        .expect("Origami font size") as f32
}

pub(super) fn color(name: &str) -> AdaptiveColor {
    tokens()
        .color(name)
        .unwrap_or_else(|| required(&PALETTE, name))
}

pub(super) fn sample_color(name: &str) -> AdaptiveColor {
    required(&PALETTE, name)
}

pub(super) fn current_color(name: &str) -> Hsla {
    let color = color(name);
    if dark() { color.dark } else { color.light }
}

pub(super) fn border_width() -> Points {
    px(tokens().number("workbench-border").unwrap_or(1.0))
}

pub(super) fn ribbon<S: Styled>(style: S) -> S {
    style
        .bg_linear(deg(120.0))
        .gradient_from(|| color("ribbon-start"))
        .gradient_via(|| color("ribbon-middle"))
        .gradient_via_position(nsh_core::geometry::percentage(0.48))
        .gradient_to(|| color("ribbon-end"))
}

pub(super) const CONTROL_STATES: &str = "hover:bg-accent hover:text-accent-foreground active:bg-sidebar-accent active:text-sidebar-accent-foreground disabled:bg-(--control-face) disabled:text-(--control-disabled) focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring";

pub(super) fn initial_tokens() -> Tokens {
    SystemVisuals::default().palette()
}

pub(super) fn install() {
    let visuals = use_context::<VisualPreferences>();
    effect(move || {
        let preferences = visuals.map(|value| value.0.get()).unwrap_or_default();
        set_tokens(preferences.palette());
    });
}

#[derive(Clone, Copy)]
pub(super) struct VisualPreferences(Signal<SystemVisuals>);

impl VisualPreferences {
    pub(super) fn watch(title: &'static str) -> Self {
        let current = signal(SystemVisuals::read(None));
        let refresh = move || {
            spawn(async move {
                if let Ok(observed) =
                    run_on_main_thread(move || SystemVisuals::read(Some(title))).await
                    && current.get_untracked() != observed
                {
                    current.set(observed);
                }
            });
        };
        refresh();
        set_interval(std::time::Duration::from_secs(1), refresh);
        Self(current)
    }

    pub(super) fn window_attributes() -> WindowAttributes {
        let mut attributes = WindowAttributes::default();
        attributes.backdrop = Some(Backdrop::Sidebar);
        attributes
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SystemVisuals {
    transparency: bool,
    mica_applied: bool,
    high_contrast: Option<SystemColors>,
}

impl SystemVisuals {
    fn mica(self) -> bool {
        self.transparency && self.mica_applied && self.high_contrast.is_none()
    }

    fn palette(self) -> Tokens {
        let mut palette = PALETTE.clone();
        if !self.mica() {
            let solid = required(&palette, "shell-solid");
            for surface in ["mica-shell", "mica-sidebar", "mica-statusbar"] {
                palette.set_color(surface, solid);
            }
            palette.set_color("chrome-edge", required(&palette, "ink-2"));
            palette.set_color("active-paper-edge", required(&palette, "iris"));
            palette.set_number("workbench-border", metric("highContrastBorder"));
        }
        if let Some(system) = self.high_contrast {
            for (names, color) in [
                (
                    &[
                        "paper",
                        "paper-subtle",
                        "apricot-soft",
                        "mica-shell",
                        "mica-sidebar",
                        "mica-statusbar",
                        "shell-solid",
                    ][..],
                    system.surface,
                ),
                (
                    &[
                        "ink-1",
                        "ink-2",
                        "muted-readable",
                        "comment-readable",
                        "number-readable",
                        "syn-kw",
                        "syn-fn",
                        "syn-str",
                        "syn-mac",
                        "seam",
                        "seam-strong",
                        "chrome-edge",
                        "active-paper-edge",
                        "success-fg",
                        "warning-fg",
                        "error-fg",
                    ][..],
                    system.text,
                ),
                (
                    &[
                        "iris",
                        "ribbon-start",
                        "ribbon-middle",
                        "ribbon-end",
                        "apricot",
                    ][..],
                    system.highlight,
                ),
                (
                    &["pill-hover", "pill-active", "tab-active-bg", "selected-bg"][..],
                    system.highlight,
                ),
                (&["selected-text", "on-primary"][..], system.highlight_text),
            ] {
                for name in names {
                    palette.set_color(name, rgb(color).into());
                }
            }
            for (alias, name) in ALIASES {
                palette.set_color(alias, required(&palette, name));
            }
            palette.set_color("editor-syntax-attribute", rgb(system.text).into());
            palette.set_color("editor-syntax-built-in", rgb(system.text).into());
            palette.set_color("accent-foreground", rgb(system.highlight_text).into());
            palette.set_color(
                "sidebar-accent-foreground",
                rgb(system.highlight_text).into(),
            );
            palette.set_color("control-face", rgb(system.button_face).into());
            palette.set_color("control-text", rgb(system.button_text).into());
            palette.set_color("control-disabled", rgb(system.disabled_text).into());
        }
        palette
    }

    fn read(title: Option<&str>) -> Self {
        use nsh_platform_windows::windows::{
            Win32::{
                System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW},
                UI::{
                    Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
                    WindowsAndMessaging::{
                        SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
                        SystemParametersInfoW,
                    },
                },
            },
            core::w,
        };

        let mut transparency = 0u32;
        let mut length = size_of::<u32>() as u32;
        let mut contrast = HIGHCONTRASTW {
            cbSize: size_of::<HIGHCONTRASTW>() as u32,
            ..Default::default()
        };
        // SAFETY: each query receives a live output value of the requested size.
        let (transparency, high_contrast) = unsafe {
            let preference = RegGetValueW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
                w!("EnableTransparency"),
                RRF_RT_REG_DWORD,
                None,
                Some((&mut transparency as *mut u32).cast()),
                Some(&mut length),
            );
            let contrast_result = SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                contrast.cbSize,
                Some((&mut contrast as *mut HIGHCONTRASTW).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
            (
                preference.is_ok() && transparency != 0,
                contrast_result.is_ok() && contrast.dwFlags.contains(HCF_HIGHCONTRASTON),
            )
        };
        Self {
            transparency,
            mica_applied: transparency
                && !high_contrast
                && title.is_some_and(workbench_uses_mica_alt),
            high_contrast: high_contrast.then(SystemColors::read),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SystemColors {
    surface: u32,
    text: u32,
    highlight: u32,
    highlight_text: u32,
    button_face: u32,
    button_text: u32,
    disabled_text: u32,
}

impl SystemColors {
    fn read() -> Self {
        use nsh_platform_windows::windows::Win32::Graphics::Gdi::{
            COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT,
            COLOR_WINDOW, COLOR_WINDOWTEXT, GetSysColor,
        };
        let rgb_value = |role| {
            // SAFETY: GetSysColor reads a system color and takes no pointers.
            let color = unsafe { GetSysColor(role) };
            ((color & 0xff) << 16) | (color & 0xff00) | ((color >> 16) & 0xff)
        };
        Self {
            surface: rgb_value(COLOR_WINDOW),
            text: rgb_value(COLOR_WINDOWTEXT),
            highlight: rgb_value(COLOR_HIGHLIGHT),
            highlight_text: rgb_value(COLOR_HIGHLIGHTTEXT),
            button_face: rgb_value(COLOR_BTNFACE),
            button_text: rgb_value(COLOR_BTNTEXT),
            disabled_text: rgb_value(COLOR_GRAYTEXT),
        }
    }
}

fn workbench_uses_mica_alt(title: &str) -> bool {
    use nsh_platform_windows::windows::{
        Win32::{
            Foundation::{HWND, LPARAM},
            Graphics::Dwm::{DWMWA_SYSTEMBACKDROP_TYPE, DwmGetWindowAttribute},
            System::Threading::GetCurrentThreadId,
            UI::WindowsAndMessaging::{
                EnumThreadWindows, GetClassNameW, GetWindowTextW, IsWindowVisible,
            },
        },
        core::{BOOL, w},
    };

    struct Query<'a> {
        title: &'a [u16],
        handle: Option<HWND>,
    }

    unsafe extern "system" fn find_visible_workbench(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: EnumThreadWindows invokes this synchronously with the live Query below.
        let query = unsafe { &mut *(data.0 as *mut Query<'_>) };
        let mut class = [0u16; 32];
        let mut title = [0u16; 96];
        // SAFETY: the calling UI thread owns hwnd and the output buffers are local.
        let matches = unsafe {
            IsWindowVisible(hwnd).as_bool()
                && GetClassNameW(hwnd, &mut class) == 9
                && &class[..9] == w!("NshWindow").as_wide()
                && GetWindowTextW(hwnd, &mut title) as usize == query.title.len()
                && title.get(..query.title.len()) == Some(query.title)
        };
        if matches {
            query.handle = Some(hwnd);
        }
        BOOL::from(!matches)
    }

    let title = title.encode_utf16().collect::<Vec<_>>();
    let mut query = Query {
        title: &title,
        handle: None,
    };
    // The platform owns one visible Texzi workbench per process. Its hidden host
    // and any OS dialogs are excluded by the visible class and caption above.
    // SAFETY: query outlives the synchronous enumeration on the UI thread.
    unsafe {
        let _ = EnumThreadWindows(
            GetCurrentThreadId(),
            Some(find_visible_workbench),
            LPARAM((&mut query as *mut Query<'_>) as isize),
        );
    }
    let Some(hwnd) = query.handle else {
        return false;
    };
    let mut kind = 0i32;
    // SAFETY: the query uses the visible HWND and an i32 output of the required size.
    unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            (&mut kind as *mut i32).cast(),
            size_of::<i32>() as u32,
        )
    }
    .is_ok()
        && kind == 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_fallback_preserves_opaque_paper_and_system_focus_colors() {
        let mica = SystemVisuals {
            transparency: true,
            mica_applied: true,
            high_contrast: None,
        }
        .palette();
        let solid = SystemVisuals {
            transparency: false,
            mica_applied: true,
            high_contrast: None,
        }
        .palette();
        let system = SystemColors {
            surface: 0x120043,
            text: 0xf7fff0,
            highlight: 0xffff10,
            highlight_text: 0x08051a,
            button_face: 0x241536,
            button_text: 0xfff8f0,
            disabled_text: 0x887791,
        };
        let contrast = SystemVisuals {
            transparency: true,
            mica_applied: true,
            high_contrast: Some(system),
        }
        .palette();

        assert_eq!(
            mica.color("paper"),
            Some(AdaptiveColor::rgb(0xfcfcfe, 0x161923))
        );
        assert_eq!(solid.color("paper"), mica.color("paper"));
        assert_ne!(mica.color("mica-shell"), solid.color("mica-shell"));
        assert_eq!(
            solid.color("mica-sidebar"),
            Some(AdaptiveColor::rgb(0xeef0f6, 0x11141c))
        );
        assert_eq!(solid.number("workbench-border"), Some(1.5));
        assert_eq!(contrast.color("ring"), Some(rgb(system.highlight).into()));
        assert_eq!(contrast.color("foreground"), Some(rgb(system.text).into()));
        assert_eq!(
            contrast.color("background"),
            Some(rgb(system.surface).into())
        );
        assert_eq!(
            contrast.color("selected-text"),
            Some(rgb(system.highlight_text).into())
        );
        assert_eq!(contrast.color("accent"), Some(rgb(system.highlight).into()));
        assert_eq!(
            contrast.color("accent-foreground"),
            Some(rgb(system.highlight_text).into())
        );
        assert_eq!(contrast.color("sidebar-accent"), contrast.color("accent"));
        assert_eq!(
            contrast.color("sidebar-accent-foreground"),
            contrast.color("accent-foreground")
        );
        assert_eq!(
            contrast.color("control-face"),
            Some(rgb(system.button_face).into())
        );
        assert_eq!(
            contrast.color("control-text"),
            Some(rgb(system.button_text).into())
        );
        assert_eq!(
            contrast.color("control-disabled"),
            Some(rgb(system.disabled_text).into())
        );
        assert_eq!(contrast.number("workbench-border"), Some(1.5));
    }
}
