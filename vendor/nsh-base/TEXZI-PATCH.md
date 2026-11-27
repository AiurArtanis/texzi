# texzi native editor fork

The source baseline is [NSH v0.7.1](https://github.com/longbridge/nsh/tree/f78eace7af23a9c1c72605c7ee46204f57f3e8e6/crates/nsh-base). `UPSTREAM-MANIFEST.json` identifies each original file under `src` and `fonts` by SHA-256. `Cargo.upstream.toml` preserves the upstream manifest. `Cargo.toml` resolves workspace dependencies outside the NSH workspace and declares the MIT terms applicable to the rich-editor ports alongside Apache-2.0. The KaTeX fonts carry their own SIL OFL 1.1 license.

## Source editor

- `EditorDecoration::outline_color` adds a pointer-transparent stroke to each displayed row of an external range. The row captures its endpoints during normal grapheme and tab layout. A zero-width range draws a narrow marker at its own position.
- `ExternalDocument::request_history` returns an immediate update for resident history. A disk-backed history owner returns a pending result, then notifies the editor after the restored revision has been published. The buffer keeps the current revision and selection while it waits.
- `editor/external.rs`, `buffer.rs`, `commands.rs`, `controller.rs` and `mod.rs` connect a mounted editor to texzi's versioned document owner. Both editor modes share the owner's history. The view consumes incremental changes and publishes composition transitions without serializing the full rope on each change.
- `editor/hooks.rs` and `view.rs` expose viewport and caret positioning, preserve an anchor during reflow and report the visible logical range when the source minimap subscribes. The source sink sizes itself to the remainder of the caret row so multiline RichEdit can position IME candidates. For large external documents, its additional accessibility value follows the caret with a byte cap. The rows retain their own accessible semantics.
- `editor/languages.rs` exposes the existing language classifier for texzi's word-wrap preference. Focus and native input selection use the v0.7.1 field's own handling.

## Rich editor

- A view-local outline is drawn above the mounted textblock using its own layout and scroll clip. Complete visible lines reuse their measured spans. Partial lines use the native text geometry, including bidi fragments. Sorted zero-width hits draw position markers on the visible lines. Clearing the external decorations also clears these shapes.
- The native view exposes the visible document range and accepts transient, view-local decorations alongside plugin decorations. Texzi uses these for the active pane's rendered-text search. They have no effect on the document, selection or history.

- `rich_editor/view/mod.rs` and `controller.rs` give the host its configured application keystrokes. Binding equivalence uses the NSH platform parser, including for v0.7.1's selection-dependent Copy. Plain Escape reaches the host at an empty text caret or a gap only after editor composition, node views, plugins and selections have had their turn. The terminal keeps its native focus behavior.
- `view/session.rs` also notifies the document owner's mode guard of composition transitions. v0.7.1's public view event remains available after the view has finished processing composition.
- `extensions/extension.rs` and `input_rules.rs` let an external history owner preflight the original typed text and its automatic formatting together. Texzi publishes the typing and conversion as separate undo entries. Other NSH editors use the upstream two-callback input rule.
- `view/controller.rs` and `render.rs` expose scroll restoration, visible caret position and caret reveal across mode changes. A distant selection is brought into the upstream block window using the same measured or estimated heights as its spacers. The local sink-width adjustment keeps the Windows candidate window on the caret row.
- The block window indexes the intrinsic height of each mounted top-level row, including its margins. Its document position is used separately for caret and table geometry. Translating the viewport leaves measured heights and prefix sums stable. Scroll, resize and selection locate rows by binary search. Reordering preserves surviving measurements. Incremental Markdown uses v0.7.1.
- Core rounds the two edges of a row to physical pixels. Within the same layout and content revision, the height estimate keeps its last measurement across a one-pixel difference. Larger changes and the first measurement after invalidation update the index. Hit testing and caret placement use the current layout.
- `view/thumb.rs` holds the displayed and requested rich pages across a Windows indicator drag. A platform mount fence precedes the scroll command. Its native offset receipt releases the previous page and starts the latest queued request. Disjoint pages share one spacer, so intermediate paragraphs stay unmounted.
- Each handoff carries the mounted receiver, grab and request number. Editing, reflow and changes in viewport dimensions replace the receiver. Keyboard navigation and a superseding native gesture discard its outstanding destination.
- Preparing a thumb destination uses its fraction on the current track and the latest height prefix. Measuring shorter blocks keeps a mid-track grab near the middle and includes the complete final viewport at the new scroll limit.
- The windowed block flow has its own intrinsic-height, noninteractive scroll scope. Native content culling can therefore mount both bounded ranges ahead of the outer scroll. The outer scroll owns the scrollbar and input. `TEXZI_TRACE_RICH_THUMB=1` records monotonic request, range, mount and offset events for this editor without recording document text.
- Width, text scale, DPI and rich-style changes invalidate measured heights by generation. Content replacement invalidates its own block. Mounted blocks report fresh geometry, including the grid and clip for a scrollable table. A distant caret waits for its target's layout. Unmounting releases the old positional and text geometry.
- A selection scroll request wakes the caret reveal even when the selection stays at the same document position. This lets Ctrl+Home return to the existing caret after scrolling away.

## Reproducing checks

From the workspace on Windows x64, run `cargo test --workspace --release --locked --target x86_64-pc-windows-msvc -- --test-threads=1`. Document/source/rich integration, mode switches, focus, Zen Escape, and shortcut arbitration are exercised by texzi's workspace tests. CI builds and packages the same checkout.
