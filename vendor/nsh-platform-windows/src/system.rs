//! Surface-wide commands and the environment: dialogs, the clipboard,
//! exiting; the system's settings followed into
//! [`Environment`].
//!
//! Dialogs are drawn inside the surface, as items the root view
//! hosts over the app's views ([`Hosted`]), rather than as separate
//! windows (`MessageBox`, `TaskDialog`): they stay with the surface when it
//! is embedded in another app's window, never run a modal message loop of
//! their own (the engine and animations keep going), and look the same in
//! every host. They follow Windows 11's content dialogs: a rounded card
//! over a dimmed backdrop, the text above a footer of buttons, the default
//! action in the accent color. The root keeps them after its other hosted
//! items ([`end_batch`]), so nothing mounted later covers them.
//!
//! The environment follows, re-read on every `WM_SETTINGCHANGE` (and
//! theme, color and DWM colorization changes, [`settings_changed`]):
//! - the color scheme: the app's appearance (`nsh.json` `"appearance"`,
//!   `setColorScheme`; [`set_appearance`]) when it forces one, else
//!   `AppsUseLightTheme` (Settings > Personalization > Colors), live;
//! - the locale (`Intl`'s default): the user's regional format
//!   (`GetUserDefaultLocaleName`), what number and date formats follow on
//!   Windows. POSIX variables (`LANG`, `LC_*`) are not looked at: Windows
//!   programs do not, and shells like Git Bash set `LANG` regardless of
//!   the user's settings;
//! - the locale settings (`localeSettings()`): the preferred languages, the
//!   user's display languages, most preferred first
//!   (`GetUserPreferredUILanguages`: Settings > Time & language > Language
//!   & region), the locale alone when it lists none; the regional
//!   preferences, the regional format's time format, first day of the
//!   week and measurement system (`GetLocaleInfoEx`, with the user's
//!   changes to the format), over the region's; Windows has no temperature
//!   unit setting: the region's; the time zone, below;
//! - the font scale: Settings > Accessibility > Text size
//!   (`TextScaleFactor`, 100 to 225 percent);
//! - reduce motion: Settings > Accessibility > Visual effects > Animation
//!   effects off (`SPI_GETCLIENTAREAANIMATION`);
//! - right to left: the reading direction of the UI language (what the
//!   shell's own layout follows);
//! - the screen reader: `SPI_GETSCREENREADER` (Narrator, NVDA and JAWS set
//!   it);
//! - the pixel ratio: the window's DPI over 96;
//! - the OS version: `RtlGetVersion`'s major and minor version (10.0 on
//!   Windows 10 and 11 alike: Windows 11 kept the version number);
//! - the app's state, followed on `WM_ACTIVATEAPP` and the window's
//!   `WM_WINDOWPOSCHANGED` ([`app_activated`], [`lifecycle_changed`]); an
//!   embedded surface hears its root window's, whose procedure is
//!   subclassed to pass them on (`window::observe_root`): background while
//!   the window (an embedded surface's root window) is minimized or hidden,
//!   active while the app is the active one (its windows get
//!   `WM_ACTIVATEAPP`; before the first, the foreground window is the
//!   process's), inactive otherwise; background too while the user cannot
//!   see it: the session locked or without a display, the display off, the
//!   window on another virtual desktop ([`crate::presence`], followed from
//!   the system's notifications), and from the moment the session may end
//!   (signing out, shutting down: [`crate::session`]);
//! - the wake count (`wakeCount()`): one more on every
//!   `WM_POWERBROADCAST` with `PBT_APMRESUMEAUTOMATIC` ([`woke`]), which
//!   top-level windows get on every resume (an embedded surface's root
//!   window passes it on);
//! - the time zone (`localeSettings()`'s): the zone of the system's setting
//!   (`GetDynamicTimeZoneInformation`, its Windows key name) by its IANA
//!   name, which the ICU Windows ships knows
//!   (`ucal_getTimeZoneIDForWindowsID`, loaded when first needed: `UTC`
//!   without it); re-read on `WM_TIMECHANGE` too, which a change of the
//!   zone sends.
//!
//! Safe area and keyboard height are zero: desktop windows have neither.
//!
//! The surface's scheme in turn drives the title bar (dark or light, DWM's
//! immersive dark mode) and the root's background (white / neutral-950,
//! the default theme's), as the root view has none of its own.
//!
//! The status bar has no desktop equivalent. The clipboard is Win32's
//! (`CF_UNICODETEXT`); reads are synchronous, so `Clipboard.readText` is
//! answered at once.

use std::{
    cell::{Cell, RefCell},
    future::Future,
    rc::Rc,
};

use nsh_base::{AlertDialogOptions, AlertDialogPresentation};
use nsh_core::{
    Bounds, Color, Command, Dialog, DialogButton, DialogButtonStyle, EventKind, EventPayload,
    NodeId, PayloadValue, Points, ROOT_ID,
    element::{ElementRef, run_on_main_thread},
    geometry::px,
    platform::{Appearance, clipboard},
};
use nsh_engine::{
    ColorScheme, Edges, Environment, HourCycle, Languages, Lifecycle, Locale, LocaleSettings,
    MeasurementSystem, TimeZone, Weekday,
};
use tokio::sync::oneshot;
use windows::{
    Win32::{
        Foundation::{LPARAM, WPARAM},
        Globalization::{
            GetLocaleInfoEx, GetUserDefaultLocaleName, GetUserDefaultUILanguage,
            GetUserPreferredUILanguages, LCIDToLocaleName, LOCALE_IFIRSTDAYOFWEEK, LOCALE_IMEASURE,
            LOCALE_IREADINGLAYOUT, LOCALE_STIMEFORMAT, MUI_LANGUAGE_NAME,
        },
        Graphics::{
            Direct2D::{D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, D2D1_ROUNDED_RECT},
            DirectWrite::{
                DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
                DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT,
                DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_METRICS,
                DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                DWRITE_WORD_WRAPPING_NO_WRAP, IDWriteTextLayout,
            },
            Gdi::{
                COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW,
                COLOR_WINDOWTEXT, GetSysColor, SYS_COLOR_INDEX,
            },
        },
        System::{
            LibraryLoader::{
                GetModuleHandleW, GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
            },
            Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS},
            Registry::{
                HKEY, HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RegGetValueW,
            },
            SystemInformation::OSVERSIONINFOW,
            Threading::GetCurrentProcessId,
            Time::{
                DYNAMIC_TIME_ZONE_INFORMATION, GetDynamicTimeZoneInformation, TIME_ZONE_ID_INVALID,
            },
        },
        UI::{
            Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
            Shell::{SetCurrentProcessExplicitAppUserModelID, ShellExecuteW},
            WindowsAndMessaging::{
                GA_ROOT, GetAncestor, GetForegroundWindow, GetWindowThreadProcessId, IsIconic,
                IsWindowVisible, PostMessageW, SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST,
                SPI_GETSCREENREADER, SW_SHOWNORMAL, SYSTEM_PARAMETERS_INFO_ACTION,
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW, WM_CLOSE,
            },
        },
    },
    core::{BOOL, HSTRING, PCWSTR, PWSTR, s, w},
};

use crate::{
    decor::frozen_modal::FrozenModal,
    geom::{Mat4, rect},
    input::{Cursor, KeyEvent, PointerKind, WheelEvent},
    render::{DrawCx, rect_f},
    surface::Surface,
    view::{Hosted, ItemPointer, NshView},
};

/// Fades of overlays, in seconds.
const FADE_IN: f64 = 0.167;
const FADE_OUT: f64 = 0.12;

/// Virtual keys a dialog answers.
const VK_TAB: u16 = 0x09;
const VK_RETURN: u16 = 0x0D;
const VK_ESCAPE: u16 = 0x1B;
const VK_SPACE: u16 = 0x20;
const VK_LEFT: u16 = 0x25;
const VK_UP: u16 = 0x26;
const VK_PAGE_UP: u16 = 0x21;
const VK_PAGE_DOWN: u16 = 0x22;
const VK_RIGHT: u16 = 0x27;
const VK_DOWN: u16 = 0x28;

thread_local! {
    /// The system appearance a test switched to (`nsh test --target`).
    static SIMULATED: Cell<Option<bool>> = const { Cell::new(None) };
    /// The dialogs of this thread's surfaces.
    static OVERLAYS: RefCell<Vec<Overlay>> = const { RefCell::new(Vec::new()) };
    /// The accent colors of the system's palette (light and dark).
    static ACCENT: Cell<Option<(Color, Color)>> = const { Cell::new(None) };
    /// Numbers overlays, to find them again from deferred calls.
    static NEXT_SERIAL: Cell<u64> = const { Cell::new(1) };
    /// Whether this thread's windows are the active app's, from the last
    /// `WM_ACTIVATEAPP` (sent per thread); unknown before the first.
    static ACTIVE: Cell<Option<bool>> = const { Cell::new(None) };
}

// ---------------------------------------------------------------------------
// The environment

/// What the system's settings say.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Settings {
    dark: bool,
    font_scale: f32,
    reduce_motion: bool,
    rtl: bool,
    screen_reader: bool,
    locale: Locale,
    /// All but the time zone, which [`update`] reads ([`time_zone`]).
    locale_settings: LocaleSettings,
}

/// Makes the system look dark (or light) from now on (`nsh test
/// --target`'s color scheme).
#[cfg(feature = "dev")]
pub(crate) fn simulate_system_scheme(dark: bool) {
    SIMULATED.with(|s| s.set(Some(dark)));
}

/// True when the system's apps use the dark theme (Settings >
/// Personalization > Colors).
pub(crate) fn system_dark() -> bool {
    if let Some(dark) = SIMULATED.with(Cell::get) {
        return dark;
    }
    reg_dword(
        HKEY_CURRENT_USER,
        w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
        w!("AppsUseLightTheme"),
    ) == Some(0)
}

/// True when the taskbar is dark (Settings > Personalization > Colors,
/// "Windows mode"), which the tray's template icons are painted for.
pub(crate) fn taskbar_dark() -> bool {
    reg_dword(
        HKEY_CURRENT_USER,
        w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
        w!("SystemUsesLightTheme"),
    ) != Some(1)
}

/// True when the surface draws in its dark appearance. The environment is
/// kept current by this module, so drawing never reads the registry.
pub(crate) fn dark(surface: &Surface) -> bool {
    surface.environment.get().color_scheme == ColorScheme::Dark
}

/// The color behind the app's views (the default theme's background).
pub(crate) fn window_background(surface: &Surface) -> Color {
    if surface.transparent() {
        Color::TRANSPARENT
    } else if dark(surface) {
        Color(0x0a0a_0aff)
    } else {
        Color(0xffff_ffff)
    }
}

/// Reads the environment and keeps it current (the window calls
/// [`settings_changed`] when the system's settings change).
pub(crate) fn watch_environment(surface: &Rc<Surface>) {
    refresh_accent();
    update(surface, settings());
    environment_applied(surface);
}

/// A system setting changed (`WM_SETTINGCHANGE`, `WM_THEMECHANGED`,
/// `WM_SYSCOLORCHANGE`, `WM_DWMCOLORIZATIONCOLORCHANGED`).
pub(crate) fn settings_changed(surface: &Rc<Surface>) {
    crate::text::invalidate_rendering_params();
    refresh_accent();
    update(surface, settings());
    recovered_draft_preferences_changed(surface);
    // The taskbar's colors may have changed: a template tray icon follows.
    crate::tray::refresh(surface.hwnd());
    // The accent may have changed without the environment.
    surface.invalidate();
}

/// The environment changed (from the system, the app or a test): the
/// title bar and the root's background follow its scheme.
pub(crate) fn environment_applied(surface: &Surface) {
    let hwnd = surface.hwnd();
    if !hwnd.is_invalid() {
        crate::shell::set_color_scheme(surface, hwnd, Appearance::System);
    }
    crate::app_window::appearance_changed(surface);
    // Menus follow the scheme; the caption buttons are drawn again.
    crate::chrome::appearance_changed(surface);
}

/// `MountItem::Appearance`: the app forces light or dark (or follows the
/// system again).
pub(crate) fn set_appearance(surface: &Rc<Surface>, appearance: Appearance) {
    if surface.appearance.replace(appearance) != appearance {
        update(surface, settings());
    }
}

/// The taskbar groups the process's windows under `id`.
pub(crate) fn set_app_id(id: &str) {
    // SAFETY: a valid string.
    let _ = unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(id)) };
}

/// The system's settings now.
fn settings() -> Settings {
    let name = user_locale_name();
    let locale = user_locale(name.as_deref());
    Settings {
        dark: system_dark(),
        font_scale: text_scale(reg_dword(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Accessibility"),
            w!("TextScaleFactor"),
        )),
        // Animation effects off is Windows' reduce motion.
        reduce_motion: spi_bool(SPI_GETCLIENTAREAANIMATION) == Some(false),
        rtl: ui_language_rtl(),
        screen_reader: spi_bool(SPI_GETSCREENREADER) == Some(true),
        locale,
        locale_settings: locale_settings(locale, name.as_deref()),
    }
}

/// The user's locale settings (`localeSettings()`) but the time zone: the
/// display languages (`locale` alone when Windows lists none), and the
/// regional format's (`name`'s, with the user's changes to it) time format,
/// first day of the week and measurement system, over the region's.
fn locale_settings(locale: Locale, name: Option<&str>) -> LocaleSettings {
    let mut preferences = LocaleSettings::parse(name.unwrap_or_default());
    preferences.languages = preferred_languages(locale);
    if let Some(cycle) = locale_info(LOCALE_STIMEFORMAT).and_then(|f| HourCycle::of_pattern(&f)) {
        preferences.hour_cycle = cycle;
    }
    // 0 for Monday to 6 for Sunday.
    let first = locale_info(LOCALE_IFIRSTDAYOFWEEK).and_then(|d| d.trim().parse::<u8>().ok());
    if let Some(day) = first.and_then(|d| Weekday::from_number(d + 1)) {
        preferences.first_day_of_week = day;
    }
    // 0 metric (as the UK's units are, which Windows does not tell
    // apart), 1 US.
    match locale_info(LOCALE_IMEASURE).as_deref().map(str::trim) {
        Some("0") if preferences.measurement_system == MeasurementSystem::Us => {
            preferences.measurement_system = MeasurementSystem::Metric;
        }
        Some("1") => preferences.measurement_system = MeasurementSystem::Us,
        _ => {}
    }
    preferences
}

/// A value of the user's regional format, with the user's changes to it.
fn locale_info(kind: u32) -> Option<String> {
    let mut buf = [0u16; 80];
    // SAFETY: the buffer's length is passed with it; no name is the user's
    // default locale.
    let len = unsafe { GetLocaleInfoEx(PCWSTR::null(), kind, Some(&mut buf)) };
    // The length counts the terminating null.
    (len > 1).then(|| String::from_utf16_lossy(&buf[..len as usize - 1]))
}

/// The user's display languages, most preferred first; `locale` alone when
/// Windows lists none.
fn preferred_languages(locale: Locale) -> Languages {
    let (mut count, mut len) = (0u32, 0u32);
    // SAFETY: a size query: no buffer, the length written to `len`.
    if unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut len) }
        .is_err()
        || len == 0
    {
        return Languages::one(locale);
    }
    let mut names = vec![0u16; len as usize];
    // SAFETY: a buffer of the `len` characters the query asked for.
    if unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            Some(PWSTR(names.as_mut_ptr())),
            &mut len,
        )
    }
    .is_err()
    {
        return Languages::one(locale);
    }
    // A list of null-terminated names, ended by an empty one.
    let names = String::from_utf16_lossy(&names);
    Languages::parse(names.split('\0')).unwrap_or(Languages::one(locale))
}

/// `WM_ACTIVATEAPP`: this thread's windows became the active app's
/// (`active`), or another app's window was activated.
pub(crate) fn app_activated(surface: &Surface, active: bool) {
    ACTIVE.with(|a| a.set(Some(active)));
    lifecycle_changed(surface);
}

/// The app may have changed state: activated, or its window shown, hidden,
/// minimized or restored (`WM_WINDOWPOSCHANGED`).
pub(crate) fn lifecycle_changed(surface: &Surface) {
    let mut env: Environment = surface.environment.get();
    env.lifecycle = lifecycle(surface, env.lifecycle);
    surface.set_environment(env);
}

/// The system woke from sleep: `surface` counts it (`wakeCount()`).
pub(crate) fn woke(surface: &Surface) {
    let mut env: Environment = surface.environment.get();
    env.wake_count = env.wake_count.wrapping_add(1);
    surface.set_environment(env);
}

/// The app's state as `surface` sees it; `current` before its window
/// exists.
fn lifecycle(surface: &Surface, current: Lifecycle) -> Lifecycle {
    let hwnd = surface.hwnd();
    if hwnd.is_invalid() {
        return current;
    }
    // SAFETY: plain calls on a window handle.
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    // SAFETY: as above.
    let shown = unsafe { IsWindowVisible(root).as_bool() && !IsIconic(root).as_bool() }
        && !crate::presence::cloaked(root);
    // A test's app runs as it does on screen whatever the screen shows (a
    // locked or remote session), so its tests see what they see unlocked.
    let seen = surface.testing() || crate::presence::screen_seen();
    lifecycle_of(shown, seen, crate::session::ending(), || {
        ACTIVE.with(Cell::get).unwrap_or_else(foreground_is_ours)
    })
}

