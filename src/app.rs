use crate::fake::FakeStore;
use crate::kit::*;
use crate::mailstore::*;
use crate::prefs::{self, Prefs, RemoteImages};
use egui::{Margin, Rect, Ui};
use egui_bench::prelude::*;
use std::collections::HashSet;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

mod cards;
pub mod menu;

const AUTOSAVE: Duration = Duration::from_secs(4);

pub const ROW_H: f32 = 25.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reply {
    One,
    All,
    Forward,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Action {
    Next,
    Previous,
    Open,
    Archive,
    Snooze,
    ToggleFlag,
    ToggleUnread,
    Junk,
    Trash,
    Reply(Reply),
    Compose,
    Search,
    Back,
    LoadImagesOnce,
    TrustImages(String),
    Find,
    WarnLink(String, String, String),
    CloseLinkWarning,
    ToggleQuotes,
    ToggleReaderWidth,
    ToggleHeaders,
    OpenLink(String),
    Unsubscribe,
    GoTo(Folder),
    GoToIn(usize, Folder),
    AccountOnly(Option<usize>),
    ClearAccount,
    Label(String),
    Expand(MessageId),
    ToggleCard(MessageId),
    ToggleCardQuotes(MessageId),
    ExpandAll,
    CollapseAll,
    ListNext,
    ListPrevious,
    FocusThread(MessageId),
    EditDraft(MessageId),
    One(Box<Action>),
    OpenAttachment(usize),
    SaveAttachment(usize),
    SaveAllAttachments,
    ViewImage(usize),
    CloseViewer,
    OpenMove,
    SnoozeUntil(i64, bool),
    MoveTo(Folder, bool),
    ToggleBranch(usize, String),
    NewFolder(usize, String),
    RenameFolder(Folder, String),
    DeleteFolder(Folder),
    ExtendNext,
    ExtendPrevious,
    SelectAll,
    SelectNone,
    Undo,
    MarkAllRead(Folder),
    OpenChats,
}

pub type SavePicker = Box<dyn FnMut(&str, &str) -> Option<std::path::PathBuf>>;

#[cfg(not(target_os = "android"))]
fn rfd_picker() -> SavePicker {
    Box::new(|name, ext| {
        let mut dialog = rfd::FileDialog::new().set_file_name(name);
        if !ext.is_empty() {
            dialog = dialog.add_filter(ext, &[ext]);
        }
        dialog.save_file()
    })
}

#[cfg(target_os = "android")]
fn rfd_picker() -> SavePicker {
    Box::new(|name, _| {
        let dir = dirs::download_dir().or_else(dirs::data_dir)?;
        Some(dir.join(name))
    })
}

#[cfg(not(target_os = "android"))]
fn pick_attachment_files() -> Vec<PickedFile> {
    rfd::FileDialog::new().set_title("Attach files").pick_files().unwrap_or_default().into_iter().map(PickedFile::Path).collect()
}

#[cfg(target_os = "android")]
fn pick_attachment_files() -> Vec<PickedFile> {
    match crate::android_bridge::open_documents() {
        Ok(items) => items
            .into_iter()
            .map(|p| match p.data {
                Ok(data) => PickedFile::Bytes(p.name, data),
                Err(e) => PickedFile::Failed(e),
            })
            .collect(),
        Err(e) => vec![PickedFile::Failed(e)],
    }
}

#[cfg(target_os = "android")]
fn save_document_async(name: String, data: Vec<u8>, done: &'static str) -> Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mime = guess_mime(&name);
        let msg = match crate::android_bridge::save_document(&name, mime, &data) {
            Ok(Some(_)) => done.to_string(),
            Ok(None) => String::new(),
            Err(e) => format!("Could not save: {e}"),
        };
        let _ = tx.send(msg);
    });
    rx
}

pub enum PickedFile {
    Path(std::path::PathBuf),
    Bytes(String, Vec<u8>),
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Draft {
    pub title: String,
    pub account: usize,
    pub to: String,
    pub cc: String,
    pub to_input: String,
    pub cc_input: String,
    pub subject: String,
    pub body: String,
    pub saved: bool,
    pub attachments: Vec<OutAttachment>,
    pruned: Vec<OutAttachment>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub message_id: Option<String>,
    pub bcc: String,
    pub agent: bool,
    pub signature: String,
    fingerprint: u64,
    changed_at: Option<std::time::Instant>,
}

pub fn png_of(img: &egui::ColorImage) -> Result<Vec<u8>, String> {
    let rgba: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect();
    let buf = image::RgbaImage::from_raw(img.width() as u32, img.height() as u32, rgba).ok_or("Could not read the pasted image")?;
    let mut out = Vec::new();
    buf.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).map_err(|e| format!("Could not read the pasted image: {e}"))?;
    Ok(out)
}

pub fn signature_block(signature: &str) -> String {
    if signature.trim().is_empty() { String::new() } else { format!("\n\n-- \n{}", signature.trim_end()) }
}

pub fn guess_mime(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "txt" | "log" | "md" => "text/plain",
        "html" | "htm" => "text/html",
        "csv" => "text/csv",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

fn quote_lines(text: &str) -> String {
    text.lines()
        .map(|l| if l.starts_with('>') { format!(">{l}") } else { format!("> {l}").trim_end().to_string() })
        .collect::<Vec<_>>()
        .join("\n")
}

fn sender_contact(m: &Message) -> Contact {
    let name = if m.summary.from == m.from_address { String::new() } else { m.summary.from.clone() };
    Contact { name, address: m.from_address.clone() }
}

fn joined(list: &[Contact]) -> String {
    list.iter().map(Contact::formatted).collect::<Vec<_>>().join(", ")
}

impl Draft {
    pub fn blank() -> Draft {
        Draft {
            title: "new message".into(),
            account: 0,
            to: String::new(),
            cc: String::new(),
            to_input: String::new(),
            cc_input: String::new(),
            subject: String::new(),
            body: String::new(),
            saved: false,
            attachments: Vec::new(),
            pruned: Vec::new(),
            in_reply_to: None,
            references: Vec::new(),
            message_id: None,
            bcc: String::new(),
            agent: false,
            signature: String::new(),
            fingerprint: 0,
            changed_at: None,
        }
    }

    pub fn blank_for(account: usize, signature: &str) -> Draft {
        let mut d = Draft::blank();
        d.account = account;
        d.signature = signature.to_string();
        d.body = signature_block(signature);
        d.fingerprint = d.content_hash();
        d
    }

    pub fn reply(m: &Message, mode: Reply, signature: &str, own: &[String], attachments: Vec<OutAttachment>, selected: Option<&str>) -> Draft {
        let (prefix, title) = match mode {
            Reply::Forward => ("Fwd: ", "forward"),
            _ => ("Re: ", "reply to"),
        };
        let subject = if m.summary.subject.to_lowercase().starts_with(&prefix.to_lowercase()) {
            m.summary.subject.clone()
        } else {
            format!("{prefix}{}", m.summary.subject)
        };
        let sender = sender_contact(m);
        let is_own = |c: &Contact| own.iter().any(|o| o.eq_ignore_ascii_case(&c.address));
        let to = if mode == Reply::Forward { String::new() } else { sender.formatted() };
        let cc = if mode == Reply::All {
            let mut others: Vec<Contact> = if m.to_full.is_empty() && m.cc_full.is_empty() {
                m.to.iter().filter(|t| t.as_str() != "me").map(|t| Contact::parse(t)).collect()
            } else {
                m.to_full.iter().chain(&m.cc_full).cloned().collect()
            };
            others.retain(|c| !is_own(c) && !c.address.eq_ignore_ascii_case(&sender.address));
            joined(&others)
        } else {
            String::new()
        };
        let text = match selected {
            Some(s) if mode != Reply::Forward && !s.trim().is_empty() => s.trim().to_string(),
            _ => m.paragraphs.join("\n\n"),
        };
        let body = if mode == Reply::Forward {
            let mut head = vec![
                "---------- Forwarded message ----------".to_string(),
                format!("From: {}", sender.formatted()),
                format!("Date: {}", m.sent),
                format!("Subject: {}", m.summary.subject),
            ];
            if !m.to.is_empty() {
                let to = if m.to_full.is_empty() { m.to.join(", ") } else { joined(&m.to_full) };
                head.push(format!("To: {to}"));
            }
            if !m.cc.is_empty() {
                let cc = if m.cc_full.is_empty() { m.cc.join(", ") } else { joined(&m.cc_full) };
                head.push(format!("Cc: {cc}"));
            }
            format!("{}\n\n{}\n\n{text}", signature_block(signature), head.join("\n"))
        } else {
            format!("{}\n\nOn {}, {} wrote:\n{}", signature_block(signature), m.sent, sender.formatted(), quote_lines(&text))
        };
        let (in_reply_to, references) = match (&m.message_id, mode) {
            (Some(id), Reply::One | Reply::All) => {
                let mut refs = m.references.clone();
                if !refs.contains(id) {
                    refs.push(id.clone());
                }
                (Some(id.clone()), refs)
            }
            _ => (None, Vec::new()),
        };
        let mut d = Draft {
            title: format!("{title} {}", m.summary.from),
            account: m.summary.account,
            to,
            cc,
            body,
            subject,
            attachments: if mode == Reply::Forward { attachments } else { Vec::new() },
            in_reply_to,
            references,
            signature: signature.to_string(),
            ..Draft::blank()
        };
        if mode == Reply::Forward {
            for (n, (_, data)) in m.html.inline.iter().enumerate() {
                let _ = d.insert_inline(&format!("image-{}", n + 1), data.to_vec(), None);
            }
        }
        d.fingerprint = d.content_hash();
        d
    }

    pub fn from_outgoing(out: Outgoing, signature: &str) -> Draft {
        let mut d = Draft {
            title: "draft".into(),
            account: out.account,
            to: out.to,
            cc: out.cc,
            subject: out.subject,
            body: out.body,
            attachments: out.attachments,
            in_reply_to: out.in_reply_to,
            references: out.references,
            message_id: out.message_id,
            bcc: out.bcc,
            agent: out.agent,
            signature: signature.to_string(),
            ..Draft::blank()
        };
        d.saved = d.message_id.is_some();
        d.fingerprint = d.content_hash();
        d
    }

    pub fn recipients(list: &str, input: &str) -> String {
        let mut all = split_recipients(list);
        all.extend(split_recipients(input));
        all.join(", ")
    }

    pub fn commit_inputs(&mut self) {
        self.to = Draft::recipients(&self.to, &self.to_input);
        self.cc = Draft::recipients(&self.cc, &self.cc_input);
        self.to_input.clear();
        self.cc_input.clear();
    }

    pub fn has_recipient(&self) -> bool {
        !self.to.trim().is_empty() || !self.to_input.trim().is_empty() || !self.cc.trim().is_empty() || !self.cc_input.trim().is_empty()
    }

    pub fn outgoing(&self) -> Outgoing {
        Outgoing {
            account: self.account,
            to: Draft::recipients(&self.to, &self.to_input),
            cc: Draft::recipients(&self.cc, &self.cc_input),
            subject: self.subject.clone(),
            body: self.body.clone(),
            attachments: self.attachments.clone(),
            in_reply_to: self.in_reply_to.clone(),
            references: self.references.clone(),
            message_id: self.message_id.clone(),
            bcc: self.bcc.clone(),
            agent: self.agent,
        }
    }

    pub fn attach_bytes(&mut self, name: &str, data: Vec<u8>) {
        let name = if name.is_empty() { "attachment" } else { name };
        self.attachments.push(OutAttachment { name: name.to_string(), mime: guess_mime(name).to_string(), data, cid: None });
    }

    pub fn insert_inline(&mut self, name: &str, data: Vec<u8>, at: Option<usize>) -> Result<Option<String>, String> {
        let prepared = crate::inline::prepare(data)?;
        let name = crate::inline::clean_name(name);
        let cid = crate::inline::new_cid();
        let reference = crate::inline::reference(&name, &cid);
        let chars = self.body.chars().count();
        let at = at.unwrap_or(chars).min(chars);
        let byte = self.body.char_indices().nth(at).map(|(i, _)| i).unwrap_or(self.body.len());
        self.body.insert_str(byte, &reference);
        self.attachments.push(OutAttachment { name, mime: prepared.mime.to_string(), data: prepared.data, cid: Some(cid) });
        Ok(prepared.note)
    }

    pub fn last_inline_cid(&self) -> Option<&str> {
        self.attachments.iter().rev().find_map(|a| a.cid.as_deref())
    }

    pub fn sync_inline(&mut self) {
        if self.attachments.iter().all(|a| a.cid.is_none()) && self.pruned.is_empty() {
            return;
        }
        let wanted = crate::inline::cids_in(&self.body);
        let (keep, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut self.attachments).into_iter().partition(|a| a.cid.as_ref().is_none_or(|c| wanted.contains(c)));
        self.attachments = keep;
        self.pruned.extend(gone);
        let (back, still): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pruned).into_iter().partition(|a| a.cid.as_ref().is_some_and(|c| wanted.contains(c)));
        self.pruned = still;
        self.attachments.extend(back);
    }

    pub fn remove_inline(&mut self, cid: &str) {
        if let Some(r) = crate::richtext::images(&self.body).into_iter().find(|r| r.cid == cid) {
            self.body.replace_range(r.start..r.end, "");
        }
        self.sync_inline();
    }

    pub fn inline_refs(&self) -> Vec<String> {
        crate::inline::cids_in(&self.body)
    }

    pub fn encoded_size(&self) -> usize {
        let raw: usize = self.body.len() + self.attachments.iter().map(|a| a.data.len()).sum::<usize>();
        raw / 3 * 4
    }

    pub fn attach_path(&mut self, path: &std::path::Path) -> Result<(), String> {
        let data = std::fs::read(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.attach_bytes(&name, data);
        Ok(())
    }

    pub fn set_account(&mut self, account: usize, signature: &str) {
        if account == self.account && signature == self.signature {
            return;
        }
        let old = signature_block(&self.signature);
        let new = signature_block(signature);
        if !old.is_empty() && self.body.contains(&old) {
            self.body = self.body.replacen(&old, &new, 1);
        } else if old.is_empty() && !new.is_empty() {
            self.body = format!("{}{new}", self.body.trim_end());
        }
        self.account = account;
        self.signature = signature.to_string();
    }

    pub fn is_empty(&self) -> bool {
        let untouched = signature_block(&self.signature);
        self.to.trim().is_empty()
            && self.cc.trim().is_empty()
            && self.to_input.trim().is_empty()
            && self.cc_input.trim().is_empty()
            && self.subject.trim().is_empty()
            && self.attachments.is_empty()
            && (self.body.trim().is_empty() || self.body.trim() == untouched.trim())
    }

    fn content_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&self.to, &self.cc, &self.to_input, &self.cc_input, &self.subject, &self.body, self.account).hash(&mut h);
        for a in &self.attachments {
            (&a.name, a.data.len(), &a.cid).hash(&mut h);
        }
        h.finish()
    }

    pub fn is_dirty(&self) -> bool {
        self.content_hash() != self.fingerprint
    }

    pub fn mark_saved(&mut self) {
        self.fingerprint = self.content_hash();
        self.changed_at = None;
        self.saved = true;
    }

    pub fn due_for_autosave(&mut self, after: std::time::Duration) -> bool {
        if !self.is_dirty() || self.is_empty() {
            self.changed_at = None;
            return false;
        }
        let since = *self.changed_at.get_or_insert_with(std::time::Instant::now);
        since.elapsed() >= after
    }
}

