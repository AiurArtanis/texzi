//! Product commands and Win32 menus share stable IDs from the design registry.
use nsh_core::{
    desktop::MenuRole,
    element::desktop::{MenuItem, set_application_menu},
};
use std::rc::Rc;

pub(super) const SHELL_SHORTCUTS: &[(&str, &str)] = &[
    ("mod+o", "texzi.file.open"),
    ("mod+s", "texzi.file.save"),
    ("mod+shift+s", "texzi.file.saveAs"),
    ("f3", "texzi.find.next"),
    ("shift+f3", "texzi.find.previous"),
    ("mod+shift+w", "texzi.window.close"),
    ("mod+shift+n", "texzi.window.new"),
    ("f1", "texzi.commandPalette.open"),
    ("mod+shift+p", "texzi.commandPalette.open"),
    ("mod+e", "texzi.navigation.quickOpen"),
    ("mod+p", "texzi.navigation.quickOpen"),
    ("mod+f4", "texzi.tab.close"),
    ("mod+w", "texzi.tab.close"),
    ("mod+shift+t", "texzi.tab.reopenClosed"),
    ("mod+h", "texzi.find.replaceOpen"),
    ("mod+d", "texzi.multicursor.addNext"),
    ("mod+shift+l", "texzi.multicursor.selectAllOccurrences"),
    ("shift+alt+down", "texzi.edit.copyLineDown"),
    ("mod+shift+f", "texzi.find.inFolder"),
    ("mod+\\", "texzi.group.splitRight"),
    ("mod+g", "texzi.navigation.gotoLine"),
    ("mod+pagedown", "texzi.tab.next"),
    ("mod+pageup", "texzi.tab.previous"),
    ("mod+f", "texzi.find.open"),
    ("mod+n", "texzi.file.new"),
    ("alt+z", "texzi.editor.toggleWrap"),
];
pub(super) const MENU_KEY: &str = "f10";

#[derive(Clone, Copy)]
pub(super) enum MenuGroup {
    File,
    Edit,
    Appearance,
}

impl MenuGroup {
    pub(super) const ALL: [Self; 3] = [Self::File, Self::Edit, Self::Appearance];

    pub(super) fn shortcut(self) -> &'static str {
        match self {
            Self::File => "alt+f",
            Self::Edit => "alt+e",
            Self::Appearance => "alt+v",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::File => "文件(&F)",
            Self::Edit => "编辑(&E)",
            Self::Appearance => "外观(&V)",
        }
    }

    pub(super) fn position(self) -> f32 {
        self as u8 as f32
    }

    pub(super) fn commands(self) -> &'static [&'static str] {
        match self {
            Self::File => &[
                "texzi.file.new",
                "texzi.file.open",
                "texzi.folder.open",
                "",
                "texzi.file.save",
                "texzi.file.saveAs",
                "texzi.tab.close",
            ],
            Self::Edit => &[
                "texzi.edit.undo",
                "texzi.edit.redo",
                "",
                "texzi.edit.cut",
                "texzi.edit.copy",
                "texzi.edit.paste",
                "texzi.edit.selectAll",
                "",
                "texzi.find.open",
            ],
            Self::Appearance => &[
                "texzi.sidebar.toggle",
                "texzi.editor.toggleWrap",
                "",
                "texzi.appearance.light",
                "texzi.appearance.dark",
            ],
        }
    }
}

const REMOVED_COMMANDS: &[&str] = &[
    "texzi.file.saveWithEncoding",
    "texzi.file.reopenWithEncoding",
    "texzi.file.changeEol",
    "texzi.file.revert",
    "texzi.file.copyPath",
    "texzi.group.focusDirection",
    "texzi.tab.moveToNewWindow",
    "texzi.tab.copyToNewWindow",
    "texzi.settings.open",
    "texzi.settings.keybindings",
    "texzi.tab.closeOthers",
    "texzi.tab.closeAll",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub id: String,
    pub name: String,
    pub group: String,
    pub keys: Vec<String>,
}