/// The app's state with its window `shown` (not minimized, hidden or on
/// another virtual desktop), the screen `seen` (the session unlocked and
/// connected, the display on: [`crate::presence`]), the session `ending`
/// ([`crate::session`]) and whether it is the `active` app.
fn lifecycle_of(shown: bool, seen: bool, ending: bool, active: impl FnOnce() -> bool) -> Lifecycle {
    if !shown || !seen || ending {
        Lifecycle::Background
    } else if active() {
        Lifecycle::Active
    } else {
        Lifecycle::Inactive
    }
}

/// Whether the foreground window is one of this process's.
fn foreground_is_ours() -> bool {
    let mut process = 0u32;
    // SAFETY: plain calls; `process` is an out value.
    unsafe {
        let window = GetForegroundWindow();
        !window.is_invalid()
            && GetWindowThreadProcessId(window, Some(&mut process as *mut u32)) != 0
            && process == GetCurrentProcessId()
    }
}

/// The embedding app set its locale or text size
/// ([`crate::AppView::set_locale`]): the system's settings are read again,
/// with the host's over them.
pub(crate) fn host_environment_changed(surface: &Surface) {
    update(surface, settings());
}

/// Writes `settings` and the surface's own values into its environment.
fn update(surface: &Surface, settings: Settings) {
    let mut env: Environment = surface.environment.get();
    let system = if settings.dark {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    };
    env.color_scheme = surface.appearance.get().resolve(system);
    env.locale = settings.locale;
    env.locale_settings = settings.locale_settings;
    env.locale_settings.time_zone = time_zone();
    env.font_scale = settings.font_scale;
    env.reduce_motion = settings.reduce_motion;
    env.rtl = settings.rtl;
    env.screen_reader = settings.screen_reader;
    if !surface.hwnd().is_invalid() {
        env.pixel_ratio = surface.scale();
    }
    env.os_version = os_version();
    env.safe_area = Edges::default();
    env.keyboard_height = px(0.0);
    env.lifecycle = lifecycle(surface, env.lifecycle);
    surface.set_environment(env);
}

/// The system's time zone: the setting's Windows zone by its
/// IANA name.
fn time_zone() -> TimeZone {
    let mut info = DYNAMIC_TIME_ZONE_INFORMATION::default();
    // SAFETY: an out structure of the size the call writes.
    if unsafe { GetDynamicTimeZoneInformation(&mut info) } == TIME_ZONE_ID_INVALID {
        return TimeZone::UTC;
    }
    let key = &info.TimeZoneKeyName;
    let len = key.iter().position(|&c| c == 0).unwrap_or(key.len());
    iana_name(&key[..len])
        .and_then(|name| TimeZone::parse(&name))
        .unwrap_or_default()
}

/// The IANA name of the Windows zone `key` (`W. Europe Standard Time`:
/// `Europe/Berlin`), from the ICU Windows ships (`icu.dll` since Windows 10
/// 1903, `icuin.dll` before), loaded at run time so that apps start where
/// it is missing.
fn iana_name(key: &[u16]) -> Option<String> {
    type ToIana = unsafe extern "C" fn(*const u16, i32, *const u8, *mut u16, i32, *mut i32) -> i32;
    // Looked up once for the process, not once per thread that asks (each
    // surface's own, when they run on several).
    static TO_IANA: std::sync::OnceLock<Option<ToIana>> = std::sync::OnceLock::new();
    let to_iana = (*TO_IANA.get_or_init(|| {
        // SAFETY: system libraries by name, from System32 only; the
        // function, if there, has the signature above (ICU's C API).
        unsafe {
            [w!("icu.dll"), w!("icuin.dll")]
                .into_iter()
                .find_map(|name| LoadLibraryExW(name, None, LOAD_LIBRARY_SEARCH_SYSTEM32).ok())
                .and_then(|icu| GetProcAddress(icu, s!("ucal_getTimeZoneIDForWindowsID")))
                .map(|proc| std::mem::transmute::<_, ToIana>(proc))
        }
    }))?;
    let mut name = [0u16; 64];
    let mut status = 0i32;
    // SAFETY: `key` and `name` hold the lengths given; no region (ICU's
    // default mapping, `001`).
    let len = unsafe {
        to_iana(
            key.as_ptr(),
            key.len() as i32,
            std::ptr::null(),
            name.as_mut_ptr(),
            name.len() as i32,
            &mut status,
        )
    };
    // ICU's errors are positive codes, its warnings negative.
    if status > 0 || len <= 0 {
        return None;
    }
    String::from_utf16(name.get(..len as usize)?).ok()
}

