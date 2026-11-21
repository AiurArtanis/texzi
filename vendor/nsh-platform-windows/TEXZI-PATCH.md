# texzi IME-01 Windows adapter patch

Fixed source: longbridge/nsh224d9a795c7ac9d10d75750de9351adac5549675,
Apache-2.0. UPSTREAM-MANIFEST.json covers 53 original src files by Git blob
SHA256; Cargo.upstream.toml and LICENSE-APACHE preserve the original bytes.
Cargo.toml expands workspace inheritance; README.md is the fixed upstream
repository readme. No NSH version upgrade or new platform is introduced.

Only src/text_input.rs and src/richedit.rs change behavior. The c3f57bb first
attempt still froze at N: no new process turn occurred after I. Preserve that
failed candidate and its native/editor/render length logs.

While composing, display-update notifications and ITextHost invalidation,
view-change or caret callbacks now mark a preedit note and use the existing
deduplicated notes timer. No services are queried inside the host callbacks;
drawing, teardown and non-composition callbacks do not queue these updates.
Each resulting native process turn re-reads the composed range.
The existing report_composition deduplication remains. No committed ChangeText
is emitted merely because preedit changed; final commit/cancel ordering stays.
This addresses the white-box notification gap consistent with the observed
frozen preedit, subject to new-package real Windows IME validation.

TEXZI_TRACE_IME=1 explicitly enables length/flag diagnostics. Default is off;
no text contents are logged. The same opt-in marker traces editor/render/owner
revision flow in the base/app adapter. It does not replace real screenshots.
The editor sink width is separately corrected in nsh-base/src/editor/view.rs
to avoid the one-pixel multiline input wrapping its native candidate caret.
The sink's transparent text plus hidden native caret also suppresses compositing
its native command-list image; offscreen recording/layout and focus stay active.
This prevents the wider low-opacity carrier leaking stale transient glyphs after
an authoritative document Undo, without removing any document/overlay text.

Backend crate-wide tests are not claimed: the original internal tests refer
to upstream gallery resources outside this vendored source subset. Existing
component tests and actual application/IME regressions are reported separately.

## Native application edit-menu relay

The application now supplies a native menu. An edit MenuRole synthesizes the
focused control's Ctrl key. Relaying it through the ordinary input entry used
to match the same native accelerator again before reaching a plain input,
forming a recursive menu -> key -> menu cycle. `input::menu_key` uses the same
key pipeline but skips only this second application-menu accelerator lookup;
the engine keymap and focused text/selectable controls remain in their original
order. Ordinary hardware input is unchanged. `chrome::choose` calls this relay
for edit roles. No clipboard read/write API is added or run by this change.
The upstream manifest is retained as the original source inventory; changed
files are input.rs and chrome.rs. Real window/menu validation is separate from
source review and compilation. This does not modify file IO or the blocked
T07 rename implementation.

## CHORD-IME-01 opt-in synchronous diagnosis

`TEXZI_TRACE_CHORD=1` explicitly enables diagnostic logs; the default is off.
This is observation only: no pre-IME shortcut branch, consumption change,
Translate/Dispatch reordering, IME setting change or new dependency is added.

`window.rs` records KeyDown flags before the existing `TranslateMessage` call
and after the existing `input::key` dispatch, only for this process's own NSH
windows validated by `data(hwnd)`. Records contain Unix-epoch UTC microseconds,
window handle, process-key/modifier/repeat flags, handled-known/handled flags
and read-only native composition flags. No VK value, scan code, character,
document text or path is logged. `text_input.rs` reads focus without invoking
the ordinary stale-focus repair path; `richedit.rs` exposes only its existing
host composition boolean, so deferred input processing can be distinguished
from the host's synchronous notification state. No borrow spans key dispatch.

The Shell uses the same UTC-microsecond basis to record prefix begin/rejection,
continuation registration, actual known/unknown continuation hits and timeout.
Only state flags and a static command ID are recorded. Registration is an
engine-side observation, not proof that Windows has received the resulting
mounted keymap. Native records do not reveal the original key behind
VK_PROCESSKEY, and UTC timestamps are subject to system-clock adjustment.
These diagnostics do not establish a CHORD-IME-01 root cause or claim a fix;
the next fixed package still requires targeted real Windows/IME observation.

## Viewport-bounded rich-editor wheel

The Windows input router preserves the native hit-test, latching, modifier
and delta handling. The tagged virtual rich-editor scroll delegates its
wheel offset to the editor, whose row views are mounted before the offset
command in the same batch. Other wheel targets use the native scroll path.
