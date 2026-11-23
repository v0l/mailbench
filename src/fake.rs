use crate::data::{ACCOUNTS, ATTACHMENTS, AccountState, BLOCKED_HOST, LABELS, THREAD, inbox};
use crate::mailstore::*;

pub struct FakeStore {
    accounts: Vec<AccountInfo>,
    messages: Vec<Message>,
    next: u64,
    older: Vec<(u64, Vec<u64>)>,
    stash: std::collections::HashMap<u64, Outgoing>,
    blobs: std::collections::HashMap<String, Vec<u8>>,
}

fn sync_of(state: AccountState) -> SyncState {
    match state {
        AccountState::Idle => SyncState::Idle,
        AccountState::Syncing => SyncState::Syncing,
        AccountState::Failed => SyncState::Failed,
    }
}

fn concept_message(id: u64, i: usize, m: &crate::data::Msg, suffix: &str) -> Message {
    let first = i == 0;
    let paragraphs: Vec<String> = if first {
        THREAD[2].paragraphs.iter().map(|p| p.to_string()).collect()
    } else if m.snippet.is_empty() {
        vec!["Thanks, the details are in the thread below.".to_string()]
    } else {
        vec![m.snippet.to_string()]
    };
    let quoted_lines = if first { THREAD[2].quoted_lines as usize } else { (m.count as usize - 1) * 3 };
    let quoted = (0..quoted_lines).map(|n| format!("> earlier line {}", n + 1)).collect();
    let attachments = if first {
        ATTACHMENTS.iter().map(|(n, s)| AttachmentInfo { name: n.to_string(), size: s.to_string(), mime: mime_of(n) }).collect()
    } else if m.attach {
        vec![AttachmentInfo { name: "attachment.pdf".into(), size: "120 KB".into(), mime: "application/pdf".into() }]
    } else {
        Vec::new()
    };
    Message {
        summary: Summary {
            id: MessageId(id),
            from: m.from.to_string(),
            subject: format!("{}{}", m.subject, suffix),
            when: m.when.to_string(),
            unread: m.unread,
            flagged: m.flagged,
            new: m.new,
            attachments: m.attach,
            account: m.account,
            thread: m.count as u32,
            remote: false,
            date: None,
        },
        folder: Folder::Inbox,
        from_address: format!("{}@example.test", m.from.to_lowercase().replace(' ', ".")),
        to: if first { vec!["me".into(), "Wren Okafor".into()] } else { vec!["me".into()] },
        cc: Vec::new(),
        sent: if first { "Fri 17 Nov 09:41".into() } else { m.when.to_string() },
        paragraphs,
        html: Default::default(),
        quoted,
        attachments,
        remote_images: if first { Some(BLOCKED_HOST.to_string()) } else { None },
        headers: if first { sample_headers() } else { Default::default() },
        body_state: Default::default(),
        message_id: None,
        references: Vec::new(),
        to_full: Vec::new(),
        cc_full: Vec::new(),
    }
}

fn mime_of(name: &str) -> String {
    match name.rsplit_once('.').map(|(_, e)| e) {
        Some("pdf") => "application/pdf".into(),
        Some("zip") => "application/zip".into(),
        _ => "application/octet-stream".into(),
    }
}

fn sample_headers() -> Headers {
    Headers {
        all: vec![("List-Id".into(), "<boards.kestrel-instruments.example>".into())],
        list_id: Some("boards.kestrel-instruments.example".into()),
        unsubscribe: Some(Unsubscribe { target: "https://kestrel-instruments.example/u".into(), one_click: true }),
        auth: AuthResult { spf: Some("pass".into()), dkim: Some("pass".into()), dmarc: Some("pass".into()) },
        ..Default::default()
    }
}

