use super::*;
use egui::Color32;

#[derive(Clone, PartialEq, Debug)]
pub struct MenuEntry {
    pub label: &'static str,
    pub tint: Color32,
    pub action: Action,
    pub group: u8,
}

pub struct MessageMenu {
    pub unread: bool,
    pub flagged: bool,
    pub server_flags: bool,
    pub folder: Folder,
    pub multi: bool,
}

pub fn message_menu(m: &MessageMenu) -> Vec<MenuEntry> {
    let mut out = Vec::new();
    let mut add = |label, tint, action, group| out.push(MenuEntry { label, tint, action, group });
    if !m.multi {
        add("reply", VALUE, Action::Reply(Reply::One), 0);
        add("reply all", VALUE, Action::Reply(Reply::All), 0);
        add("forward", VALUE, Action::Reply(Reply::Forward), 0);
    }
    if m.server_flags {
        add(if m.flagged { "unflag" } else { "flag" }, VALUE, Action::ToggleFlag, 1);
    }
    add(if m.unread { "mark read" } else { "mark unread" }, VALUE, Action::ToggleUnread, 1);
    add("snooze", VALUE, Action::Snooze, 1);
    if m.folder != Folder::Archive {
        add("archive", VALUE, Action::Archive, 2);
    }
    if m.folder != Folder::Junk {
        add("move to junk", VALUE, Action::Junk, 2);
    }
    add("move to folder", VALUE, Action::OpenMove, 2);
    add("delete", FAULT, Action::Trash, 3);
    out
}

pub fn show_message_menu(ui: &mut Ui, narrow: bool, state: &MessageMenu, scope: Option<MessageId>, actions: &mut Vec<Action>) {
    let entries = message_menu(state);
    let labels: Vec<&str> = entries.iter().map(|e| e.label).collect();
    menu_column(ui, narrow, &labels, |ui, menu| {
        let mut group = None;
        for e in entries {
            if group.is_some_and(|g| g != e.group) {
                menu.separator(ui);
            }
            group = Some(e.group);
            if menu.key(ui, e.label, e.tint) {
                match scope {
                    Some(id) => {
                        actions.push(Action::FocusThread(id));
                        actions.push(if matches!(e.action, Action::Reply(_)) { e.action } else { Action::One(Box::new(e.action)) });
                    }
                    None => actions.push(e.action),
                }
                ui.close();
            }
        }
    });
}
