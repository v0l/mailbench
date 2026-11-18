use crate::data::{ACCOUNTS, ATTACHMENTS, AccountState, BLOCKED_HOST, LABELS, THREAD, inbox};
use crate::mailstore::*;

pub struct FakeStore {
    accounts: Vec<AccountInfo>,
    messages: Vec<Message>,
    next: u64,
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
        ATTACHMENTS.iter().map(|(n, s)| AttachmentInfo { name: n.to_string(), size: s.to_string() }).collect()
    } else if m.attach {
        vec![AttachmentInfo { name: "attachment.pdf".into(), size: "120 KB".into() }]
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
        },
        folder: Folder::Inbox,
        from_address: format!("{}@example.test", m.from.to_lowercase().replace(' ', ".")),
        to: if first { vec!["me".into(), "Wren Okafor".into()] } else { vec!["me".into()] },
        cc: Vec::new(),
        sent: if first { "Fri 17 Nov 09:41".into() } else { m.when.to_string() },
        paragraphs,
        blocks: Vec::new(),
        quoted,
        attachments,
        remote_images: if first { Some(BLOCKED_HOST.to_string()) } else { None },
        headers: Default::default(),
        body_state: Default::default(),
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
        },
        folder: Folder::Drafts,
        from_address: "mara.ilves@kestrel-instruments.example".into(),
        to: vec!["Jonas Weber".into()],
        cc: Vec::new(),
        sent: when.into(),
        paragraphs: vec!["Not finished.".into()],
        blocks: Vec::new(),
        quoted: Vec::new(),
        attachments: Vec::new(),
        remote_images: None,
        headers: Default::default(),
        body_state: Default::default(),
    }
}

impl FakeStore {
    fn base_accounts() -> Vec<AccountInfo> {
        ACCOUNTS
            .iter()
            .map(|a| AccountInfo {
                name: a.name.into(),
                address: a.address.into(),
                unread: 0,
                sync: sync_of(a.state),
                detail: None,
            })
            .collect()
    }

    pub fn concept() -> FakeStore {
        let mut messages: Vec<Message> =
            inbox().iter().enumerate().map(|(i, m)| concept_message(i as u64 + 1, i, m, "")).collect();
        let n = messages.len() as u64;
        messages.push(draft(n + 1, "Re: Firmware release notes for 2.4", "Wed"));
        messages.push(draft(n + 2, "Bench night checklist", "Tue"));
        FakeStore { accounts: Self::base_accounts(), messages, next: n + 3 }
    }

    pub fn empty() -> FakeStore {
        FakeStore { accounts: Self::base_accounts(), messages: Vec::new(), next: 1 }
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
        FakeStore { accounts: Self::base_accounts(), messages, next: count as u64 + 1 }
    }

    fn matches(m: &Message, q: &Query) -> bool {
        if m.folder != q.folder {
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
                    folder: *f,
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
        for m in self.messages.iter().filter(|m| Self::matches(m, &base)) {
            c.all += 1;
            c.unread += m.summary.unread as usize;
            c.flagged += m.summary.flagged as usize;
        }
        c
    }

    fn list(&self, query: &Query, offset: usize, limit: usize) -> Page {
        let mut total = 0;
        let mut items = Vec::new();
        for m in self.messages.iter().filter(|m| Self::matches(m, query)) {
            if total >= offset && items.len() < limit {
                items.push(m.summary.clone());
            }
            total += 1;
        }
        Page { offset, total, items }
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

    fn append(&mut self, folder: Folder, message: Outgoing) -> Result<MessageId, StoreError> {
        let id = MessageId(self.next);
        self.next += 1;
        let mut m = draft(id.0, &message.subject, "09:58");
        m.folder = folder;
        m.summary.account = message.account;
        m.to = vec![message.to];
        m.cc = if message.cc.is_empty() { Vec::new() } else { vec![message.cc] };
        m.paragraphs = message.body.split("\n\n").map(|p| p.to_string()).collect();
        self.messages.insert(0, m);
        Ok(id)
    }
}
