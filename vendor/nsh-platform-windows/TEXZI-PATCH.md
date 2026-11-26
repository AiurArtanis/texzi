# texzi Windows adapter

The baseline is [NSH v0.7.1](https://github.com/longbridge/nsh/tree/f78eace7af23a9c1c72605c7ee46204f57f3e8e6/crates/nsh-platform-windows). `UPSTREAM-MANIFEST.json` records SHA-256 for the original source files. The upstream crate manifest and Apache-2.0 license are retained. The workspace resolves this adapter through its pinned NSH dependency graph.

## Input and menus

- `richedit.rs` and `text_input.rs` schedule a deduplicated preedit observation when Windows TSF updates display text or caret geometry during composition. The host callback schedules work. The native field reads the composed span after the callback and publishes changes once. The v0.7.1 end/start ordering, programmatic-edit guard, and transparent glyph compositing remain active.
- `chrome.rs`, `input.rs` and `window.rs` route a chosen native edit role to the focused editor without re-entering the menu accelerator. Bare Alt can open the custom shell menu through its F10 binding. Synthetic events are marked untranslated so that they cannot remove another key's text from the queue. Physical keys use v0.7.1's translation-capture and release routing.
- A pointer press on an explicitly non-focusable trigger inside a semantic menu bar preserves the current field for native edit commands. Buttons and other keyboard stops retain v0.7.1 focus and Tab behavior.
- Set `TEXZI_TRACE_CHORD=1` for key dispatch stages, modifier and composition flags and handled status in this app's window. The trace omits key values, characters, document data and paths. `TEXZI_TRACE_IME=1` enables host-update and composition-length diagnostics. Both are opt-in.

## Presentation and fonts

- `render.rs`, `surface.rs`, and `lib.rs` expose `node_presentation` to the active engine task. It pairs mounted node geometry with the surface's successful swap-chain presentation counter. Texzi uses it to show the bounded first page before promoting a large document to its complete editable model. A return from Present records submission by the app.
- `text.rs` exposes the installed font family selected by the platform's normal DirectWrite family lookup for the settings preview.
- Long rich documents use NSH's v0.7.1 block window and Windows native wheel, thumb and culling paths. The older fork's mount/scroll registry and wheel redirects are no longer used.
