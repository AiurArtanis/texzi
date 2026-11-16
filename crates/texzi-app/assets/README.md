# Ink Z application icon

User-supplied `texzi-ink-z-developer-pack-v1.zip`, version 1.0 (2026-11-16).
Only `windows/app.ico` is included, unchanged as `ink-z.ico`.

- ZIP SHA256: `E9B459CD561BBAC124A2E700E6366A58D238F094F534F885FC7F068AAF993B57`
- ICO SHA256: `37BE184A1DAAB79DDA729F02D50D0C3F7C6314BF8051ED024B8D6EEBAF531839`
- Frames: 16, 20, 24, 32, 40, 48, 64, 96, 128, 256 pixels.

The supplied pack describes the original as AI-generated artwork and the
packaging as size/format exports only. The user explicitly supplied it for
the application. No separate third-party license was provided in the pack;
this record does not assert trademark clearance or additional rights.

`build.rs` embeds the frames as Win32 RT_ICON resources and RT_GROUP_ICON 1.
The pinned NSH backend loads group 1 as its window class icon, so the native
title bar, taskbar, Alt+Tab and executable use the same embedded identity.
The existing ZIP packaging copies this executable unchanged; no loose icon
file, launch-directory assumption, runtime extraction or new dependency is needed.

This project currently targets Windows x64 MSVC only and has no installer
or separate custom title bar. The Satin Z repository artwork is unrelated.
