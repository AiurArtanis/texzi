//! Window-local Zen Escape sequence. It owns no document or editor history.
//! The shell consumes IME/popup/find Escape first, then supplies its live stamp.
use nsh_core::{
    keymap::{Key, Keystroke, Modifiers, When},
    platform::Os,
    prelude::*,
};
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub document: u64,
    pub group: u8,
    pub rich_mode: bool,
    pub revision: u64,
    pub view_epoch: u64,
    pub rich_state_revision: Option<u64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscapeAction {
    Ignored,
    Armed,
    Exit,
}
pub struct ZenState {
    pending: Cell<Option<Stamp>>,
    pub armed: Signal<bool>,
}
impl ZenState {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            pending: Cell::new(None),
            armed: signal(false),
        })
    }
    pub fn reset(&self) {
        self.pending.set(None);
        if self.armed.get() {
            self.armed.set(false);
        }
    }
    /// `None` means no active Zen editor; `nested` means this Escape belongs
    /// to IME/menu/popup/find instead. That Escape never contributes to exit.
    pub fn escape(&self, stamp: Option<Stamp>, nested: bool) -> EscapeAction {
        let Some(stamp) = stamp.filter(|_| !nested) else {
            self.reset();
            return EscapeAction::Ignored;
        };
        if self.armed.get() && self.pending.get() == Some(stamp) {
            self.reset();
            EscapeAction::Exit
        } else {
            self.pending.set(Some(stamp));
            self.armed.set(true);
            EscapeAction::Armed
        }
    }
}

fn reset_options() -> KeyOptions {
    KeyOptions {
        when: When::Always,
        repeat: true,
        pass_through: true,
        ..Default::default()
    }
}
fn reset_strokes(os: Os) -> Vec<Keystroke> {
    // Every non-Escape key represented by NSH's current physical-key enum.
    // Shift covers US uppercase/punctuation; this is not a raw Windows hook.
    let mut keys: Vec<_> = "abcdefghijklmnopqrstuvwxyz0123456789`-=[]\\;',./ "
        .chars()
        .map(Key::Char)
        .collect();
    keys.extend([
        Key::Enter,
        Key::Tab,
        Key::Backspace,
        Key::Delete,
        Key::Insert,
        Key::Up,
        Key::Down,
        Key::Left,
        Key::Right,
        Key::Home,
        Key::End,
        Key::PageUp,
        Key::PageDown,
    ]);
    keys.extend((1..=24).map(Key::F));
    let mut strokes = Vec::new();
    for key in keys {
        for bits in 0..16 {
            let stroke = Keystroke {
                key,
                modifiers: Modifiers {
                    ctrl: bits & 1 != 0,
                    alt: bits & 2 != 0,
                    shift: bits & 4 != 0,
                    cmd: bits & 8 != 0,
                    alt_graph: false,
                },
            };
            if stroke.reserved(os).is_none() {
                strokes.push(stroke);
            }
        }
    }
    strokes
}

