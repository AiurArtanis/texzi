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
    ("f1", "texzi.commandPalette.open"),
    ("mod+e", "texzi.navigation.quickOpen"),
    ("mod+f4", "texzi.tab.close"),
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

const MENU_GROUPS: &[(&str, &[&str])] = &[
    (
        "文件(&F)",
        &[
            "texzi.file.new",
            "texzi.file.open",
            "texzi.folder.open",
            "texzi.folder.close",
            "",
            "texzi.file.save",
            "texzi.file.saveAs",
            "texzi.file.saveAll",
            "",
            "texzi.window.new",
            "texzi.tab.close",
            "texzi.tab.reopenClosed",
            "texzi.window.close",
        ],
    ),
    (
        "编辑(&E)",
        &[
            "texzi.edit.undo",
            "texzi.edit.redo",
            "",
            "texzi.edit.cut",
            "texzi.edit.copy",
            "texzi.edit.paste",
            "",
            "texzi.find.open",
            "texzi.find.replaceOpen",
            "",
            "texzi.settings.open",
            "texzi.settings.keybindings",
        ],
    ),
    (
        "选择(&S)",
        &[
            "texzi.edit.selectAll",
            "texzi.multicursor.addNext",
            "texzi.multicursor.selectAllOccurrences",
            "texzi.edit.copyLineDown",
            "",
            "texzi.multicursor.clear",
        ],
    ),
    (
        "查看(&V)",
        &[
            "texzi.commandPalette.open",
            "texzi.sidebar.toggle",
            "texzi.find.inFolder",
            "",
            "texzi.group.splitRight",
            "texzi.group.splitDown",
            "texzi.group.joinAll",
            "texzi.markdown.toggleSource",
            "texzi.editor.toggleWrap",
            "texzi.view.toggleZen",
            "",
            "texzi.appearance.light",
            "texzi.appearance.dark",
        ],
    ),
    (
        "转到(&G)",
        &[
            "texzi.navigation.quickOpen",
            "texzi.navigation.gotoLine",
            "texzi.tab.next",
            "texzi.tab.previous",
            "",
            "texzi.group.focusIndex",
        ],
    ),
    ("帮助(&H)", &["texzi.help.guide", "texzi.help.about"]),
];

/// Window commands belong to the host rather than the rich text model. The
/// native menu and engine aliases use this same list; text-edit roles stay in
/// the currently focused input, and Ctrl+B remains rich bold.
pub fn application_shortcuts() -> Vec<nsh_core::prelude::SharedString> {
    let commands = commands();
    let mut shortcuts = std::collections::BTreeSet::new();
    for (_, ids) in MENU_GROUPS {
        for id in *ids {
            if id.is_empty() || role(id).is_some() {
                continue;
            }
            let command = commands.iter().find(|c| c.id == *id).expect("menu command");
            if let Some(stroke) = displayed_key(command).and_then(key) {
                shortcuts.insert(stroke);
            }
        }
    }
    shortcuts.extend(
        SHELL_SHORTCUTS
            .iter()
            .map(|(stroke, _)| (*stroke).to_owned()),
    );
    shortcuts.insert("mod+k".to_owned());
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
    let commands = commands();
    let item = |id: &str| {
        let command = commands
            .iter()
            .find(|c| c.id == id)
            .unwrap_or_else(|| panic!("unknown menu command: {id}"));
        // Ctrl+B belongs to bold in rich text. The shared native menu cannot
        // install a context-free accelerator that steals it before the editor.
        let displayed_key = displayed_key(command);
        let reason = blocked(id);
        let label = match &reason {
            Some(_) => format!("{}（暂不可用）", command.name),
            None => command.name.clone(),
        };
        let mut item = match role(id).filter(|_| reason.is_none()) {
            Some(role) => MenuItem::role(role).label(label),
            None => {
                let run = run.clone();
                let id = id.to_owned();
                MenuItem::new(label).on_select(move || run(&id))
            }
        };
        if let Some(shortcut) = displayed_key.and_then(key).filter(|_| !chord_pending) {
            item = item.shortcut(&shortcut);
        } else if let Some(chord) = displayed_key {
            // A tab separates the native accelerator column without claiming
            // that the native single-stroke matcher implements the chord.
            item = item.label(format!(
                "{}\t{chord}",
                match &reason {
                    Some(_) => format!("{}（暂不可用）", command.name),
                    None => command.name.clone(),
                }
            ));
        }
        item
    };
    let group = |name: &str, ids: &[&str]| {
        MenuItem::submenu(
            name,
            ids.iter()
                .map(|id| {
                    if id.is_empty() {
                        MenuItem::separator()
                    } else {
                        item(id)
                    }
                })
                .collect::<Vec<_>>(),
        )
    };
    set_application_menu(
        MENU_GROUPS
            .iter()
            .map(|(name, ids)| group(name, ids))
            .collect::<Vec<_>>(),
    );
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
            "ctrl+o", "ctrl+e", "ctrl+k", "ctrl+1", "ctrl+8", "f1", "ctrl+f4",
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
        for (_, ids) in MENU_GROUPS {
            for id in *ids {
                if id.is_empty() {
                    continue;
                }
                let command = commands
                    .iter()
                    .find(|c| c.id == *id)
                    .unwrap_or_else(|| panic!("missing command: {id}"));
                assert_eq!(
                    crate::app_shell::supported(id),
                    !matches!(*id, "texzi.settings.open" | "texzi.settings.keybindings"),
                    "expected application command route for {id}"
                );
                if let Some(stroke) = command.keys.first().and_then(|s| key(s)) {
                    nsh_core::desktop::Shortcut::parse(&stroke, false)
                        .unwrap_or_else(|e| panic!("{id}: {e}"));
                }
            }
        }
    }
}