fn draft(id: u64, subject: &str, when: &str) -> Message {
    Message {
        summary: Summary {
            id: MessageId(id),
            from: "me".into(),
            subject: subject.into(),
            when: when.into(),
            unread: false,
            flagged: false,
            new: false,
            attachments: false,
            account: 0,
            thread: 1,
            remote: false,
            date: None,
        },
        folder: Folder::Drafts,
        from_address: "mara.ilves@kestrel-instruments.example".into(),
        to: vec!["Jonas Weber".into()],
        cc: Vec::new(),
        sent: when.into(),
        paragraphs: vec!["Not finished.".into()],
        html: Default::default(),
        quoted: Vec::new(),
        attachments: Vec::new(),
        remote_images: None,
        headers: Default::default(),
        body_state: Default::default(),
        message_id: None,
        references: Vec::new(),
        to_full: Vec::new(),
        cc_full: Vec::new(),
    }
}

#[derive(Clone)]
pub struct ConvSpec {
    pub from: &'static str,
    pub address: &'static str,
    pub to: &'static [&'static str],
    pub when: &'static str,
    pub sent: &'static str,
    pub unread: bool,
    pub attach: bool,
    pub account: usize,
    pub paragraphs: &'static [&'static str],
    pub quoted: &'static [&'static str],
}

impl FakeStore {
    pub fn conversation(subject: &str, specs: &[ConvSpec]) -> FakeStore {
        let n = specs.len() as u64;
        let messages = specs
            .iter()
            .enumerate()
            .map(|(i, s)| Message {
                summary: Summary {
                    id: MessageId(i as u64 + 1),
                    from: s.from.into(),
                    subject: subject.into(),
                    when: s.when.into(),
                    unread: s.unread,
                    flagged: false,
                    new: s.unread,
                    attachments: s.attach,
                    account: s.account,
                    thread: if i as u64 + 1 == n { n as u32 } else { 1 },
                    remote: false,
                    date: None,
                },
                folder: Folder::Inbox,
                from_address: s.address.into(),
                to: s.to.iter().map(|t| t.to_string()).collect(),
                cc: Vec::new(),
                sent: s.sent.into(),
                paragraphs: s.paragraphs.iter().map(|p| p.to_string()).collect(),
                html: Default::default(),
                quoted: s.quoted.iter().map(|q| q.to_string()).collect(),
                attachments: if s.attach { vec![AttachmentInfo { name: "notes.pdf".into(), size: "84 KB".into(), mime: mime_of("notes.pdf") }] } else { Vec::new() },
                remote_images: None,
                headers: Default::default(),
                body_state: Default::default(),
                message_id: None,
                references: Vec::new(),
                to_full: Vec::new(),
                cc_full: Vec::new(),
            })
            .collect();
        FakeStore {
            accounts: Self::base_accounts(),
            messages,
            next: n + 1,
            older: vec![(n, (1..n).collect())],
            stash: Default::default(),
            blobs: Default::default(),
        }
    }

    pub fn from_messages(messages: Vec<Message>) -> FakeStore {
        let next = messages.iter().map(|m| m.summary.id.0).max().unwrap_or(0) + 1;
        FakeStore { accounts: Self::base_accounts(), messages, next, older: Vec::new(), stash: Default::default(), blobs: Default::default() }
    }

    pub fn set_headers(&mut self, index: usize, headers: Headers) {
        if let Some(m) = self.messages.get_mut(index) {
            m.headers = headers;
        }
    }

    pub fn with_draft(mut self, index: usize) -> FakeStore {
        if let Some(m) = self.messages.get_mut(index) {
            m.folder = Folder::Drafts;
        }
        let live = self.messages.iter().filter(|m| m.folder != Folder::Drafts).count() as u32;
        if let Some(head) = self.messages.last_mut() {
            head.summary.thread = live.max(1);
        }
        self
    }

    pub fn set_paragraphs(&mut self, index: usize, paragraphs: Vec<String>) {
        if let Some(m) = self.messages.get_mut(index) {
            m.paragraphs = paragraphs;
        }
    }

