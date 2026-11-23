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
        "snoozed" => Some(Folder::Snoozed),
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

pub fn default_name(folder: &Folder) -> &str {
    match folder {
        Folder::Inbox => "INBOX",
        Folder::Drafts => "Drafts",
        Folder::Sent => "Sent",
        Folder::Archive => "Archive",
        Folder::Junk => "Junk",
        Folder::Trash => "Trash",
        Folder::Snoozed => "Snoozed",
        Folder::Other(_, name) => name,
    }
}

pub fn server_folder(folders: &[Remote], target: &Folder) -> Option<String> {
    let found = folders.iter().filter(|f| ui_folder(f).as_ref() == Some(target));
    let mut all: Vec<&Remote> = found.collect();
    all.sort_by_key(|f| f.role.is_none());
    all.first().map(|f| f.name.clone())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Op {
    Flags { account: String, folder: String, uid: u32, uid_validity: u32, seen: Option<bool>, flagged: Option<bool>, base_seen: bool, base_flagged: bool },
    Move { account: String, folder: String, uid: u32, uid_validity: u32, to: String },
    Send { account: String, local: String },
    CreateFolder { account: String, name: String },
    RenameFolder { account: String, from: String, to: String },
    DeleteFolder { account: String, name: String },
    Wake { account: String, folder: String, uid: u32, uid_validity: u32, to: String, key: String },
    Draft { account: String, local: String },
    DropDraft { account: String, message_id: String },
    MoveSet { account: String, folder: String, uids: Vec<u32>, uid_validity: u32, to: String },
    FlagsSet { account: String, folder: String, uids: Vec<u32>, uid_validity: u32, seen: Option<bool>, flagged: Option<bool> },
    DeleteSet { account: String, folder: String, uids: Vec<u32>, uid_validity: u32 },
}

impl Op {
    pub fn account(&self) -> &str {
        match self {
            Op::Flags { account, .. }
            | Op::Move { account, .. }
            | Op::Send { account, .. }
            | Op::CreateFolder { account, .. }
            | Op::RenameFolder { account, .. }
            | Op::DeleteFolder { account, .. }
            | Op::Wake { account, .. }
            | Op::Draft { account, .. }
            | Op::DropDraft { account, .. }
            | Op::MoveSet { account, .. }
            | Op::DeleteSet { account, .. }
            | Op::FlagsSet { account, .. } => account,
        }
    }
}

pub const DEFAULT_DELIMITER: &str = "/";

fn within(name: &str, root: &str, delimiter: &str) -> bool {
    name == root || name.strip_prefix(root).is_some_and(|rest| rest.starts_with(delimiter))
}

pub fn with_pending(folders: Vec<Remote>, ops: &[&Op]) -> Vec<(Option<String>, Remote)> {
    let delimiter = folders.iter().find_map(|f| f.delimiter.clone()).unwrap_or_else(|| DEFAULT_DELIMITER.to_string());
    let mut out: Vec<(Option<String>, Remote)> = folders.into_iter().map(|f| (Some(f.name.clone()), f)).collect();
    for op in ops {
        match op {
            Op::CreateFolder { name, .. } => {
                if !out.iter().any(|(_, f)| f.name == *name) {
                    out.push((None, Remote { name: name.clone(), delimiter: Some(delimiter.clone()), role: None, selectable: true }));
                }
            }
            Op::RenameFolder { from, to, .. } => {
                for (_, f) in out.iter_mut() {
                    let d = f.delimiter.clone().unwrap_or_else(|| delimiter.clone());
                    if within(&f.name, from, &d) {
                        f.name = format!("{to}{}", &f.name[from.len()..]);
                    }
                }
            }
            Op::DeleteFolder { name, .. } => {
                out.retain(|(_, f)| !within(&f.name, name, f.delimiter.as_deref().unwrap_or(&delimiter)));
            }
            _ => {}
        }
    }
    out
}

fn yes() -> bool {
    true
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
    #[serde(default)]
    pub attachments: Vec<AttMeta>,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub hold_until_ms: i64,
    #[serde(default)]
    pub rev: u64,
    #[serde(default = "yes")]
    pub dirty: bool,
    #[serde(default)]
    pub sent_ok: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttMeta {
    pub name: String,
    pub mime: String,
    pub size: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContactRecord {
    pub name: String,
    pub address: String,
    pub count: u32,
    pub last: i64,
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
    fn pending_folder_ops_show_before_the_server_has_them() {
        let folders = vec![f("INBOX", Some(FolderRole::Inbox)), f("Work", None), f("Work/Old", None)];
        let ops = [
            Op::CreateFolder { account: "a".into(), name: "Receipts".into() },
            Op::RenameFolder { account: "a".into(), from: "Work".into(), to: "Jobs".into() },
            Op::DeleteFolder { account: "a".into(), name: "Jobs/Old".into() },
        ];
        let refs: Vec<&Op> = ops.iter().collect();
        let shown = with_pending(folders, &refs);
        let names: Vec<&str> = shown.iter().map(|(_, f)| f.name.as_str()).collect();
        assert_eq!(names, vec!["INBOX", "Jobs", "Receipts"]);
        assert_eq!(shown[1].0.as_deref(), Some("Work"));
        assert_eq!(shown[2].0, None);
    }

    #[test]
    fn server_folder_prefers_the_role() {
        let folders = vec![f("archive", None), f("Old", Some(FolderRole::Archive))];
        assert_eq!(server_folder(&folders, &Folder::Archive).as_deref(), Some("Old"));
        assert_eq!(server_folder(&folders, &Folder::Trash), None);
    }
}