struct Snoozer {
    custom: String,
    whole: bool,
    error: bool,
}

struct Mover {
    query: String,
    selected: usize,
    whole: bool,
}

enum FolderDialog {
    Create { account: usize, prefix: String, name: String },
    Rename { folder: Folder, prefix: String, name: String },
    Delete { folder: Folder },
    Purge { ids: Vec<MessageId> },
}

fn fuzzy_score(query: &str, text: &str) -> Option<usize> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut at = 0;
    let mut score = 0;
    let mut last: Option<usize> = None;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let found = text[at..].iter().position(|c| *c == q)? + at;
        score += match last {
            Some(l) => found - l - 1,
            None => found * 2,
        };
        last = Some(found);
        at = found + 1;
    }
    Some(score + text.len() / 8)
}

struct Target {
    folder: Folder,
    label: String,
}

struct View {
    query: Query,
    counts: Counts,
    total: usize,
    current: Option<Message>,
    head: Option<Summary>,
    thread: Vec<ThreadItem>,
}

fn unsubscribe(u: Unsubscribe) -> String {
    if u.one_click {
        let target = u.target;
        std::thread::spawn(move || {
            let _ = ureq::post(&target).send_string("List-Unsubscribe=One-Click");
        });
        "Unsubscribe request sent".into()
    } else {
        match open::that(&u.target) {
            Ok(()) => "Unsubscribe page opened".into(),
            Err(_) => "Could not open the unsubscribe link".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PendingSend {
    pub id: MessageId,
    pub until: Instant,
    pub to: String,
}

pub struct App {
    pub store: Box<dyn MailStore>,
    pub folder: Folder,
    pub account: Option<usize>,
    pub filter: Filter,
    pub search: String,
    pub selected: usize,
    pub marked: std::collections::BTreeSet<usize>,
    anchor: usize,
    star_pending: bool,
    undo: Option<Vec<Restore>>,
    pub draft: Option<Draft>,
    pub notice: Option<String>,
    pub images_loaded: HashSet<MessageId>,
    pub headers_open: bool,
    pub rendered_rows: usize,
    pub reader_focus: bool,
    pub narrow: bool,
    pub folders_open: bool,
    pub force_failed: bool,
    mover: Option<Mover>,
    snoozer: Option<Snoozer>,
    dialog: Option<FolderDialog>,
    collapsed: HashSet<(usize, String)>,
    search_id: Option<egui::Id>,
    scroll_to_selected: bool,
    go_pending: bool,
    pub cards: crate::thread::OpenCards,
    pending_focus: Option<MessageId>,
    scroll_card: Option<MessageId>,
    base_focus: Option<MessageId>,
    snippets: std::collections::HashMap<MessageId, String>,
    on_screen: HashSet<MessageId>,
    last_prefetched: Option<MessageId>,
    dwell: std::collections::HashMap<MessageId, Instant>,
    dwell_skip: HashSet<MessageId>,
    mark_now: bool,
    last_head: Option<MessageId>,
    pub thread_focus: Option<MessageId>,
    pub settings_requested: bool,
    pub allow_open: bool,
    pub prefs: crate::prefs::Prefs,
    applied_theme: Option<crate::prefs::Theme>,
    pub open_dir: std::path::PathBuf,
    pub save_picker: SavePicker,
    pub opener: Box<dyn FnMut(&std::path::Path) -> bool>,
    previews: Option<(MessageId, Vec<Option<egui::TextureHandle>>)>,
    thumbs: std::collections::HashMap<String, egui::TextureHandle>,
    plain_inline_told: bool,
    pub viewing: Option<usize>,
    pub autosave: Duration,
    pub pending_send: Option<PendingSend>,
    picker: Option<Receiver<Vec<PickedFile>>>,
    save_result: Option<Receiver<String>>,
    html: crate::htmlview::Renderer,
    pub html_state: crate::kit::HtmlState,
    pub trusted: HashSet<String>,
    pub trust_saver: Box<dyn FnMut(&str)>,
    pub prefs_saver: Box<dyn FnMut(&Prefs)>,
    pub reader_pane_w: f32,
    pub reader_content_w: f32,
    side_epoch: u32,
    list_epoch: u32,
    link_warning: Option<(String, String, String)>,
    pub chats: Option<crate::chat::ChatView>,
    pub chat_opener: Option<Box<dyn FnMut() -> Option<crate::chat::ChatView>>>,
    pub chats_open: bool,
}

const NARROW_BELOW: f32 = 700.0;

struct DividerOutcome {
    width: u32,
    epoch: u32,
}

impl App {
    pub fn new(ctx: &egui::Context) -> App {
        crate::kit::setup(ctx);
        let mut app = App::with_store(Box::new(FakeStore::concept()));
        app.chat_opener = Some(Box::new(|| Some(crate::chat::ChatView::demo())));
        app
    }

    pub fn with_store(store: Box<dyn MailStore>) -> App {
        App {
            store,
            folder: Folder::Inbox,
            account: None,
            filter: Filter::All,
            search: String::new(),
            selected: 0,
            marked: Default::default(),
            anchor: 0,
            star_pending: false,
            undo: None,
            draft: None,
            notice: None,
            images_loaded: HashSet::new(),
            headers_open: false,
            rendered_rows: 0,
            reader_focus: false,
            narrow: false,
            folders_open: false,
            force_failed: false,
            mover: None,
            snoozer: None,
            dialog: None,
            collapsed: HashSet::new(),
            search_id: None,
            scroll_to_selected: false,
            go_pending: false,
            cards: Default::default(),
            pending_focus: None,
            scroll_card: None,
            base_focus: None,
            snippets: Default::default(),
            on_screen: HashSet::new(),
            last_prefetched: None,
            dwell: Default::default(),
            dwell_skip: HashSet::new(),
            mark_now: false,
            last_head: None,
            thread_focus: None,
            settings_requested: false,
            allow_open: true,
            prefs: Default::default(),
            applied_theme: None,
            open_dir: crate::attach::default_open_dir(),
            save_picker: rfd_picker(),
            opener: Box::new(|p| open::that(p).is_ok()),
            previews: None,
            thumbs: std::collections::HashMap::new(),
            plain_inline_told: false,
            viewing: None,
            autosave: AUTOSAVE,
            pending_send: None,
            picker: None,
            save_result: None,
            html: crate::htmlview::Renderer::default(),
            html_state: Default::default(),
            trusted: HashSet::new(),
            trust_saver: Box::new(|_| {}),
            prefs_saver: Box::new(|_| {}),
            reader_pane_w: 0.0,
            reader_content_w: 0.0,
            side_epoch: 0,
            list_epoch: 0,
            link_warning: None,
            chats: None,
            chat_opener: None,
            chats_open: false,
        }
    }

    pub fn query(&self) -> Query {
        Query { folder: self.folder.clone(), filter: self.filter, text: self.search.clone(), account: self.account }
    }

    fn view(&mut self) -> View {
        let query = self.query();
        let counts = self.store.counts(&query);
        let total = match self.filter {
            Filter::All => counts.all,
            Filter::Unread => counts.unread,
            Filter::Flagged => counts.flagged,
        };
        if total > 0 && self.selected >= total {
            self.selected = total - 1;
        }
        let head = self.store.list(&query, self.selected, 1).items.into_iter().next();
        let head_id = head.as_ref().map(|s| s.id);
        if head_id != self.last_head {
            self.thread_focus = None;
            self.dwell.clear();
            self.dwell_skip.clear();
            self.base_focus = None;
            self.last_head = head_id;
        }
        let thread = head_id.map(|id| self.store.thread(id)).unwrap_or_default();
        let ids: Vec<u64> = thread.iter().map(|t| t.summary.id.0).collect();
        if self.base_focus.is_none_or(|b| !thread.iter().any(|t| t.summary.id == b)) {
            self.base_focus = thread.iter().rev().find(|t| t.summary.unread).or(thread.last()).map(|t| t.summary.id).or(head_id);
        }
        self.cards.reset(head_id.map(|h| h.0), &ids, self.base_focus.map(|b| b.0));
        if let Some(p) = self.pending_focus.take() {
            if thread.iter().any(|t| t.summary.id == p) {
                self.thread_focus = Some(p);
                self.cards.open_one(p.0);
                self.scroll_card = Some(p);
            }
        }
        let shown = self
            .thread_focus
            .filter(|e| thread.iter().any(|t| t.summary.id == *e))
            .or(self.base_focus);
        let current = shown.and_then(|id| self.store.get(id).ok());
        if head_id != self.last_prefetched {
            self.last_prefetched = head_id;
            let around: Vec<MessageId> = [self.selected.checked_sub(1), Some(self.selected + 1).filter(|n| *n < total)]
                .into_iter()
                .flatten()
                .filter_map(|n| self.store.list(&query, n, 1).items.first().map(|s| s.id))
                .collect();
            self.store.prefetch(&around);
        }
        View { query, counts, total, current, head, thread }
    }

    fn pressed(ctx: &egui::Context, key: egui::Key) -> bool {
        ctx.input(|i| i.key_pressed(key) && !i.modifiers.command && !i.modifiers.alt)
    }

    fn key_actions(&mut self, ctx: &egui::Context) -> Vec<Action> {
        use egui::Key as K;
        if self.draft.is_some() || self.dialog.is_some() {
            return Vec::new();
        }
        if self.mover.is_some() || self.snoozer.is_some() {
            return Vec::new();
        }
        if Self::pressed(ctx, K::Escape) {
            return vec![Action::Back];
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, K::F)) {
            return vec![Action::Find];
        }
        if ctx.egui_wants_keyboard_input() && !ctx.memory(|m| m.focused()).is_some_and(|f| f == egui::Id::new(PLAIN_BODY)) {
            return Vec::new();
        }
        if Self::pressed(ctx, crate::prefs::SETTINGS_KEY) {
            self.settings_requested = true;
            return Vec::new();
        }
        let plain = ctx.input(|i| !i.modifiers.shift);
        let keys = &self.prefs.keys;
        let table: [(K, Action); 17] = [
            (keys.key_for("next"), Action::Next),
            (K::ArrowDown, Action::Next),
            (keys.key_for("previous"), Action::Previous),
            (K::ArrowUp, Action::Previous),
            (keys.key_for("open"), Action::Open),
            (keys.key_for("archive"), Action::Archive),
            (keys.key_for("snooze"), Action::Snooze),
            (keys.key_for("flag"), Action::ToggleFlag),
            (keys.key_for("unread"), Action::ToggleUnread),
            (keys.key_for("reply"), Action::Reply(Reply::One)),
            (keys.key_for("reply_all"), Action::Reply(Reply::All)),
            (keys.key_for("forward"), Action::Reply(Reply::Forward)),
            (keys.key_for("compose"), Action::Compose),
            (keys.key_for("move"), Action::OpenMove),
            (keys.key_for("search"), Action::Search),
            (keys.key_for("reader_width"), Action::ToggleReaderWidth),
            (keys.key_for("account_only"), Action::AccountOnly(None)),
        ];
        let snooze_key = keys.key_for("snooze");
        let mut out = Vec::new();
        if self.star_pending {
            if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))) {
                self.star_pending = false;
                if Self::pressed(ctx, K::A) {
                    return vec![Action::SelectAll];
                }
                if Self::pressed(ctx, K::N) {
                    return vec![Action::SelectNone];
                }
            }
        } else if ctx.input(|i| {
            i.events.iter().any(|e| match e {
                egui::Event::Text(t) => t == "*",
                egui::Event::Key { key: K::Num8, pressed: true, modifiers, .. } => modifiers.shift,
                _ => false,
            })
        }) {
            self.star_pending = true;
            return out;
        }
        if self.prefs.developer_chat && Self::pressed(ctx, prefs::CHATS_KEY) {
            return vec![Action::OpenChats];
        }
        if !ctx.input(|i| i.modifiers.shift) && Self::pressed(ctx, K::Z) {
            return vec![Action::Undo];
        }
        if ctx.input(|i| i.modifiers.shift && !i.modifiers.command && !i.modifiers.alt && i.key_pressed(K::I)) {
            return vec![Action::MarkAllRead(self.folder.clone())];
        }
        if !plain {
            if Self::pressed(ctx, K::J) || Self::pressed(ctx, K::ArrowDown) {
                return vec![Action::ExtendNext];
            }
            if Self::pressed(ctx, K::K) || Self::pressed(ctx, K::ArrowUp) {
                return vec![Action::ExtendPrevious];
            }
        }
        if self.go_pending {
            if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))) {
                self.go_pending = false;
                if Self::pressed(ctx, K::A) {
                    return vec![Action::GoTo(Folder::Archive)];
                }
                if Self::pressed(ctx, K::I) {
                    return vec![Action::GoTo(Folder::Inbox)];
                }
            }
        } else if Self::pressed(ctx, K::G) {
            self.go_pending = true;
            return out;
        }
        for (key, action) in table {
            if key == snooze_key && !plain {
                continue;
            }
            if !plain && matches!(key, K::J | K::K | K::ArrowDown | K::ArrowUp) {
                continue;
            }
            if Self::pressed(ctx, key) {
                out.push(action);
            }
        }
        if plain && Self::pressed(ctx, K::O) {
            out.push(Action::OpenAttachment(0));
        }
        if self.reader_focus {
            if Self::pressed(ctx, K::N) {
                out.push(Action::ListNext);
            }
            if Self::pressed(ctx, K::P) {
                out.push(Action::ListPrevious);
            }
            if Self::pressed(ctx, crate::prefs::EXPAND_ALL_KEY) {
                out.push(Action::ExpandAll);
            }
            if Self::pressed(ctx, crate::prefs::COLLAPSE_ALL_KEY) {
                out.push(Action::CollapseAll);
            }
        }
        if !plain && Self::pressed(ctx, K::S) {
            out.push(Action::SaveAllAttachments);
        }
        if plain && Self::pressed(ctx, K::Num3) && ctx.input(|i| i.modifiers.shift) {
            out.push(Action::Trash);
        }
        if Self::pressed(ctx, K::Delete) {
            out.push(Action::Trash);
        }
        out
    }

    fn targets(v: &View, whole: bool, same_folder: bool) -> Vec<(MessageId, Folder)> {
        if whole && !v.thread.is_empty() {
            let row = v.query.folder.clone();
            return v
                .thread
                .iter()
                .filter(|t| !same_folder || t.folder == row)
                .map(|t| (t.summary.id, t.folder.clone()))
                .collect();
        }
        v.current.iter().map(|m| (m.summary.id, m.folder.clone())).collect()
    }

    fn marked_ids(&self, v: &View) -> Vec<MessageId> {
        let mut ids = Vec::new();
        for i in &self.marked {
            let Some(row) = self.store.list(&v.query, *i, 1).items.into_iter().next() else { continue };
            let thread = self.store.thread(row.id);
            if thread.is_empty() {
                ids.push(row.id);
            }
            for t in thread.iter().filter(|t| t.folder == v.query.folder) {
                if !ids.contains(&t.summary.id) {
                    ids.push(t.summary.id);
                }
            }
        }
        ids
    }

    fn act_ids(&self, v: &View, whole: bool, same_folder: bool) -> Vec<MessageId> {
        if !self.marked.is_empty() {
            return self.marked_ids(v);
        }
        Self::targets(v, whole, same_folder).into_iter().map(|(id, _)| id).collect()
    }

    fn move_current(&mut self, v: &View, to: Folder, done: &str, whole: bool) {
        let ids = self.act_ids(v, whole, true);
        let mut before = self.store.snapshot(&ids);
        before.retain(|r| r.folder != to);
        let moving: Vec<MessageId> = before.iter().map(|r| r.id).collect();
        if moving.is_empty() {
            return;
        }
        for r in before.iter_mut() {
            r.moved_to = Some(to.clone());
        }
        match self.store.bulk_move(&moving, to) {
            Ok(()) => {
                self.notice = Some(if moving.len() > 1 { format!("{done} ({})", moving.len()) } else { done.into() });
                self.undo = Some(before);
                self.marked.clear();
            }
            Err(e) => self.notice = Some(format!("Could not move the message: {e:?}")),
        }
    }

    fn change_flags(&mut self, ids: &[MessageId], change: FlagChange, said: &str) {
        let before = self.store.snapshot(ids);
        if self.store.bulk_flags(ids, change).is_ok() {
            self.undo = Some(before);
            self.notice = Some(said.into());
        }
    }

    fn undo_last(&mut self) {
        let Some(items) = self.undo.take() else {
            self.notice = Some("Nothing to undo".into());
            return;
        };
        self.notice = Some(match self.store.restore(&items) {
            Ok(()) => "Undone".into(),
            Err(e) => format!("Could not undo: {e:?}"),
        });
        self.marked.clear();
    }

    fn mark_shown_read(&mut self, ctx: &egui::Context, v: &View) {
        let Some(after) = self.prefs.reading.mark_read_after() else {
            self.mark_now = false;
            return;
        };
        let Some(m) = &v.current else { return };
        let id = m.summary.id;
        if self.mark_now {
            self.mark_now = false;
            if m.summary.unread && !self.dwell_skip.contains(&id) {
                let _ = self.store.set_flags(id, FlagChange { unread: Some(false), flagged: None });
            }
        }
        let mut wait: Option<Duration> = None;
        let visible: Vec<MessageId> = self.on_screen.iter().copied().collect();
        self.dwell.retain(|k, _| visible.contains(k));
        for id in visible {
            let unread = v.thread.iter().find(|t| t.summary.id == id).is_some_and(|t| t.summary.unread);
            if !unread || self.dwell_skip.contains(&id) {
                continue;
            }
            let since = self.dwell.entry(id).or_insert_with(Instant::now).elapsed();
            if since >= after {
                self.dwell.remove(&id);
                let _ = self.store.set_flags(id, FlagChange { unread: Some(false), flagged: None });
            } else {
                wait = Some(wait.map_or(after - since, |w| w.min(after - since)));
            }
        }
        if let Some(w) = wait {
            ctx.request_repaint_after(w);
        }
    }

    fn extend_to(&mut self, to: usize) {
        if self.marked.is_empty() {
            self.anchor = self.selected;
        }
        self.selected = to;
        let (a, b) = (self.anchor.min(to), self.anchor.max(to));
        self.marked = (a..=b).collect();
        self.scroll_to_selected = true;
    }

    fn apply(&mut self, ctx: &egui::Context, actions: Vec<Action>) {
        for a in actions {
            let v = self.view();
            let (a, whole) = match a {
                Action::One(inner) => (*inner, false),
                a => (a, true),
            };
            match a {
                Action::One(_) => {}
                Action::Expand(id) => {
                    self.cards.open_one(id.0);
                    self.thread_focus = Some(id);
                    self.scroll_card = Some(id);
                }
                Action::ToggleCard(id) => {
                    self.cards.toggle(id.0);
                    self.thread_focus = Some(id);
                    if self.cards.is_open(id.0) {
                        self.scroll_card = Some(id);
                    }
                }
                Action::ExpandAll => {
                    let ids: Vec<u64> = v.thread.iter().map(|t| t.summary.id.0).collect();
                    self.cards.expand_all(&ids);
                }
                Action::CollapseAll => self.cards.collapse_all(),
                Action::ToggleCardQuotes(id) => self.cards.toggle_quotes(id.0),
                Action::FocusThread(id) => self.thread_focus = Some(id),
                Action::EditDraft(id) => self.open_draft(id),
                Action::ListNext => {
                    if self.selected + 1 < v.total {
                        self.selected += 1;
                    }
                    self.scroll_to_selected = true;
                }
                Action::ListPrevious => {
                    self.selected = self.selected.saturating_sub(1);
                    self.scroll_to_selected = true;
                }
                Action::Next | Action::Previous if self.reader_focus && v.thread.len() > 1 => {
                    let shown = v.current.as_ref().map(|m| m.summary.id);
                    let at = self
                        .thread_focus
                        .or(shown)
                        .and_then(|f| v.thread.iter().position(|t| t.summary.id == f))
                        .unwrap_or(v.thread.len() - 1);
                    let to = if a == Action::Next { (at + 1).min(v.thread.len() - 1) } else { at.saturating_sub(1) };
                    self.thread_focus = Some(v.thread[to].summary.id);
                    self.scroll_card = self.thread_focus;
                }
                Action::Next => {
                    if self.selected + 1 < v.total {
                        self.selected += 1;
                    }
                    self.scroll_to_selected = true;
                }
                Action::Previous => {
                    self.selected = self.selected.saturating_sub(1);
                    self.scroll_to_selected = true;
                }
                Action::Open if v.current.as_ref().is_some_and(|m| m.folder == Folder::Drafts) => {
                    if let Some(m) = &v.current {
                        self.open_draft(m.summary.id);
                    }
                }
                Action::Open if self.reader_focus && v.thread.len() > 1 => {
                    if let Some(id) = self.thread_focus.or(v.current.as_ref().map(|m| m.summary.id)) {
                        self.cards.toggle(id.0);
                        self.thread_focus = Some(id);
                    }
                }
                Action::Open => {
                    self.mark_now = true;
                    self.reader_focus = true;
                }
                Action::ExtendNext => {
                    let to = (self.selected + 1).min(v.total.saturating_sub(1));
                    self.extend_to(to);
                }
                Action::ExtendPrevious => self.extend_to(self.selected.saturating_sub(1)),
                Action::SelectAll => {
                    self.marked = (0..v.total).collect();
                    self.anchor = 0;
                    self.notice = Some(format!("{} selected", v.total));
                }
                Action::SelectNone => self.marked.clear(),
                Action::OpenChats => self.open_chats(),
                Action::Undo => self.undo_last(),
                Action::MarkAllRead(f) => match self.store.mark_all_read_in(&f, self.account) {
                    Ok(before) => {
                        self.notice = Some(format!("Marked {} as read", before.len()));
                        self.undo = Some(before);
                    }
                    Err(e) => self.notice = Some(format!("Could not mark as read: {e:?}")),
                },
                Action::Archive => self.move_current(&v, Folder::Archive, "Archived", whole),
                Action::Snooze => {
                    if v.current.is_some() || !self.marked.is_empty() {
                        self.snoozer = Some(Snoozer { custom: String::new(), whole: whole && !self.reader_focus, error: false });
                        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(t) if t.eq_ignore_ascii_case("s"))));
                    }
                }
                Action::SnoozeUntil(at, whole) => {
                    let ids = self.act_ids(&v, whole, true);
                    let mut before = self.store.snapshot(&ids);
                    before.retain(|r| r.folder != Folder::Snoozed);
                    let moving: Vec<MessageId> = before.iter().map(|r| r.id).collect();
                    for r in before.iter_mut() {
                        r.moved_to = Some(Folder::Snoozed);
                    }
                    if !moving.is_empty() {
                        match self.store.bulk_snooze(&moving, at) {
                            Ok(()) => {
                                self.notice = Some(format!("Snoozed until {}", crate::snooze::describe(at)));
                                self.undo = Some(before);
                                self.marked.clear();
                            }
                            Err(e) => self.notice = Some(format!("Could not snooze the message: {e:?}")),
                        }
                    }
                }
                Action::Junk => self.move_current(&v, Folder::Junk, "Moved to junk", whole),
                Action::Trash if self.folder == Folder::Trash => {
                    let ids = self.act_ids(&v, whole, true);
                    if !ids.is_empty() {
                        self.dialog = Some(FolderDialog::Purge { ids });
                    }
                }
                Action::Trash => self.move_current(&v, Folder::Trash, "Moved to trash", whole),
                Action::ToggleFlag => {
                    let current = if whole { v.head.as_ref() } else { v.current.as_ref().map(|m| &m.summary) };
                    if let Some(s) = current {
                        let on = !s.flagged;
                        let ids = self.act_ids(&v, whole, false);
                        self.change_flags(&ids, FlagChange { unread: None, flagged: Some(on) }, if on { "Flagged" } else { "Flag removed" });
                    }
                }
                Action::ToggleUnread => {
                    if let Some(m) = &v.current {
                        self.dwell_skip.insert(m.summary.id);
                    }
                    let current = if whole { v.head.as_ref() } else { v.current.as_ref().map(|m| &m.summary) };
                    if let Some(s) = current {
                        let ids = self.act_ids(&v, whole, false);
                        let unread = if self.marked.is_empty() { !s.unread } else { !self.store.snapshot(&ids).iter().any(|r| r.unread) };
                        self.change_flags(&ids, FlagChange { unread: Some(unread), flagged: None }, if unread { "Marked unread" } else { "Marked read" });
                    }
                }
                Action::Reply(mode) => {
                    let focused = self.thread_focus.filter(|f| v.thread.iter().any(|t| t.summary.id == *f)).and_then(|f| self.store.get(f).ok());
                    if let Some(m) = focused.as_ref().or(v.current.as_ref()) {
                        let accounts = self.store.accounts();
                        let own: Vec<String> = accounts.iter().map(|a| a.address.clone()).collect();
                        let signature = accounts.get(m.summary.account).map(|a| a.signature.clone()).unwrap_or_default();
                        let attachments = if mode == Reply::Forward { self.store.attachments(m.summary.id).unwrap_or_default().into_iter().map(|a| OutAttachment { name: a.name, mime: a.mime, data: a.data.to_vec(), cid: None }).collect() } else { Vec::new() };
                        let picked = self.html_state.selected_text.as_ref().filter(|(id, _)| *id == m.summary.id.0).map(|(_, t)| t.clone());
                        self.draft = Some(Draft::reply(m, mode, &signature, &own, attachments, picked.as_deref()));
                    }
                }
                Action::Compose => {
                    let account = v.current.as_ref().map(|m| m.summary.account).unwrap_or(0);
                    let signature = self.store.accounts().get(account).map(|a| a.signature.clone()).unwrap_or_default();
                    self.draft = Some(Draft::blank_for(account, &signature));
                }
                Action::Search => {
                    if let Some(id) = self.search_id {
                        ctx.memory_mut(|m| m.request_focus(id));
                        ctx.input_mut(|i| {
                            i.events.retain(|e| !matches!(e, egui::Event::Text(t) if t == "/"))
                        });
                    }
                }
                Action::Back if self.link_warning.is_some() => self.link_warning = None,
                Action::Back if self.html_state.find.is_some() => self.html_state.find = None,
                Action::Back => {
                    if let Some(id) = self.search_id {
                        ctx.memory_mut(|m| m.surrender_focus(id));
                    }
                    if self.reader_focus {
                        self.reader_focus = false;
                    } else if !self.search.is_empty() {
                        self.search.clear();
                        self.selected = 0;
                    } else if self.account.is_some() {
                        self.account = None;
                        self.selected = 0;
                    }
                }
                Action::LoadImagesOnce => {
                    if let Some(m) = &v.current {
                        self.images_loaded.insert(m.summary.id);
                    }
                }
                Action::TrustImages(key) => {
                    (self.trust_saver)(&key);
                    self.trusted.insert(key);
                }
                Action::Find => {
                    if v.current.as_ref().is_some_and(|m| !m.html.is_empty()) {
                        let find = self.html_state.find.get_or_insert_with(Default::default);
                        find.focus = true;
                    }
                }
                Action::WarnLink(href, shown, real) => self.link_warning = Some((href, shown, real)),
                Action::CloseLinkWarning => self.link_warning = None,
                Action::ToggleReaderWidth => {
                    self.prefs.reading.reader_width = self.prefs.reading.reader_width.toggled();
                    (self.prefs_saver)(&self.prefs);
                }
                Action::ToggleQuotes => {}
                Action::ToggleHeaders => self.headers_open = !self.headers_open,
                Action::OpenLink(link) => {
                    let _ = open::that(link);
                }
                Action::Unsubscribe => {
                    if let Some(u) = v.current.as_ref().and_then(|m| m.headers.unsubscribe.clone()) {
                        self.notice = Some(unsubscribe(u));
                    }
                }
                Action::GoTo(f) => {
                    if self.account.take().is_some() {
                        self.selected = 0;
                    }
                    self.go_folder(f);
                }
                Action::GoToIn(account, f) => {
                    self.account = Some(account);
                    self.folder = f;
                    self.selected = 0;
                    self.filter = Filter::All;
                }
                Action::AccountOnly(target) => {
                    let target = target.or_else(|| v.head.as_ref().map(|h| h.account));
                    self.account = if self.account.is_some() && (target.is_none() || self.account == target) { None } else { target };
                    self.selected = 0;
                }
                Action::ClearAccount => {
                    self.account = None;
                    self.selected = 0;
                }
                Action::Label(name) => {
                    self.search = if name.contains(' ') { format!("label:\"{name}\"") } else { format!("label:{name}") };
                    self.selected = 0;
                    self.filter = Filter::All;
                }
                Action::OpenAttachment(i) => self.open_attachment(&v, i),
                Action::SaveAttachment(i) => self.save_attachment(&v, i),
                Action::SaveAllAttachments => self.save_all(&v),
                Action::ViewImage(i) => self.viewing = Some(i),
                Action::CloseViewer => self.viewing = None,
                Action::OpenMove => {
                    if v.current.is_some() {
                        self.mover = Some(Mover { query: String::new(), selected: 0, whole: whole && !self.reader_focus });
                        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(t) if t.eq_ignore_ascii_case("m"))));
                    }
                }
                Action::MoveTo(f, whole) => {
                    let said = format!("Moved to {}", f.name());
                    self.move_current(&v, f, &said, whole);
                }
                Action::ToggleBranch(account, path) => {
                    if !self.collapsed.remove(&(account, path.clone())) {
                        self.collapsed.insert((account, path));
                    }
                }
                Action::NewFolder(account, prefix) => {
                    self.dialog = Some(FolderDialog::Create { account, prefix, name: String::new() });
                }
                Action::RenameFolder(folder, prefix) => {
                    let name = folder.name().strip_prefix(prefix.as_str()).unwrap_or(folder.name()).to_string();
                    self.dialog = Some(FolderDialog::Rename { folder, prefix, name });
                }
                Action::DeleteFolder(folder) => self.dialog = Some(FolderDialog::Delete { folder }),
            }
            ctx.request_repaint();
        }
    }

    fn attachment_target(&self, v: &View) -> Option<MessageId> {
        let id = self
            .thread_focus
            .filter(|f| self.reader_focus && v.thread.iter().any(|t| t.summary.id == *f))
            .or_else(|| v.current.as_ref().map(|m| m.summary.id))?;
        let has = match &v.current {
            Some(m) if m.summary.id == id => !m.attachments.is_empty(),
            _ => self.store.get(id).is_ok_and(|m| !m.attachments.is_empty()),
        };
        has.then_some(id)
    }

    fn attachment_data(&mut self, v: &View) -> Option<Vec<AttachmentData>> {
        let id = self.attachment_target(v)?;
        match self.store.attachments(id) {
            Ok(items) if !items.is_empty() => Some(items),
            Ok(_) => None,
            Err(e) => {
                self.notice = Some(format!("Could not read the attachment: {e:?}"));
                None
            }
        }
    }

    fn save_attachment(&mut self, v: &View, index: usize) {
        let Some(items) = self.attachment_data(v) else { return };
        let Some(item) = items.get(index) else { return };
        let name = crate::attach::safe_name(&item.name, "attachment");
        #[cfg(target_os = "android")]
        {
            self.save_result = Some(save_document_async(name.clone(), item.data.as_ref().clone(), "Saved the attachment"));
            return;
        }
        #[cfg(not(target_os = "android"))]
        {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_string()).unwrap_or_default();
        let Some(path) = (self.save_picker)(&name, &ext) else { return };
        self.notice = Some(match std::fs::write(&path, item.data.as_slice()) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(e) => format!("Could not save: {e}"),
        });
        }
    }

    fn save_all(&mut self, v: &View) {
        let Some(items) = self.attachment_data(v) else { return };
        #[cfg(target_os = "android")]
        {
            match crate::attach::zip_all(&items) {
                Ok(bytes) => self.save_result = Some(save_document_async("attachments.zip".into(), bytes, "Saved the attachments")),
                Err(e) => self.notice = Some(format!("Could not save: {e}")),
            }
        }
        #[cfg(not(target_os = "android"))]
        {
            let Some(path) = (self.save_picker)("attachments.zip", "zip") else { return };
            self.notice = Some(match crate::attach::zip_all(&items).and_then(|bytes| std::fs::write(&path, bytes)) {
                Ok(()) => format!("Saved {} attachments to {}", items.len(), path.display()),
                Err(e) => format!("Could not save: {e}"),
            });
        }
    }

    fn open_attachment(&mut self, v: &View, index: usize) {
        if !self.allow_open {
            if self.attachment_target(v).is_some() {
                self.notice = Some("Opening attachments is turned off in settings. Use save as instead.".into());
            }
            return;
        }
        let Some(items) = self.attachment_data(v) else { return };
        let Some(item) = items.get(index) else { return };
        self.notice = Some(match crate::attach::write_private(&self.open_dir, &item.name, &item.data) {
            Ok(path) => {
                if (self.opener)(&path) {
                    format!("Opened a plain copy of {} at {}. It is deleted when mailbench closes.", item.name, path.display())
                } else {
                    format!("No program could open {}. A plain copy is at {} until mailbench closes.", item.name, path.display())
                }
            }
            Err(e) => format!("Could not write the copy: {e}"),
        });
    }

    fn sync_previews(&mut self, ctx: &egui::Context, m: &Message) {
        if self.previews.as_ref().is_some_and(|(id, _)| *id == m.summary.id) {
            return;
        }
        self.viewing = None;
        if !m.attachments.iter().any(|a| crate::attach::is_previewable(&a.mime, &a.name)) {
            self.previews = Some((m.summary.id, Vec::new()));
            return;
        }
        let data = self.store.attachments(m.summary.id).unwrap_or_default();
        let textures = data
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let image = crate::attach::preview_image(&d.mime, &d.name, &d.data)?;
                Some(ctx.load_texture(format!("attachment-{}-{i}", m.summary.id.0), image, egui::TextureOptions::LINEAR))
            })
            .collect();
        self.previews = Some((m.summary.id, textures));
    }

    fn preview_texture(&self, index: usize) -> Option<&egui::TextureHandle> {
        self.previews.as_ref()?.1.get(index)?.as_ref()
    }

    fn previews_ui(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let Some((_, textures)) = &self.previews else { return };
        for (i, t) in textures.iter().enumerate() {
            let Some(t) = t else { continue };
            let size = t.size_vec2();
            let scale = (360.0 / size.x).min(240.0 / size.y).min(1.0);
            let resp = ui.add(egui::Image::new(t).fit_to_exact_size(size * scale).sense(egui::Sense::click()));
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                actions.push(Action::ViewImage(i));
            }
        }
    }

    fn viewer(&self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(i) = self.viewing else { return };
        let Some(t) = self.preview_texture(i) else { return };
        let r = egui::Modal::new(egui::Id::new("image-viewer"))
            .backdrop_color(egui::Color32::from_black_alpha(200))
            .show(ctx, |ui| {
                egui::ScrollArea::both().max_height(ctx.content_rect().height() * 0.8).max_width(ctx.content_rect().width() * 0.8).show(ui, |ui| {
                    ui.add(egui::Image::new(t).fit_to_original_size(1.0));
                });
                if ui.add(Key::new("close").outline(VALUE)).clicked() {
                    actions.push(Action::CloseViewer);
                }
            });
        if r.should_close() {
            actions.push(Action::CloseViewer);
        }
    }

    fn go_folder(&mut self, f: Folder) {
        if f != self.folder {
            self.folder = f;
            self.selected = 0;
            self.filter = Filter::All;
        }
    }

    pub fn apply_theme(&mut self, ctx: &egui::Context) {
        if self.applied_theme != Some(self.prefs.theme) {
            ctx.set_theme(self.prefs.theme.preference());
            self.applied_theme = Some(self.prefs.theme);
        }
    }

    pub fn open_chats(&mut self) {
        if !self.prefs.developer_chat {
            return;
        }
        if self.chats.is_none() {
            self.chats = self.chat_opener.as_mut().and_then(|open| open());
        }
        self.chats_open = self.chats.is_some();
    }

    pub fn show(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.apply_theme(&ctx);
        if !self.prefs.developer_chat {
            self.chats_open = false;
        }
        if self.chats_open {
            if let Some(chats) = &mut self.chats {
                let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
                if esc && chats.back() == crate::chat::ChatExit::Mail {
                    self.chats_open = false;
                } else if chats.show(ui) == crate::chat::ChatExit::Mail {
                    self.chats_open = false;
                }
                return;
            }
        }
        self.follow_notification();
        self.poll_save_result(&ctx);
        let keys = self.key_actions(&ctx);
        self.apply(&ctx, keys);
        let v = self.view();
        self.mark_shown_read(&ctx, &v);
        let accounts = self.store.accounts();
        let failed = self.force_failed || accounts.iter().any(|a| a.sync == SyncState::Failed);
        let mut actions = Vec::new();
        self.narrow = ui.available_width() < NARROW_BELOW;
        let account_before = self.account;
        let folder_before = self.folder.clone();
        self.header(ui, &accounts, failed, &mut actions);
        if !self.narrow {
            self.hint_bar(ui);
        }
        if self.narrow {
            self.show_narrow(ui, &v, &accounts, &mut actions);
        } else {
            self.show_wide(ui, &v, &accounts, &mut actions);
        }
        self.composer(&ctx, &accounts);
        self.viewer(&ctx, &mut actions);
        self.move_picker(&ctx, v.current.as_ref(), &mut actions);
        self.link_warning_dialog(&ctx, &mut actions);
        self.snooze_picker(&ctx, &mut actions);
        self.folder_dialog(&ctx);
        self.undo_toast(&ctx);
        self.apply(&ctx, actions);
        if self.narrow && (self.folder != folder_before || self.account != account_before) {
            self.folders_open = false;
        }
    }

    fn show_narrow(&mut self, ui: &mut Ui, v: &View, accounts: &[AccountInfo], actions: &mut Vec<Action>) {
        egui::CentralPanel::no_frame().show(ui, |ui| {
            pane_fill(ui, CHASSIS);
            if self.folders_open {
                egui::Frame::NONE.inner_margin(Margin::same(8)).show(ui, |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.side(ui, accounts, actions));
                });
            } else if self.reader_focus && v.current.is_some() {
                egui::Frame::NONE.inner_margin(Margin { left: 12, right: 12, top: 8, bottom: 8 }).show(ui, |ui| {
                    if ui.add(Key::new("back to list").outline(VALUE)).clicked() {
                        self.reader_focus = false;
                    }
                    ui.add_space(6.0);
                    self.reader(ui, v, accounts, actions);
                });
            } else {
                self.list(ui, v, accounts, actions);
            }
        });
    }

    fn show_wide(&mut self, ui: &mut Ui, v: &View, accounts: &[AccountInfo], actions: &mut Vec<Action>) {
        let side = self.divider_panel("side", self.side_epoch, self.prefs.layout.side_w, prefs::SIDE_DEFAULT, prefs::SIDE_RANGE, true, ui, |this, ui| {
            this.side(ui, accounts, actions)
        });
        self.side_epoch = side.epoch;
        let list = self.divider_panel("list", self.list_epoch, self.prefs.layout.list_w, prefs::LIST_DEFAULT, prefs::LIST_RANGE, false, ui, |this, ui| {
            this.list(ui, v, accounts, actions)
        });
        self.list_epoch = list.epoch;
        let layout = prefs::LayoutPrefs { side_w: side.width, list_w: list.width };
        if layout != self.prefs.layout {
            self.prefs.layout = layout;
            (self.prefs_saver)(&self.prefs);
        }
        egui::CentralPanel::no_frame().show(ui, |ui| {
            pane_fill(ui, CHASSIS);
            egui::Frame::NONE.inner_margin(Margin { left: 22, right: 22, top: 16, bottom: 10 }).show(ui, |ui| {
                self.reader(ui, v, accounts, actions);
            });
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn divider_panel(
        &mut self,
        name: &'static str,
        epoch: u32,
        width: u32,
        default: u32,
        range: (f32, f32),
        padded: bool,
        ui: &mut Ui,
        add: impl FnOnce(&mut Self, &mut Ui),
    ) -> DividerOutcome {
        let frame = if padded {
            egui::Frame::NONE.fill(CHASSIS).inner_margin(Margin::same(8))
        } else {
            egui::Frame::NONE.fill(CHASSIS)
        };
        let shown = egui::Panel::left((name, epoch))
            .default_size(width as f32)
            .size_range(range.0..=range.1)
            .resizable(true)
            .frame(frame)
            .show(ui, |ui| add(self, ui));
        let rect = shown.response.rect;
        let edge = Rect::from_x_y_ranges(rect.right() - 4.0..=rect.right() + 16.0, rect.y_range());
        let reset = ui.interact(edge, ui.scope_id().with(("divider-reset", name)), egui::Sense::click()).double_clicked();
        if reset {
            return DividerOutcome { width: default, epoch: epoch + 1 };
        }
        DividerOutcome { width: rect.width().round() as u32, epoch }
    }

    fn header(&mut self, ui: &mut Ui, accounts: &[AccountInfo], failed: bool, actions: &mut Vec<Action>) {
        if self.narrow {
            self.narrow_header(ui, accounts, actions);
            return;
        }
        egui::Panel::top("head").frame(header_frame()).show(ui, |ui| {
            ui.horizontal(|ui| {
                title(ui);
                ui.add_space(18.0);
                ui.allocate_ui(egui::vec2(360.0, KEY_H), |ui| {
                    let r = field(ui, &mut self.search, "search all accounts, press /");
                    self.search_id = Some(r.id);
                    if r.changed() {
                        self.selected = 0;
                    }
                });
                ui.add_space(8.0);
                self.account_chip(ui, accounts, actions);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(Key::new("compose").solid(READOUT).min_width(100.0)).clicked() {
                        actions.push(Action::Compose);
                    }
                    ui.add_space(8.0);
                    if ui.add(Key::new("settings").outline(VALUE).min_width(90.0)).clicked() {
                        self.settings_requested = true;
                    }
                    ui.add_space(8.0);
                    if failed {
                        let n = accounts.iter().filter(|a| a.sync == SyncState::Failed).count().max(1);
                        let text = if n == 1 { "1 account needs you".to_string() } else { format!("{n} accounts need you") };
                        chip(ui, &text, FAULT, true);
                    } else {
                        let idle = accounts.iter().filter(|a| a.sync == SyncState::Idle).count();
                        let syncing = accounts.iter().any(|a| a.sync == SyncState::Syncing);
                        let text = if syncing && idle < accounts.len() {
                            format!("syncing, idle on {} of {}", idle, accounts.len())
                        } else {
                            format!("idle on {} of {}", idle, accounts.len())
                        };
                        chip(ui, &text, TRACE, true);
                    }
                });
            });
        });
    }

    fn account_chip(&self, ui: &mut Ui, accounts: &[AccountInfo], actions: &mut Vec<Action>) {
        let Some(info) = self.account.and_then(|a| accounts.get(a)) else { return };
        let text = format!("showing {} only, click to clear", info.name);
        let h = if self.narrow { 44.0 } else { KEY_H };
        ui.scope(|ui| {
            ui.set_min_height(h);
            if chip_button(ui, &text, READOUT).clicked() {
                actions.push(Action::ClearAccount);
            }
        });
    }

    fn narrow_header(&mut self, ui: &mut Ui, accounts: &[AccountInfo], actions: &mut Vec<Action>) {
        egui::Panel::top("head").frame(header_frame()).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.add(Key::new("folders").outline(VALUE)).clicked() {
                    self.folders_open = !self.folders_open;
                }
                if ui.add(Key::new("+").solid(READOUT)).clicked() {
                    actions.push(Action::Compose);
                }
                if ui.add(Key::new("set").outline(VALUE)).clicked() {
                    self.settings_requested = true;
                }
            });
            ui.add_space(4.0);
            self.account_chip(ui, accounts, actions);
            let r = field(ui, &mut self.search, "search, press /");
            self.search_id = Some(r.id);
            if r.changed() {
                self.selected = 0;
                self.reader_focus = false;
            }
        });
    }

    fn hint_bar(&self, ui: &mut Ui) {
        egui::Panel::bottom("foot").frame(footer_frame()).show(ui, |ui| {
            let (text, tint) = match &self.notice {
                Some(n) => (n.as_str(), READOUT),
                None => ("synced 09:41", TRACE),
            };
            let count = self.marked.len().to_string();
            let mut items: Vec<(String, String)> = Vec::new();
            if self.draft.is_some() {
                let send = if cfg!(target_os = "macos") { "Cmd Enter" } else { "Ctrl Enter" };
                items.push((send.to_string(), "send".into()));
                items.push(("Enter".to_string(), "new line".into()));
                items.push(("Tab".to_string(), "next field".into()));
                let items: Vec<(&str, &str)> = items.iter().map(|(k, a)| (k.as_str(), a.as_str())).collect();
                keycap_bar(ui, &items, Some((text, tint)));
                return;
            }
            if !self.marked.is_empty() {
                items.push((count, "selected".into()));
            }
            items.extend(
                [("J K", "extend"), ("*a", "all"), ("*n", "none"), ("I", "all read"), ("z", "undo")]
                    .map(|(k, a)| (k.to_string(), a.to_string())),
            );
            items.extend(self.prefs.keys.hint_items());
            let items: Vec<(&str, &str)> = items.iter().map(|(k, a)| (k.as_str(), a.as_str())).collect();
            keycap_bar(ui, &items, Some((text, tint)));
        });
    }

    fn side(&mut self, ui: &mut Ui, accounts: &[AccountInfo], actions: &mut Vec<Action>) {
        ui.spacing_mut().item_spacing.y = 2.0;
        if self.prefs.developer_chat {
            heading(ui, "chats");
            ui.add_space(4.0);
            let unread = self.chats.as_ref().map_or(0, |c| c.unread());
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), if self.narrow { 44.0 } else { 26.0 }), egui::Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, 2.0, BAND);
            }
            put(ui.painter(), rect.left() + 12.0, 160.0, rect.center().y, "open chats", body(13.0), LEGEND);
            if unread > 0 {
                put_right(ui.painter(), rect.right() - 6.0, rect.center().y, &unread.to_string(), figure(11.0), READOUT);
            }
            ui.ctx().accesskit_node_builder(resp.id, |n| {
                n.set_label("open chats");
                n.set_value(unread.to_string());
            });
            if resp.clicked() {
                actions.push(Action::OpenChats);
            }
            ui.add_space(12.0);
        }
        heading(ui, "accounts");
        ui.add_space(4.0);
        {
            let on = self.account.is_none();
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), if self.narrow { 44.0 } else { 26.0 }), egui::Sense::click());
            let p = ui.painter();
            if on {
                selection(p, rect);
            } else if resp.hovered() {
                p.rect_filled(rect, 2.0, BAND);
            }
            put(p, rect.left() + 12.0, 160.0, rect.center().y, "all accounts", body(13.0), if on { VALUE } else { LEGEND });
            ui.ctx().accesskit_node_builder(resp.id, |n| n.set_label("all accounts"));
            if resp.clicked() {
                actions.push(Action::ClearAccount);
            }
        }
        for (i, a) in accounts.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), if self.narrow { 44.0 } else { 38.0 }), egui::Sense::click());
            ui.ctx().accesskit_node_builder(resp.id, |n| n.set_label(format!("account {}", a.name)));
            if resp.clicked() {
                actions.push(Action::AccountOnly(Some(i)));
            }
            let p = ui.painter();
            if self.account == Some(i) {
                selection(p, rect);
            } else if resp.hovered() {
                p.rect_filled(rect, 2.0, BAND);
            }
            let tint = match a.sync {
                SyncState::Idle => OK,
                SyncState::Syncing => TRACE,
                SyncState::Failed => FAULT,
            };
            p.circle_filled(egui::pos2(rect.left() + 10.0, rect.top() + 13.0), 3.5, tint);
            put(p, rect.left() + 22.0, 100.0, rect.top() + 13.0, &a.name, body_strong(13.0), VALUE);
            let status = match a.sync {
                SyncState::Idle => "idle",
                SyncState::Syncing => "sync",
                SyncState::Failed => "sign in",
            };
            let col = if a.sync == SyncState::Failed { FAULT } else { LEGEND };
            let status = match a.indexed {
                Some((done, total)) if a.sync != SyncState::Failed && done < total => format!("{status}, indexing {done} of {total}"),
                Some(_) if a.sync == SyncState::Idle => format!("{status}, indexed"),
                _ => status.to_string(),
            };
            put(p, rect.left() + 22.0, 150.0, rect.top() + 28.0, &status, legend_font(9.5), col);
            if a.unread > 0 {
                put_right(p, rect.right() - 6.0, rect.top() + 13.0, &a.unread.to_string(), figure(11.0), READOUT);
            }
        }
        ui.add_space(12.0);
        heading(ui, "inbox");
        ui.add_space(4.0);
        for f in self.store.folders() {
            let on = f.folder == self.folder && self.account.is_none();
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), self.row_h()), egui::Sense::click());
            let p = ui.painter();
            if on {
                selection(p, rect);
            } else if resp.hovered() {
                p.rect_filled(rect, 2.0, BAND);
            }
            put(p, rect.left() + 12.0, 120.0, rect.center().y, f.folder.name(), body(13.0), if on { VALUE } else { LEGEND });
            let n = f.unread;
            if n > 0 {
                put_right(p, rect.right() - 6.0, rect.center().y, &n.to_string(), figure(11.0), if on { READOUT } else { LEGEND });
            }
            ui.ctx().accesskit_node_builder(resp.id, |node| {
                node.set_label(format!("folder {}", f.folder.name()));
                node.set_value(n.to_string());
            });
            let target = f.folder.clone();
            resp.context_menu(|ui| {
                let narrow = self.narrow;
                menu_column(ui, narrow, &["mark all as read"], |ui, menu| {
                    if menu.key(ui, "mark all as read", VALUE) {
                        actions.push(Action::MarkAllRead(target.clone()));
                        ui.close();
                    }
                });
            });
            if resp.clicked() {
                actions.push(Action::GoTo(f.folder));
            }
        }
        let labels = self.store.labels();
        if !labels.is_empty() {
            ui.add_space(12.0);
            heading(ui, "labels");
            ui.add_space(4.0);
            for l in labels {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click());
                let p = ui.painter();
                if resp.hovered() {
                    p.rect_filled(rect, 2.0, BAND);
                }
                put(p, rect.left() + 12.0, 120.0, rect.center().y, &l.name, body(13.0), LEGEND);
                if l.unread > 0 {
                    put_right(p, rect.right() - 6.0, rect.center().y, &l.unread.to_string(), figure(11.0), LEGEND);
                }
                ui.ctx().accesskit_node_builder(resp.id, |n| n.set_label(format!("label {}", l.name)));
                if resp.clicked() {
                    actions.push(Action::Label(l.name.clone()));
                }
            }
        }
        self.tree(ui, accounts, actions);
    }

    fn tree(&mut self, ui: &mut Ui, accounts: &[AccountInfo], actions: &mut Vec<Action>) {
        let tree = self.store.tree();
        if tree.is_empty() && accounts.is_empty() {
            return;
        }
        for (account, info) in accounts.iter().enumerate() {
            ui.add_space(12.0);
            heading(ui, &info.name.to_lowercase());
            ui.add_space(4.0);
            let mut hidden_under: Option<usize> = None;
            let scoped = self.account == Some(account);
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), self.row_h()), egui::Sense::click());
            let p = ui.painter();
            if resp.hovered() {
                p.rect_filled(rect, 2.0, BAND);
            }
            put(p, rect.left() + 12.0, 160.0, rect.center().y, if scoped { "this account only: on" } else { "this account only" }, body(12.0), if scoped { READOUT } else { LEGEND });
            ui.ctx().accesskit_node_builder(resp.id, |n| n.set_label(format!("only {}", info.name)));
            if resp.clicked() {
                actions.push(Action::AccountOnly(Some(account)));
            }
            let listed = self.store.account_folders(account);
            let waiting = listed.is_empty() && !tree.iter().any(|n| n.account == account);
            if waiting {
                for f in Folder::UNIFIED {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), self.row_h()), egui::Sense::hover());
                    put(ui.painter(), rect.left() + 12.0, 120.0, rect.center().y, f.name(), body(13.0), LEGEND.gamma_multiply(0.4));
                    ui.ctx().accesskit_node_builder(resp.id, |n| {
                        n.set_label(format!("{} {} waiting", info.name, f.name()));
                        n.set_disabled();
                        n.set_bounds(egui::accesskit::Rect::new(rect.min.x as f64, rect.min.y as f64, rect.max.x as f64, rect.max.y as f64));
                    });
                }
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::hover());
                put(ui.painter(), rect.left() + 12.0, 160.0, rect.center().y, "syncing folders", body(12.0), LEGEND);
                ui.ctx().accesskit_node_builder(resp.id, |n| n.set_label(format!("{} syncing folders", info.name)));
                continue;
            }
            for f in listed {
                let on = scoped && f.folder == self.folder;
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), self.row_h()), egui::Sense::click());
                let p = ui.painter();
                if on {
                    selection(p, rect);
                } else if resp.hovered() {
                    p.rect_filled(rect, 2.0, BAND);
                }
                put(p, rect.left() + 12.0, 120.0, rect.center().y, f.folder.name(), body(13.0), if on { VALUE } else { LEGEND });
                let n = f.unread;
                if n > 0 {
                    put_right(p, rect.right() - 6.0, rect.center().y, &n.to_string(), figure(11.0), if on { READOUT } else { LEGEND });
                }
                ui.ctx().accesskit_node_builder(resp.id, |node| {
                    node.set_label(format!("{} {}", info.name, f.folder.name()));
                    node.set_value(n.to_string());
                });
                if resp.clicked() {
                    actions.push(Action::GoToIn(account, f.folder));
                }
            }
            for node in tree.iter().filter(|n| n.account == account) {
                if let Some(depth) = hidden_under {
                    if node.depth > depth {
                        continue;
                    }
                    hidden_under = None;
                }
                let folded = self.collapsed.contains(&(account, node.path.clone()));
                if folded && node.children {
                    hidden_under = Some(node.depth);
                }
                self.tree_row(ui, node, folded, info.server_folders, actions);
            }
            if !info.server_folders {
                continue;
            }
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click());
            let p = ui.painter();
            if resp.hovered() {
                p.rect_filled(rect, 2.0, BAND);
            }
            put(p, rect.left() + 12.0, 120.0, rect.center().y, "+ new folder", body(12.0), LEGEND);
            ui.ctx().accesskit_node_builder(resp.id, |node| node.set_label(format!("new folder in {}", info.name)));
            if resp.clicked() {
                actions.push(Action::NewFolder(account, String::new()));
            }
        }
    }

    fn row_h(&self) -> f32 {
        if self.narrow { 44.0 } else { 22.0 }
    }

    fn tree_row(&self, ui: &mut Ui, node: &TreeFolder, folded: bool, editable: bool, actions: &mut Vec<Action>) {
        let on = node.folder == self.folder && self.account.is_none();
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), self.row_h()), egui::Sense::click());
        let p = ui.painter();
        if on {
            selection(p, rect);
        } else if resp.hovered() {
            p.rect_filled(rect, 2.0, BAND);
        }
        let indent = rect.left() + 12.0 + node.depth as f32 * 12.0;
        if node.children {
            let c = egui::pos2(indent - 1.0, rect.center().y);
            let points = if folded {
                vec![c + egui::vec2(-2.0, -4.0), c + egui::vec2(-2.0, 4.0), c + egui::vec2(3.0, 0.0)]
            } else {
                vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(4.0, -2.0), c + egui::vec2(0.0, 3.0)]
            };
            p.add(egui::Shape::convex_polygon(points, LEGEND, egui::Stroke::NONE));
        }
        let text_x = indent + 8.0;
        let col = if !node.selectable { TRACE } else if on { VALUE } else { LEGEND };
        put(p, text_x, (rect.right() - text_x - 28.0).max(20.0), rect.center().y, &node.leaf, body(13.0), col);
        if node.unread > 0 {
            put_right(p, rect.right() - 6.0, rect.center().y, &node.unread.to_string(), figure(11.0), if on { READOUT } else { LEGEND });
        }
        ui.ctx().accesskit_node_builder(resp.id, |n| n.set_label(node.path.clone()));
        if resp.clicked() {
            let on_arrow = node.children
                && resp.interact_pointer_pos().is_some_and(|pos| pos.x < indent + 6.0);
            if on_arrow || !node.selectable {
                actions.push(Action::ToggleBranch(node.account, node.path.clone()));
            } else {
                actions.push(Action::GoTo(node.folder.clone()));
            }
        }
        let parent = node.path[..node.path.len() - node.leaf.len()].to_string();
        if !editable {
            return;
        }
        resp.context_menu(|ui| {
            let child_prefix = format!("{}{}", node.path, node.delimiter);
            let items = [
                ("new subfolder", Action::NewFolder(node.account, child_prefix)),
                ("mark all as read", Action::MarkAllRead(node.folder.clone())),
                ("rename", Action::RenameFolder(node.folder.clone(), parent)),
                ("delete", Action::DeleteFolder(node.folder.clone())),
            ];
            let labels: Vec<&str> = items.iter().map(|(l, _)| *l).collect();
            menu_column(ui, self.narrow, &labels, |ui, menu| {
                for (label, action) in items {
                    let tint = if label == "delete" { FAULT } else { VALUE };
                    if menu.key(ui, label, tint) {
                        actions.push(action);
                        ui.close();
                    }
                }
            });
        });
    }

    fn move_targets(&self, account: usize) -> Vec<Target> {
        let mut out: Vec<Target> = self
            .store
            .folders()
            .into_iter()
            .map(|f| Target { label: f.folder.name().to_string(), folder: f.folder })
            .filter(|t| t.folder != Folder::Drafts && t.folder != Folder::Sent)
            .collect();
        out.extend(
            self.store
                .tree()
                .into_iter()
                .filter(|n| n.account == account && n.selectable)
                .map(|n| Target { label: n.path, folder: n.folder }),
        );
        out
    }

    fn poll_save_result(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.save_result else { return };
        match rx.try_recv() {
            Ok(msg) => {
                self.save_result = None;
                if !msg.is_empty() {
                    self.notice = Some(msg);
                }
            }
            Err(TryRecvError::Disconnected) => self.save_result = None,
            Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(200)),
        }
    }

    fn follow_notification(&mut self) {
        let Some(id) = self.store.take_focus() else { return };
        self.folder = Folder::Inbox;
        self.filter = Filter::All;
        self.search.clear();
        let query = self.query();
        let total = self.store.counts(&query).all;
        let page = self.store.list(&query, 0, total.max(1));
        let at = page.items.iter().position(|s| s.id == id || self.store.thread(s.id).iter().any(|t| t.summary.id == id));
        self.selected = at.unwrap_or(0);
        self.pending_focus = Some(id);
        self.reader_focus = true;
        self.scroll_to_selected = true;
    }

    fn link_warning_dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some((href, shown, real)) = self.link_warning.clone() else { return };
        let mut open = false;
        let mut close = false;
        let r = egui::Modal::new(egui::Id::new("link-warning"))
            .backdrop_color(egui::Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(420.0);
                modal_title(ui, "this link goes somewhere else");
                Line::new().note(format!("The link text says {shown}, but it opens {real}. Phishing mail does this.")).size(12.5).wrapped(ui);
                ui.add_space(4.0);
                Line::new().measured(href.as_str()).size(11.0).show(ui);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(Key::new("cancel").solid(READOUT).min_width(90.0)).clicked() {
                        close = true;
                    }
                    if ui.add(Key::new("open anyway").outline(WARN).min_width(110.0)).clicked() {
                        open = true;
                    }
                });
            });
        if r.should_close() {
            close = true;
        }
        if open {
            actions.push(Action::OpenLink(href));
            actions.push(Action::CloseLinkWarning);
        } else if close {
            actions.push(Action::CloseLinkWarning);
        }
    }

    fn snooze_picker(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(snoozer) = self.snoozer.as_mut() else { return };
        let now = chrono::Local::now();
        let presets = crate::snooze::presets(&now);
        let whole = snoozer.whole;
        let (mut picked, mut close) = (None, false);
        let r = egui::Modal::new(egui::Id::new("snooze"))
            .backdrop_color(egui::Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                modal_title(ui, "snooze until");
                for p in &presets {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
                    let painter = ui.painter();
                    if resp.hovered() {
                        painter.rect_filled(rect, 2.0, BAND);
                    }
                    put(painter, rect.left() + 12.0, 150.0, rect.center().y, p.label, body(13.0), VALUE);
                    put_right(painter, rect.right() - 8.0, rect.center().y, &crate::snooze::describe(p.at), figure(11.0), LEGEND);
                    ui.ctx().accesskit_node_builder(resp.id, |n| n.set_label(format!("snooze {}", p.label)));
                    if resp.clicked() {
                        picked = Some(p.at);
                    }
                }
                ui.add_space(6.0);
                let r = field(ui, &mut snoozer.custom, "custom date, 2025-12-31 or 2025-12-31 17:30");
                if r.changed() {
                    snoozer.error = false;
                }
                if !r.has_focus() && snoozer.custom.is_empty() {
                    r.request_focus();
                }
                if snoozer.error {
                    Line::new().note("Use YYYY-MM-DD or YYYY-MM-DD HH:MM, in the future").size(11.5).show(ui);
                }
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    match crate::snooze::parse_custom(&snoozer.custom, &now) {
                        Some(at) => picked = Some(at),
                        None => snoozer.error = true,
                    }
                }
                for (n, p) in presets.iter().enumerate() {
                    let key = [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3][n.min(2)];
                    if snoozer.custom.is_empty() && n < 3 && ui.input(|i| i.key_pressed(key)) {
                        picked = Some(p.at);
                    }
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
            });
        if r.should_close() {
            close = true;
        }
        if let Some(at) = picked {
            actions.push(Action::SnoozeUntil(at, whole));
            self.snoozer = None;
        } else if close {
            self.snoozer = None;
        }
    }

    fn move_picker(&mut self, ctx: &egui::Context, current: Option<&Message>, actions: &mut Vec<Action>) {
        let Some(mover) = self.mover.as_mut() else { return };
        let Some(m) = current else {
            self.mover = None;
            return;
        };
        let account = m.summary.account;
        let from = m.folder.clone();
        let whole = mover.whole;
        let query = mover.query.clone();
        let mut ranked: Vec<(usize, Target)> = self
            .move_targets(account)
            .into_iter()
            .filter(|t| t.folder != from)
            .filter_map(|t| fuzzy_score(&query, &t.label).map(|s| (s, t)))
            .collect();
        ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
        let Some(mover) = self.mover.as_mut() else { return };
        let (mut picked, mut close) = (None, false);
        let r = egui::Modal::new(egui::Id::new("move-to"))
            .backdrop_color(egui::Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                modal_title(ui, "move to");
                let r = field(ui, &mut mover.query, "type a folder name");
                if !r.has_focus() {
                    r.request_focus();
                }
                if r.changed() {
                    mover.selected = 0;
                }
                ui.input(|i| {
                    if i.key_pressed(egui::Key::ArrowDown) && mover.selected + 1 < ranked.len() {
                        mover.selected += 1;
                    }
                    if i.key_pressed(egui::Key::ArrowUp) {
                        mover.selected = mover.selected.saturating_sub(1);
                    }
                });
                ui.add_space(6.0);
                if ranked.is_empty() {
                    Line::new().note("No folder matches").size(11.5).show(ui);
                }
                for (i, (_, t)) in ranked.iter().take(10).enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click());
                    let p = ui.painter();
                    if i == mover.selected {
                        selection(p, rect);
                    } else if resp.hovered() {
                        p.rect_filled(rect, 2.0, BAND);
                    }
                    put(p, rect.left() + 12.0, rect.width() - 20.0, rect.center().y, &t.label, body(13.0), VALUE);
                    ui.ctx().accesskit_node_builder(resp.id, |n| n.set_label(format!("move to {}", t.label)));
                    if resp.clicked() {
                        picked = Some(t.folder.clone());
                    }
                }
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    picked = ranked.get(mover.selected).map(|(_, t)| t.folder.clone());
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
            });
        if r.should_close() {
            close = true;
        }
        if let Some(f) = picked {
            actions.push(Action::MoveTo(f, whole));
            self.mover = None;
        } else if close {
            self.mover = None;
        }
    }

    fn folder_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.dialog.as_mut() else { return };
        let (mut submit, mut cancel) = (false, false);
        let r = egui::Modal::new(egui::Id::new("folder-dialog"))
            .backdrop_color(egui::Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                match dialog {
                    FolderDialog::Create { prefix, name, .. } => {
                        modal_title(ui, "new folder");
                        if !prefix.is_empty() {
                            Line::new().note(format!("inside {prefix}")).size(11.5).show(ui);
                        }
                        let r = field(ui, name, "folder name");
                        if !r.has_focus() {
                            r.request_focus();
                        }
                    }
                    FolderDialog::Rename { name, .. } => {
                        modal_title(ui, "rename folder");
                        let r = field(ui, name, "folder name");
                        if !r.has_focus() {
                            r.request_focus();
                        }
                    }
                    FolderDialog::Delete { folder } => {
                        modal_title(ui, "delete folder");
                        Line::new()
                            .note(format!("Delete {} and every message in it on the server?", folder.name()))
                            .size(11.5)
                            .show(ui);
                    }
                    FolderDialog::Purge { ids } => {
                        modal_title(ui, "delete for good");
                        Line::new()
                            .note(if ids.len() == 1 { "Delete this message for good? It cannot be undone.".to_string() } else { format!("Delete {} messages for good? They cannot be undone.", ids.len()) })
                            .size(11.5)
                            .show(ui);
                    }
                }
                let (label, tint) = match dialog {
                    FolderDialog::Purge { .. } => ("delete", FAULT),
                    FolderDialog::Create { .. } => ("create", READOUT),
                    FolderDialog::Rename { .. } => ("rename", READOUT),
                    FolderDialog::Delete { .. } => ("delete", FAULT),
                };
                form_footer(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        submit = ui.add(Key::new(label).solid(tint).min_width(100.0)).clicked();
                        cancel = ui.add(Key::new("cancel").outline(VALUE).min_width(90.0)).clicked();
                    });
                });
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    submit = true;
                }
            });
        if r.should_close() {
            cancel = true;
        }
        if submit {
            let result = match self.dialog.take() {
                Some(FolderDialog::Create { account, prefix, name }) if !name.trim().is_empty() => {
                    self.store.create_folder(account, &format!("{prefix}{}", name.trim())).map(|_| "Folder created")
                }
                Some(FolderDialog::Rename { folder, prefix, name }) if !name.trim().is_empty() => {
                    self.store.rename_folder(&folder, &format!("{prefix}{}", name.trim())).map(|_| "Folder renamed")
                }
                Some(FolderDialog::Purge { ids }) => {
                    self.marked.clear();
                    self.store.delete_forever(&ids).map(|_| "Deleted for good")
                }
                Some(FolderDialog::Delete { folder }) => {
                    if folder == self.folder {
                        self.go_folder(Folder::Inbox);
                    }
                    self.store.delete_folder(&folder).map(|_| "Folder deleted")
                }
                _ => return,
            };
            self.notice = Some(match result {
                Ok(said) => said.into(),
                Err(e) => format!("Could not change the folder: {e:?}"),
            });
        } else if cancel {
            self.dialog = None;
        }
    }

    fn list(&mut self, ui: &mut Ui, v: &View, accounts: &[AccountInfo], actions: &mut Vec<Action>) {
        pane_fill(ui, CHASSIS);
        ui.painter().line_segment(
            [ui.max_rect().right_top(), ui.max_rect().right_bottom()],
            egui::Stroke::new(1.0, ETCH),
        );
        egui::Frame::NONE.inner_margin(Margin { left: 0, right: 0, top: 8, bottom: 0 }).show(ui, |ui| {
            egui::Frame::NONE.inner_margin(Margin::symmetric(8, 0)).show(ui, |ui| {
                let before = self.filter;
                segments(
                    ui,
                    &mut self.filter,
                    &[
                        (Filter::All, "all", Some(v.counts.all)),
                        (Filter::Unread, "unread", Some(v.counts.unread)),
                        (Filter::Flagged, "flagged", Some(v.counts.flagged)),
                    ],
                );
                if before != self.filter {
                    self.selected = 0;
                }
            });
            ui.add_space(6.0);
            for (i, a) in accounts.iter().enumerate() {
                if a.sync == SyncState::Failed || (self.force_failed && i == 0) {
                    egui::Frame::NONE.inner_margin(Margin::symmetric(8, 0)).show(ui, |ui| {
                        fault_card(ui, &a.name, &a.address);
                    });
                    ui.add_space(8.0);
                    break;
                }
            }
            if v.total == 0 {
                self.rendered_rows = 0;
                self.empty_list(ui);
                return;
            }
            self.rows(ui, v, accounts, actions);
        });
    }

    fn empty_list(&self, ui: &mut Ui) {
        let filtered = !self.search.is_empty() || self.filter != Filter::All;
        if filtered {
            empty_state(ui, "No matches", "Nothing here fits the search and filter. Press Esc to clear the search.", &[("Esc", "clear"), ("/", "search")]);
        } else if self.folder == Folder::Inbox {
            empty_state(
                ui,
                "Inbox zero",
                "Everything in kestrel, home and lists is read and filed. New mail shows up here as it arrives.",
                &[("g a", "go to archive"), ("c", "compose")],
            );
        } else {
            empty_state(ui, &format!("Nothing in {}", self.folder.name()), "Messages you move here will show up in this folder.", &[("g i", "go to inbox"), ("c", "compose")]);
        }
    }

    fn rows(&mut self, ui: &mut Ui, v: &View, accounts: &[AccountInfo], actions: &mut Vec<Action>) {
        let columns: [(&str, f32); 5] = [("", 20.0), ("from", 150.0), ("subject", 200.0), ("account", 110.0), ("time", 60.0)];
        let (head, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), egui_bench::table::ROW_H), egui::Sense::hover());
        egui_bench::table::header(&ui.painter_at(head), head, &columns);
        let selected = self.selected;
        let scroll_to = std::mem::take(&mut self.scroll_to_selected);
        let mut painted = 0;
        let mut clicked = None;
        let mut opened = None;
        let row_h = self.prefs.density.row_height();
        let narrow = self.narrow;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
            let total = v.total;
            ui.set_height(total as f32 * row_h);
            let first = (viewport.min.y / row_h).floor().max(0.0) as usize;
            let last = ((viewport.max.y / row_h).ceil() as usize + 1).min(total);
            let page = self.store.list(&v.query, first, last.saturating_sub(first));
            let origin = ui.min_rect().min;
            let width = ui.available_width();
            let marks = crate::search::Parsed::parse(&v.query.text).highlights();
            for (n, s) in page.items.iter().enumerate() {
                let i = first + n;
                let rect = egui::Rect::from_min_size(
                    origin + egui::vec2(0.0, i as f32 * row_h),
                    egui::vec2(width, row_h),
                );
                let resp = ui.interact(rect, ui.id().with(("row", i)), egui::Sense::click());
                let name = accounts.get(s.account).map(|a| a.name.as_str());
                paint_row(&ui.painter_at(rect), rect, s, i == selected, self.marked.contains(&i), resp.hovered(), name, &marks);
                ui.ctx().accesskit_node_builder(resp.id, |node| {
                    node.set_label(format!("{} {}", s.from, s.subject));
                });
                painted += 1;
                if resp.clicked() {
                    let m = ui.input(|inp| inp.modifiers);
                    clicked = Some((i, m.shift, m.command));
                }
                if resp.secondary_clicked() && !self.marked.contains(&i) {
                    self.marked.clear();
                    self.selected = i;
                }
                resp.context_menu(|ui| {
                    let state = menu::MessageMenu {
                        unread: s.unread,
                        flagged: s.flagged,
                        server_flags: accounts.get(s.account).is_none_or(|a| a.server_flags),
                        folder: v.query.folder.clone(),
                        multi: self.marked.len() > 1,
                    };
                    menu::show_message_menu(ui, narrow, &state, None, actions);
                });
                if resp.double_clicked() || (resp.clicked() && self.narrow && !ui.input(|inp| inp.modifiers.shift || inp.modifiers.command)) {
                    opened = Some(i);
                }
            }
            if scroll_to {
                let rect = egui::Rect::from_min_size(
                    origin + egui::vec2(0.0, selected as f32 * row_h),
                    egui::vec2(width, row_h),
                );
                ui.scroll_to_rect(rect, None);
            }
        });
        self.rendered_rows = painted;
        if let Some((i, shift, command)) = clicked {
            if shift {
                self.extend_to(i);
            } else if command {
                if self.marked.is_empty() {
                    self.marked.insert(self.selected);
                }
                if !self.marked.remove(&i) {
                    self.marked.insert(i);
                }
                self.anchor = i;
                self.selected = i;
            } else {
                self.marked.clear();
                self.selected = i;
                self.mark_now = true;
            }
        }
        if let Some(i) = opened {
            self.selected = i;
            self.mark_now = true;
            actions.push(Action::Open);
        }
    }

    fn open_draft(&mut self, id: MessageId) {
        match self.store.open_draft(id) {
            Ok(out) => {
                let signature = self.store.accounts().get(out.account).map(|a| a.signature.clone()).unwrap_or_default();
                self.draft = Some(Draft::from_outgoing(out, &signature));
            }
            Err(_) => self.notice = Some("The draft is still loading, try again in a moment".into()),
        }
    }

    fn save_draft(&mut self, said: Option<&str>) {
        let Some(d) = self.draft.as_mut() else { return };
        let out = d.outgoing();
        match self.store.save_draft(out) {
            Ok(mid) => {
                d.message_id = Some(mid);
                d.mark_saved();
                if let Some(said) = said {
                    self.notice = Some(said.into());
                }
            }
            Err(e) => self.notice = Some(format!("Could not save: {e:?}")),
        }
    }

    fn send_draft(&mut self) {
        let Some(mut d) = self.draft.take() else { return };
        d.commit_inputs();
        if d.encoded_size() > crate::inline::MAX_MESSAGE_BYTES {
            self.notice = Some(format!("The message is about {} MB, over the {} MB limit. Remove an image or attachment", d.encoded_size() / 1_048_576, crate::inline::MAX_MESSAGE_BYTES / 1_048_576));
            self.draft = Some(d);
            return;
        }
        self.store.set_send_html(!self.prefs.plain_text_only);
        let to = d.to.clone();
        match self.store.append(Folder::Sent, d.outgoing()) {
            Ok(id) => {
                if let Some(mid) = &d.message_id {
                    self.store.discard_draft(d.account, mid);
                }
                self.notice = Some("Sent".into());
                self.pending_send = Some(PendingSend { id, until: Instant::now() + self.store.send_hold(), to });
            }
            Err(e) => {
                self.notice = Some(format!("Could not send: {e:?}"));
                self.draft = Some(d);
            }
        }
    }

    fn discard_draft(&mut self) {
        if let Some(d) = self.draft.take() {
            if let Some(mid) = &d.message_id {
                self.store.discard_draft(d.account, mid);
            }
        }
    }

    pub fn undo_send(&mut self) {
        let Some(p) = self.pending_send.take() else { return };
        match self.store.undo_send(p.id) {
            Ok(Some(out)) => {
                let signature = self.store.accounts().get(out.account).map(|a| a.signature.clone()).unwrap_or_default();
                let mut d = Draft::from_outgoing(out, &signature);
                d.title = "message".into();
                self.draft = Some(d);
                self.notice = Some("Send cancelled".into());
            }
            Ok(None) => self.notice = Some("Too late to undo, the message is on its way".into()),
            Err(e) => self.notice = Some(format!("Could not undo: {e:?}")),
        }
    }

    fn pick_files(&mut self, ctx: &egui::Context) {
        if self.picker.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let files = pick_attachment_files();
            let _ = tx.send(files);
            ctx.request_repaint();
        });
        self.picker = Some(rx);
    }

    fn collect_attachments(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.draft.as_mut() else { return };
        let mut errors = Vec::new();
        let mut images: Vec<(String, Vec<u8>)> = Vec::new();
        let mut picked = Vec::new();
        if let Some(rx) = &self.picker {
            match rx.try_recv() {
                Ok(files) => {
                    picked = files;
                    self.picker = None;
                }
                Err(TryRecvError::Disconnected) => self.picker = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        for file in picked {
            match file {
                PickedFile::Path(path) => {
                    if let Err(e) = draft.attach_path(&path) {
                        errors.push(e);
                    }
                }
                PickedFile::Bytes(name, data) => draft.attach_bytes(&name, data),
                PickedFile::Failed(e) => errors.push(e),
            }
        }
        for f in ctx.input(|i| i.raw.dropped_files.clone()) {
            let name = f.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            match f.bytes() {
                Ok(bytes) if crate::inline::sniff(&bytes).is_some() => images.push((name, bytes)),
                Ok(bytes) => draft.attach_bytes(&name, bytes),
                Err(e) => errors.push(format!("Could not read {name}: {e}")),
            }
        }
        let pasted: Vec<std::sync::Arc<egui::ColorImage>> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::PasteImage(img) => Some(img.clone()),
                    _ => None,
                })
                .collect()
        });
        for img in pasted {
            match png_of(&img) {
                Ok(bytes) => images.push(("pasted-image.png".into(), bytes)),
                Err(e) => errors.push(e),
            }
        }
        let mut notes = Vec::new();
        for (name, bytes) in images {
            match self.insert_image(ctx, &name, bytes) {
                Ok(Some(note)) => notes.push(note),
                Ok(None) => {}
                Err(e) => errors.push(e),
            }
        }
        if let Some(e) = errors.into_iter().next() {
            self.notice = Some(e);
        } else if let Some(n) = notes.into_iter().next() {
            self.notice = Some(n);
        }
    }

    fn insert_image(&mut self, ctx: &egui::Context, name: &str, bytes: Vec<u8>) -> Result<Option<String>, String> {
        let Some(draft) = self.draft.as_mut() else { return Ok(None) };
        let id = egui::Id::new("compose-body");
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        let at: Option<usize> = state.cursor.char_range().map(|r| r.primary.index.into());
        let before = draft.body.chars().count();
        let note = draft.insert_inline(name, bytes, at)?;
        let cid = draft.last_inline_cid().map(str::to_string);
        if let Some(cid) = cid {
            if let Some(thumb) = draft.attachments.iter().find(|a| a.cid.as_deref() == Some(cid.as_str())).and_then(|a| crate::inline::thumbnail(&a.data)) {
                let tex = ctx.load_texture(format!("inline-{cid}"), thumb, egui::TextureOptions::LINEAR);
                self.thumbs.insert(cid, tex);
            }
        }
        let inserted = draft.body.chars().count() - before;
        let start = at.unwrap_or(before).min(before);
        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(start + inserted))));
        state.store(ctx, id);
        if self.prefs.plain_text_only && !self.plain_inline_told {
            self.plain_inline_told = true;
            return Ok(Some("Plain text only is on, so inline images are sent as attachments".into()));
        }
        Ok(note)
    }

    fn ensure_thumbs(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.draft.as_ref() else { return };
        for cid in draft.inline_refs() {
            if self.thumbs.contains_key(&cid) {
                continue;
            }
            let found = draft.attachments.iter().find(|a| a.cid.as_deref() == Some(cid.as_str()));
            if let Some(thumb) = found.and_then(|a| crate::inline::thumbnail(&a.data)) {
                let tex = ctx.load_texture(format!("inline-{cid}"), thumb, egui::TextureOptions::LINEAR);
                self.thumbs.insert(cid, tex);
            }
        }
    }

    fn composer(&mut self, ctx: &egui::Context, accounts: &[AccountInfo]) {
        if self.draft.is_none() {
            return;
        }
        self.collect_attachments(ctx);
        if let Some(d) = self.draft.as_mut() {
            d.sync_inline();
        }
        self.ensure_thumbs(ctx);
        let thumbs = &self.thumbs;
        let hovering = egui_bench::drop::hovering(ctx).is_some();
        let store: &dyn MailStore = self.store.as_ref();
        let Some(draft) = self.draft.as_mut() else { return };
        let (mut send, mut save, mut discard, mut attach) = (false, false, false, false);
        if draft.has_recipient() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
            send = true;
        }
        let r = egui::Modal::new(egui::Id::new("compose"))
            .backdrop_color(egui::Color32::from_black_alpha(150))
            .show(ctx, |ui| {
                ui.set_width(640.0_f32.min(ctx.content_rect().width() - 48.0));
                modal_title(ui, &draft.title);
                if draft.agent {
                    Line::new().note(crate::mailstore::AGENT_BANNER).size(12.0).show(ui);
                }
                row(ui, "to", |ui| {
                    recipients(ui, "to", &mut draft.to, &mut draft.to_input, "name or address", store);
                });
                row(ui, "cc", |ui| {
                    recipients(ui, "cc", &mut draft.cc, &mut draft.cc_input, "copy to", store);
                });
                row(ui, "subject", |ui| {
                    field(ui, &mut draft.subject, "subject");
                });
                if accounts.len() > 1 {
                    row(ui, "from", |ui| {
                        ui.allocate_ui(egui::vec2(260.0, KEY_H), |ui| {
                            let mut picked = draft.account;
                            let changed = choice(ui, "from", &mut picked, accounts.iter().enumerate().map(|(i, a)| (i, a.address.clone())));
                            if changed {
                                let signature = accounts.get(picked).map(|a| a.signature.clone()).unwrap_or_default();
                                draft.set_account(picked, &signature);
                            }
                        });
                    });
                }
                ui.add_space(6.0);
                body_editor(ui, &mut draft.body, 9);
                ui.add_space(4.0);
                let refs = draft.inline_refs();
                if !refs.is_empty() {
                    let mut remove = None;
                    ui.horizontal_wrapped(|ui| {
                        let mut seen = Vec::new();
                        for cid in &refs {
                            if seen.contains(cid) {
                                continue;
                            }
                            seen.push(cid.clone());
                            let name = draft.attachments.iter().find(|a| a.cid.as_deref() == Some(cid.as_str())).map(|a| a.name.clone()).unwrap_or_default();
                            if let Some(tex) = thumbs.get(cid) {
                                let size = tex.size_vec2();
                                let scale = (72.0 / size.x.max(size.y)).min(1.0);
                                let r = ui.add(egui::Button::image(egui::Image::new(tex).fit_to_exact_size(size * scale)).frame(true));
                                let label = format!("remove image {name}");
                                r.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
                                if r.on_hover_text(format!("{name}, click to remove")).clicked() {
                                    remove = Some(cid.clone());
                                }
                            }
                        }
                    });
                    if let Some(cid) = remove {
                        draft.remove_inline(&cid);
                    }
                    ui.add_space(4.0);
                }
                if draft.attachments.iter().any(|a| a.cid.is_none()) {
                    ui.horizontal_wrapped(|ui| {
                        let mut remove = None;
                        for (i, a) in draft.attachments.iter().enumerate().filter(|(_, a)| a.cid.is_none()) {
                            let r = attachment(ui, &a.name, &human_size(a.data.len()));
                            let label = format!("remove {}", a.name);
                            r.widget_info(move || egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
                            if r.on_hover_text("click to remove").clicked() {
                                remove = Some(i);
                            }
                        }
                        if let Some(i) = remove {
                            draft.attachments.remove(i);
                        }
                    });
                    ui.add_space(4.0);
                }
                ui.horizontal(|ui| {
                    attach = ui.add(Key::new("attach").outline(VALUE).min_width(90.0)).clicked();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let note = if draft.saved && !draft.is_dirty() { "Saved to drafts, encrypted" } else { "Not saved yet" };
                        Line::new().note(note).size(11.0).show(ui);
                    });
                });
                form_footer(ui, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        send |= ui.add(Key::new("send").solid(READOUT).min_width(100.0).enabled(draft.has_recipient())).clicked();
                        save = ui.add(Key::new("save draft").outline(VALUE).min_width(100.0)).clicked();
                        discard = ui.add(Key::new("discard").outline(FAULT).min_width(90.0)).clicked();
                    });
                });
            });
        if hovering {
            egui_bench::drop::overlay(ctx, None, "drop to attach", "files are encrypted while queued", READOUT);
        }
        if r.should_close() {
            discard = true;
        }
        if attach {
            self.pick_files(ctx);
        }
        if send {
            self.send_draft();
        } else if save {
            self.save_draft(Some("Saved to drafts"));
        } else if discard {
            self.discard_draft();
        } else if let Some(d) = self.draft.as_mut() {
            if d.due_for_autosave(self.autosave) {
                self.save_draft(None);
            }
            if self.draft.as_ref().is_some_and(|d| d.is_dirty()) {
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
    }

    fn undo_toast(&mut self, ctx: &egui::Context) {
        let Some(p) = self.pending_send.clone() else { return };
        if Instant::now() >= p.until {
            self.pending_send = None;
            return;
        }
        let left = p.until.saturating_duration_since(Instant::now()).as_secs() + 1;
        let mut undo = false;
        egui::Area::new(egui::Id::new("undo-send"))
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(20.0, -52.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(PANEL)
                    .stroke(egui::Stroke::new(1.0, READOUT))
                    .inner_margin(Margin::symmetric(14, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            Line::new().note(format!("Sending to {} in {left}s", p.to)).show(ui);
                            undo = ui.add(Key::new("undo").outline(VALUE).min_width(70.0)).clicked();
                        });
                    });
            });
        if undo {
            self.undo_send();
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

fn recipients(ui: &mut Ui, id: &str, list: &mut String, input: &mut String, hint: &str, store: &dyn MailStore) {
    ui.vertical(|ui| {
        let chips = split_recipients(list);
        if !chips.is_empty() {
            ui.horizontal_wrapped(|ui| {
                let mut keep = Vec::new();
                for c in &chips {
                    let contact = Contact::parse(c);
                    let shown = if contact.name.is_empty() { contact.address.clone() } else { contact.name.clone() };
                    if !chip_button(ui, &shown, VALUE).on_hover_text(format!("{} (click to remove)", contact.address)).clicked() {
                        keep.push(c.clone());
                    }
                }
                if keep.len() != chips.len() {
                    *list = keep.join(", ");
                }
            });
        }
        let out = egui::Frame::NONE
            .fill(WELL)
            .stroke(egui::Stroke::new(1.0, ETCH))
            .corner_radius(2)
            .inner_margin(Margin { left: 6, right: 6, top: 3, bottom: 3 })
            .show(ui, |ui| {
                let edit = egui::TextEdit::singleline(input)
                    .frame(egui::Frame::NONE)
                    .font(egui_bench::theme::figure(12.5))
                    .text_color(VALUE)
                    .hint_text(egui::RichText::new(hint).font(egui_bench::theme::figure(12.5)).color(LEGEND.gamma_multiply(0.6)))
                    .desired_width(ui.available_width() - 12.0);
                egui::CompletionPopup::new(ui.make_persistent_id(("recipient", id)))
                    .word_boundary(|c| c == ',' || c == ';')
                    .align(egui::RectAlign::BOTTOM_START)
                    .show(ui, edit, |q| {
                        store
                            .suggest(q.word.trim(), 6)
                            .into_iter()
                            .map(|c| {
                                let shown = c.formatted();
                                egui::Suggestion::new(format!("{shown}, ")).content(shown)
                            })
                            .collect()
                    })
            })
            .inner;
        let edit_id = out.text_edit.response.id;
        ui.ctx().accesskit_node_builder(edit_id, |node| node.set_label(hint));
        let pending_split = split_recipients(input);
        let finished = input.ends_with([',', ';']) || out.text_edit.response.lost_focus();
        if finished && !pending_split.is_empty() {
            let mut all = split_recipients(list);
            all.extend(pending_split);
            *list = all.join(", ");
            input.clear();
        }
    });
}

fn paint_row(p: &egui::Painter, rect: egui::Rect, m: &Summary, selected: bool, picked: bool, hovered: bool, account: Option<&str>, marks: &[String]) {
    use egui::Pos2;
    if selected || picked {
        selection(p, rect);
    } else if hovered {
        p.rect_filled(rect, 0.0, BAND.gamma_multiply(0.6));
    }
    let cy = rect.center().y;
    if m.new {
        dot(p, Pos2::new(rect.left() + 13.0, cy), TRACE);
    } else if m.unread {
        dot(p, Pos2::new(rect.left() + 13.0, cy), READOUT);
    }
    let (name_font, name_col, subj_col) =
        if m.unread { (body_strong(13.0), VALUE, VALUE) } else { (body(13.0), LEGEND, LEGEND) };
    let mut x = rect.left() + 26.0;
    marked(p, x, 140.0, cy, &m.from, name_font, name_col, marks);
    x += 150.0;
    let right = rect.right() - 10.0;
    put_right(p, right, cy, &m.when, figure(11.0), if m.new { TRACE } else { LEGEND });
    let mut tail = right - 56.0;
    if m.thread > 1 {
        put_right(p, tail, cy, &m.thread.to_string(), figure(10.5), LEGEND);
        tail -= 22.0;
    }
    if m.attachments {
        clip_glyph(p, Pos2::new(tail - 6.0, cy), LEGEND);
        tail -= 18.0;
    }
    if m.flagged {
        flag(p, Pos2::new(tail - 8.0, cy), READOUT);
        tail -= 18.0;
    }
    if let Some(name) = account {
        let g = p.layout_no_wrap(name.to_uppercase(), legend_font(9.5), LEGEND.gamma_multiply(0.8));
        let w = g.size().x;
        p.galley(Pos2::new(tail - w, cy - g.size().y / 2.0), g, LEGEND);
        tail -= w + 14.0;
    }
    if m.remote {
        let g = p.layout_no_wrap("SERVER".into(), legend_font(9.5), TRACE);
        let w = g.size().x;
        p.galley(Pos2::new(tail - w, cy - g.size().y / 2.0), g, TRACE);
        tail -= w + 14.0;
    }
    marked(p, x, (tail - x).max(10.0), cy, if m.subject.trim().is_empty() { "(no subject)" } else { &m.subject }, body(13.0), if m.subject.trim().is_empty() { LEGEND.gamma_multiply(0.6) } else { subj_col }, marks);
}

fn marked(p: &egui::Painter, x: f32, w: f32, cy: f32, text: &str, font: egui::FontId, col: egui::Color32, marks: &[String]) {
    let spans = crate::search::spans(text, marks);
    if spans.is_empty() {
        put(p, x, w, cy, text, font, col);
        return;
    }
    let clip = egui::Rect::from_min_max(egui::pos2(x, cy - 20.0), egui::pos2(x + w, cy + 20.0)).intersect(p.clip_rect());
    let p = p.with_clip_rect(clip);
    let mut at = x;
    let mut last = 0;
    let mut piece = |p: &egui::Painter, s: &str, hit: bool| {
        if s.is_empty() {
            return;
        }
        let g = p.layout_no_wrap(s.to_string(), font.clone(), if hit { VALUE } else { col });
        let size = g.size();
        if hit {
            p.rect_filled(egui::Rect::from_min_size(egui::pos2(at, cy - size.y / 2.0), size), 2.0, READOUT.gamma_multiply(0.35));
        }
        p.galley(egui::pos2(at, cy - size.y / 2.0), g, col);
        at += size.x;
    };
    for (s, e) in spans {
        piece(&p, &text[last..s], false);
        piece(&p, &text[s..e], true);
        last = e;
    }
    piece(&p, &text[last..], false);
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::no_frame().show(ui, |ui| self.show(ui));
    }
}

pub fn trust_keys(address: &str) -> [String; 2] {
    let address = address.trim().to_ascii_lowercase();
    let domain = address.rsplit_once('@').map(|(_, d)| d).unwrap_or(&address).to_string();
    [format!("sender:{address}"), format!("domain:{domain}")]
}
