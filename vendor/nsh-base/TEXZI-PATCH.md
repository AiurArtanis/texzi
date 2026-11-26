# texzi native editor fork

The source baseline is [NSH v0.7.1](https://github.com/longbridge/nsh/tree/f78eace7af23a9c1c72605c7ee46204f57f3e8e6/crates/nsh-base). `UPSTREAM-MANIFEST.json` identifies each original file under `src` and `fonts` by SHA-256. `Cargo.upstream.toml` preserves the upstream manifest. `Cargo.toml` resolves workspace dependencies outside the NSH workspace and declares the MIT terms applicable to the rich-editor ports alongside Apache-2.0. The KaTeX fonts carry their own SIL OFL 1.1 license.

## Source editor

- `editor/external.rs`, `buffer.rs`, `commands.rs`, `controller.rs` and `mod.rs` connect a mounted editor to texzi's versioned document owner. Both editor modes share the owner's history. The view consumes incremental changes and publishes composition transitions without serializing the full rope on each change.
- `editor/hooks.rs` and `view.rs` expose viewport and caret positioning, preserve an anchor during reflow and report the visible logical range when the source minimap subscribes. The source sink sizes itself to the remainder of the caret row so multiline RichEdit can position IME candidates. For large external documents, its additional accessibility value follows the caret with a byte cap. The rows retain their own accessible semantics.
- `editor/languages.rs` exposes the existing language classifier for texzi's word-wrap preference. Focus and native input selection use the v0.7.1 field's own handling.

## Rich editor

- `rich_editor/view/mod.rs` and `controller.rs` give the host its configured application keystrokes. Binding equivalence uses the NSH platform parser, including for v0.7.1's selection-dependent Copy. Plain Escape reaches the host at an empty text caret or a gap only after editor composition, node views, plugins and selections have had their turn. The terminal keeps its native focus behavior.
- `view/session.rs` also notifies the document owner's mode guard of composition transitions. v0.7.1's public view event remains available after the view has finished processing composition.
- `extensions/extension.rs` and `input_rules.rs` let an external history owner preflight the original typed text and its automatic formatting together. Texzi publishes the typing and conversion as separate undo entries. Other NSH editors use the upstream two-callback input rule.
- `view/controller.rs` and `render.rs` expose scroll restoration, visible caret position and caret reveal across mode changes. A distant selection is brought into the upstream block window using the same measured or estimated heights as its spacers. The local sink-width adjustment keeps the Windows candidate window on the caret row.
- The upstream block window uses `view/block_heights.rs` to index measured heights and neighboring gaps. Scroll, resize and selection locate the visible blocks with prefix sums and binary search. Reordering preserves the measurements of surviving blocks. Native scrolling and incremental Markdown use v0.7.1.

## Reproducing checks

From the workspace on Windows x64, run `cargo test --workspace --release --locked --target x86_64-pc-windows-msvc -- --test-threads=1`. Document/source/rich integration, mode switches, focus, Zen Escape, and shortcut arbitration are exercised by texzi's workspace tests. CI builds and packages the same checkout.