    pub fn with_older_attachment(mut self, head: MessageId, index: usize, name: &str) -> FakeStore {
        let id = self.older.iter().find(|(h, _)| *h == head.0).map(|(_, ids)| ids[index]);
        if let Some(m) = self.messages.iter_mut().find(|m| Some(m.summary.id.0) == id) {
            m.attachments = vec![AttachmentInfo { name: name.into(), size: "1 KB".into(), mime: mime_of(name) }];
        }
        self
    }

    pub fn without_attachments(mut self, id: MessageId) -> FakeStore {
        if let Some(m) = self.messages.iter_mut().find(|m| m.summary.id == id) {
            m.attachments = Vec::new();
        }
        self
    }

    pub fn with_attachment_bytes(mut self, name: &str, data: Vec<u8>) -> FakeStore {
        self.blobs.insert(name.to_string(), data);
        self
    }

    fn base_accounts() -> Vec<AccountInfo> {
        ACCOUNTS
            .iter()
            .map(|a| AccountInfo {
                name: a.name.into(),
                address: a.address.into(),
                unread: 0,
                sync: sync_of(a.state),
                detail: None,
                indexed: None,
                signature: String::new(),
                server_folders: true,
                server_search: true,
                server_flags: true,
            })
            .collect()
    }

    pub fn set_html(&mut self, id: MessageId, html: &str) {
        if let Some(m) = self.messages.iter_mut().find(|m| m.summary.id == id) {
            m.html = crate::htmlview::HtmlBody { html: html.into(), inline: Vec::new() };
        }
    }

    pub fn concept() -> FakeStore {
        let mut messages: Vec<Message> =
            inbox().iter().enumerate().map(|(i, m)| concept_message(i as u64 + 1, i, m, "")).collect();
        let n = messages.len() as u64;
        messages.push(draft(n + 1, "Re: Firmware release notes for 2.4", "Wed"));
        messages.push(draft(n + 2, "Bench night checklist", "Tue"));
        let mut store = FakeStore { accounts: Self::base_accounts(), messages, next: n + 3, older: Vec::new(), stash: Default::default(), blobs: Default::default() };
        store.add_older(1, &[THREAD[0].clone(), THREAD[1].clone()]);
        store
    }

    fn add_older(&mut self, head: u64, turns: &[crate::data::Turn]) {
        let base = self.messages.iter().find(|m| m.summary.id.0 == head).cloned().expect("head");
        let mut ids = Vec::new();
        for t in turns {
            let id = self.next;
            self.next += 1;
            let mut m = base.clone();
            m.summary.id = MessageId(id);
            m.summary.from = t.from.to_string();
            m.summary.when = t.when.to_string();
            m.summary.unread = false;
            m.summary.new = false;
            m.summary.thread = 1;
            m.sent = t.when.to_string();
            m.paragraphs = t.paragraphs.iter().map(|p| p.to_string()).collect();
            m.quoted = Vec::new();
            m.attachments = Vec::new();
            m.remote_images = None;
            m.headers = Default::default();
            self.messages.push(m);
            ids.push(id);
        }
        self.older.push((head, ids));
    }

    fn is_older(&self, id: MessageId) -> bool {
        self.older.iter().any(|(_, ids)| ids.contains(&id.0))
    }

    pub fn empty() -> FakeStore {
        FakeStore { accounts: Self::base_accounts(), messages: Vec::new(), next: 1, older: Vec::new(), stash: Default::default(), blobs: Default::default() }
    }

    pub fn failed() -> FakeStore {
        let mut s = FakeStore::concept();
        s.accounts[0].sync = SyncState::Failed;
        s.accounts[0].detail = Some("AUTHENTICATIONFAILED".into());
        s
    }