/// A `REG_DWORD` value, if it is set.
fn reg_dword(key: HKEY, subkey: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut data: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: a DWORD out value of the size given.
    let result = unsafe {
        RegGetValueW(
            key,
            subkey,
            value,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    result.is_ok().then_some(data)
}

/// A `BOOL` system parameter, if the system answers.
fn spi_bool(action: SYSTEM_PARAMETERS_INFO_ACTION) -> Option<bool> {
    let mut value = BOOL(0);
    // SAFETY: a BOOL out value, as these actions take.
    unsafe {
        SystemParametersInfoW(
            action,
            0,
            Some(&mut value as *mut BOOL as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .ok()
    .map(|()| value.as_bool())
}

/// The text scale of `TextScaleFactor` (percent; unset means 100). Values
/// outside the range Settings offers are ignored.
fn text_scale(percent: Option<u32>) -> f32 {
    percent
        .filter(|p| (100..=225).contains(p))
        .map_or(1.0, |p| p as f32 / 100.0)
}

/// The user's regional format (`de-DE`), if the system has one.
fn user_locale_name() -> Option<String> {
    let mut buf = [0u16; 85];
    // SAFETY: the buffer's length is passed with it.
    let len = unsafe { GetUserDefaultLocaleName(&mut buf) };
    // The length counts the terminating null.
    (len > 1).then(|| String::from_utf16_lossy(&buf[..len as usize - 1]))
}

/// The locale of a Windows locale name (`en-US`, `zh-Hans-CN`,
/// `de-DE_phoneb` with a sort order); `en-US` without one (the invariant
/// locale's name is empty).
fn user_locale(name: Option<&str>) -> Locale {
    name.and_then(Locale::parse).unwrap_or_default()
}

/// True when the UI language is written right to left.
fn ui_language_rtl() -> bool {
    let mut name = [0u16; 85];
    // SAFETY: plain calls; buffers with their lengths.
    unsafe {
        let lcid = u32::from(GetUserDefaultUILanguage());
        if LCIDToLocaleName(lcid, Some(&mut name), 0) == 0 {
            return false;
        }
        let mut layout = [0u16; 4];
        let len = GetLocaleInfoEx(
            PCWSTR(name.as_ptr()),
            LOCALE_IREADINGLAYOUT,
            Some(&mut layout),
        );
        reading_layout_rtl(&String::from_utf16_lossy(
            &layout[..(len.max(1) as usize - 1).min(layout.len())],
        ))
    }
}

/// `LOCALE_IREADINGLAYOUT`: 0 left to right, 1 right to left, 2 and 3
/// vertical (which lays out left to right here).
fn reading_layout_rtl(value: &str) -> bool {
    value.trim() == "1"
}

/// The OS version (major, minor), as `RtlGetVersion` has it: unlike
/// `GetVersionEx` it does not depend on the executable's manifest.
fn os_version() -> (u16, u16) {
    let (major, minor, _) = os_info();
    (major, minor)
}

/// The OS build number (Windows 11's from 22000), read with
/// [`os_version`].
pub(crate) fn os_build() -> u32 {
    os_info().2
}

fn os_info() -> (u16, u16, u32) {
    thread_local! {
        static VERSION: (u16, u16, u32) = read_os_version();
    }
    VERSION.with(|v| *v)
}

fn read_os_version() -> (u16, u16, u32) {
    type RtlGetVersion = unsafe extern "system" fn(*mut OSVERSIONINFOW) -> i32;
    // SAFETY: ntdll is loaded in every process; `RtlGetVersion` has the
    // signature above and fills the structure whose size is set.
    unsafe {
        let Ok(ntdll) = GetModuleHandleW(w!("ntdll.dll")) else {
            return (0, 0, 0);
        };
        let Some(proc) = GetProcAddress(ntdll, s!("RtlGetVersion")) else {
            return (0, 0, 0);
        };
        let get: RtlGetVersion = std::mem::transmute(proc);
        let mut info = OSVERSIONINFOW {
            dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
            ..Default::default()
        };
        if get(&mut info) != 0 {
            return (0, 0, 0);
        }
        (
            info.dwMajorVersion.min(u16::MAX as u32) as u16,
            info.dwMinorVersion.min(u16::MAX as u32) as u16,
            info.dwBuildNumber,
        )
    }
}

/// Reads the accent palette again (dialogs' default buttons).
fn refresh_accent() {
    ACCENT.with(|a| a.set(read_accent()));
}

/// The system's accent for light and dark backgrounds: the palette's
/// "dark 1" and "light 2" shades, which Windows 11's own accent buttons
/// use (Settings > Personalization > Colors > Accent color).
fn read_accent() -> Option<(Color, Color)> {
    let mut palette = [0u8; 32];
    let mut size = palette.len() as u32;
    // SAFETY: a binary out value of the size given.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent"),
            w!("AccentPalette"),
            RRF_RT_REG_BINARY,
            None,
            Some(palette.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if result.is_err() || size < 24 {
        return None;
    }
    Some(accent_shades(&palette))
}

/// The light-theme and dark-theme accents of an `AccentPalette` (eight RGBX
/// colors: light 3, light 2, light 1, the accent, dark 1, dark 2, dark 3).
fn accent_shades(palette: &[u8]) -> (Color, Color) {
    let at = |i: usize| Color::rgba(palette[i * 4], palette[i * 4 + 1], palette[i * 4 + 2], 255);
    (at(4), at(1))
}

/// The accent a default button has in the light or dark theme.
fn accent(dark: bool) -> Color {
    let (light, dark_shade) = ACCENT
        .with(Cell::get)
        // Windows 11's default blue.
        .unwrap_or((Color(0x005f_b8ff), Color(0x60cd_ffff)));
    if dark { dark_shade } else { light }
}

// ---------------------------------------------------------------------------
// Commands

/// Opens `url` with the app registered for its scheme (`ShellExecuteW`'s
/// `open`): whether one took it. The URL passed `nsh_core::open_url`'s
/// check, so it names no file or program to run: `ShellExecuteW` would
/// run one given its path, which is why the check takes no `C:/…`.
pub(crate) fn open_url(url: &str) -> bool {
    // SAFETY: plain call with valid NUL-terminated strings.
    let instance = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(url),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // Above 32 is success; the rest are error codes.
    instance.0 as isize > 32
}

/// Reports a clipboard write's result (`clipboard_write_result`).
fn clipboard_written(surface: &Rc<Surface>, written: bool) {
    surface.event(
        ROOT_ID,
        EventKind::ClipboardWrite,
        EventPayload::Map(vec![("written", PayloadValue::Bool(written))]),
    );
}

/// A command not addressed to a kind of view.
pub(crate) fn command(surface: &Rc<Surface>, _id: NodeId, command: &Command) {
    match command {
        Command::CloseDialog(dialog) => close_dialog(surface, *dialog),
        Command::SetClipboard(text) => {
            let written = set_clipboard_text(text);
            if !written {
                log::debug!("windows: cannot set the clipboard");
            }
            clipboard_written(surface, written);
        }
        Command::ReadClipboard { id, has_only } => {
            let has = clipboard_has_text();
            let text = if has && !*has_only {
                clipboard_text()
            } else {
                None
            };
            surface.event(
                ROOT_ID,
                EventKind::Clipboard,
                clipboard_answer(*id, has, text),
            );
        }
        Command::SetClipboardItem(item) => {
            let units: Vec<u16> = item.text.encode_utf16().chain(Some(0)).collect();
            let text: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
            let unicode = u32::from(windows::Win32::System::Ole::CF_UNICODETEXT.0);
            let mut formats = vec![(unicode, text)];
            if let Some(html) = &item.html {
                let mut html = clipboard::cf_html(html).into_bytes();
                html.push(0);
                formats.push((html_format(), html));
            }
            let formats: Vec<(u32, &[u8])> = formats
                .iter()
                .map(|(kind, bytes)| (*kind, bytes.as_slice()))
                .collect();
            let written = set_clipboard_formats(&formats);
            if !written {
                log::debug!("windows: cannot set the clipboard");
            }
            clipboard_written(surface, written);
        }
        Command::SetClipboardImage(uri) => match clipboard::decode_data_uri(uri) {
            Some(("image/png", bytes)) => {
                if !set_clipboard_bytes(png_format(), &bytes) {
                    log::debug!("windows: cannot set the clipboard");
                }
            }
            _ => log::warn!("Clipboard.writeImage: Windows takes a base64 PNG data: URI"),
        },
        Command::ReadClipboardImage { id, has_only } => {
            let has = clipboard_has_image();
            let bytes = (has && !*has_only).then(clipboard_image).flatten();
            let image = bytes.as_deref().and_then(clipboard::Image::sniff);
            surface.event(
                ROOT_ID,
                EventKind::Clipboard,
                clipboard::image_answer(*id, has, image),
            );
        }
        Command::ReadClipboardFiles { id } => surface.event(
            ROOT_ID,
            EventKind::Clipboard,
            clipboard::files_answer(*id, &clipboard_files()),
        ),
        Command::PickImages { id, request } => crate::picker::pick_images(surface, *id, *request),
        Command::PickFiles { id, request } => {
            crate::file_requests::pick_files(surface, *id, request);
        }
        Command::FileRequest { id, request } => {
            crate::file_requests::request(surface, *id, request)
        }
        Command::RenderCanvas {
            id,
            drawing,
            width,
            height,
            scale,
            format,
        } => crate::canvas::render_offscreen(
            surface,
            *id,
            drawing.clone(),
            (*width, *height),
            *scale,
            *format,
        ),
        Command::OpenUrl { id, url } => {
            let opened = open_url(url);
            surface.event(
                ROOT_ID,
                EventKind::OpenUrl,
                EventPayload::Map(vec![
                    ("id", PayloadValue::Number(*id as f64)),
                    ("opened", PayloadValue::Bool(opened)),
                ]),
            );
        }
        Command::Back => crate::navigation::default_back(surface),
        Command::ExitApp => {
            // Closing destroys the window, whose handler stops the surface:
            // not while the surface is applying this batch, so the message
            // is posted. An embedded surface closes its top-level window,
            // as the app leaving would.
            let hwnd = surface.hwnd();
            if !hwnd.is_invalid() {
                // SAFETY: plain calls on a live window handle.
                unsafe {
                    let top = GetAncestor(hwnd, GA_ROOT);
                    let top = if top.is_invalid() { hwnd } else { top };
                    let _ = PostMessageW(Some(top), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
        }
        Command::Announce(text) => crate::access::announce(surface, text),
        Command::PerfMonitor(true) => surface.enable_fps(),
        Command::PerfMonitor(false) => surface.disable_fps(),
        Command::Notification(command) => crate::notifications::command(surface, command),
        Command::Audio(command) => {
            let base = surface.asset_base.borrow().clone();
            surface.control(|c| {
                crate::audio::command(&surface.audio, surface.key(), c, command, base.as_ref())
            })
        }
        Command::Desktop(command) => crate::chrome::command(surface, command),
        // The status bar is a phone's; layout animations arrive as mount
        // items.
        Command::StatusBar(_)
        | Command::SetStatusBar(_)
        | Command::LayoutAnimation(_)
        | Command::CancelPress
        // The app's address: this platform keeps none.
        | Command::SetAppUrl(_)
        | Command::PushAppUrl(_) => {}
        other => log::debug!("windows: command {other:?} is not supported"),
    }
}

/// The answer to `Clipboard.readText` / `hasString` read `id`.
fn clipboard_answer(id: u32, has: bool, text: Option<String>) -> EventPayload {
    let mut fields = vec![
        ("id", PayloadValue::Number(id as f64)),
        ("has", PayloadValue::Bool(has)),
    ];
    if let Some(text) = text {
        fields.push(("text", PayloadValue::Str(text)));
    }
    EventPayload::Map(fields)
}

/// The answer to dialog `id`: the button's index, -1 when dismissed.
fn dialog_answer(id: u32, index: i32) -> EventPayload {
    EventPayload::Map(vec![
        ("id", PayloadValue::Number(id as f64)),
        ("index", PayloadValue::Number(index as f64)),
    ])
}

/// Action chosen in texzi's window-bound recovery confirmation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveredDraftChoice {
    Continue,
    Discard,
    SaveAs,
}

/// Optional information shown above and below the recovery explanation.
#[derive(Debug, Default)]
pub struct RecoveredDraftDetails {
    /// Short status text, displayed before the confirmation title.
    pub badge: Option<String>,
    /// A single-line preview of the recovered content.
    pub summary: Option<String>,
}

struct RecoveredDraftRequest {
    dialog: Dialog,
    details: RecoveredDraftDetails,
    actions: Vec<RecoveredDraftChoice>,
    keep: usize,
    save_as: Option<usize>,
    resume_save_as: bool,
}

impl RecoveredDraftRequest {
    fn new(options: AlertDialogOptions, resume_save_as: bool) -> Option<Self> {
        if options.presentation != AlertDialogPresentation::Alert || options.input.is_some() {
            return None;
        }
        let buttons: Vec<DialogButton> = options
            .actions
            .iter()
            .map(|action| DialogButton {
                text: action.label.to_string(),
                style: action.role.into(),
            })
            .collect();
        let actions: Option<Vec<_>> = options
            .actions
            .iter()
            .zip(&buttons)
            .map(|(action, button)| match (&*action.id, button.style) {
                ("continue", DialogButtonStyle::Cancel | DialogButtonStyle::Default) => {
                    Some(RecoveredDraftChoice::Continue)
                }
                ("discard", DialogButtonStyle::Destructive) => Some(RecoveredDraftChoice::Discard),
                ("save-as", DialogButtonStyle::Default) => Some(RecoveredDraftChoice::SaveAs),
                _ => None,
            })
            .collect();
        let actions = actions?;
        actions
            .iter()
            .position(|a| *a == RecoveredDraftChoice::Continue)?;
        actions
            .iter()
            .position(|a| *a == RecoveredDraftChoice::Discard)?;
        let save_as = actions
            .iter()
            .position(|a| *a == RecoveredDraftChoice::SaveAs);
        if actions.len() != 2 + usize::from(save_as.is_some()) {
            return None;
        }
        // The stock caller may put Continue last to select it by default.
        // Recovery has explicit keyboard focus, so it uses visual order.
        let mut actions_and_buttons: Vec<_> = actions.into_iter().zip(buttons).collect();
        actions_and_buttons.sort_by_key(|(action, _)| match action {
            RecoveredDraftChoice::Continue => 0,
            RecoveredDraftChoice::Discard => 1,
            RecoveredDraftChoice::SaveAs => 2,
        });
        let (actions, buttons) = actions_and_buttons.into_iter().unzip();
        Some(Self {
            // The reply below has a different type and lifetime from a Core
            // Dialog ID; this stored layout input is not a Core request.
            dialog: Dialog {
                id: 0,
                title: options.title.map(|text| text.to_string()),
                message: options.description.map(|text| text.to_string()),
                buttons,
                cancelable: false,
                sheet: false,
                input: None,
            },
            details: RecoveredDraftDetails::default(),
            actions,
            keep: 0,
            save_as: save_as.map(|_| 2),
            resume_save_as,
        })
    }
}

/// Opens texzi's recovery confirmation over the declared AppWindow which
/// contains `host`, after the requesting engine's current mount batch.
/// Use the `continue`, `discard` and (when available) `save-as` actions.
/// If the host vanished or cannot show a window, the confirmation is dismissed.
pub fn show_recovered_draft_dialog(
    host: &ElementRef,
    options: AlertDialogOptions,
    resume_save_as: bool,
) -> impl Future<Output = Result<Option<RecoveredDraftChoice>, String>> + use<> {
    show_recovered_draft_dialog_with_details(
        host,
        options,
        resume_save_as,
        RecoveredDraftDetails::default(),
    )
}

/// Shows the same recovery confirmation with a badge and a one-line preview.
pub fn show_recovered_draft_dialog_with_details(
    host: &ElementRef,
    options: AlertDialogOptions,
    resume_save_as: bool,
    details: RecoveredDraftDetails,
) -> impl Future<Output = Result<Option<RecoveredDraftChoice>, String>> + use<> {
    let request = RecoveredDraftRequest::new(options, resume_save_as).map(|mut request| {
        request.details = RecoveredDraftDetails {
            badge: details.badge.filter(|text| !text.trim().is_empty()),
            summary: details.summary.filter(|text| !text.trim().is_empty()),
        };
        request
    });
    let (send, receive) = oneshot::channel();
    let shown = host.node().zip(request).map(|(host, request)| {
        run_on_main_thread(move || {
            if send.is_closed() {
                return None;
            }
            let surface = crate::surface::task_surface()?;
            open_recovered_draft(&surface, host, request, send)
        })
    });
    async move {
        let Some(shown) = shown else { return Ok(None) };
        let Some(mut guard) = shown.await.map_err(|error| error.to_string())? else {
            return Ok(None);
        };
        let choice = receive.await.ok();
        guard.armed = false;
        Ok(choice)
    }
}

/// Lives in the main-thread result slot even if the engine future is dropped
/// before it can take the result. Closing is idempotent and checks the route.
struct RecoveredOverlayGuard {
    key: u64,
    serial: u64,
    ui: crate::window::UiThread,
    armed: bool,
}

impl Drop for RecoveredOverlayGuard {
    fn drop(&mut self) {
        if self.armed {
            let (key, serial) = (self.key, self.serial);
            self.ui.invoke(move || {
                if let Some(surface) = Surface::by_key(key)
                    && let Some(dialog) = find_dialog(&surface, serial)
                {
                    dismiss_recovered(&surface, &dialog);
                }
            });
        }
    }
}

pub(crate) fn recovery_window(surface: &Surface, host: NodeId) -> Option<(NodeId, u64)> {
    let view = surface.view(host).filter(|view| view.attached())?;
    let window = crate::app_window::window_of(surface, &view);
    let frame = crate::app_window::presentation(surface, window, false)?;
    Some((window, frame))
}

fn open_recovered_draft(
    surface: &Rc<Surface>,
    host: NodeId,
    request: RecoveredDraftRequest,
    reply: oneshot::Sender<RecoveredDraftChoice>,
) -> Option<RecoveredOverlayGuard> {
    let (window, frame) = recovery_window(surface, host)?;
    if dialog_in_window(surface, window, DialogItem::takes_input).is_some() {
        return None;
    }
    // A stock prompt may already have answered and still be fading out.
    // Opening another recovery choice must not freeze that previous card.
    let ready = frame != 0 && dialog_in_window(surface, window, |_| true).is_none();
    let serial = show_dialog_item(
        surface,
        window,
        &request.dialog,
        Some((&request, reply, ready)),
    )?;
    Some(RecoveredOverlayGuard {
        key: surface.key(),
        serial,
        ui: surface.ui_thread(),
        armed: true,
    })
}

// ---------------------------------------------------------------------------
// Overlays

/// A dialog over a surface.
struct Overlay {
    /// The surface's key.
    key: u64,
    serial: u64,
    kind: OverlayKind,
}

#[derive(Clone)]
enum OverlayKind {
    Dialog(Rc<DialogItem>),
}

impl OverlayKind {
    fn item(&self) -> Rc<dyn Hosted> {
        match self {
            OverlayKind::Dialog(d) => d.clone(),
        }
    }

    fn opacity(&self) -> &Cell<f32> {
        match self {
            OverlayKind::Dialog(d) => &d.opacity,
        }
    }
}

fn next_serial() -> u64 {
    NEXT_SERIAL.with(|n| n.replace(n.get() + 1))
}

/// The overlay `serial` (a clone of its kind).
fn find(serial: u64) -> Option<(u64, OverlayKind)> {
    OVERLAYS.with(|o| {
        o.borrow()
            .iter()
            .find(|o| o.serial == serial)
            .map(|o| (o.key, o.kind.clone()))
    })
}

fn recovered_drafts(mut matches_surface: impl FnMut(u64) -> bool) -> Vec<Rc<DialogItem>> {
    OVERLAYS.with(|overlays| {
        overlays
            .borrow()
            .iter()
            .filter_map(|overlay| {
                let OverlayKind::Dialog(dialog) = &overlay.kind;
                (matches_surface(overlay.key) && dialog.recovered.is_some()).then(|| dialog.clone())
            })
            .collect()
    })
}

pub(crate) fn recovered_draft_preferences_changed(surface: &Surface) {
    let dialogs = recovered_drafts(|key| key == surface.key());
    if dialogs.is_empty() {
        return;
    }
    let appearance = RecoveredAppearance::read();
    for dialog in dialogs {
        if let Some(recovered) = &dialog.recovered
            && recovered.appearance.replace(appearance) != appearance
        {
            recovered.frozen.release();
            if let Some(root) = crate::app_window::root_view(surface, dialog.window) {
                root.changed();
            }
        }
    }
}

/// All surfaces on the UI thread use this device generation.
pub(crate) fn release_recovered_draft_device_images() {
    for dialog in recovered_drafts(|_| true) {
        dialog.release_frozen_backdrop();
    }
}

pub(crate) fn release_recovered_draft_window_images(surface: &Surface, window: NodeId) {
    for dialog in recovered_drafts(|key| key == surface.key()) {
        if dialog.window == window {
            dialog.release_frozen_backdrop();
        }
    }
}

/// The batch is applied: overlays stay over items hosted on the root
/// since (they are drawn in the order hosted).
pub(crate) fn end_batch(surface: &Surface) {
    let ours: Vec<Rc<dyn Hosted>> = OVERLAYS.with(|o| {
        o.borrow()
            .iter()
            .filter(|o| o.key == surface.key())
            .map(|o| o.kind.item())
            .collect()
    });
    if ours.is_empty() {
        return;
    }
    let mut state = surface.root.state_mut();
    let natives = &mut state.natives;
    let n = natives.len();
    let last = natives
        .iter()
        .rposition(|n| !ours.iter().any(|o| Rc::ptr_eq(o, &n.item)));
    let first_ours = natives
        .iter()
        .position(|n| ours.iter().any(|o| Rc::ptr_eq(o, &n.item)));
    if let (Some(last), Some(first)) = (last, first_ours)
        && last > first
    {
        // Stable: overlays keep their own order, after everything else.
        let (mut others, mut overlays): (Vec<_>, Vec<_>) = natives
            .drain(..)
            .partition(|n| !ours.iter().any(|o| Rc::ptr_eq(o, &n.item)));
        others.append(&mut overlays);
        *natives = others;
        debug_assert_eq!(natives.len(), n);
        drop(state);
        surface.root.changed();
    }
}

/// The surface is `width` × `height`: overlays follow.
pub(crate) fn surface_resized(surface: &Surface, width: f32, height: f32) {
    let ours: Vec<OverlayKind> = OVERLAYS.with(|o| {
        o.borrow()
            .iter()
            .filter(|o| o.key == surface.key())
            .map(|o| o.kind.clone())
            .collect()
    });
    for kind in ours {
        place(surface, &kind, width, height);
    }
}

/// The app stopped: its dialogs go too.
pub(crate) fn clear(surface: &Surface) {
    let removed: Vec<Overlay> = OVERLAYS.with(|o| {
        let mut o = o.borrow_mut();
        let (removed, kept) = std::mem::take(&mut *o)
            .into_iter()
            .partition(|o| o.key == surface.key());
        *o = kept;
        removed
    });
    for overlay in removed {
        let OverlayKind::Dialog(dialog) = &overlay.kind;
        dialog.release_frozen_backdrop();
        dialog.close_reply();
        if let Some(root) = overlay_root(surface, &overlay.kind) {
            root.remove_native(&overlay.kind.item());
        }
    }
}

/// The root view overlay `kind` is hosted in: the window that presented
/// it, else the main window's (the app root).
fn overlay_root(surface: &Surface, kind: &OverlayKind) -> Option<NshView> {
    match kind {
        OverlayKind::Dialog(d) if d.window != ROOT_ID => {
            crate::app_window::root_view(surface, d.window)
        }
        _ => Some(surface.root.clone()),
    }
}

/// Hosts the overlay where it goes in a `width` × `height` surface; one a
/// window presented fills that window.
fn place(surface: &Surface, kind: &OverlayKind, width: f32, height: f32) {
    // Its window is gone: nothing to place it over.
    let Some(root) = overlay_root(surface, kind) else {
        return;
    };
    let (width, height) = match kind {
        OverlayKind::Dialog(d) if d.window != ROOT_ID => {
            let size = root.state().frame.size;
            (size.width.0, size.height.0)
        }
        _ => (width, height),
    };
    let r = match kind {
        OverlayKind::Dialog(_) => rect(0.0, 0.0, width, height),
    };
    root.set_native(&kind.item(), r);
}

/// The window `id` is `width` × `height` points now: the dialogs it
/// presented follow.
pub(crate) fn window_resized(surface: &Surface, id: NodeId, width: f32, height: f32) {
    let ours: Vec<Rc<DialogItem>> = OVERLAYS.with(|o| {
        o.borrow()
            .iter()
            .filter_map(|o| match &o.kind {
                OverlayKind::Dialog(d) if o.key == surface.key() && d.window == id => {
                    Some(d.clone())
                }
                _ => None,
            })
            .collect()
    });
    for d in ours {
        if let Some(recovered) = &d.recovered {
            recovered.frozen.resize((width, height));
        }
        let kind = OverlayKind::Dialog(d);
        if let Some(root) = overlay_root(surface, &kind) {
            root.set_native(&kind.item(), rect(0.0, 0.0, width, height));
        }
    }
}

/// Hosts a new overlay over `surface`'s views and fades it in.
fn add_overlay(surface: &Rc<Surface>, serial: u64, kind: OverlayKind) {
    let OverlayKind::Dialog(dialog) = &kind;
    if dialog.recovered.is_none() {
        crate::native::take_keyboard(surface);
    }
    let env = surface.environment.get();
    place(surface, &kind, env.window.width.0, env.window.height.0);
    OVERLAYS.with(|o| {
        o.borrow_mut().push(Overlay {
            key: surface.key(),
            serial,
            kind: kind.clone(),
        })
    });
    // The same-window shield is live before cancellation: IMM can
    // synchronously send the window messages while it cancels.
    if dialog.recovered.is_some() {
        crate::text_input::cancel_recovery_composition(surface, dialog.window);
        if let Some(hwnd) = crate::app_window::hwnd_of(surface, dialog.window) {
            crate::native::take_keyboard_in(hwnd);
        }
    }
    if env.reduce_motion {
        kind.opacity().set(1.0);
    } else {
        kind.opacity().set(0.0);
        fade(surface, serial, 1.0, FADE_IN, false);
    }
    if recovery_blocks_background(surface, dialog.window) {
        crate::access::recovery_changed(surface, dialog.window);
    }
}

/// Window `id` is closing: the dialogs it showed are dismissed (`-1`), so
/// nothing is left over another window or takes its keys.
pub(crate) fn window_closing(surface: &Surface, id: NodeId) {
    let ours: Vec<Rc<DialogItem>> = OVERLAYS.with(|overlays| {
        overlays
            .borrow()
            .iter()
            .filter_map(|overlay| match &overlay.kind {
                OverlayKind::Dialog(d) if overlay.key == surface.key() && d.window == id => {
                    Some(d.clone())
                }
                _ => None,
            })
            .collect()
    });
    for dialog in ours {
        if !dismiss_recovered(surface, &dialog) {
            dialog.answer(-1);
        }
    }
}

fn find_dialog(surface: &Surface, serial: u64) -> Option<Rc<DialogItem>> {
    let (key, OverlayKind::Dialog(dialog)) = find(serial)?;
    (surface.key() == key).then_some(dialog)
}

/// Stops a recovery overlay before allowing its request to settle.
fn dismiss_recovered(surface: &Surface, dialog: &DialogItem) -> bool {
    if dialog.recovered.is_none() {
        return false;
    }
    dialog.closing.set(true);
    dialog.release_frozen_backdrop();
    drop_overlay(surface, dialog.serial);
    dialog.close_reply();
    true
}

/// Takes back the dialog [`show_dialog`] showed as `id`
/// (`Command::CloseDialog`): it fades out as an answered one does, and
/// reports nothing (the core answered it).
fn close_dialog(surface: &Rc<Surface>, id: u32) {
    let serial = OVERLAYS.with(|o| {
        o.borrow().iter().find_map(|o| match &o.kind {
            OverlayKind::Dialog(d)
                if o.key == surface.key()
                    && matches!(d.reply, DialogReply::Core(core_id) if core_id == id)
                    && !d.closing.get() =>
            {
                Some(o.serial)
            }
            _ => None,
        })
    });
    if let Some(serial) = serial {
        remove_overlay(surface, serial);
    }
}

/// Fades overlay `serial` out, then removes it. No-op when it is gone.
fn remove_overlay(surface: &Rc<Surface>, serial: u64) {
    let Some((_, kind)) = find(serial) else {
        return;
    };
    let OverlayKind::Dialog(d) = &kind;
    d.closing.set(true);
    if d.recovered.is_some() || surface.environment.get().reduce_motion {
        d.release_frozen_backdrop();
        drop_overlay(surface, serial);
    } else {
        fade(surface, serial, 0.0, FADE_OUT, true);
    }
}

/// Stops hosting overlay `serial` at once.
fn drop_overlay(surface: &Surface, serial: u64) {
    let removed = OVERLAYS.with(|o| {
        let mut o = o.borrow_mut();
        let i = o.iter().position(|o| o.serial == serial)?;
        Some(o.remove(i))
    });
    if let Some(overlay) = removed {
        if let Some(root) = overlay_root(surface, &overlay.kind) {
            root.remove_native(&overlay.kind.item());
        }
        let OverlayKind::Dialog(dialog) = overlay.kind;
        if dialog.recovered.is_some() || recovery_blocks_background(surface, dialog.window) {
            crate::access::recovery_changed(surface, dialog.window);
        }
    }
}

/// Animates overlay `serial`'s opacity to `to` over `secs` (eased out) on
/// the surface's frames; removes it at the end when `remove`.
fn fade(surface: &Rc<Surface>, serial: u64, to: f32, secs: f64, remove: bool) {
    let Some((_, kind)) = find(serial) else {
        return;
    };
    let from = kind.opacity().get();
    let mut start = None;
    surface.on_draw_frame(move |surface, now| {
        let Some((_, kind)) = find(serial) else {
            return false;
        };
        let t0 = *start.get_or_insert(now);
        let t = ((now - t0) / secs).clamp(0.0, 1.0) as f32;
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        kind.opacity().set(from + (to - from) * eased);
        if let Some(root) = overlay_root(surface, &kind) {
            root.changed();
        }
        if t < 1.0 {
            // A newer fade took over (a dialog answered while fading in).
            return !matches!(&kind, OverlayKind::Dialog(d) if d.closing.get() && !remove);
        }
        if remove {
            drop_overlay(surface, serial);
        }
        false
    });
}

// ---------------------------------------------------------------------------
// Drawing helpers

/// One piece of text laid out by DirectWrite.
struct Label {
    layout: IDWriteTextLayout,
}

impl Label {
    /// `text` in the UI font at `size` points and `weight`, wrapped at
    /// `max_width` (or on one line when `wrap` is false).
    fn new(
        text: &str,
        size: f32,
        weight: i32,
        align: DWRITE_TEXT_ALIGNMENT,
        wrap: bool,
    ) -> Option<Label> {
        let factory = crate::gfx::dwrite();
        let family = HSTRING::from(&*crate::text::ui_family());
        let units: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: COM calls with valid strings.
        unsafe {
            let format = factory
                .CreateTextFormat(
                    &family,
                    None,
                    DWRITE_FONT_WEIGHT(weight),
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    &HSTRING::new(),
                )
                .ok()?;
            let _ = format.SetTextAlignment(align);
            if !wrap {
                let _ = format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
                let _ = format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            }
            let layout = factory
                .CreateTextLayout(&units, &format, 100_000.0, 100_000.0)
                .ok()?;
            Some(Label { layout })
        }
    }

    /// Its size when at most `max_width` wide: the width of its widest
    /// line and its height.
    fn measure(&self, max_width: f32) -> (f32, f32) {
        let mut m = DWRITE_TEXT_METRICS::default();
        // SAFETY: COM calls; a local out value.
        unsafe {
            let _ = self.layout.SetMaxWidth(max_width.max(1.0));
            let _ = self.layout.GetMetrics(&mut m);
        }
        (m.width.ceil(), m.height.ceil())
    }

    /// Lays it out in a box `w` × `h` (alignment applies inside it).
    fn fit(&self, w: f32, h: f32) {
        // SAFETY: COM calls on a live layout.
        unsafe {
            let _ = self.layout.SetMaxWidth(w.max(1.0));
            let _ = self.layout.SetMaxHeight(h.max(1.0));
        }
    }

    fn ellipsized(self) -> Self {
        let trimming = DWRITE_TRIMMING {
            granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
            ..Default::default()
        };
        // SAFETY: both COM objects belong to this layout's DirectWrite factory.
        unsafe {
            if let Ok(sign) = crate::gfx::dwrite().CreateEllipsisTrimmingSign(&self.layout) {
                let _ = self.layout.SetTrimming(&trimming, &sign);
            }
        }
        self
    }

    fn draw(&self, cx: &DrawCx<'_>, x: f32, y: f32, color: Color) {
        // SAFETY: a COM call between BeginDraw and EndDraw.
        unsafe {
            cx.dc.DrawTextLayout(
                windows_numerics::Vector2 { X: x, Y: y },
                &self.layout,
                cx.solid(color),
                D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
            );
        }
    }
}

fn rounded(r: Bounds<Points>, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: rect_f(r.origin.x.0, r.origin.y.0, r.size.width.0, r.size.height.0),
        radiusX: radius,
        radiusY: radius,
    }
}

fn fill_rounded(cx: &DrawCx<'_>, r: Bounds<Points>, radius: f32, color: Color) {
    // SAFETY: a COM call between BeginDraw and EndDraw.
    unsafe {
        cx.dc
            .FillRoundedRectangle(&rounded(r, radius), cx.solid(color))
    };
}

fn stroke_rounded(cx: &DrawCx<'_>, r: Bounds<Points>, radius: f32, color: Color, width: f32) {
    let i = width / 2.0;
    let r = rect(
        r.origin.x.0 + i,
        r.origin.y.0 + i,
        r.size.width.0 - width,
        r.size.height.0 - width,
    );
    // SAFETY: a COM call between BeginDraw and EndDraw.
    unsafe {
        cx.dc.DrawRoundedRectangle(
            &rounded(r, (radius - i).max(0.0)),
            cx.solid(color),
            width,
            None,
        )
    };
}

/// A soft shadow under a card: rounded rectangles growing outward with
/// falling opacity (Direct2D's blur would need an offscreen target in the
/// middle of the pass).
fn draw_shadow(cx: &DrawCx<'_>, r: Bounds<Points>, radius: f32, dy: f32, alpha: f32) {
    const STEPS: usize = 14;
    for i in (0..STEPS).rev() {
        let grow = (i + 1) as f32 * 2.0;
        let a = alpha * (1.0 - i as f32 / STEPS as f32).powi(2) / STEPS as f32 * 2.5;
        let shadow = rect(
            r.origin.x.0 - grow,
            r.origin.y.0 - grow + dy,
            r.size.width.0 + 2.0 * grow,
            r.size.height.0 + 2.0 * grow,
        );
        fill_rounded(cx, shadow, radius + grow, with_alpha(Color::BLACK, a));
    }
}

fn with_alpha(c: Color, alpha: f32) -> Color {
    Color((c.0 & 0xffff_ff00) | (alpha.clamp(0.0, 1.0) * 255.0).round() as u32)
}

/// `c` mixed with `other` by `t` (0 is `c`).
fn mix(c: Color, other: Color, t: f32) -> Color {
    let ch = |v: u32, s: u32| ((v >> s) & 0xff) as f32;
    let part = |s: u32| {
        let v = ch(c.0, s) + (ch(other.0, s) - ch(c.0, s)) * t;
        (v.round().clamp(0.0, 255.0) as u32) << s
    };
    Color(part(24) | part(16) | part(8) | part(0))
}

/// Windows 11's colors for dialogs in the light or dark theme.
struct Palette {
    scrim: Color,
    card: Color,
    footer: Color,
    stroke: Color,
    text: Color,
    secondary: Color,
    button: Color,
    button_hover: Color,
    button_pressed: Color,
    button_stroke: Color,
    accent: Color,
    accent_text: Color,
    critical: Color,
    focus: Color,
    separator: Color,
    row_hover: Color,
}

impl Palette {
    fn new(dark: bool) -> Palette {
        if dark {
            Palette {
                scrim: Color(0x0000_0080),
                card: Color(0x2b2b_2bff),
                footer: Color(0x2020_20ff),
                stroke: Color(0x0000_0066),
                text: Color(0xffff_ffff),
                secondary: Color(0xc5c5_c5ff),
                button: Color(0x3737_37ff),
                button_hover: Color(0x3d3d_3dff),
                button_pressed: Color(0x3030_30ff),
                button_stroke: Color(0x4444_44ff),
                accent: accent(true),
                accent_text: Color(0x0000_00ff),
                critical: Color(0xd134_38ff),
                focus: Color(0xffff_ffff),
                separator: Color(0x3d3d_3dff),
                row_hover: Color(0xffff_ff0f),
            }
        } else {
            Palette {
                scrim: Color(0x0000_004d),
                card: Color(0xffff_ffff),
                footer: Color(0xf3f3_f3ff),
                stroke: Color(0x0000_0024),
                text: Color(0x1a1a_1aff),
                secondary: Color(0x5d5d_5dff),
                button: Color(0xfbfb_fbff),
                button_hover: Color(0xf6f6_f6ff),
                button_pressed: Color(0xf0f0_f0ff),
                button_stroke: Color(0xe0e0_e0ff),
                accent: accent(false),
                accent_text: Color(0xffff_ffff),
                critical: Color(0xc42b_1cff),
                focus: Color(0x0000_00e4),
                separator: Color(0xe5e5_e5ff),
                row_hover: Color(0x0000_000a),
            }
        }
    }

    fn recovered(&mut self, dark: bool, contrast: Option<DialogContrast>, glass: bool) {
        if let Some(contrast) = contrast {
            self.card = contrast.window;
            self.footer = contrast.window;
            self.text = contrast.text;
            self.secondary = contrast.text;
            self.stroke = contrast.text;
            self.focus = contrast.text;
            return;
        }
        // Origami iris/on-primary, as used by the workbench.
        (self.accent, self.accent_text) = if dark {
            (Color(0x9b92_ffff), Color(0x1719_27ff))
        } else {
            (Color(0x5b50_e6ff), Color(0xffff_ffff))
        };
        self.focus = self.accent;
        self.stroke = self.secondary;
        if glass {
            self.card = if dark {
                Color(0x1a1f_2dc2)
            } else {
                Color(0xffff_ffc2)
            };
            // Keep the prepared local backdrop visible under the footer.
            self.footer = if dark {
                Color(0x1a1f_2d26)
            } else {
                Color(0xffff_ff26)
            };
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct DialogContrast {
    window: Color,
    text: Color,
    button: Color,
    button_text: Color,
    highlight: Color,
    highlight_text: Color,
}

impl DialogContrast {
    fn read() -> Self {
        let color = |index: SYS_COLOR_INDEX| {
            // SAFETY: asks for a known system color, in COLORREF order.
            let value = unsafe { GetSysColor(index) };
            Color::rgba(value as u8, (value >> 8) as u8, (value >> 16) as u8, 255)
        };
        Self {
            window: color(COLOR_WINDOW),
            text: color(COLOR_WINDOWTEXT),
            button: color(COLOR_BTNFACE),
            button_text: color(COLOR_BTNTEXT),
            highlight: color(COLOR_HIGHLIGHT),
            highlight_text: color(COLOR_HIGHLIGHTTEXT),
        }
    }

    fn button(self, selected: bool) -> (Color, Color) {
        if selected {
            (self.highlight, self.highlight_text)
        } else {
            (self.button, self.button_text)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct RecoveredAppearance {
    capture: bool,
    contrast: Option<DialogContrast>,
}

impl RecoveredAppearance {
    fn read() -> Self {
        let mut contrast = HIGHCONTRASTW {
            cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
            ..Default::default()
        };
        // SAFETY: the output is writable and valid for its declared size.
        let known_contrast = unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                contrast.cbSize,
                Some((&mut contrast as *mut HIGHCONTRASTW).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        }
        .is_ok();
        let contrast = (known_contrast && contrast.dwFlags.contains(HCF_HIGHCONTRASTON))
            .then(DialogContrast::read);
        let transparent = reg_dword(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("EnableTransparency"),
        )
        .is_some_and(|enabled| enabled != 0);
        let mut power = SYSTEM_POWER_STATUS::default();
        // SAFETY: the output structure is writable for the call.
        let normal_power =
            unsafe { GetSystemPowerStatus(&mut power) }.is_ok() && power.SystemStatusFlag == 0;
        Self {
            capture: known_contrast && contrast.is_none() && transparent && normal_power,
            contrast,
        }
    }
}

struct RecoveredDraftStyle {
    frozen: FrozenModal,
    appearance: Cell<RecoveredAppearance>,
    body_offset: Cell<f32>,
    title: Option<String>,
    description: Option<String>,
}

impl RecoveredDraftStyle {
    fn new(serial: u64, ready_to_capture: bool, request: &RecoveredDraftRequest) -> Self {
        let appearance = RecoveredAppearance::read();
        let spoken = |lead: Option<&str>, tail: Option<&str>| match (
            lead.filter(|s| !s.is_empty()),
            tail.filter(|s| !s.is_empty()),
        ) {
            (Some(lead), Some(tail)) => Some(format!("{lead}\n{tail}")),
            (Some(text), None) | (None, Some(text)) => Some(text.to_owned()),
            (None, None) => None,
        };
        Self {
            frozen: FrozenModal::new(serial, appearance.capture && ready_to_capture),
            appearance: Cell::new(appearance),
            body_offset: Cell::new(0.0),
            title: spoken(
                request.details.badge.as_deref(),
                request.dialog.title.as_deref(),
            ),
            description: spoken(
                request.dialog.message.as_deref(),
                request.details.summary.as_deref(),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Dialog layout

/// Sizes of dialogs (points), after Windows 11's content dialog.
mod metrics {
    pub(super) const GUTTER: f32 = 16.0;
    pub(super) const PAD: f32 = 24.0;
    pub(super) const MIN_WIDTH: f32 = 320.0;
    pub(super) const MAX_WIDTH: f32 = 548.0;
    pub(super) const TITLE_GAP: f32 = 12.0;
    pub(super) const BUTTON_HEIGHT: f32 = 32.0;
    pub(super) const BUTTON_PAD: f32 = 12.0;
    pub(super) const BUTTON_GAP: f32 = 8.0;
    pub(super) const RADIUS: f32 = 8.0;
    pub(super) const RECOVERED_WIDTH: f32 = 420.0;
    pub(super) const RECOVERED_RADIUS: f32 = 18.0;
    pub(super) const BADGE_PAD_X: f32 = 8.0;
    pub(super) const BADGE_PAD_Y: f32 = 4.0;
    pub(super) const DETAILS_GAP: f32 = 8.0;
    pub(super) const SUMMARY_PAD_X: f32 = 10.0;
    pub(super) const SUMMARY_PAD_Y: f32 = 6.0;
    pub(super) const SUMMARY_RADIUS: f32 = 6.0;
    pub(super) const BUTTON_RADIUS: f32 = 4.0;
    pub(super) const SHEET_MAX_WIDTH: f32 = 420.0;
    pub(super) const SHEET_ROW: f32 = 40.0;
    pub(super) const SHEET_INSET: f32 = 4.0;
    pub(super) const SHEET_TEXT_PAD: f32 = 12.0;
}

/// A part of a dialog that has text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Badge,
    Title,
    Message,
    Summary,
    Button(usize),
}

#[derive(Default)]
struct RecoveryParts {
    badge: bool,
    summary: bool,
}

/// Where a dialog's parts go (surface coordinates).
#[derive(Debug, Default, Clone, PartialEq)]
struct Geometry {
    /// The cards (an alert's one; a sheet's options and its cancel card).
    cards: Vec<Bounds<Points>>,
    /// An alert's footer (the buttons' band at the bottom of the card).
    footer: Option<Bounds<Points>>,
    /// A sheet's line between its text and its options.
    separator: Option<Bounds<Points>>,
    badge: Option<Bounds<Points>>,
    title: Option<Bounds<Points>>,
    message: Option<Bounds<Points>>,
    summary: Option<Bounds<Points>>,
    /// A capped title and description viewport, when the recovery card needs one.
    body_viewport: Option<Bounds<Points>>,
    /// The buttons, by index.
    buttons: Vec<Bounds<Points>>,
    /// An alert's buttons are in one row (else a column).
    row: bool,
}

impl Geometry {
    fn body_travel(&self) -> f32 {
        let Some(viewport) = self.body_viewport else {
            return 0.0;
        };
        self.summary
            .or(self.message)
            .or(self.title)
            .or(self.badge)
            .map_or(0.0, |last| {
                (last.origin.y.0 + last.size.height.0
                    - viewport.origin.y.0
                    - viewport.size.height.0)
                    .max(0.0)
            })
    }
}

/// Lays out an alert in a surface of `view` size. `measure(part, width)`
/// gives the size of a part's text at most `width` wide (unbounded with
/// `f32::INFINITY`); `title` / `message` say which parts exist.
fn layout_alert(
    view: (f32, f32),
    title: bool,
    message: bool,
    buttons: usize,
    recovery_parts: Option<RecoveryParts>,
    measure: &dyn Fn(Part, f32) -> (f32, f32),
) -> Geometry {
    use metrics::*;
    let available = (view.0 - 2.0 * GUTTER).max(0.0);
    let button_widths: Vec<f32> = (0..buttons)
        .map(|i| measure(Part::Button(i), f32::INFINITY).0 + 2.0 * BUTTON_PAD)
        .collect();
    let widest_button = button_widths.iter().copied().fold(0.0, f32::max);
    let n = buttons.max(1) as f32;
    let natural = [
        title.then(|| measure(Part::Title, f32::INFINITY).0),
        message.then(|| measure(Part::Message, f32::INFINITY).0),
        Some(widest_button * n + BUTTON_GAP * (n - 1.0)),
    ]
    .into_iter()
    .flatten()
    .fold(0.0, f32::max)
        + 2.0 * PAD;
    let recovery = recovery_parts.is_some();
    let details = recovery_parts.unwrap_or_default();
    let width = if recovery {
        RECOVERED_WIDTH
    } else {
        natural.clamp(MIN_WIDTH, MAX_WIDTH)
    }
    .min(available);
    let inner = (width - 2.0 * PAD).max(0.0);
    // Buttons share the footer in equal columns (as Windows' do) while
    // there are at most three and every label fits its column.
    let column = (inner - BUTTON_GAP * (n - 1.0)) / n;
    let row = buttons <= 3 && button_widths.iter().all(|w| *w <= column);

    let title_h = if title {
        measure(Part::Title, inner).1
    } else {
        0.0
    };
    let message_h = if message {
        measure(Part::Message, inner).1
    } else {
        0.0
    };
    let (badge_w, badge_h) = if details.badge {
        let (w, h) = measure(Part::Badge, (inner - 2.0 * BADGE_PAD_X).max(0.0));
        ((w + 2.0 * BADGE_PAD_X).min(inner), h + 2.0 * BADGE_PAD_Y)
    } else {
        (0.0, 0.0)
    };
    let summary_h = if details.summary {
        measure(Part::Summary, (inner - 2.0 * SUMMARY_PAD_X).max(0.0)).1 + 2.0 * SUMMARY_PAD_Y
    } else {
        0.0
    };
    let badge_gap = if details.badge && (title || message || details.summary) {
        DETAILS_GAP
    } else {
        0.0
    };
    let gap = if title && message { TITLE_GAP } else { 0.0 };
    let summary_gap = if details.summary && (title || message) {
        DETAILS_GAP
    } else {
        0.0
    };
    let text_h = badge_h + badge_gap + title_h + gap + message_h + summary_gap + summary_h;
    let buttons_h = if row {
        BUTTON_HEIGHT
    } else {
        n * BUTTON_HEIGHT + (n - 1.0) * BUTTON_GAP
    };
    let window_height = (view.1 - 2.0 * GUTTER).max(0.0);
    let footer_pad = if recovery {
        PAD.min(((window_height - buttons_h) / 4.0).max(0.0))
    } else {
        PAD
    };
    let footer_h = 2.0 * footer_pad + buttons_h;
    let body = if recovery {
        (text_h + 2.0 * PAD).min((window_height - footer_h).max(0.0))
    } else {
        text_h + 2.0 * PAD
    };
    let body_pad = if recovery { PAD.min(body / 4.0) } else { PAD };
    let body_view_height = (body - 2.0 * body_pad).max(0.0);
    let height = body + footer_h;
    let x = ((view.0 - width) / 2.0).round();
    let y = ((view.1 - height) / 2.0).round().max(0.0);
    let card = rect(x, y, width, height);
    let footer = rect(x, y + body, width, footer_h);
    let buttons = (0..buttons)
        .map(|i| {
            let i = i as f32;
            if row {
                rect(
                    x + PAD + i * (column + BUTTON_GAP),
                    footer.origin.y.0 + footer_pad,
                    column,
                    BUTTON_HEIGHT,
                )
            } else {
                rect(
                    x + PAD,
                    footer.origin.y.0 + footer_pad + i * (BUTTON_HEIGHT + BUTTON_GAP),
                    inner,
                    BUTTON_HEIGHT,
                )
            }
        })
        .collect();
    let body_y = y + body_pad;
    let title_y = body_y + badge_h + badge_gap;
    let message_y = title_y + title_h + gap;
    Geometry {
        cards: vec![card],
        footer: Some(footer),
        separator: None,
        badge: details
            .badge
            .then(|| rect(x + PAD, body_y, badge_w, badge_h)),
        title: title.then(|| rect(x + PAD, title_y, inner, title_h)),
        message: message.then(|| rect(x + PAD, message_y, inner, message_h)),
        summary: details.summary.then(|| {
            rect(
                x + PAD,
                message_y + message_h + summary_gap,
                inner,
                summary_h,
            )
        }),
        body_viewport: (recovery && body_view_height < text_h)
            .then(|| rect(x + PAD, y + body_pad, inner, body_view_height)),
        buttons,
        row,
    }
}

/// Lays out an action sheet at the bottom of a surface of `view` size: its
/// text and options in one card, the cancel button (`cancel`) in its own
/// below.
fn layout_sheet(
    view: (f32, f32),
    title: bool,
    message: bool,
    buttons: usize,
    cancel: Option<usize>,
    measure: &dyn Fn(Part, f32) -> (f32, f32),
) -> Geometry {
    use metrics::*;
    let width = (view.0 - 2.0 * BUTTON_GAP).clamp(0.0, SHEET_MAX_WIDTH);
    let x = ((view.0 - width) / 2.0).round();
    let text_w = (width - 2.0 * SHEET_TEXT_PAD).max(0.0);
    let title_h = if title {
        measure(Part::Title, text_w).1
    } else {
        0.0
    };
    let message_h = if message {
        measure(Part::Message, text_w).1
    } else {
        0.0
    };
    let has_text = title || message;
    let header = if has_text {
        SHEET_TEXT_PAD
            + title_h
            + if title && message { 4.0 } else { 0.0 }
            + message_h
            + SHEET_TEXT_PAD
    } else {
        0.0
    };
    let options = (0..buttons).filter(|i| Some(*i) != cancel).count();
    let has_options_card = has_text || options > 0;
    let options_h = if has_options_card {
        header
            + if has_text && options > 0 { 1.0 } else { 0.0 }
            + if options > 0 {
                2.0 * SHEET_INSET + options as f32 * SHEET_ROW
            } else {
                0.0
            }
    } else {
        0.0
    };
    let cancel_h = if cancel.is_some() {
        2.0 * SHEET_INSET + SHEET_ROW
    } else {
        0.0
    };
    let gap = if has_options_card && cancel.is_some() {
        BUTTON_GAP
    } else {
        0.0
    };
    let bottom = (view.1 - 2.0 * BUTTON_GAP).max(options_h + gap + cancel_h);
    let cancel_y = bottom - cancel_h;
    let options_y = cancel_y - gap - options_h;
    let mut cards = Vec::new();
    if has_options_card {
        cards.push(rect(x, options_y, width, options_h));
    }
    if cancel.is_some() {
        cards.push(rect(x, cancel_y, width, cancel_h));
    }
    let row_w = width - 2.0 * SHEET_INSET;
    let first_row =
        options_y + header + if has_text && options > 0 { 1.0 } else { 0.0 } + SHEET_INSET;
    let mut k = 0.0;
    let buttons = (0..buttons)
        .map(|i| {
            if Some(i) == cancel {
                rect(x + SHEET_INSET, cancel_y + SHEET_INSET, row_w, SHEET_ROW)
            } else {
                let r = rect(x + SHEET_INSET, first_row + k * SHEET_ROW, row_w, SHEET_ROW);
                k += 1.0;
                r
            }
        })
        .collect();
    let title_y = options_y + SHEET_TEXT_PAD;
    Geometry {
        cards,
        footer: None,
        separator: (has_text && options > 0).then(|| rect(x, options_y + header, width, 1.0)),
        badge: None,
        title: title.then(|| rect(x + SHEET_TEXT_PAD, title_y, text_w, title_h)),
        message: message.then(|| {
            rect(
                x + SHEET_TEXT_PAD,
                title_y + title_h + if title { 4.0 } else { 0.0 },
                text_w,
                message_h,
            )
        }),
        summary: None,
        body_viewport: None,
        buttons,
        row: false,
    }
}

/// The button Enter presses at first: the last regular button of an alert
/// (its default action), else the first.
fn default_button(styles: &[DialogButtonStyle], sheet: bool) -> Option<usize> {
    if styles.is_empty() {
        return None;
    }
    if sheet {
        return Some(0);
    }
    Some(
        (0..styles.len())
            .rev()
            .find(|i| styles[*i] == DialogButtonStyle::Default)
            .unwrap_or(0),
    )
}

/// The buttons in focus order: a sheet's options, then its cancel button
/// (as drawn); an alert's in order.
fn focus_order(count: usize, cancel: Option<usize>, sheet: bool) -> Vec<usize> {
    let mut order: Vec<usize> = (0..count)
        .filter(|i| !sheet || Some(*i) != cancel)
        .collect();
    if sheet && let Some(c) = cancel {
        order.push(c);
    }
    order
}

/// The button after (`step` 1) or before (-1) `current` in `order`,
/// wrapping around.
fn step_focus(order: &[usize], current: Option<usize>, step: isize) -> Option<usize> {
    let n = order.len() as isize;
    if n == 0 {
        return None;
    }
    let at = current.and_then(|c| order.iter().position(|i| *i == c));
    let next = match at {
        Some(at) => (at as isize + step).rem_euclid(n),
        None if step < 0 => n - 1,
        None => 0,
    };
    Some(order[next as usize])
}

// ---------------------------------------------------------------------------
// Dialogs

/// What a press on a dialog started on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Button(usize),
    Card,
    Outside,
}

struct DialogTexts {
    badge: Option<Label>,
    title: Option<Label>,
    message: Option<Label>,
    summary: Option<Label>,
    buttons: Vec<Label>,
}

/// A dialog's laid-out state, made again when the surface's size or text
/// scale changes.
struct DialogLayout {
    size: (f32, f32),
    geometry: Geometry,
}

enum DialogReply {
    Core(u32),
    Recovered {
        actions: Vec<RecoveredDraftChoice>,
        sender: RefCell<Option<oneshot::Sender<RecoveredDraftChoice>>>,
    },
}

/// An alert or action sheet, hosted by the root over the whole surface
/// (it takes every pointer event: the dialog is modal).
struct DialogItem {
    recovered: Option<RecoveredDraftStyle>,
    /// The window of the app's own showing it (`ROOT_ID`: the main one).
    window: NodeId,
    key: u64,
    /// The UI thread of the surface: the answer is posted back to it.
    ui: crate::window::UiThread,
    serial: u64,
    reply: DialogReply,
    sheet: bool,
    cancelable: bool,
    cancel: Option<usize>,
    /// The accent button (an alert's default action).
    default: Option<usize>,
    styles: Vec<DialogButtonStyle>,
    texts: DialogTexts,
    layout: RefCell<Option<DialogLayout>>,
    hover: Cell<Option<usize>>,
    pressed: Cell<Option<Target>>,
    focus: Cell<Option<usize>>,
    /// The focus was moved with the keyboard: its ring shows.
    keyboard: Cell<bool>,
    opacity: Cell<f32>,
    /// Answered: fading out, no more input.
    closing: Cell<bool>,
    /// The buttons' texts, in order.
    labels: Vec<String>,
}

impl DialogItem {
    fn takes_input(&self) -> bool {
        self.recovered.is_some() || !self.closing.get()
    }

    fn geometry(&self, size: (f32, f32)) -> Geometry {
        let mut layout = self.layout.borrow_mut();
        if let Some(l) = &*layout
            && l.size == size
        {
            return l.geometry.clone();
        }
        let t = &self.texts;
        let measure = |part: Part, width: f32| {
            let label = match part {
                Part::Badge => t.badge.as_ref(),
                Part::Title => t.title.as_ref(),
                Part::Message => t.message.as_ref(),
                Part::Summary => t.summary.as_ref(),
                Part::Button(i) => t.buttons.get(i),
            };
            label.map_or((0.0, 0.0), |l| l.measure(width))
        };
        let geometry = if self.sheet {
            layout_sheet(
                size,
                t.title.is_some(),
                t.message.is_some(),
                t.buttons.len(),
                self.cancel,
                &measure,
            )
        } else {
            layout_alert(
                size,
                t.title.is_some(),
                t.message.is_some(),
                t.buttons.len(),
                self.recovered.as_ref().map(|_| RecoveryParts {
                    badge: t.badge.is_some(),
                    summary: t.summary.is_some(),
                }),
                &measure,
            )
        };
        if let (Some(l), Some(r)) = (&t.badge, geometry.badge) {
            l.fit(
                r.size.width.0 - 2.0 * metrics::BADGE_PAD_X,
                r.size.height.0 - 2.0 * metrics::BADGE_PAD_Y,
            );
        }
        if let (Some(l), Some(r)) = (&t.title, geometry.title) {
            l.fit(r.size.width.0, r.size.height.0);
        }
        if let (Some(l), Some(r)) = (&t.message, geometry.message) {
            l.fit(r.size.width.0, r.size.height.0);
        }
        if let (Some(l), Some(r)) = (&t.summary, geometry.summary) {
            l.fit(
                r.size.width.0 - 2.0 * metrics::SUMMARY_PAD_X,
                r.size.height.0 - 2.0 * metrics::SUMMARY_PAD_Y,
            );
        }
        for (l, r) in t.buttons.iter().zip(&geometry.buttons) {
            let pad = if self.sheet {
                metrics::SHEET_TEXT_PAD
            } else {
                metrics::BUTTON_PAD
            };
            l.fit(r.size.width.0 - 2.0 * pad, r.size.height.0);
        }
        *layout = Some(DialogLayout {
            size,
            geometry: geometry.clone(),
        });
        geometry
    }

    fn target(&self, x: f32, y: f32) -> Target {
        let Some(g) = self.layout.borrow().as_ref().map(|l| l.geometry.clone()) else {
            return Target::Outside;
        };
        let inside = |r: &Bounds<Points>| {
            x >= r.origin.x.0
                && y >= r.origin.y.0
                && x < r.origin.x.0 + r.size.width.0
                && y < r.origin.y.0 + r.size.height.0
        };
        if let Some(i) = g.buttons.iter().position(inside) {
            Target::Button(i)
        } else if g.cards.iter().any(inside) {
            Target::Card
        } else {
            Target::Outside
        }
    }

    fn invalidate(&self) {
        if let Some(surface) = Surface::by_key(self.key) {
            if self.recovered.is_some()
                && let Some(root) = crate::app_window::root_view(&surface, self.window)
            {
                root.changed();
            } else {
                surface.root.changed();
            }
        }
    }

    fn release_frozen_backdrop(&self) {
        if let Some(recovered) = &self.recovered {
            recovered.frozen.release();
        }
    }

    fn close_reply(&self) {
        if let DialogReply::Recovered { sender, .. } = &self.reply {
            drop(sender.take());
        }
    }

    /// Answers with button `index` (-1: dismissed), once; after the
    /// current event, as answering removes the item.
    fn answer(&self, index: i32) {
        if self.closing.replace(true) {
            return;
        }
        let (key, serial) = (self.key, self.serial);
        let core_id = match self.reply {
            DialogReply::Core(id) => Some(id),
            DialogReply::Recovered { .. } => None,
        };
        self.ui.invoke(move || {
            let Some(surface) = Surface::by_key(key) else {
                return;
            };
            if let Some(id) = core_id {
                remove_overlay(&surface, serial);
                surface.event(ROOT_ID, EventKind::Dialog, dialog_answer(id, index));
            } else if let Some(dialog) = find_dialog(&surface, serial) {
                remove_overlay(&surface, serial);
                if let DialogReply::Recovered { actions, sender } = &dialog.reply
                    && let Some(sender) = sender.take()
                    && let Ok(index) = usize::try_from(index)
                    && let Some(choice) = actions.get(index)
                {
                    let _ = sender.send(*choice);
                }
            }
        });
    }

    /// Escape: the cancel button, else dismissed when cancelable.
    fn escape(&self) {
        match self.cancel {
            Some(cancel) => self.answer(cancel as i32),
            None if self.cancelable => self.answer(-1),
            None => {}
        }
    }

    fn scroll_body(&self, change: f32) {
        if self.closing.get() {
            return;
        }
        let Some(recovered) = &self.recovered else {
            return;
        };
        let max = self
            .layout
            .borrow()
            .as_ref()
            .map_or(0.0, |l| l.geometry.body_travel());
        let old = recovered.body_offset.get();
        let new = (old + change).clamp(0.0, max);
        if old != new {
            recovered.body_offset.set(new);
            self.invalidate();
        }
    }

    fn key(&self, event: &KeyEvent) {
        if !event.down || self.closing.get() {
            return;
        }
        let order = focus_order(self.styles.len(), self.cancel, self.sheet);
        let step = match event.vk {
            VK_PAGE_UP | VK_PAGE_DOWN if self.recovered.is_some() => {
                let height = self
                    .layout
                    .borrow()
                    .as_ref()
                    .and_then(|layout| layout.geometry.body_viewport)
                    .map_or(0.0, |r| r.size.height.0);
                self.scroll_body(if event.vk == VK_PAGE_UP {
                    -height
                } else {
                    height
                });
                return;
            }
            VK_TAB if event.modifiers.shift => -1,
            VK_TAB | VK_RIGHT | VK_DOWN => 1,
            VK_LEFT | VK_UP => -1,
            VK_RETURN | VK_SPACE => {
                if let Some(i) = self.focus.get() {
                    self.answer(i as i32);
                }
                return;
            }
            VK_ESCAPE => return self.escape(),
            _ => return,
        };
        self.set_focus(step_focus(&order, self.focus.get(), step));
    }

    fn set_focus(&self, index: Option<usize>) {
        let changed = self.focus.replace(index) != index;
        self.keyboard.set(true);
        self.invalidate();
        if changed
            && self.recovered.is_some()
            && let Some(surface) = Surface::by_key(self.key)
        {
            crate::access::recovery_focus_changed(&surface, self.window);
        }
    }
}

impl Hosted for DialogItem {
    fn draw(&self, cx: &mut DrawCx<'_>, size: (f32, f32)) {
        use metrics::*;
        let opacity = self.opacity.get();
        if opacity <= 0.0 {
            return;
        }
        let g = self.geometry(size);
        let contrast = self
            .recovered
            .as_ref()
            .and_then(|draft| draft.appearance.get().contrast);
        let mut p = Palette::new(cx.dark);
        let mut glass = false;
        let radius = if let Some(recovered) = &self.recovered {
            if let [card] = g.cards.as_slice() {
                glass = recovered.frozen.backdrop(cx, size, *card);
            }
            p.recovered(cx.dark, contrast, glass);
            RECOVERED_RADIUS
        } else {
            RADIUS
        };
        let offset = self.recovered.as_ref().map_or(0.0, |recovered| {
            let offset = recovered.body_offset.get().min(g.body_travel());
            recovered.body_offset.set(offset);
            offset
        });
        let border_width = if self.recovered.is_some() && !glass {
            1.5
        } else {
            1.0
        };
        if opacity < 1.0 {
            cx.push_opacity(opacity);
        }
        // SAFETY: a COM call between BeginDraw and EndDraw.
        unsafe {
            cx.dc
                .FillRectangle(&rect_f(0.0, 0.0, size.0, size.1), cx.solid(p.scrim))
        };
        // Opening and closing scale the cards a little around their middle.
        let world = cx.transform;
        if opacity < 1.0
            && let Some(first) = g.cards.first()
        {
            let last = g.cards.last().unwrap_or(first);
            let (mx, my) = (
                first.origin.x.0 + first.size.width.0 / 2.0,
                (first.origin.y.0 + last.origin.y.0 + last.size.height.0) / 2.0,
            );
            let s = 1.0 + 0.04 * (1.0 - opacity);
            cx.set_transform(
                Mat4::new()
                    .translate(mx, my)
                    .scale(s, s)
                    .translate(-mx, -my)
                    .then(&world),
            );
        }
        for card in &g.cards {
            if contrast.is_none() {
                draw_shadow(cx, *card, radius, 8.0, if cx.dark { 0.6 } else { 0.25 });
            }
            if glass && let Some(recovered) = &self.recovered {
                recovered.frozen.card(cx, *card, radius);
            }
            fill_rounded(cx, *card, radius, p.card);
        }
        if let (Some(footer), Some(card)) = (g.footer, g.cards.first()) {
            // The footer's band, its bottom corners the card's.
            cx.push_clip_rounded_shapes(*card, [radius; 4]);
            // SAFETY: a COM call between BeginDraw and EndDraw.
            unsafe {
                cx.dc.FillRectangle(
                    &rect_f(
                        footer.origin.x.0,
                        footer.origin.y.0,
                        footer.size.width.0,
                        footer.size.height.0,
                    ),
                    cx.solid(p.footer),
                )
            };
            cx.pop();
        }
        for card in &g.cards {
            stroke_rounded(cx, *card, radius, p.stroke, border_width);
        }
        if let Some(sep) = g.separator {
            // SAFETY: a COM call between BeginDraw and EndDraw.
            unsafe {
                cx.dc.FillRectangle(
                    &rect_f(
                        sep.origin.x.0,
                        sep.origin.y.0,
                        sep.size.width.0,
                        sep.size.height.0,
                    ),
                    cx.solid(p.separator),
                )
            };
        }
        if let Some(clip) = g.body_viewport {
            cx.push_clip_rect(
                clip.origin.x.0,
                clip.origin.y.0,
                clip.size.width.0,
                clip.size.height.0,
            );
        }
        let t = &self.texts;
        if let (Some(label), Some(r)) = (&t.badge, g.badge) {
            let (fill, fg) = match contrast {
                Some(contrast) => (contrast.window, contrast.text),
                None if cx.dark => (Color(0xf5b1_8c26), Color(0xffc4_9aff)),
                None => (Color(0xde76_4521), Color(0x8c43_1bff)),
            };
            let r = rect(
                r.origin.x.0,
                r.origin.y.0 - offset,
                r.size.width.0,
                r.size.height.0,
            );
            let pill_radius = (r.size.height.0 * 0.5).min(12.0);
            fill_rounded(cx, r, pill_radius, fill);
            if contrast.is_some() {
                stroke_rounded(cx, r, pill_radius, fg, 1.5);
            }
            label.draw(
                cx,
                r.origin.x.0 + BADGE_PAD_X,
                r.origin.y.0 + BADGE_PAD_Y,
                fg,
            );
        }
        if let (Some(l), Some(r)) = (&t.title, g.title) {
            l.draw(
                cx,
                r.origin.x.0,
                r.origin.y.0 - offset,
                if self.sheet { p.secondary } else { p.text },
            );
        }
        if let (Some(l), Some(r)) = (&t.message, g.message) {
            l.draw(
                cx,
                r.origin.x.0,
                r.origin.y.0 - offset,
                if self.sheet { p.secondary } else { p.text },
            );
        }
        if let (Some(label), Some(r)) = (&t.summary, g.summary) {
            let paper = match contrast {
                Some(contrast) => contrast.window,
                None if cx.dark => Color(0x1619_23ff),
                None => Color(0xfcfc_feff),
            };
            let r = rect(
                r.origin.x.0,
                r.origin.y.0 - offset,
                r.size.width.0,
                r.size.height.0,
            );
            fill_rounded(cx, r, SUMMARY_RADIUS, paper);
            stroke_rounded(cx, r, SUMMARY_RADIUS, p.stroke, border_width);
            let x = r.origin.x.0 + SUMMARY_PAD_X;
            let y = r.origin.y.0 + SUMMARY_PAD_Y;
            cx.push_clip_rect(
                x,
                y,
                (r.size.width.0 - 2.0 * SUMMARY_PAD_X).max(0.0),
                r.size.height.0 - 2.0 * SUMMARY_PAD_Y,
            );
            label.draw(cx, x, y, p.text);
            cx.pop();
        }
        if let Some(clip) = g.body_viewport {
            cx.pop();
            let viewport = clip.size.height.0;
            let travel = g.body_travel();
            if viewport > 0.0 && travel > 0.0 {
                let thumb = (viewport * viewport / (viewport + travel)).clamp(0.0, viewport);
                let y = clip.origin.y.0 + (viewport - thumb) * offset / travel;
                fill_rounded(
                    cx,
                    rect(clip.origin.x.0 + clip.size.width.0 + 6.0, y, 3.0, thumb),
                    1.5,
                    p.secondary,
                );
            }
        }
        let hover = self.hover.get();
        let pressed = match self.pressed.get() {
            Some(Target::Button(i)) => Some(i),
            _ => None,
        };
        for (i, (label, r)) in t.buttons.iter().zip(&g.buttons).enumerate() {
            let style = self.styles[i];
            let (is_hover, is_pressed) = (hover == Some(i), pressed == Some(i) && hover == Some(i));
            let text = if self.sheet {
                if is_hover || is_pressed {
                    let a = if is_pressed { 2.0 } else { 1.0 };
                    let c = p.row_hover;
                    fill_rounded(
                        cx,
                        *r,
                        BUTTON_RADIUS,
                        with_alpha(c, (c.0 & 0xff) as f32 / 255.0 * a),
                    );
                }
                match style {
                    DialogButtonStyle::Destructive => p.critical,
                    _ => p.text,
                }
            } else if let Some(contrast) = contrast {
                let (fill, text) =
                    contrast.button(Some(i) == self.default || is_hover || is_pressed);
                fill_rounded(cx, *r, BUTTON_RADIUS, fill);
                stroke_rounded(cx, *r, BUTTON_RADIUS, p.stroke, 1.5);
                text
            } else {
                let accent = Some(i) == self.default;
                let (fill, text) = match style {
                    // Origami danger-soft/error-fg, respecting the window's scheme.
                    DialogButtonStyle::Destructive if self.recovered.is_some() => {
                        if cx.dark {
                            (Color(0xf26d_7829), Color(0xff9c_a6ff))
                        } else {
                            (Color(0xd645_501c), Color(0xa32c_3aff))
                        }
                    }
                    DialogButtonStyle::Destructive => (p.critical, Color(0xffff_ffff)),
                    _ if accent => (p.accent, p.accent_text),
                    _ => (p.button, p.text),
                };
                let filled = accent || style == DialogButtonStyle::Destructive;
                let fill = match (filled, is_pressed, is_hover) {
                    (true, true, _) => mix(fill, p.card, 0.2),
                    (true, false, true) => mix(fill, p.card, 0.1),
                    (false, true, _) => p.button_pressed,
                    (false, false, true) => p.button_hover,
                    _ => fill,
                };
                fill_rounded(cx, *r, BUTTON_RADIUS, fill);
                if !filled {
                    stroke_rounded(cx, *r, BUTTON_RADIUS, p.button_stroke, 1.0);
                }
                if is_pressed && !filled {
                    p.secondary
                } else {
                    text
                }
            };
            let pad = if self.sheet {
                SHEET_TEXT_PAD
            } else {
                BUTTON_PAD
            };
            label.draw(cx, r.origin.x.0 + pad, r.origin.y.0, text);
            if self.keyboard.get() && self.focus.get() == Some(i) {
                let ring = rect(
                    r.origin.x.0 - 3.0,
                    r.origin.y.0 - 3.0,
                    r.size.width.0 + 6.0,
                    r.size.height.0 + 6.0,
                );
                stroke_rounded(cx, ring, BUTTON_RADIUS + 3.0, p.focus, 2.0);
            }
        }
        cx.set_transform(world);
        if opacity < 1.0 {
            cx.pop();
        }
    }

    fn hit(&self, _x: f32, _y: f32, _size: (f32, f32)) -> bool {
        // Keep recovery input within this dialog until its overlay is removed.
        self.takes_input()
    }

    fn pointer(&self, event: &ItemPointer) -> bool {
        if self.closing.get() {
            return true;
        }
        let target = self.target(event.x, event.y);
        let over = match target {
            Target::Button(i) => Some(i),
            _ => None,
        };
        if self.hover.replace(over) != over {
            self.invalidate();
        }
        match event.kind {
            PointerKind::Down => {
                self.pressed.set(Some(target));
                // Pointer input hides the keyboard focus ring.
                if self.keyboard.replace(false) {
                    self.invalidate();
                }
                if over.is_some() {
                    self.invalidate();
                }
            }
            PointerKind::Up => {
                let pressed = self.pressed.take();
                match (pressed, target) {
                    (Some(Target::Button(a)), Target::Button(b)) if a == b => self.answer(a as i32),
                    // An outside press never closes a recovery choice.
                    (Some(Target::Outside), Target::Outside)
                        if self.cancelable && self.recovered.is_none() =>
                    {
                        self.answer(-1)
                    }
                    _ => {}
                }
                self.invalidate();
            }
            PointerKind::Cancel | PointerKind::Leave => {
                self.pressed.set(None);
                self.hover.set(None);
                self.invalidate();
            }
            PointerKind::Move => {}
        }
        true
    }

    fn cursor(&self, _x: f32, _y: f32) -> Option<Cursor> {
        Some(Cursor::Arrow)
    }
}

/// Shows `dialog` over the surface: a card in the middle (an alert) or at
/// the bottom (an action sheet) over a dimmed backdrop. Answers the
/// button's index; -1 when dismissed (a click outside or Escape, when
/// cancelable); Escape presses the cancel button when there is one.
pub(crate) fn show_dialog(surface: &Rc<Surface>, window: NodeId, dialog: &Dialog) {
    show_dialog_item(surface, window, dialog, None);
}

fn show_dialog_item(
    surface: &Rc<Surface>,
    window: NodeId,
    dialog: &Dialog,
    recovery: Option<(
        &RecoveredDraftRequest,
        oneshot::Sender<RecoveredDraftChoice>,
        bool,
    )>,
) -> Option<u64> {
    // `AlertDialog` always brings the app's actions (it rejects none): no
    // label of NSH's own.
    let buttons = &dialog.buttons[..];
    let styles: Vec<DialogButtonStyle> = buttons.iter().map(|b| b.style).collect();
    let cancel = styles.iter().position(|s| *s == DialogButtonStyle::Cancel);
    let fs = surface.environment.get().font_scale;
    let sheet = dialog.sheet;
    let (title_size, title_weight, text_align) = if sheet {
        (13.0, 400, DWRITE_TEXT_ALIGNMENT_CENTER)
    } else {
        (20.0, 600, DWRITE_TEXT_ALIGNMENT_LEADING)
    };
    let details = recovery.as_ref().map(|(request, _, _)| &request.details);
    let texts = DialogTexts {
        badge: details
            .and_then(|details| details.badge.as_deref())
            .and_then(|text| Label::new(text, 12.0 * fs, 600, text_align, true)),
        title: dialog
            .title
            .as_deref()
            .and_then(|t| Label::new(t, title_size * fs, title_weight, text_align, true)),
        message: dialog.message.as_deref().and_then(|t| {
            Label::new(
                t,
                if sheet { 13.0 } else { 14.0 } * fs,
                400,
                text_align,
                true,
            )
        }),
        summary: details
            .and_then(|details| details.summary.as_deref())
            .and_then(|text| Label::new(text, 13.0 * fs, 400, text_align, false))
            .map(Label::ellipsized),
        buttons: buttons
            .iter()
            .filter_map(|b| {
                let weight = if sheet && b.style == DialogButtonStyle::Cancel {
                    600
                } else {
                    400
                };
                Label::new(
                    &b.text,
                    14.0 * fs,
                    weight,
                    DWRITE_TEXT_ALIGNMENT_CENTER,
                    false,
                )
            })
            .collect(),
    };
    if texts.buttons.len() != buttons.len() {
        log::error!("windows: cannot lay out the dialog's text");
        if recovery.is_none() {
            surface.event(ROOT_ID, EventKind::Dialog, dialog_answer(dialog.id, -1));
        }
        return None;
    }
    let default = default_button(&styles, sheet);
    let (default, focus, cancel) = if let Some((recovery, _, _)) = &recovery {
        (
            recovery.save_as,
            if recovery.resume_save_as {
                recovery.save_as.or(Some(recovery.keep))
            } else {
                Some(recovery.keep)
            },
            Some(recovery.keep),
        )
    } else {
        (default, default, cancel)
    };
    let serial = next_serial();
    let keyboard = recovery.is_some();
    let (recovered, reply) = match recovery {
        Some((request, sender, capture)) => (
            Some(RecoveredDraftStyle::new(serial, capture, request)),
            DialogReply::Recovered {
                actions: request.actions.clone(),
                sender: RefCell::new(Some(sender)),
            },
        ),
        None => (None, DialogReply::Core(dialog.id)),
    };
    let item = Rc::new(DialogItem {
        recovered,
        window,
        key: surface.key(),
        ui: surface.ui_thread(),
        serial,
        reply,
        sheet,
        cancelable: dialog.cancelable,
        cancel,
        default: if sheet { None } else { default },
        styles,
        texts,
        layout: RefCell::new(None),
        hover: Cell::new(None),
        pressed: Cell::new(None),
        focus: Cell::new(focus),
        keyboard: Cell::new(keyboard),
        opacity: Cell::new(0.0),
        closing: Cell::new(false),
        labels: buttons.iter().map(|b| b.text.clone()).collect(),
    });
    add_overlay(surface, serial, OverlayKind::Dialog(item));
    Some(serial)
}

/// Where a user presses to answer the newest dialog on `surface` (root
/// points): the middle of its button titled `button`, or outside its card
/// to close it without one (`None`; only a dialog that can be closed so).
#[cfg(feature = "dev")]
pub(crate) fn dialog_point(
    surface: &Rc<Surface>,
    button: Option<&str>,
) -> Result<(f32, f32), String> {
    let dialog = OVERLAYS
        .with(|o| {
            o.borrow().iter().rev().find_map(|o| match &o.kind {
                OverlayKind::Dialog(d) if o.key == surface.key() && !d.closing.get() => {
                    Some(d.clone())
                }
                _ => None,
            })
        })
        .ok_or("no dialog is shown")?;
    let layout = dialog.layout.borrow();
    let layout = layout.as_ref().ok_or("the dialog is not drawn yet")?;
    match button {
        Some(text) => {
            let index = dialog
                .labels
                .iter()
                .position(|l| l == text)
                .ok_or_else(|| format!("the dialog has no button {text:?}"))?;
            let r = layout
                .geometry
                .buttons
                .get(index)
                .ok_or("the dialog's button is not laid out")?;
            Ok((
                r.origin.x.0 + r.size.width.0 / 2.0,
                r.origin.y.0 + r.size.height.0 / 2.0,
            ))
        }
        None if dialog.cancelable => Ok((1.0, 1.0)),
        None => Err("the dialog can only be answered with a button".into()),
    }
}

/// Whether a dialog shows over the app: it owns the keyboard, so a view
/// under it hears no key releases either.
pub(crate) fn dialog_shown(surface: &Surface) -> bool {
    // The dialog is modal over the window that showed it, not the app:
    // another window's keyboard is its own.
    let window = surface.input_root().id();
    OVERLAYS.with(|overlays| {
        overlays.borrow().iter().any(|overlay| {
            overlay.key == surface.key()
                && matches!(&overlay.kind, OverlayKind::Dialog(d)
                    if d.window == window && d.takes_input())
        })
    })
}

/// Keys while a dialog shows over the app (it is modal: every key goes to
/// it). Enter and Space press the focused button (the default action at
/// first), Escape cancels, Tab and the arrows move between the buttons.
/// True when handled.
pub(crate) fn key(surface: &Rc<Surface>, event: &KeyEvent) -> bool {
    // The key belongs to the window that received it: a dialog in another
    // window does not hear it.
    match dialog_in_window(surface, surface.input_root().id(), DialogItem::takes_input) {
        Some(dialog) => {
            dialog.key(event);
            true
        }
        None => false,
    }
}

pub(crate) fn recovery_wheel(surface: &Surface, event: &WheelEvent) -> bool {
    let Some(dialog) =
        dialog_in_window(surface, surface.input_root().id(), DialogItem::takes_input)
            .filter(|dialog| dialog.recovered.is_some())
    else {
        return false;
    };
    dialog.scroll_body(event.dy);
    true
}

/// A recovery overlay shields its workbench through teardown, including
/// when an ordinary prompt temporarily takes the top position.
pub(crate) fn recovery_blocks_background(surface: &Surface, window: NodeId) -> bool {
    dialog_in_window(surface, window, |d| d.recovered.is_some()).is_some()
}

/// The current, same-window recovery overlay. Closing keeps the background
/// shielded until the overlay is actually removed.
pub(crate) fn recovery_accessibility(
    surface: &Surface,
    window: NodeId,
) -> Option<RecoveryAccessibility> {
    let dialog = dialog_in_window(surface, window, DialogItem::takes_input)?;
    dialog.recovered.as_ref()?;
    Some(RecoveryAccessibility(dialog))
}

#[cfg(test)]
pub(crate) fn show_recovery_for_access_test(
    surface: &Rc<Surface>,
    window: NodeId,
    resume_save_as: bool,
    details: RecoveredDraftDetails,
) -> (u64, oneshot::Receiver<RecoveredDraftChoice>) {
    let options = nsh_base::alert_dialog()
        .title("Recovered draft")
        .description("A new file can keep this draft.")
        .cancel("continue", "Keep editing")
        .destructive("discard", "Discard")
        .action("save-as", "Save As");
    let mut request = RecoveredDraftRequest::new(options, resume_save_as).unwrap();
    request.details = details;
    let (sender, reply) = oneshot::channel();
    let serial = show_dialog_item(
        surface,
        window,
        &request.dialog,
        Some((&request, sender, false)),
    )
    .expect("recovery dialog");
    (serial, reply)
}

/// An ephemeral view of the active card, backed by its actual drawing,
/// focus and button handlers. UIA retains IDs and resolves this afresh.
pub(crate) struct RecoveryAccessibility(Rc<DialogItem>);

impl RecoveryAccessibility {
    pub(crate) fn serial(&self) -> u64 {
        self.0.serial
    }

    pub(crate) fn labels(&self) -> &[String] {
        &self.0.labels
    }

    pub(crate) fn choice(&self, index: usize) -> Option<RecoveredDraftChoice> {
        let DialogReply::Recovered { actions, .. } = &self.0.reply else {
            return None;
        };
        actions.get(index).copied()
    }

    pub(crate) fn title(&self) -> Option<&str> {
        self.0.recovered.as_ref()?.title.as_deref()
    }

    pub(crate) fn description(&self) -> Option<&str> {
        self.0.recovered.as_ref()?.description.as_deref()
    }

    pub(crate) fn focus(&self) -> Option<usize> {
        self.0.focus.get()
    }

    pub(crate) fn enabled(&self) -> bool {
        !self.0.closing.get()
    }

    pub(crate) fn bounds(&self, size: (f32, f32), button: Option<usize>) -> Option<Bounds<Points>> {
        let layout = self.0.geometry(size);
        match button {
            Some(i) => layout.buttons.get(i).copied(),
            None => layout.cards.first().copied(),
        }
    }

    /// Whether the point hit the card, and which button (if any).
    pub(crate) fn at_point(&self, size: (f32, f32), x: f32, y: f32) -> Option<Option<usize>> {
        if x < 0.0 || y < 0.0 || x >= size.0 || y >= size.1 {
            return None;
        }
        self.0.geometry(size);
        match self.0.target(x, y) {
            Target::Button(index) => Some(Some(index)),
            Target::Card => Some(None),
            Target::Outside => None,
        }
    }

    pub(crate) fn focus_button(&self, index: usize) -> bool {
        if !self.enabled() || index >= self.labels().len() {
            return false;
        }
        self.0.set_focus(Some(index));
        true
    }

    pub(crate) fn invoke(&self, index: usize) -> bool {
        if !self.enabled() || index >= self.labels().len() {
            return false;
        }
        self.0.answer(index as i32);
        true
    }
}

fn dialog_in_window(
    surface: &Surface,
    window: NodeId,
    accept: impl Fn(&DialogItem) -> bool,
) -> Option<Rc<DialogItem>> {
    OVERLAYS.with(|overlays| {
        overlays
            .borrow()
            .iter()
            .rev()
            .find_map(|overlay| match &overlay.kind {
                OverlayKind::Dialog(d)
                    if overlay.key == surface.key() && d.window == window && accept(d) =>
                {
                    Some(d.clone())
                }
                _ => None,
            })
    })
}

// ---------------------------------------------------------------------------
// The clipboard

/// Puts `text` on the clipboard (Unicode text).
pub(crate) fn set_clipboard_text(text: &str) -> bool {
    use windows::Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL},
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
            Ole::CF_UNICODETEXT,
        },
    };
    let units: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    // SAFETY: the clipboard is opened and closed here; the global memory
    // holds `units` and is owned by the clipboard once `SetClipboardData`
    // succeeds (freed here otherwise).
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let mut ok = false;
        if EmptyClipboard().is_ok()
            && let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2)
        {
            let ptr = GlobalLock(memory) as *mut u16;
            if !ptr.is_null() {
                std::ptr::copy_nonoverlapping(units.as_ptr(), ptr, units.len());
                let _ = GlobalUnlock(memory);
                ok = SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(memory.0))).is_ok();
            }
            if !ok {
                let _ = GlobalFree(Some(HGLOBAL(memory.0)));
            }
        }
        let _ = CloseClipboard();
        ok
    }
}

/// The clipboard's text, if it has some.
pub(crate) fn clipboard_text() -> Option<String> {
    use windows::Win32::{
        Foundation::HGLOBAL,
        System::{
            DataExchange::{
                CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
            },
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Ole::CF_UNICODETEXT,
        },
    };
    // SAFETY: the clipboard is opened and closed here; the data is read
    // while locked and copied out.
    unsafe {
        IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT.0)).ok()?;
        OpenClipboard(None).ok()?;
        let text = GetClipboardData(u32::from(CF_UNICODETEXT.0))
            .ok()
            .and_then(|h| {
                let memory = HGLOBAL(h.0);
                let ptr = GlobalLock(memory) as *const u16;
                if ptr.is_null() {
                    return None;
                }
                let max = GlobalSize(memory) / 2;
                let units = std::slice::from_raw_parts(ptr, max);
                let len = units.iter().position(|u| *u == 0).unwrap_or(max);
                let text = String::from_utf16_lossy(&units[..len]);
                let _ = GlobalUnlock(memory);
                Some(text)
            });
        let _ = CloseClipboard();
        text
    }
}

/// The clipboard format browsers, Office and the Snipping Tool put PNG
/// images in.
fn png_format() -> u32 {
    use windows::{Win32::System::DataExchange::RegisterClipboardFormatW, core::w};
    // SAFETY: a plain registration of a constant name.
    unsafe { RegisterClipboardFormatW(w!("PNG")) }
}

/// The bytes of clipboard `format`, if it holds that format.
fn clipboard_bytes(format: u32) -> Option<Vec<u8>> {
    use windows::Win32::{
        Foundation::HGLOBAL,
        System::{
            DataExchange::{
                CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
            },
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        },
    };
    // SAFETY: the clipboard is opened and closed here; the data is read
    // while locked and copied out.
    unsafe {
        IsClipboardFormatAvailable(format).ok()?;
        OpenClipboard(None).ok()?;
        let bytes = GetClipboardData(format).ok().and_then(|h| {
            let memory = HGLOBAL(h.0);
            let ptr = GlobalLock(memory) as *const u8;
            if ptr.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(ptr, GlobalSize(memory)).to_vec();
            let _ = GlobalUnlock(memory);
            Some(bytes)
        });
        let _ = CloseClipboard();
        bytes
    }
}

/// Windows' `HTML Format` (CF_HTML).
fn html_format() -> u32 {
    use windows::{Win32::System::DataExchange::RegisterClipboardFormatW, core::w};
    // SAFETY: a plain registration of a constant name.
    unsafe { RegisterClipboardFormatW(w!("HTML Format")) }
}

/// Puts `bytes` on the clipboard as `format`, replacing what it holds.
fn set_clipboard_bytes(format: u32, bytes: &[u8]) -> bool {
    set_clipboard_formats(&[(format, bytes)])
}

/// Puts each of `formats`' bytes on the clipboard as its format, together,
/// replacing what it holds: the same content in several formats.
fn set_clipboard_formats(formats: &[(u32, &[u8])]) -> bool {
    use windows::Win32::{
        Foundation::{GlobalFree, HANDLE, HGLOBAL},
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
            Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
        },
    };
    // SAFETY: as in `set_clipboard_text`: each format's memory is the
    // clipboard's once `SetClipboardData` succeeds, freed here otherwise.
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let mut ok = EmptyClipboard().is_ok();
        for &(format, bytes) in formats {
            if !ok {
                break;
            }
            ok = false;
            if let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) {
                let ptr = GlobalLock(memory) as *mut u8;
                if !ptr.is_null() {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
                    let _ = GlobalUnlock(memory);
                    ok = SetClipboardData(format, Some(HANDLE(memory.0))).is_ok();
                }
                if !ok {
                    let _ = GlobalFree(Some(HGLOBAL(memory.0)));
                }
            }
        }
        let _ = CloseClipboard();
        ok
    }
}

/// True when the clipboard has an image (a PNG or a bitmap).
pub(crate) fn clipboard_has_image() -> bool {
    use windows::Win32::System::{DataExchange::IsClipboardFormatAvailable, Ole::CF_DIB};
    // SAFETY: plain queries.
    unsafe {
        IsClipboardFormatAvailable(png_format()).is_ok()
            || IsClipboardFormatAvailable(u32::from(CF_DIB.0)).is_ok()
    }
}

/// The clipboard's image: its PNG, else its bitmap as a BMP file.
pub(crate) fn clipboard_image() -> Option<Vec<u8>> {
    use windows::Win32::System::Ole::CF_DIB;
    clipboard_bytes(png_format())
        .or_else(|| clipboard_bytes(u32::from(CF_DIB.0)).and_then(|dib| bmp_file(&dib)))
}

/// A BMP file of a packed device-independent bitmap (`CF_DIB`): the file
/// header, which says where the pixels start, before it.
fn bmp_file(dib: &[u8]) -> Option<Vec<u8>> {
    let u32_at = |i: usize| Some(u32::from_le_bytes(dib.get(i..i + 4)?.try_into().ok()?));
    let header = u32_at(0)? as usize;
    let bit_count = u32::from(u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?));
    let compression = u32_at(16)?;
    let colors_used = u32_at(32)? as usize;
    // BI_BITFIELDS after a BITMAPINFOHEADER: three masks follow it.
    let masks = if header == 40 && compression == 3 {
        12
    } else {
        0
    };
    let palette = match colors_used {
        0 if bit_count <= 8 => (1usize << bit_count) * 4,
        n => n * 4,
    };
    let offset = 14 + header + masks + palette;
    let size = 14 + dib.len();
    let mut file = Vec::with_capacity(size);
    file.extend_from_slice(b"BM");
    file.extend_from_slice(&u32::try_from(size).ok()?.to_le_bytes());
    file.extend_from_slice(&[0; 4]);
    file.extend_from_slice(&u32::try_from(offset).ok()?.to_le_bytes());
    file.extend_from_slice(dib);
    Some(file)
}

/// The `file:` URLs of the files on the clipboard (`CF_HDROP`: files
/// copied in Explorer).
pub(crate) fn clipboard_files() -> Vec<String> {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};

    use windows::Win32::{
        System::{
            DataExchange::{
                CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
            },
            Ole::CF_HDROP,
        },
        UI::Shell::{DragQueryFileW, HDROP},
    };
    let mut files = Vec::new();
    // SAFETY: the clipboard is opened and closed here; the HDROP is the
    // clipboard's and only read while it is open.
    unsafe {
        if IsClipboardFormatAvailable(u32::from(CF_HDROP.0)).is_err()
            || OpenClipboard(None).is_err()
        {
            return files;
        }
        if let Ok(handle) = GetClipboardData(u32::from(CF_HDROP.0)) {
            let drop = HDROP(handle.0);
            for index in 0..DragQueryFileW(drop, u32::MAX, None) {
                let length = DragQueryFileW(drop, index, None);
                let mut name = vec![0; length as usize + 1];
                let copied = DragQueryFileW(drop, index, Some(&mut name));
                if copied == 0 || copied != length {
                    continue;
                }
                let path = PathBuf::from(OsString::from_wide(&name[..copied as usize]));
                if path.is_absolute() {
                    files.push(clipboard::file_url(&path.to_string_lossy()));
                }
            }
        }
        let _ = CloseClipboard();
    }
    files
}

/// True when the clipboard has text.
pub(crate) fn clipboard_has_text() -> bool {
    use windows::Win32::System::{DataExchange::IsClipboardFormatAvailable, Ole::CF_UNICODETEXT};
    // SAFETY: a plain query.
    unsafe { IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT.0)) }.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ending_session_puts_the_app_in_the_background() {
        assert_eq!(lifecycle_of(true, true, false, || true), Lifecycle::Active);
        assert_eq!(
            lifecycle_of(true, true, false, || false),
            Lifecycle::Inactive
        );
        assert_eq!(
            lifecycle_of(false, true, false, || true),
            Lifecycle::Background
        );
        assert_eq!(
            lifecycle_of(true, true, true, || true),
            Lifecycle::Background
        );
        assert_eq!(
            lifecycle_of(true, true, true, || false),
            Lifecycle::Background
        );
    }

    #[test]
    fn a_screen_the_user_cannot_see_puts_the_app_in_the_background() {
        // Locked, disconnected or turned off: active or not, nothing shows.
        assert_eq!(
            lifecycle_of(true, false, false, || true),
            Lifecycle::Background
        );
        assert_eq!(
            lifecycle_of(true, false, false, || false),
            Lifecycle::Background
        );
        let locked = crate::presence::Screen::default()
            .session_changed(windows::Win32::UI::WindowsAndMessaging::WTS_SESSION_LOCK);
        assert_eq!(
            lifecycle_of(true, locked.seen(), false, || true),
            Lifecycle::Background
        );
    }

    #[test]
    fn bitmaps_become_bmp_files() {
        // A BITMAPINFOHEADER of a 2x1 32-bit bitmap, then its pixels.
        let mut dib = vec![0u8; 40];
        dib[0] = 40;
        dib[4] = 2;
        dib[8] = 1;
        dib[12] = 1;
        dib[14] = 32;
        dib.extend([0xFF; 8]);
        let file = bmp_file(&dib).unwrap();
        assert_eq!(&file[..2], b"BM");
        assert_eq!(u32::from_le_bytes(file[2..6].try_into().unwrap()), 62);
        assert_eq!(u32::from_le_bytes(file[10..14].try_into().unwrap()), 54);
        assert_eq!(clipboard::image_size(&file), Some((2, 1)));
    }

    #[test]
    fn locales() {
        let tag = |name: Option<&str>| user_locale(name).as_str().to_owned();
        assert_eq!(tag(Some("de-DE")), "de-DE");
        assert_eq!(tag(Some("zh-Hans-CN")), "zh-Hans-CN");
        assert_eq!(tag(Some("sr-Latn-RS")), "sr-Latn-RS");
        // A sort order is no part of the tag.
        assert_eq!(tag(Some("de-DE_phoneb")), "de-DE");
        assert_eq!(tag(Some("es-419")), "es-419");
        // The invariant locale, or none.
        assert_eq!(tag(Some("")), "en-US");
        assert_eq!(tag(None), "en-US");
    }

    #[test]
    fn text_scales() {
        assert_eq!(text_scale(None), 1.0);
        assert_eq!(text_scale(Some(100)), 1.0);
        assert_eq!(text_scale(Some(150)), 1.5);
        assert_eq!(text_scale(Some(225)), 2.25);
        assert_eq!(text_scale(Some(0)), 1.0);
        assert_eq!(text_scale(Some(1000)), 1.0);
    }

    #[test]
    fn reading_layouts() {
        assert!(reading_layout_rtl("1"));
        assert!(!reading_layout_rtl("0"));
        assert!(!reading_layout_rtl("2"));
        assert!(!reading_layout_rtl(""));
    }

    #[test]
    fn accent_palette() {
        let mut palette = [0u8; 32];
        palette[4..8].copy_from_slice(&[0x99, 0xeb, 0xff, 0]);
        palette[16..20].copy_from_slice(&[0x00, 0x5a, 0x9e, 0]);
        let (light, dark) = accent_shades(&palette);
        assert_eq!(light, Color(0x005a_9eff));
        assert_eq!(dark, Color(0x99eb_ffff));
    }

    #[test]
    fn os_version_is_windows_10_or_later() {
        assert!(os_version().0 >= 10);
    }

    /// Text 8 points per character, 20 points per line.
    fn measure(texts: &'static [&'static str]) -> impl Fn(Part, f32) -> (f32, f32) {
        move |part, width| {
            let text = match part {
                Part::Badge => "Recovered draft",
                Part::Title => "Title",
                Part::Message => "A message that is quite long, long enough to wrap twice or more",
                Part::Summary => "A preview of the actual recovered document",
                Part::Button(i) => texts[i],
            };
            let natural = text.len() as f32 * 8.0;
            let lines = (natural / width).ceil().max(1.0);
            (natural.min(width), lines * 20.0)
        }
    }

    #[test]
    fn alert_layout() {
        let g = layout_alert(
            (1000.0, 800.0),
            true,
            true,
            2,
            None,
            &measure(&["Cancel", "OK"]),
        );
        let card = g.cards[0];
        // As wide as the message, up to the maximum, centered.
        assert_eq!(card.size.width.0, metrics::MAX_WIDTH);
        assert_eq!(
            card.origin.x.0,
            ((1000.0 - card.size.width.0) / 2.0).round()
        );
        assert!(g.row);
        // Equal columns in the footer.
        assert_eq!(g.buttons[0].size.width.0, g.buttons[1].size.width.0);
        assert_eq!(g.buttons[0].origin.y.0, g.buttons[1].origin.y.0);
        assert_eq!(
            g.buttons[1].origin.x.0 + g.buttons[1].size.width.0,
            card.origin.x.0 + card.size.width.0 - metrics::PAD
        );
        let footer = g.footer.unwrap();
        assert_eq!(
            footer.origin.y.0 + footer.size.height.0,
            card.origin.y.0 + card.size.height.0
        );
        // The message wraps at the inner width, below the title.
        let message = g.message.unwrap();
        assert_eq!(message.size.width.0, card.size.width.0 - 2.0 * metrics::PAD);
        assert!(message.size.height.0 > 20.0);
        assert_eq!(
            message.origin.y.0,
            g.title.unwrap().origin.y.0 + 20.0 + metrics::TITLE_GAP
        );
    }

    #[test]
    fn alert_buttons_wrap() {
        // More than three buttons stack.
        let g = layout_alert(
            (1000.0, 800.0),
            true,
            false,
            4,
            None,
            &measure(&["A", "B", "C", "D"]),
        );
        assert!(!g.row);
        assert!(
            g.buttons
                .windows(2)
                .all(|b| b[1].origin.y.0 > b[0].origin.y.0 && b[1].origin.x.0 == b[0].origin.x.0)
        );
        // So do labels too long for their column.
        let g = layout_alert(
            (360.0, 800.0),
            true,
            false,
            2,
            None,
            &measure(&["A rather long button label", "OK"]),
        );
        assert!(!g.row);
        // A narrow window: the card fits it, less the gutters.
        assert_eq!(g.cards[0].size.width.0, 360.0 - 2.0 * metrics::GUTTER);
    }

    fn recovery_options() -> AlertDialogOptions {
        nsh_base::alert_dialog()
            .title("Recovered draft")
            .action("save-as", "Save As")
            .destructive("discard", "Discard")
            .cancel("continue", "Keep editing")
            .close_on_escape(true)
    }

    thread_local! {
        static RECOVERY_RESULT: Cell<Option<Option<RecoveredDraftChoice>>> = const { Cell::new(None) };
        static RECOVERY_PICKER_CANCELLED: Cell<Option<bool>> = const { Cell::new(None) };
    }

    #[nsh_core::element::component]
    fn RecoveryBindingFixture() -> impl nsh_core::element::IntoElement {
        use nsh_core::prelude::*;
        let host = element_ref();
        view()
            .child(
                view()
                    .h(px(24.0))
                    .element_ref(&host)
                    .label("recovery trigger")
                    .on_press(move || {
                        let answer = show_recovered_draft_dialog(&host, recovery_options(), true);
                        spawn(async move {
                            RECOVERY_RESULT.set(Some(answer.await.unwrap()));
                        });
                    }),
            )
            .child(
                view()
                    .h(px(24.0))
                    .label("ordinary trigger")
                    .on_press(move || {
                        let _ = recovery_options().show();
                    }),
            )
            .child(
                view()
                    .h(px(24.0))
                    .label("recovery picker trigger")
                    .on_press(move || {
                        let reply = crate::file_requests::pick_recovered_draft_save_path(
                            &host,
                            nsh_core::file_dialog::SaveDialog::default(),
                        );
                        spawn(async move {
                            RECOVERY_PICKER_CANCELLED.set(Some(matches!(reply.await, Ok(None))));
                        });
                    }),
            )
    }

    #[test]
    fn recovery_without_a_declared_window_settles_without_a_core_dialog() {
        use nsh_core::{
            ApproxTextEngine, Core, MountItem,
            element::{Environment as CoreEnvironment, Root},
            geometry::size,
            platform::Os,
        };
        RECOVERY_RESULT.set(None);
        RECOVERY_PICKER_CANCELLED.set(None);
        let viewport = size(px(800.0), px(600.0));
        let mut core = Core::new(ApproxTextEngine, viewport);
        let mut environment = CoreEnvironment::default();
        environment.os = Os::Windows;
        core.set_environment(environment);
        core.mount_root(&Root::new::<RecoveryBindingFixture>(|| ()));
        let mut controls = [ROOT_ID; 3];
        for _ in 0..3 {
            core.run_component_tasks();
            if let Some(batch) = core.commit() {
                for item in batch.items {
                    if let MountItem::ViewProps { id, props } = item {
                        match props.accessibility.label.as_deref() {
                            Some("recovery trigger") => controls[0] = id,
                            Some("ordinary trigger") => controls[1] = id,
                            Some("recovery picker trigger") => controls[2] = id,
                            _ => {}
                        }
                    }
                }
            }
        }
        assert!(controls.into_iter().all(|id| id != ROOT_ID));
        let core_dialog = |item: &MountItem| {
            matches!(
                item,
                MountItem::Command {
                    command: Command::Dialog(_),
                    ..
                }
            )
        };
        assert!(core.component_event(controls[0], EventKind::Press, &EventPayload::None));
        core.run_component_tasks();
        assert!(
            core.commit()
                .is_none_or(|batch| !batch.items.iter().any(core_dialog))
        );
        let tasks = core.take_main_thread_tasks();
        assert_eq!(
            tasks.len(),
            1,
            "the window-bound request uses the platform channel"
        );

        let surface = Surface::new(Environment::default());
        let host = controls[0];
        assert!(recovery_window(&surface, host).is_none());
        for item in [
            MountItem::Create {
                id: host,
                kind: nsh_core::ViewKind::View,
            },
            MountItem::Insert {
                parent: ROOT_ID,
                child: host,
                before: None,
            },
        ] {
            surface.mounter.borrow_mut().apply(&surface, &item);
        }
        let hidden = surface.view(host).expect("mounted test host");
        assert_eq!(crate::app_window::window_of(&surface, &hidden), ROOT_ID);
        assert!(recovery_window(&surface, host).is_none());
        surface.run_engine_task(tasks.into_iter().next().unwrap());
        core.run_component_tasks();
        assert_eq!(RECOVERY_RESULT.get(), Some(None));

        let missing = Surface::new(Environment::default());
        for target in [&surface, &missing] {
            RECOVERY_PICKER_CANCELLED.set(None);
            assert!(core.component_event(controls[2], EventKind::Press, &EventPayload::None));
            core.run_component_tasks();
            assert!(core.commit().is_none_or(|batch| {
                !batch.items.iter().any(|item| {
                    matches!(
                        item,
                        MountItem::Command {
                            command: Command::FileRequest { .. },
                            ..
                        }
                    )
                })
            }));
            let mut picker_tasks = core.take_main_thread_tasks().into_iter();
            target.run_engine_task(picker_tasks.next().expect("the typed picker task"));
            assert!(picker_tasks.next().is_none());
            core.run_component_tasks();
            assert_eq!(RECOVERY_PICKER_CANCELLED.get(), Some(true));
        }

        assert!(core.component_event(controls[1], EventKind::Press, &EventPayload::None));
        core.run_component_tasks();
        let ordinary = core.commit().expect("ordinary alert command");
        assert!(ordinary.items.iter().any(core_dialog));
        assert!(core.take_main_thread_tasks().is_empty());
    }

    #[test]
    fn recovery_reply_is_not_closed_by_a_core_id_or_another_surface() {
        // The test mounts only the overlay registry on an in-memory Surface.
        // A real P3 request cannot open on ROOT_ID.
        let env = Environment {
            reduce_motion: true,
            ..Environment::default()
        };
        let surface = Surface::new(env);
        let other_surface = Surface::new(env);
        let request = RecoveredDraftRequest::new(recovery_options(), true).unwrap();
        let (sender, mut reply) = oneshot::channel();
        let recovery = show_dialog_item(
            &surface,
            ROOT_ID,
            &request.dialog,
            Some((&request, sender, false)),
        )
        .expect("recovery overlay");
        let ordinary = show_dialog_item(&surface, ROOT_ID, &request.dialog, None)
            .expect("core overlay with id zero");
        assert_ne!(recovery, ordinary);
        assert!(find_dialog(&other_surface, recovery).is_none());

        close_dialog(&surface, request.dialog.id);
        assert!(find_dialog(&surface, ordinary).is_none());
        let dialog = find_dialog(&surface, recovery).expect("recovery overlay survives");
        assert_eq!(dialog.focus.get(), Some(2), "return from Save As");
        assert_eq!(reply.try_recv(), Err(oneshot::error::TryRecvError::Empty));

        window_closing(&surface, ROOT_ID);
        assert!(find_dialog(&surface, recovery).is_none());
        assert!(dialog.closing.get());
        assert_eq!(reply.try_recv(), Err(oneshot::error::TryRecvError::Closed));
    }

    #[test]
    fn recovery_request_validates_roles_and_picker_return_focus() {
        let request = RecoveredDraftRequest::new(recovery_options(), true).unwrap();
        assert_eq!(request.keep, 0);
        assert_eq!(request.save_as, Some(2));
        assert!(request.resume_save_as);
        assert_eq!(
            request.actions,
            [
                RecoveredDraftChoice::Continue,
                RecoveredDraftChoice::Discard,
                RecoveredDraftChoice::SaveAs
            ]
        );
        let invalid = nsh_base::alert_dialog()
            .cancel("continue", "Keep editing")
            .action("discard", "Discard");
        assert!(RecoveredDraftRequest::new(invalid, false).is_none());
    }

    #[test]
    fn recovery_card_caps_the_text_and_preserves_all_actions() {
        let g = layout_alert(
            (321.0, 280.0),
            true,
            true,
            3,
            Some(RecoveryParts::default()),
            &|part, width| match part {
                Part::Badge | Part::Summary => unreachable!(),
                Part::Title => (width, 56.0),
                Part::Message => (width, 416.0),
                Part::Button(_) => (130.0, 16.0),
            },
        );
        let bounds = g.cards[0];
        let text = g.body_viewport.expect("body scroll viewport");
        assert_eq!(bounds.origin.y.0, metrics::GUTTER);
        assert_eq!(bounds.size.height.0, 280.0 - 2.0 * metrics::GUTTER);
        assert!(g.body_travel() > 300.0);
        assert_eq!(g.buttons.len(), 3);
        assert!(g.buttons.iter().all(|button| {
            button.origin.y.0 >= text.origin.y.0 + text.size.height.0
                && button.origin.y.0 + button.size.height.0 <= 280.0 - metrics::GUTTER
        }));
    }

    #[test]
    fn recovery_badge_and_summary_scroll_inside_the_pinned_actions() {
        let g = layout_alert(
            (321.0, 280.0),
            true,
            true,
            3,
            Some(RecoveryParts {
                badge: true,
                summary: true,
            }),
            &|part, width| match part {
                Part::Badge => (100.0, 18.0),
                Part::Title => (width, 38.0),
                Part::Message => (width, 125.0),
                Part::Summary => (width, 18.0),
                Part::Button(_) => (130.0, 16.0),
            },
        );
        let badge = g.badge.unwrap();
        let title = g.title.unwrap();
        let message = g.message.unwrap();
        let paper = g.summary.unwrap();
        let body = g.body_viewport.unwrap();
        let bottom = |r: Bounds<Points>| r.origin.y.0 + r.size.height.0;
        assert!(badge.size.width.0 < title.size.width.0);
        assert!(bottom(badge) < title.origin.y.0);
        assert!(bottom(title) < message.origin.y.0);
        assert!(bottom(message) < paper.origin.y.0);
        assert_eq!(paper.size.width, title.size.width);
        assert_eq!(paper.size.height.0, 18.0 + 2.0 * metrics::SUMMARY_PAD_Y);
        assert_eq!(bottom(paper) - bottom(body), g.body_travel());
        assert!(
            g.buttons
                .iter()
                .all(|button| button.origin.y.0 >= bottom(body))
        );
    }

    #[test]
    fn recovery_card_fits_the_available_width() {
        let window = (321.0, 650.0);
        let g = layout_alert(
            window,
            true,
            true,
            3,
            Some(RecoveryParts::default()),
            &measure(&["Keep editing", "Discard", "Save As"]),
        );
        assert_eq!(g.cards[0].size.width.0, 289.0);
        assert!(!g.row);
        assert!(g.buttons.iter().all(|button| {
            button.origin.x.0 >= metrics::GUTTER
                && button.origin.x.0 + button.size.width.0 <= window.0 - metrics::GUTTER
                && button.origin.y.0 + button.size.height.0 < window.1
        }));
    }

    #[test]
    fn sheet_layout() {
        let g = layout_sheet(
            (1000.0, 800.0),
            true,
            false,
            3,
            Some(1),
            &measure(&["Copy", "Cancel", "Delete"]),
        );
        assert_eq!(g.cards.len(), 2);
        let (options, cancel) = (g.cards[0], g.cards[1]);
        assert_eq!(options.size.width.0, metrics::SHEET_MAX_WIDTH);
        assert_eq!(
            cancel.origin.y.0 + cancel.size.height.0,
            800.0 - 2.0 * metrics::BUTTON_GAP
        );
        assert_eq!(
            options.origin.y.0 + options.size.height.0 + metrics::BUTTON_GAP,
            cancel.origin.y.0
        );
        // The options in order in the first card, the cancel button alone.
        assert!(g.buttons[0].origin.y.0 < g.buttons[2].origin.y.0);
        assert!(g.buttons[1].origin.y.0 > cancel.origin.y.0);
        assert!(g.separator.is_some());
    }

    #[test]
    fn dialog_focus() {
        use DialogButtonStyle::*;
        assert_eq!(default_button(&[Cancel, Default, Default], false), Some(2));
        assert_eq!(default_button(&[Default, Destructive], false), Some(0));
        assert_eq!(default_button(&[Cancel, Destructive], false), Some(0));
        assert_eq!(default_button(&[], false), None);
        assert_eq!(focus_order(3, Some(0), true), vec![1, 2, 0]);
        assert_eq!(focus_order(3, Some(0), false), vec![0, 1, 2]);
        let order = [1, 2, 0];
        assert_eq!(step_focus(&order, Some(0), 1), Some(1));
        assert_eq!(step_focus(&order, Some(1), -1), Some(0));
        assert_eq!(step_focus(&order, None, -1), Some(0));
        assert_eq!(step_focus(&order, None, 1), Some(1));
        assert_eq!(step_focus(&[], None, 1), None);
    }

    #[test]
    fn answers() {
        let field = |p: &EventPayload, key: &str| match p {
            EventPayload::Map(fields) => fields
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone()),
            _ => None,
        };
        let a = dialog_answer(7, -1);
        assert_eq!(field(&a, "id"), Some(PayloadValue::Number(7.0)));
        assert_eq!(field(&a, "index"), Some(PayloadValue::Number(-1.0)));
        let c = clipboard_answer(3, true, Some("hi".into()));
        assert_eq!(field(&c, "has"), Some(PayloadValue::Bool(true)));
        assert_eq!(field(&c, "text"), Some(PayloadValue::Str("hi".into())));
        assert_eq!(field(&clipboard_answer(3, true, None), "text"), None);
    }

    #[test]
    fn colors() {
        assert_eq!(with_alpha(Color(0x1020_30ff), 0.0), Color(0x1020_3000));
        assert_eq!(
            mix(Color(0x0000_00ff), Color(0xffff_ffff), 0.5),
            Color(0x8080_80ff)
        );
    }
}
