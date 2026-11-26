# texzi native editor fork

Source: longbridge/nsh commit224d9a795c7ac9d10d75750de9351adac5549675,
Apache-2.0 (LICENSE-APACHE). UPSTREAM-MANIFEST.json records the original src/fonts
Git blob SHA256. Cargo.upstream.toml is the original manifest; Cargo.toml expands
workspace inheritance and excludes upstream test-only workspace dependencies.
Only editor/{external,buffer,commands,controller,mod}.rs adds the external owner path.
Unbound upstream Editor retains its existing implementation. This is not an
upstream defect claim or upgrade. The T04 app used feature editor; T05/T06 additionally enables rich-editor.

ExternalBuffer comes from each component context. The owner accepts versioned
source-byte plans and restores its authoritative snapshot/selection. NSH's Rope
is a persistent rendering cache, never the second editable truth; its internal
undo/redo queues stay empty in bound mode. Full onChange serialization is disabled
for bound mode; incremental receipt patches update layout/highlighter state.
Notifications cause complete snapshot refresh when the view missed updates;
no event replay or blindly advancing cursors is needed by this UI adapter.

This fork requires actual Windows/IME/clipboard/selection testing. New code is
not an assertion that all T04, rectangular/multi-caret, IME or rich text passed.

T05/T06 candidate additionally enables rich-editor and changes rich_editor/view/render.rs native sink width from 1px to remaining viewport. Offscreen layout remains active. Rich IME and source/rich integration need their own evidence; the prior source-editor IME PASS is not inherited. Original upstream manifest/source files retained.

Rich-editor credits in fixed upstream name MIT ProseMirror/Tiptap ported code. Derived package license metadata is Apache-2.0 AND MIT; original Cargo.upstream.toml remains unchanged. Official license texts and source blob identities are included in docs/tx00/licenses and docs/t05/LICENSE-SOURCES.json, not invented port revision claims.

T05 source review corrections additionally report transient source composition
through the external owner, and provide an opt-in Rust session composition
listener for the rich mode guard. These metadata hooks do not edit text or
create history. Target projection preparation precedes visible mode publication.

2026-11-18: rich_editor/view/{mod,controller}.rs additionally exposes the opt-in
RichEditorProps.application_shortcuts (default empty Vec<SharedString>). The
focused key-registration effect normalizes valid host declarations with the
same platform-aware Keystroke parser as the editor, and omits equivalent rich
view bindings. The host remains responsible for its engine-keymap/native-menu
commands; no application callback is invoked inside the rich key handler.
Invalid declarations are ignored. An empty list preserves upstream bindings;
all other rich key handling, source ownership, document/history behavior,
dependencies, features and original upstream manifests remain unchanged.
Validation for this change: NOT_RUN (no Cargo, component execution or GUI in
this scoped edit); the host integration and focused RichEditor regression are
validated separately before any product acceptance claim.

### 2026-11-26 RichEditor Escape

`rich_editor/view/{mod,controller}.rs` emits `UnhandledEscape`
for a plain Escape at an empty text caret or gap cursor on non-terminal
platforms. The focused RichEditor still receives the keystroke first. Composition, selected
node views, plugins, and the base keymap have priority. Non-empty selections
retain their existing collapse behavior, and the terminal retains its focus
behavior. Texzi uses the event for Zen only in the active rich document and
group, using the existing priority for modals, editor panels, and format controls.
A composition notification also resets any pending Zen exit.

The focused RichEditor and source Editor cases run as part of `Windows x64
verify`. Run the focused regressions from the workspace on Windows x64 with
the following command.

```powershell
cargo test -p texzi-app --release --locked --target x86_64-pc-windows-msvc zen_ -- --test-threads=1
```

### Settings source language classification

`editor::known_language` exposes the editor's existing language resolution for
paths and aliases. Texzi uses it to apply the code word-wrap preference. The
editor retains its existing rendering language and plain-text fallback.
