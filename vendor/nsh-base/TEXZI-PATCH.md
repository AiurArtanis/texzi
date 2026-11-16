# texzi native editor fork

Source: longbridge/nsh commit224d9a795c7ac9d10d75750de9351adac5549675,
Apache-2.0 (LICENSE-APACHE). UPSTREAM-MANIFEST.json records the original src/fonts
Git blob SHA256. Cargo.upstream.toml is the original manifest; Cargo.toml expands
workspace inheritance and excludes upstream test-only workspace dependencies.
Only editor/{external,buffer,commands,controller,mod}.rs adds the external owner path.
Unbound upstream Editor retains its existing implementation. This is not an
upstream defect claim or upgrade. Default app uses only feature editor.

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