    pub fn bulk(count: usize) -> FakeStore {
        let base = inbox();
        let messages = (0..count)
            .map(|n| {
                let i = n % base.len();
                concept_message(n as u64 + 1, i, &base[i], &format!(" {}", n + 1))
            })
            .map(|mut m| {
                m.remote_images = None;
                m
            })
            .collect();
        FakeStore { accounts: Self::base_accounts(), messages, next: count as u64 + 1, older: Vec::new(), stash: Default::default(), blobs: Default::default() }
    }

    fn matches(&self, m: &Message, q: &Query) -> bool {
        if m.folder != q.folder || self.is_older(m.summary.id) || q.account.is_some_and(|a| a != m.summary.account) {
            return false;
        }
        match q.filter {
            Filter::All => {}
            Filter::Unread if !m.summary.unread => return false,
            Filter::Flagged if !m.summary.flagged => return false,
            _ => {}
        }
        if q.text.is_empty() {
            return true;
        }
        let needle = q.text.to_lowercase();
        m.summary.from.to_lowercase().contains(&needle) || m.summary.subject.to_lowercase().contains(&needle)
    }

    fn find(&mut self, id: MessageId) -> Result<&mut Message, StoreError> {
        self.messages.iter_mut().find(|m| m.summary.id == id).ok_or(StoreError::NotFound)
    }
}

impl MailStore for FakeStore {
    fn accounts(&self) -> Vec<AccountInfo> {
        let mut out = self.accounts.clone();
        for (i, a) in out.iter_mut().enumerate() {
            a.unread = self
                .messages
                .iter()
                .filter(|m| m.folder == Folder::Inbox && m.summary.account == i && m.summary.unread)
                .count();
        }
        out
    }

    fn folders(&self) -> Vec<FolderInfo> {
        Folder::UNIFIED
            .iter()
            .map(|f| {
                let inside = self.messages.iter().filter(|m| m.folder == *f);
                FolderInfo {
                    folder: f.clone(),
                    unread: inside.clone().filter(|m| m.summary.unread).count(),
                    total: inside.count(),
                }
            })
            .collect()
    }

    fn labels(&self) -> Vec<LabelInfo> {
        LABELS.iter().map(|(n, u)| LabelInfo { name: n.to_string(), unread: *u as usize }).collect()
    }

    fn counts(&self, query: &Query) -> Counts {
        let base = Query { filter: Filter::All, ..query.clone() };
        let mut c = Counts::default();
        for m in self.messages.iter().filter(|m| self.matches(m, &base)) {
            c.all += 1;
            c.unread += m.summary.unread as usize;
            c.flagged += m.summary.flagged as usize;
        }
        c
    }

    fn list(&self, query: &Query, offset: usize, limit: usize) -> Page {
        let mut total = 0;
        let mut items = Vec::new();
        for m in self.messages.iter().filter(|m| self.matches(m, query)) {
            if total >= offset && items.len() < limit {
                items.push(m.summary.clone());
            }
            total += 1;
        }
        Page { offset, total, items }
    }

    fn thread(&self, id: MessageId) -> Vec<ThreadItem> {
        let mut ids: Vec<u64> = self.older.iter().find(|(h, _)| *h == id.0).map(|(_, o)| o.clone()).unwrap_or_default();
        ids.push(id.0);
        ids.iter()
            .filter_map(|i| self.messages.iter().find(|m| m.summary.id.0 == *i))
            .map(|m| ThreadItem {
                summary: m.summary.clone(),
                folder: m.folder.clone(),
                from_address: m.from_address.clone(),
                to: m.to.clone(),
                sent: m.sent.clone(),
                draft: m.folder == Folder::Drafts,
                mine: m.summary.from == "me" || m.folder == Folder::Sent || self.accounts.iter().any(|a| a.address.eq_ignore_ascii_case(&m.from_address)),
            })
            .collect()
    }

    fn get(&self, id: MessageId) -> Result<Message, StoreError> {
        self.messages.iter().find(|m| m.summary.id == id).cloned().ok_or(StoreError::NotFound)
    }

