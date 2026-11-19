#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct MessageId(pub u64);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Folder {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
    Snoozed,
}

impl Folder {
    pub const UNIFIED: [Folder; 6] =
        [Folder::Inbox, Folder::Drafts, Folder::Sent, Folder::Archive, Folder::Junk, Folder::Trash];

    pub fn name(self) -> &'static str {
        match self {
            Folder::Inbox => "inbox",
            Folder::Drafts => "drafts",
            Folder::Sent => "sent",
            Folder::Archive => "archive",
            Folder::Junk => "junk",
            Folder::Trash => "trash",
            Folder::Snoozed => "snoozed",
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
}

#[derive(Clone, Debug)]
pub struct ThreadItem {
    pub summary: Summary,
    pub folder: Folder,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FlagChange {
    pub unread: Option<bool>,
    pub flagged: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct Outgoing {
    pub account: usize,
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub body: String,
}

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
    fn get(&self, id: MessageId) -> Result<Message, StoreError>;
    fn thread(&self, id: MessageId) -> Vec<ThreadItem> {
        self.get(id).map(|m| vec![ThreadItem { summary: m.summary, folder: m.folder }]).unwrap_or_default()
    }
    fn set_flags(&mut self, id: MessageId, change: FlagChange) -> Result<(), StoreError>;
    fn move_to(&mut self, id: MessageId, folder: Folder) -> Result<(), StoreError>;
    fn append(&mut self, folder: Folder, message: Outgoing) -> Result<MessageId, StoreError>;
}
