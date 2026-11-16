# texzi IME-01 Windows adapter patch

Fixed source: longbridge/nsh224d9a795c7ac9d10d75750de9351adac5549675,
Apache-2.0. UPSTREAM-MANIFEST.json covers 53 original src files by Git blob
SHA256; Cargo.upstream.toml and LICENSE-APACHE preserve the original bytes.
Cargo.toml expands workspace inheritance; README.md is the fixed upstream
repository readme. No NSH version upgrade or new platform is introduced.

Only src/text_input.rs changes behavior: while composing, every native process
turn re-reads the composed range, including selection-only RichEdit notes.
The existing report_composition deduplication remains. No committed ChangeText
is emitted merely because preedit changed; final commit/cancel ordering stays.
This addresses the white-box notification gap consistent with the observed
frozen preedit, subject to new-package real Windows IME validation.

TEXZI_TRACE_IME=1 explicitly enables length/flag diagnostics. Default is off;
no text contents are logged. The same opt-in marker traces editor/render/owner
revision flow in the base/app adapter. It does not replace real screenshots.
The editor sink width is separately corrected in nsh-base/src/editor/view.rs
to avoid the one-pixel multiline input wrapping its native candidate caret.

Backend crate-wide tests are not claimed: the original internal tests refer
to upstream gallery resources outside this vendored source subset. Existing
component tests and actual application/IME regressions are reported separately.
