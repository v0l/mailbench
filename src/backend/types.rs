use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Security {
    Tls,
    StartTls,
    Plain,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Auth {
    Password { user: String, password: String },
    XOAuth2 { user: String, access_token: String },
}

impl Auth {
    pub fn user(&self) -> &str {
        match self {
            Auth::Password { user, .. } | Auth::XOAuth2 { user, .. } => user,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub idle: bool,
    pub move_command: bool,
    pub condstore: bool,
    pub qresync: bool,
    pub uidplus: bool,
    pub gmail_ext: bool,
    pub send: bool,
    pub server_folders: bool,
    pub server_search: bool,
    pub server_flags: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FolderRole {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
    All,
    Flagged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub name: String,
    pub delimiter: Option<String>,
    pub role: Option<FolderRole>,
    pub selectable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncToken {
    pub uid_validity: u32,
    pub uid_next: u32,
    pub highest_mod_seq: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags {
    pub seen: bool,
    pub answered: bool,
    pub flagged: bool,
    pub deleted: bool,
    pub draft: bool,
    pub keywords: Vec<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlagUpdate {
    Add(Flags),
    Remove(Flags),
    Set(Flags),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    pub name: Option<String>,
    pub address: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSummary {
    pub uid: u32,
    pub flags: Flags,
    pub subject: Option<String>,
    pub from: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub date: Option<i64>,
    pub message_id: Option<String>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    pub size: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderState {
    pub token: SyncToken,
    pub uids: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderDelta {
    pub token: SyncToken,
    pub reset: bool,
    pub added: Vec<MessageSummary>,
    pub flag_changes: Vec<(u32, Flags)>,
    pub uids: Vec<u32>,
    pub more: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutgoingMessage {
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub subject: String,
    pub body: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub attachments: Vec<Attachment>,
    pub message_id: Option<String>,
    pub html_alternative: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    FolderChanged { folder: String },
    Disconnected { reason: String },
}
