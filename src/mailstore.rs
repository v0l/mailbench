#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct MessageId(pub u64);

#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub enum Folder {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
    Snoozed,
    Other(usize, String),
}

impl Folder {
    pub const UNIFIED: [Folder; 6] =
        [Folder::Inbox, Folder::Drafts, Folder::Sent, Folder::Archive, Folder::Junk, Folder::Trash];

    pub fn name(&self) -> &str {
        match self {
            Folder::Inbox => "inbox",
            Folder::Drafts => "drafts",
            Folder::Sent => "sent",
            Folder::Archive => "archive",
            Folder::Junk => "junk",
            Folder::Trash => "trash",
            Folder::Snoozed => "snoozed",
            Folder::Other(_, name) => name,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Filter {
    All,
    Unread,
    Flagged,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SyncState {
    Idle,
    Syncing,
    Failed,
}

#[derive(Clone, Debug)]
pub struct AccountInfo {
    pub name: String,
    pub address: String,
    pub unread: usize,
    pub sync: SyncState,
    pub detail: Option<String>,
    pub indexed: Option<(usize, usize)>,
    pub signature: String,
    pub server_folders: bool,
    pub server_search: bool,
    pub server_flags: bool,
}

#[derive(Clone, Debug)]
pub struct FolderInfo {
    pub folder: Folder,
    pub unread: usize,
    pub total: usize,
}

#[derive(Clone, Debug)]
pub struct LabelInfo {
    pub name: String,
    pub unread: usize,
}

#[derive(Clone, Debug)]
pub struct TreeFolder {
    pub folder: Folder,
    pub account: usize,
    pub path: String,
    pub delimiter: String,
    pub leaf: String,
    pub depth: usize,
    pub selectable: bool,
    pub unread: usize,
    pub total: usize,
    pub children: bool,
}

#[derive(Clone, Debug)]
pub struct Query {
    pub folder: Folder,
    pub filter: Filter,
    pub text: String,
}

impl Query {
    pub fn new(folder: Folder) -> Query {
        Query { folder, filter: Filter::All, text: String::new() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Counts {
    pub all: usize,
    pub unread: usize,
    pub flagged: usize,
}

#[derive(Clone, Debug)]
pub struct Summary {
    pub id: MessageId,
    pub from: String,
    pub subject: String,
    pub when: String,
    pub unread: bool,
    pub flagged: bool,
    pub new: bool,
    pub attachments: bool,
    pub account: usize,
    pub thread: u32,
    pub remote: bool,
}

#[derive(Clone, Debug)]
pub struct Page {
    pub offset: usize,
    pub total: usize,
    pub items: Vec<Summary>,
}

#[derive(Clone, Debug)]
pub struct AttachmentInfo {
    pub name: String,
    pub size: String,
    pub mime: String,
}

#[derive(Clone, Debug)]
pub struct AttachmentData {
    pub name: String,
    pub mime: String,
    pub data: std::sync::Arc<Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthResult {
    pub spf: Option<String>,
    pub dkim: Option<String>,
    pub dmarc: Option<String>,
}

impl AuthResult {
    pub fn failed(&self) -> bool {
        [&self.spf, &self.dkim, &self.dmarc].iter().any(|r| matches!(r.as_deref(), Some("fail" | "softfail" | "permerror")))
    }

    pub fn present(&self) -> bool {
        self.spf.is_some() || self.dkim.is_some() || self.dmarc.is_some()
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        for (name, v) in [("spf", &self.spf), ("dkim", &self.dkim), ("dmarc", &self.dmarc)] {
            if let Some(v) = v {
                parts.push(format!("{name} {v}"));
            }
        }
        parts.join("  ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsubscribe {
    pub target: String,
    pub one_click: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Headers {
    pub all: Vec<(String, String)>,
    pub list_id: Option<String>,
    pub unsubscribe: Option<Unsubscribe>,
    pub auth: AuthResult,
    pub reply_to_differs: Option<String>,
    pub spoof: Vec<crate::spoof::SpoofWarning>,
    pub pgp: Option<crate::openpgp::PgpStatus>,
    pub bulk: bool,
    pub hops: usize,
    pub spam: bool,
    pub spam_score: Option<f32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BodyState {
    #[default]
    Ready,
    Loading,
    Failed,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub summary: Summary,
    pub folder: Folder,
    pub from_address: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub sent: String,
    pub paragraphs: Vec<String>,
    pub html: crate::htmlview::HtmlBody,
    pub quoted: Vec<String>,
    pub attachments: Vec<AttachmentInfo>,
    pub remote_images: Option<String>,
    pub headers: Headers,
    pub body_state: BodyState,
    pub message_id: Option<String>,
    pub references: Vec<String>,
    pub to_full: Vec<Contact>,
    pub cc_full: Vec<Contact>,
}

#[derive(Clone, Debug)]
pub struct ThreadItem {
    pub summary: Summary,
    pub folder: Folder,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contact {
    pub name: String,
    pub address: String,
}

impl Contact {
    pub fn of(a: &crate::backend::Address) -> Contact {
        Contact { name: a.name.clone().unwrap_or_default(), address: a.address.clone() }
    }
}

impl Contact {
    pub fn formatted(&self) -> String {
        if self.name.trim().is_empty() || self.name == self.address {
            self.address.clone()
        } else {
            let name = self.name.trim();
            if name.contains([',', ';', '"', '<', '>']) {
                format!("\"{}\" <{}>", name.replace('"', ""), self.address)
            } else {
                format!("{name} <{}>", self.address)
            }
        }
    }

    pub fn parse(text: &str) -> Contact {
        let text = text.trim();
        match (text.rfind('<'), text.rfind('>')) {
            (Some(a), Some(b)) if a < b => Contact {
                name: text[..a].trim().trim_matches('"').to_string(),
                address: text[a + 1..b].trim().to_string(),
            },
            _ => Contact { name: String::new(), address: text.to_string() },
        }
    }
}

pub fn split_recipients(list: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in list.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            ',' | ';' if !quoted => {
                out.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    out.push(current);
    out.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

#[derive(Clone, Debug)]
pub struct Restore {
    pub id: MessageId,
    pub message_id: Option<String>,
    pub account: usize,
    pub folder: Folder,
    pub moved_to: Option<Folder>,
    pub unread: bool,
    pub flagged: bool,
}

pub use crate::backend::Attachment as OutAttachment;

#[derive(Clone, Copy, Debug, Default)]
pub struct FlagChange {
    pub unread: Option<bool>,
    pub flagged: Option<bool>,
}

#[derive(Clone, Debug, Default)]
pub struct Outgoing {
    pub account: usize,
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<OutAttachment>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub message_id: Option<String>,
}

pub const SEND_HOLD_SECS: u64 = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    NotFound,
    Backend(String),
}

pub trait MailStore {
    fn accounts(&self) -> Vec<AccountInfo>;
    fn folders(&self) -> Vec<FolderInfo>;
    fn labels(&self) -> Vec<LabelInfo>;
    fn counts(&self, query: &Query) -> Counts;
    fn list(&self, query: &Query, offset: usize, limit: usize) -> Page;
    fn searching(&self, _query: &Query) -> bool {
        false
    }
    fn get(&self, id: MessageId) -> Result<Message, StoreError>;
    fn thread(&self, id: MessageId) -> Vec<ThreadItem> {
        self.get(id).map(|m| vec![ThreadItem { summary: m.summary, folder: m.folder }]).unwrap_or_default()
    }
    fn set_flags(&mut self, id: MessageId, change: FlagChange) -> Result<(), StoreError>;
    fn move_to(&mut self, id: MessageId, folder: Folder) -> Result<(), StoreError>;
    fn bulk_flags(&mut self, ids: &[MessageId], change: FlagChange) -> Result<(), StoreError> {
        for id in ids {
            self.set_flags(*id, change)?;
        }
        Ok(())
    }
    fn bulk_move(&mut self, ids: &[MessageId], folder: Folder) -> Result<(), StoreError> {
        for id in ids {
            self.move_to(*id, folder.clone())?;
        }
        Ok(())
    }
    fn bulk_snooze(&mut self, ids: &[MessageId], wake_at: i64) -> Result<(), StoreError> {
        for id in ids {
            self.snooze(*id, wake_at)?;
        }
        Ok(())
    }
    fn snapshot(&self, ids: &[MessageId]) -> Vec<Restore> {
        ids.iter()
            .filter_map(|id| self.get(*id).ok())
            .map(|m| Restore {
                id: m.summary.id,
                message_id: m.message_id.clone(),
                account: m.summary.account,
                folder: m.folder.clone(),
                moved_to: None,
                unread: m.summary.unread,
                flagged: m.summary.flagged,
            })
            .collect()
    }
    fn mark_all_read(&mut self, folder: &Folder) -> Result<Vec<Restore>, StoreError> {
        let query = Query { filter: Filter::Unread, ..Query::new(folder.clone()) };
        let ids: Vec<MessageId> = self.list(&query, 0, usize::MAX).items.iter().map(|s| s.id).collect();
        let before = self.snapshot(&ids);
        self.bulk_flags(&ids, FlagChange { unread: Some(false), flagged: None })?;
        Ok(before)
    }
    fn restore(&mut self, items: &[Restore]) -> Result<(), StoreError> {
        for r in items {
            if r.moved_to.is_some() {
                self.move_to(r.id, r.folder.clone())?;
            }
            self.set_flags(r.id, FlagChange { unread: Some(r.unread), flagged: Some(r.flagged) })?;
        }
        Ok(())
    }
    fn pending_ops(&self) -> usize {
        0
    }
    fn append(&mut self, folder: Folder, message: Outgoing) -> Result<MessageId, StoreError>;
    fn snooze(&mut self, id: MessageId, _wake_at: i64) -> Result<(), StoreError> {
        self.move_to(id, Folder::Snoozed)
    }
    fn take_focus(&mut self) -> Option<MessageId> {
        None
    }
    fn attachments(&self, _id: MessageId) -> Result<Vec<AttachmentData>, StoreError> {
        Ok(Vec::new())
    }

    fn tree(&self) -> Vec<TreeFolder> {
        Vec::new()
    }

    fn create_folder(&mut self, _account: usize, _name: &str) -> Result<(), StoreError> {
        Err(StoreError::Backend("folders cannot be created here".into()))
    }

    fn rename_folder(&mut self, _folder: &Folder, _name: &str) -> Result<(), StoreError> {
        Err(StoreError::Backend("folders cannot be renamed here".into()))
    }

    fn delete_folder(&mut self, _folder: &Folder) -> Result<(), StoreError> {
        Err(StoreError::Backend("folders cannot be deleted here".into()))
    }

    fn save_draft(&mut self, message: Outgoing) -> Result<String, StoreError> {
        let kept = message.message_id.clone();
        let id = self.append(Folder::Drafts, message)?;
        Ok(kept.unwrap_or_else(|| id.0.to_string()))
    }

    fn open_draft(&mut self, _id: MessageId) -> Result<Outgoing, StoreError> {
        Err(StoreError::NotFound)
    }

    fn discard_draft(&mut self, _account: usize, _message_id: &str) {}

    fn undo_send(&mut self, _id: MessageId) -> Result<Option<Outgoing>, StoreError> {
        Ok(None)
    }

    fn suggest(&self, _text: &str, _limit: usize) -> Vec<Contact> {
        Vec::new()
    }

    fn send_hold(&self) -> std::time::Duration {
        std::time::Duration::from_secs(SEND_HOLD_SECS)
    }
}

pub fn human_size(bytes: usize) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1000 {
        format!("{} KB", bytes / 1000)
    } else {
        format!("{bytes} B")
    }
}
