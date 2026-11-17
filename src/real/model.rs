use crate::backend::{Folder as Remote, FolderRole};
use crate::mailstore::Folder;
use serde::{Deserialize, Serialize};

pub fn folder_from_name(name: &str) -> Option<Folder> {
    [Folder::Inbox, Folder::Drafts, Folder::Sent, Folder::Archive, Folder::Junk, Folder::Trash, Folder::Snoozed]
        .into_iter()
        .find(|f| f.name() == name)
}

fn by_name(name: &str) -> Option<Folder> {
    let last = name.rsplit(['/', '.']).next().unwrap_or(name).to_lowercase();
    match last.as_str() {
        "inbox" => Some(Folder::Inbox),
        "sent" | "sent items" | "sent mail" => Some(Folder::Sent),
        "drafts" => Some(Folder::Drafts),
        "trash" | "deleted" | "deleted items" => Some(Folder::Trash),
        "junk" | "spam" | "junk email" => Some(Folder::Junk),
        "archive" | "archives" => Some(Folder::Archive),
        _ => None,
    }
}

pub fn ui_folder(f: &Remote) -> Option<Folder> {
    if !f.selectable {
        return None;
    }
    match f.role {
        Some(FolderRole::Inbox) => Some(Folder::Inbox),
        Some(FolderRole::Sent) => Some(Folder::Sent),
        Some(FolderRole::Drafts) => Some(Folder::Drafts),
        Some(FolderRole::Trash) => Some(Folder::Trash),
        Some(FolderRole::Junk) => Some(Folder::Junk),
        Some(FolderRole::Archive) | Some(FolderRole::All) => Some(Folder::Archive),
        Some(FolderRole::Flagged) => None,
        None => by_name(&f.name),
    }
}

pub fn default_name(folder: Folder) -> &'static str {
    match folder {
        Folder::Inbox => "INBOX",
        Folder::Drafts => "Drafts",
        Folder::Sent => "Sent",
        Folder::Archive => "Archive",
        Folder::Junk => "Junk",
        Folder::Trash => "Trash",
        Folder::Snoozed => "Snoozed",
    }
}

pub fn server_folder(folders: &[Remote], target: Folder) -> Option<String> {
    let found = folders.iter().filter(|f| ui_folder(f) == Some(target));
    let mut all: Vec<&Remote> = found.collect();
    all.sort_by_key(|f| f.role.is_none());
    all.first().map(|f| f.name.clone())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Op {
    Flags { account: String, folder: String, uid: u32, uid_validity: u32, seen: Option<bool>, flagged: Option<bool>, base_seen: bool, base_flagged: bool },
    Move { account: String, folder: String, uid: u32, uid_validity: u32, to: String },
    Send { account: String, local: String },
}

impl Op {
    pub fn account(&self) -> &str {
        match self {
            Op::Flags { account, .. } | Op::Move { account, .. } | Op::Send { account, .. } => account,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalMsg {
    pub account: String,
    pub folder: String,
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub body: String,
    pub date: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str, role: Option<FolderRole>) -> Remote {
        Remote { name: name.into(), delimiter: Some("/".into()), role, selectable: true }
    }

    #[test]
    fn roles_and_names_map_to_ui_folders() {
        assert_eq!(ui_folder(&f("INBOX", Some(FolderRole::Inbox))), Some(Folder::Inbox));
        assert_eq!(ui_folder(&f("[Gmail]/All Mail", Some(FolderRole::All))), Some(Folder::Archive));
        assert_eq!(ui_folder(&f("Sent Items", None)), Some(Folder::Sent));
        assert_eq!(ui_folder(&f("Spam", None)), Some(Folder::Junk));
        assert_eq!(ui_folder(&f("Receipts", None)), None);
    }

    #[test]
    fn server_folder_prefers_the_role() {
        let folders = vec![f("archive", None), f("Old", Some(FolderRole::Archive))];
        assert_eq!(server_folder(&folders, Folder::Archive).as_deref(), Some("Old"));
        assert_eq!(server_folder(&folders, Folder::Trash), None);
    }
}