pub fn commands() -> Vec<Command> {
    let registry: serde_json::Value =
        serde_json::from_str(include_str!("../../../docs/design-v1/data/commands.json"))
            .expect("bundled command registry");
    let mut commands: Vec<_> = registry["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .filter(|c| !REMOVED_COMMANDS.contains(&c["id"].as_str().unwrap()))
        .map(|c| Command {
            id: c["id"].as_str().unwrap().into(),
            name: c["name"].as_str().unwrap().into(),
            group: c["category"].as_str().unwrap().into(),
            keys: c["defaultKeys"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|k| k.as_str().map(str::to_owned))
                .collect(),
        })
        .collect();
    for (id, name) in [
        ("texzi.help.guide", "使用说明"),
        ("texzi.help.about", "关于 texzi"),
        ("texzi.appearance.light", "浅色主题"),
        ("texzi.appearance.dark", "深色主题"),
        ("texzi.multicursor.clear", "清除额外选区"),
    ] {
        commands.push(Command {
            id: id.into(),
            name: name.into(),
            group: "帮助与外观".into(),
            keys: vec![],
        });
    }
    commands
}

pub fn role(id: &str) -> Option<MenuRole> {
    Some(match id {
        "texzi.edit.undo" => MenuRole::Undo,
        "texzi.edit.redo" => MenuRole::Redo,
        "texzi.edit.cut" => MenuRole::Cut,
        "texzi.edit.copy" => MenuRole::Copy,
        "texzi.edit.paste" | "texzi.edit.pastePlain" => MenuRole::Paste,
        "texzi.edit.selectAll" => MenuRole::SelectAll,
        _ => return None,
    })
}

fn key(text: &str) -> Option<String> {
    // Native accelerators are single strokes. Chords are registered once in
    // the application keymap, preserving the second stroke's modifiers.
    (!text.contains(' ') && !text.contains("Drag"))
        .then(|| text.to_ascii_lowercase().replace("ctrl", "mod"))
}

fn displayed_key(command: &Command) -> Option<&str> {
    match command.id.as_str() {
        "texzi.sidebar.toggle" => Some("Ctrl+K Ctrl+B"),
        "texzi.group.focusIndex" => Some("Ctrl+1 / Ctrl+2"),
        _ => command.keys.first().map(String::as_str),
    }
}

/// Window commands belong to the host rather than the rich text model. The
/// application keymap is independent of menu placement; text-edit roles stay in
/// the currently focused input, and Ctrl+B remains rich bold.
pub fn application_shortcuts() -> Vec<nsh_core::prelude::SharedString> {
    let mut shortcuts = std::collections::BTreeSet::new();
    shortcuts.extend(
        SHELL_SHORTCUTS
            .iter()
            .map(|(stroke, _)| (*stroke).to_owned()),
    );
    shortcuts.insert("mod+k".to_owned());
    shortcuts.insert(MENU_KEY.to_owned());
    shortcuts.extend(MenuGroup::ALL.map(|group| group.shortcut().to_owned()));
    for index in 1..=8 {
        shortcuts.insert(format!("mod+{index}"));
    }
    shortcuts.into_iter().map(Into::into).collect()
}

pub fn install(
    run: Rc<dyn Fn(&str)>,
    blocked: impl Fn(&str) -> Option<String>,
    chord_pending: bool,
) {
    set_application_menu(groups(&run, &blocked, chord_pending));
}

pub(super) fn groups(
    run: &Rc<dyn Fn(&str)>,
    blocked: &impl Fn(&str) -> Option<String>,
    chord_pending: bool,
) -> Vec<MenuItem> {
    MenuGroup::ALL
        .into_iter()
        .map(|group| {
            MenuItem::submenu(
                group.label(),
                entries(group.commands(), run, blocked, chord_pending),
            )
        })
        .collect()
}

pub(super) fn entries(
    ids: &[&str],
    run: &Rc<dyn Fn(&str)>,
    blocked: &impl Fn(&str) -> Option<String>,
    chord_pending: bool,
) -> Vec<MenuItem> {
    let commands = commands();
    let mut entries = Vec::new();
    let mut separator = false;
    for &id in ids {
        if id.is_empty() {
            separator = !entries.is_empty();
            continue;
        }
        if blocked(id).is_some() {
            continue;
        }
        if std::mem::take(&mut separator) {
            entries.push(MenuItem::separator());
        }
        let command = commands
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("unknown menu command: {id}"));
        // Ctrl+B belongs to bold in rich text. The shared native menu cannot
        // install a context-free accelerator that steals it before the editor.
        let displayed_key = displayed_key(command);
        let mut item = match role(id) {
            Some(role) => MenuItem::role(role).label(command.name.clone()),
            None => {
                let run = run.clone();
                let id = id.to_owned();
                MenuItem::new(command.name.clone()).on_select(move || run(&id))
            }
        };
        if let Some(shortcut) = displayed_key.and_then(key).filter(|_| !chord_pending) {
            item = item.shortcut(&shortcut);
        } else if let Some(chord) = displayed_key {
            // A tab separates the native accelerator column without claiming
            // that the native single-stroke matcher implements the chord.
            item = item.label(format!("{}\t{chord}", command.name));
        }
        entries.push(item);
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn application_shortcuts_keep_window_commands_and_leave_text_roles_to_the_editor() {
        use nsh_core::keymap::Keystroke;
        let strokes: Vec<_> = application_shortcuts()
            .iter()
            .map(|s| Keystroke::parse_with(s, false).expect("valid Windows host shortcut"))
            .collect();
        let contains = |s| strokes.contains(&Keystroke::parse_with(s, false).unwrap());
        for global in [
            "ctrl+o", "ctrl+e", "ctrl+k", "ctrl+1", "ctrl+8", "f1", "ctrl+f4", "f10", "alt+f",
        ] {
            assert!(
                contains(global),
                "window command {global} must remain reachable in rich text"
            );
        }
        // Ctrl+E is the product's global Quick Open alias. Inline code is
        // deliberately Unassigned in COMMANDS, despite NSH's default binding.
        for editor in [
            "ctrl+b",
            "ctrl+i",
            "ctrl+z",
            "ctrl+y",
            "ctrl+shift+z",
            "ctrl+a",
            "ctrl+c",
            "ctrl+x",
            "ctrl+v",
            "escape",
            "enter",
            "backspace",
            "delete",
        ] {
            assert!(
                !contains(editor),
                "host must not take rich editing key {editor}"
            );
        }
    }

    #[test]
    fn every_native_menu_entry_resolves_and_its_accelerator_parses() {
        let commands = commands();
        for group in MenuGroup::ALL {
            for id in group.commands() {
                if id.is_empty() {
                    continue;
                }
                let command = commands
                    .iter()
                    .find(|c| c.id == *id)
                    .unwrap_or_else(|| panic!("missing command: {id}"));
                assert!(
                    crate::app_shell::supported(id),
                    "expected application command route for {id}"
                );
                if let Some(stroke) = command.keys.first().and_then(|s| key(s)) {
                    nsh_core::desktop::Shortcut::parse(&stroke, false)
                        .unwrap_or_else(|e| panic!("{id}: {e}"));
                }
            }
        }
    }

    #[test]
    fn moved_commands_keep_their_bindings_and_placeholders_are_absent() {
        let commands = commands();
        assert!(
            commands
                .iter()
                .all(|command| !REMOVED_COMMANDS.contains(&command.id.as_str()))
        );
        for (key, id) in [
            ("mod+h", "texzi.find.replaceOpen"),
            ("mod+shift+p", "texzi.commandPalette.open"),
            ("mod+p", "texzi.navigation.quickOpen"),
        ] {
            assert!(SHELL_SHORTCUTS.contains(&(key, id)));
            assert!(commands.iter().any(|command| command.id == id));
            assert!(
                MenuGroup::ALL
                    .into_iter()
                    .all(|group| !group.commands().contains(&id))
            );
        }
        let strokes = application_shortcuts();
        assert!(
            ["alt+s", "alt+g", "alt+h"]
                .iter()
                .all(|removed| { strokes.iter().all(|stroke| stroke.as_ref() != *removed) })
        );
    }
}