/// Install ONCE, before the shell's real shortcut bindings. NSH picks only
/// the newest match: installing this observer later would mask old commands.
/// Newer shell commands must reset through their existing run/focus routes.
///
/// Notify/pass-through preserves Editor input, repeats, Tab and native menu
/// processing. NSH exposes no wildcard for pure modifiers, CapsLock, consumed
/// IME/native-menu events or mouse focus changes. The shell must reset on its
/// own operations/window-focus changes; the live stamp backs up real edits
/// and navigation. This does not claim every physical key is observable.
pub fn bind_reset_on_other_keys(armed: Signal<bool>, reset: Rc<dyn Fn()>) {
    let options = reset_options();
    for stroke in reset_strokes(os()) {
        let reset = reset.clone();
        on_key_with(&stroke.to_string(), options, move |_| {
            if armed.get() {
                reset();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_core::keymap::{KeyBinding, KeyMatch, Keymap};

    fn stamp() -> Stamp {
        Stamp {
            document: 3,
            group: 0,
            rich_mode: true,
            revision: 9,
            view_epoch: 4,
            rich_state_revision: Some(7),
        }
    }
    #[test]
    fn double_escape_needs_same_live_editor_and_nested_escape_does_not_count() {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        let state = ZenState::new();
        let s = stamp();
        assert_eq!(state.escape(Some(s), false), EscapeAction::Armed);
        assert!(state.armed.get());
        assert_eq!(state.escape(Some(s), true), EscapeAction::Ignored);
        assert!(!state.armed.get());
        assert_eq!(state.escape(Some(s), false), EscapeAction::Armed);
        assert_eq!(state.escape(Some(s), false), EscapeAction::Exit);
        assert!(!state.armed.get());
        assert_eq!(state.escape(Some(s), false), EscapeAction::Armed);
        state.reset(); // a real shell/focus/editor operation
        assert_eq!(state.escape(Some(s), false), EscapeAction::Armed);
        assert_eq!(state.escape(None, false), EscapeAction::Ignored);
        assert!(!state.armed.get());
    }
    #[test]
    fn changed_context_or_native_revision_rearms_instead_of_exiting() {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        let state = ZenState::new();
        let original = stamp();
        for changed in [
            Stamp {
                document: 4,
                ..original
            },
            Stamp {
                group: 1,
                ..original
            },
            Stamp {
                rich_mode: false,
                rich_state_revision: None,
                ..original
            },
            Stamp {
                revision: 10,
                ..original
            },
            Stamp {
                view_epoch: 5,
                ..original
            },
            Stamp {
                rich_state_revision: Some(8),
                ..original
            },
        ] {
            state.reset();
            assert_eq!(state.escape(Some(original), false), EscapeAction::Armed);
            assert_eq!(state.escape(Some(changed), false), EscapeAction::Armed);
            assert_eq!(state.escape(Some(changed), false), EscapeAction::Exit);
        }
    }
    fn bindings() -> Vec<KeyBinding> {
        let options = reset_options();
        reset_strokes(Os::Windows)
            .into_iter()
            .inspect(|stroke| {
                assert_eq!(
                    Keystroke::parse_with(&stroke.to_string(), false).ok(),
                    Some(*stroke),
                    "every installed reset stroke must roundtrip through the real binding parser"
                )
            })
            .enumerate()
            .map(|(i, stroke)| {
                KeyBinding::new(i as u32 + 1, stroke, options.when, options.repeat)
                    .passing_through(options.pass_through)
            })
            .collect()
    }
    #[test]
    fn real_nsh_keymap_reset_observers_notify_without_consuming_editor_input_or_repeats() {
        let table = Keymap::new(bindings());
        for name in ["a", "shift+1", "left", "ctrl+left", "tab", "ctrl+z", "f1"] {
            let stroke = Keystroke::parse_with(name, false).unwrap();
            for repeat in [false, true] {
                let matched = table.find(stroke, true, repeat);
                assert!(
                    matches!(matched, KeyMatch::Notify(_)),
                    "{name}: {matched:?}"
                );
                assert!(
                    !matched.consumed(),
                    "reset observation must not take Editor input"
                );
            }
        }
        for name in ["escape", "shift+escape", "alt+f4"] {
            let stroke = Keystroke::parse_with(name, false).unwrap();
            assert_eq!(
                table.find(stroke, true, false),
                KeyMatch::None,
                "Escape belongs to shell; reserved keys stay unbound"
            );
        }
    }
    #[test]
    fn real_nsh_latest_shell_binding_keeps_priority_over_early_reset_observer() {
        let mut entries = bindings();
        let stroke = Keystroke::parse_with("ctrl+k", false).unwrap();
        let table = Keymap::new(entries.clone());
        assert!(matches!(
            table.find(stroke, true, false),
            KeyMatch::Notify(_)
        ));
        entries.push(KeyBinding::new(50_000, stroke, When::Always, false));
        assert_eq!(
            Keymap::new(entries).find(stroke, true, false),
            KeyMatch::Fire(50_000)
        );
    }
}