    fn set_flags(&mut self, id: MessageId, change: FlagChange) -> Result<(), StoreError> {
        let m = self.find(id)?;
        if let Some(u) = change.unread {
            m.summary.unread = u;
            if !u {
                m.summary.new = false;
            }
        }
        if let Some(f) = change.flagged {
            m.summary.flagged = f;
        }
        Ok(())
    }

    fn move_to(&mut self, id: MessageId, folder: Folder) -> Result<(), StoreError> {
        self.find(id)?.folder = folder;
        Ok(())
    }

    fn attachments(&self, id: MessageId) -> Result<Vec<AttachmentData>, StoreError> {
        let m = self.messages.iter().find(|m| m.summary.id == id).ok_or(StoreError::NotFound)?;
        Ok(m.attachments
            .iter()
            .map(|a| AttachmentData {
                name: a.name.clone(),
                mime: a.mime.clone(),
                data: std::sync::Arc::new(self.blobs.get(&a.name).cloned().unwrap_or_else(|| format!("fake content of {}", a.name).into_bytes())),
            })
            .collect())
    }

    fn append(&mut self, folder: Folder, message: Outgoing) -> Result<MessageId, StoreError> {
        let id = MessageId(self.next);
        self.next += 1;
        let mut m = draft(id.0, &message.subject, "09:58");
        m.folder = folder;
        m.summary.account = message.account;
        m.to = vec![message.to.clone()];
        m.cc = if message.cc.is_empty() { Vec::new() } else { vec![message.cc.clone()] };
        m.paragraphs = message.body.split("\n\n").map(|p| p.to_string()).collect();
        m.attachments = message
            .attachments
            .iter()
            .map(|a| AttachmentInfo { name: a.name.clone(), size: human_size(a.data.len()), mime: a.mime.clone() })
            .collect();
        m.message_id = message.message_id.clone();
        m.references = message.references.clone();
        if message.agent {
            m.headers.all.push((AGENT_HEADER.to_string(), "1".to_string()));
        }
        self.stash.insert(id.0, message);
        self.messages.insert(0, m);
        Ok(id)
    }

    fn save_draft(&mut self, message: Outgoing) -> Result<String, StoreError> {
        let mid = message.message_id.clone().unwrap_or_else(|| format!("draft-{}", self.next));
        self.messages.retain(|m| !(m.folder == Folder::Drafts && m.message_id.as_deref() == Some(mid.as_str())));
        let mut message = message;
        message.message_id = Some(mid.clone());
        self.append(Folder::Drafts, message)?;
        Ok(mid)
    }

    fn open_draft(&mut self, id: MessageId) -> Result<Outgoing, StoreError> {
        self.stash.get(&id.0).cloned().ok_or(StoreError::NotFound)
    }

    fn discard_draft(&mut self, _account: usize, message_id: &str) {
        self.messages.retain(|m| !(m.folder == Folder::Drafts && m.message_id.as_deref() == Some(message_id)));
    }

    fn undo_send(&mut self, id: MessageId) -> Result<Option<Outgoing>, StoreError> {
        let Some(out) = self.stash.remove(&id.0) else { return Ok(None) };
        self.messages.retain(|m| m.summary.id != id);
        Ok(Some(out))
    }

    fn suggest(&self, text: &str, limit: usize) -> Vec<Contact> {
        let needle = text.to_lowercase();
        let mut seen: Vec<Contact> = Vec::new();
        for m in &self.messages {
            let c = Contact { name: m.summary.from.clone(), address: m.from_address.clone() };
            if m.summary.from != "me" && !seen.iter().any(|s| s.address == c.address) {
                seen.push(c);
            }
        }
        seen.into_iter().filter(|c| c.name.to_lowercase().contains(&needle) || c.address.to_lowercase().contains(&needle)).take(limit).collect()
    }
}
