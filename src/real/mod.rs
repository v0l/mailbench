mod backfill;
mod display;
mod headers;
mod model;
mod threads;
mod search;
mod worker;

use crate::backend::Folder as Remote;
use crate::backend::*;
use crate::mailstore::{self as ms, *};
use crate::store::Store;
use model::*;
use std::collections::HashMap;
use std::result::Result;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const OUTBOX: &str = "outbox";
const QUICK_PAGE: usize = 500;
const FETCH_RETRY: Duration = Duration::from_secs(5);
const LOCAL: &str = "local";

pub struct AccountSetup {
    pub id: String,
    pub name: String,
    pub address: String,
    pub sender: String,
    pub backend: Box<dyn Backend>,
}

pub(crate) struct AccountMeta {
    pub id: String,
    pub name: String,
    pub address: String,
    pub sender: String,
}

#[derive(Clone)]
struct Entry {
    id: u64,
    account: usize,
    folder: ms::Folder,
    server_folder: String,
    uid: u32,
    validity: u32,
    local: Option<String>,
    from: String,
    from_address: String,
    to: Vec<String>,
    cc: Vec<String>,
    subject: String,
    date: i64,
    unread: bool,
    flagged: bool,
    attachments: bool,
    search_key: String,
    thread: u64,
    head: bool,
    count: u32,
    thread_unread: bool,
    thread_flagged: bool,
    to_key: String,
}

fn people_key(to: &[Address], cc: &[Address]) -> String {
    to.iter()
        .chain(cc)
        .map(|a| format!("{} {}", a.name.as_deref().unwrap_or_default(), a.address))
        .collect::<Vec<_>>()
        .join(" ")
}

fn search_key(from: &str, address: &str, subject: &str) -> String {
    format!("{from}\0{address}\0{subject}").to_lowercase()
}

impl Cache {
    fn index(&mut self) {
        self.by_id = self.entries.iter().enumerate().map(|(i, e)| (e.id, i)).collect();
        self.by_thread.clear();
        for (i, e) in self.entries.iter().enumerate() {
            self.by_thread.entry(e.thread).or_default().push(i);
        }
        let threads: Vec<u64> = self.by_thread.keys().copied().collect();
        for t in threads {
            self.reindex(t);
        }
        self.index_folders();
    }

    fn reindex(&mut self, thread: u64) {
        let Some(members) = self.by_thread.get(&thread) else { return };
        let count = members.len() as u32;
        let unread = members.iter().any(|i| self.entries[*i].unread);
        let flagged = members.iter().any(|i| self.entries[*i].flagged);
        let mut folders: Vec<ms::Folder> = Vec::new();
        for i in members.clone() {
            let e = &mut self.entries[i];
            e.head = !folders.contains(&e.folder);
            if e.head {
                folders.push(e.folder.clone());
            }
            e.count = count;
            e.thread_unread = unread;
            e.thread_flagged = flagged;
        }
    }
}

#[derive(Default, Clone)]
struct Cache {
    entries: Vec<Entry>,
    by_id: HashMap<u64, usize>,
    by_thread: HashMap<u64, Vec<usize>>,
    by_folder: HashMap<ms::Folder, Vec<usize>>,
    folder_counts: HashMap<ms::Folder, Counts>,
    inbox_unread: Vec<usize>,
    shape: Vec<TreeFolder>,
    validity: HashMap<(usize, String), u32>,
}

impl Cache {
    fn index_folders(&mut self) {
        self.by_id = HashMap::with_capacity(self.entries.len());
        self.by_folder.clear();
        self.folder_counts.clear();
        self.inbox_unread.clear();
        for (i, e) in self.entries.iter().enumerate() {
            self.by_id.insert(e.id, i);
            if e.folder == ms::Folder::Inbox && e.unread {
                if self.inbox_unread.len() <= e.account {
                    self.inbox_unread.resize(e.account + 1, 0);
                }
                self.inbox_unread[e.account] += 1;
            }
            if !e.head {
                continue;
            }
            self.by_folder.entry(e.folder.clone()).or_default().push(i);
            let counts = self.folder_counts.entry(e.folder.clone()).or_default();
            counts.all += 1;
            counts.unread += e.thread_unread as usize;
            counts.flagged += e.thread_flagged as usize;
        }
    }

    fn tree(&self) -> Vec<TreeFolder> {
        let mut out = self.shape.clone();
        let mut carried: Vec<usize> = Vec::new();
        let mut totals: Vec<usize> = Vec::new();
        for node in out.iter_mut().rev() {
            let own = self.folder_counts.get(&node.folder).copied().unwrap_or_default();
            let depth = node.depth;
            if carried.len() < depth + 2 {
                carried.resize(depth + 2, 0);
                totals.resize(depth + 2, 0);
            }
            node.unread = own.unread + std::mem::take(&mut carried[depth + 1]);
            node.total = own.all + std::mem::take(&mut totals[depth + 1]);
            carried[depth] += node.unread;
            totals[depth] += node.total;
        }
        out
    }
}

fn tree_shape(account: usize, folders: &[Remote]) -> Vec<TreeFolder> {
    let delimiter = folders.iter().find_map(|f| f.delimiter.clone()).unwrap_or_else(|| DEFAULT_DELIMITER.to_string());
    let delimiter_of = |f: &Remote| f.delimiter.clone().unwrap_or_else(|| delimiter.clone());
    let mut nodes: std::collections::BTreeMap<Vec<String>, (String, String, bool)> = Default::default();
    for f in folders.iter().filter(|f| ui_folder(f).is_none()) {
        let d = delimiter_of(f);
        let parts: Vec<String> = f.name.split(d.as_str()).map(str::to_string).collect();
        for n in 1..=parts.len() {
            let path = parts[..n].join(&d);
            let exists = folders.iter().find(|x| x.name == path);
            if exists.is_some_and(|x| ui_folder(x).is_some()) {
                continue;
            }
            let selectable = exists.is_some_and(|x| x.selectable);
            let entry = nodes.entry(parts[..n].to_vec()).or_insert((path, d.clone(), selectable));
            entry.2 |= selectable;
        }
    }
    let paths: std::collections::HashSet<Vec<String>> = nodes.keys().cloned().collect();
    nodes
        .iter()
        .map(|(parts, (path, d, selectable))| {
            let depth = (1..parts.len()).filter(|n| paths.contains(&parts[..*n])).count();
            let children = nodes.keys().any(|k| k.len() > parts.len() && k[..parts.len()] == parts[..]);
            TreeFolder {
                folder: ms::Folder::Other(account, path.clone()),
                account,
                path: path.clone(),
                delimiter: d.clone(),
                leaf: parts.last().cloned().unwrap_or_default(),
                depth,
                selectable: *selectable,
                unread: 0,
                total: 0,
                children,
            }
        })
        .collect()
}

pub(crate) struct Shared {
    store: Mutex<Store>,
    pub accounts: Vec<AccountMeta>,
    states: Mutex<Vec<(SyncState, Option<String>)>>,
    ctx: Mutex<Option<egui::Context>>,
    pub stop: AtomicBool,
    generation: AtomicU64,
    user_gen: AtomicU64,
    cache: Mutex<Arc<Cache>>,
    known_attachments: Mutex<HashMap<u64, bool>>,
    fetch_state: Mutex<HashMap<u64, Option<std::time::Instant>>>,
    opened: Mutex<Vec<(u64, Message)>>,
    parse_pending: Mutex<std::collections::HashSet<u64>>,
    parser: Mutex<Option<std::sync::mpsc::Sender<Entry>>>,
    fetchers: Mutex<Vec<tokio::sync::mpsc::UnboundedSender<worker::FetchRequest>>>,
    searchers: Mutex<Vec<tokio::sync::mpsc::UnboundedSender<worker::SearchRequest>>>,
    search: Mutex<search::SearchState>,
    search_tx: Mutex<Option<std::sync::mpsc::Sender<search::SearchJob>>>,
    index_epoch: AtomicU64,
    policies: Mutex<Vec<crate::search::BackfillPolicy>>,
    progress: Mutex<Vec<Option<(usize, usize)>>>,
    backfill_wake: Mutex<Vec<Arc<tokio::sync::Notify>>>,
    wake: Condvar,
    wake_lock: Mutex<()>,
    seq: AtomicU64,
    kicks: Mutex<Vec<tokio::sync::mpsc::UnboundedSender<()>>>,
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn folder_for(account: usize, folders: &[crate::backend::Folder], name: &str) -> ms::Folder {
    folders
        .iter()
        .find(|f| f.name == name)
        .and_then(ui_folder)
        .or_else(|| {
            [ms::Folder::Archive, ms::Folder::Trash, ms::Folder::Junk, ms::Folder::Sent, ms::Folder::Drafts, ms::Folder::Inbox]
                .into_iter()
                .find(|f| default_name(f) == name)
        })
        .unwrap_or_else(|| ms::Folder::Other(account, name.to_string()))
}

fn backend_err(e: impl std::fmt::Display) -> StoreError {
    StoreError::Backend(e.to_string())
}

fn stable_id(key: &str) -> u64 {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(key.as_bytes());
    u64::from_be_bytes(digest[..8].try_into().expect("8 bytes")) & (u64::MAX >> 1)
}

impl Shared {
    pub fn lock_store(&self) -> MutexGuard<'_, Store> {
        lock(&self.store)
    }

    pub fn set_state(&self, index: usize, state: SyncState, detail: Option<String>) {
        lock(&self.states)[index] = (state, detail);
    }

    pub fn refresh(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.wake.notify_all();
        if let Some(ctx) = lock(&self.ctx).as_ref() {
            ctx.request_repaint();
        }
    }

    fn patch(&self, id: u64, f: impl FnOnce(&mut Entry)) {
        let mut guard = lock(&self.cache);
        let cache = Arc::make_mut(&mut guard);
        if let Some(i) = cache.by_id.get(&id).copied() {
            f(&mut cache.entries[i]);
            let thread = cache.entries[i].thread;
            cache.reindex(thread);
            cache.index_folders();
        }
    }

    fn insert(&self, entry: Entry) {
        let mut guard = lock(&self.cache);
        let cache = Arc::make_mut(&mut guard);
        let at = cache.entries.partition_point(|x| (x.date, x.id) > (entry.date, entry.id));
        cache.entries.insert(at, entry);
        cache.index();
    }

    fn request_body(&self, e: &Entry) -> BodyState {
        let mut state = lock(&self.fetch_state);
        match state.get(&e.id) {
            Some(None) => return BodyState::Loading,
            Some(Some(at)) if at.elapsed() < FETCH_RETRY => return BodyState::Failed,
            _ => {}
        }
        let sent = lock(&self.fetchers)
            .get(e.account)
            .is_some_and(|tx| tx.send(worker::FetchRequest { id: e.id, folder: e.server_folder.clone(), uid: e.uid }).is_ok());
        if sent {
            state.insert(e.id, None);
            BodyState::Loading
        } else {
            BodyState::Failed
        }
    }

    fn body_failed(&self, id: u64) -> bool {
        lock(&self.fetch_state).get(&id).is_some_and(|s| s.is_some_and(|at| at.elapsed() < FETCH_RETRY))
    }

    pub fn finish_fetch(&self, id: u64, has_attachments: Option<bool>) {
        {
            let mut state = lock(&self.fetch_state);
            match has_attachments {
                Some(_) => {
                    state.remove(&id);
                }
                None => {
                    state.insert(id, Some(std::time::Instant::now()));
                }
            }
        }
        lock(&self.parse_pending).remove(&id);
        if let Some(a) = has_attachments {
            self.index_epoch.fetch_add(1, Ordering::SeqCst);
            lock(&self.known_attachments).insert(id, a);
            self.patch(id, |e| e.attachments = a);
            let entry = {
                let cache = lock(&self.cache);
                cache.by_id.get(&id).map(|i| cache.entries[*i].clone())
            };
            if let Some(e) = entry {
                self.queue_parse(&e);
            }
        }
        self.repaint();
    }

    fn queue_parse(&self, e: &Entry) {
        if lock(&self.parse_pending).insert(e.id) {
            if let Some(tx) = lock(&self.parser).as_ref() {
                let _ = tx.send(e.clone());
            }
        }
    }

    fn parse_one(&self, e: &Entry) {
        let raw = {
            let store = self.lock_store();
            StoreSink::new(&store, &self.accounts[e.account].id, true).body(&e.server_folder, e.uid).ok().flatten()
        };
        let Some(raw) = raw else {
            if self.request_body(e) == BodyState::Failed {
                lock(&self.parse_pending).remove(&e.id);
            }
            self.repaint();
            return;
        };
        let body = display::parse_body(&raw);
        let headers = headers::parse_headers(&raw);
        let message = Message {
            summary: summary(e, now()),
            folder: e.folder.clone(),
            from_address: e.from_address.clone(),
            to: e.to.clone(),
            cc: e.cc.clone(),
            sent: display::sent(e.date),
            paragraphs: body.paragraphs,
            html: body.html,
            quoted: body.quoted,
            attachments: body.attachments,
            remote_images: body.remote_images,
            headers,
            body_state: BodyState::Ready,
        };
        {
            let mut opened = lock(&self.opened);
            opened.retain(|(i, _)| *i != e.id);
            opened.push((e.id, message));
            if opened.len() > 16 {
                opened.remove(0);
            }
        }
        lock(&self.parse_pending).remove(&e.id);
        self.repaint();
    }

    pub fn repaint(&self) {
        if let Some(ctx) = lock(&self.ctx).as_ref() {
            ctx.request_repaint();
        }
    }

    fn entry_id(&self, index: usize, folder: &str, uid: u32) -> Option<u64> {
        self.with_cache(|c| {
            c.entries.iter().find(|e| e.account == index && e.uid == uid && e.server_folder == folder).map(|e| e.id)
        })
    }

    pub fn note_attachments(&self, id: u64, has: bool) {
        lock(&self.known_attachments).insert(id, has);
        self.patch(id, |e| e.attachments = has);
    }

    fn kick(&self, account: usize) {
        if let Some(k) = lock(&self.kicks).get(account) {
            let _ = k.send(());
        }
    }

    fn index_of(&self, account: &str) -> Option<usize> {
        self.accounts.iter().position(|a| a.id == account)
    }

    pub fn validity(&self, account: &str, folder: &str) -> Option<u32> {
        let store = self.lock_store();
        StoreSink::new(&store, account, true).sync_token(folder).ok().flatten().map(|t| t.uid_validity)
    }

    pub fn stored_flags(&self, account: &str, folder: &str, uid: u32) -> Option<Flags> {
        let store = self.lock_store();
        StoreSink::new(&store, account, true).summary(folder, uid).ok().flatten().map(|m| m.flags)
    }

    pub fn apply_flags(&self, account: &str, folder: &str, uid: u32, seen: Option<bool>, flagged: Option<bool>) {
        let store = self.lock_store();
        let mut sink = StoreSink::new(&store, account, true);
        let Ok(Some(m)) = sink.summary(folder, uid) else { return };
        let mut flags = m.flags;
        if let Some(s) = seen {
            flags.seen = s;
        }
        if let Some(f) = flagged {
            flags.flagged = f;
        }
        let _ = sink.put_flags(folder, &[(uid, flags)]);
    }

    pub fn folders_of(&self, account: &str) -> Vec<crate::backend::Folder> {
        let store = self.lock_store();
        StoreSink::new(&store, account, true).folders().unwrap_or_default()
    }

    fn outbox_in(store: &Store) -> Vec<(String, Op)> {
        let mut out = Vec::new();
        for key in store.list(OUTBOX).unwrap_or_default() {
            if let Ok(Some(raw)) = store.get(OUTBOX, &key) {
                if let Ok(op) = serde_json::from_slice::<Op>(&raw) {
                    out.push((key, op));
                }
            }
        }
        out
    }

    pub fn outbox(&self, account: &str) -> Vec<(String, Op)> {
        Self::outbox_in(&self.lock_store()).into_iter().filter(|(_, op)| op.account() == account).collect()
    }

    pub fn remove_ops(&self, keys: &[String]) {
        let store = self.lock_store();
        for k in keys {
            let _ = store.delete(OUTBOX, k);
        }
        drop(store);
        self.refresh();
    }

    fn enqueue(&self, store: &Store, op: &Op) -> Result<(), StoreError> {
        let n = self.seq.fetch_add(1, Ordering::SeqCst);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let key = format!("{nanos:030}{n:06}");
        let raw = serde_json::to_vec(op).map_err(backend_err)?;
        store.put(OUTBOX, &key, &raw).map_err(backend_err)
    }

    fn queue_folder_op(&self, account: usize, op: Op) -> Result<(), StoreError> {
        {
            let store = self.lock_store();
            self.enqueue(&store, &op)?;
        }
        self.user_gen.fetch_add(1, Ordering::SeqCst);
        self.refresh();
        self.kick(account);
        Ok(())
    }

    pub fn outgoing(&self, local: &str) -> Option<OutgoingMessage> {
        let store = self.lock_store();
        let raw = store.get(LOCAL, local).ok().flatten()?;
        let msg: LocalMsg = serde_json::from_slice(&raw).ok()?;
        let account = self.accounts.iter().find(|a| a.id == msg.account)?;
        let list = |s: &str| {
            s.split(',')
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(|a| Address { name: None, address: a.to_string() })
                .collect::<Vec<_>>()
        };
        Some(OutgoingMessage {
            from: Address {
                name: Some(if account.sender.is_empty() { account.name.clone() } else { account.sender.clone() }),
                address: account.address.clone(),
            },
            to: list(&msg.to),
            cc: list(&msg.cc),
            subject: msg.subject,
            body: msg.body,
            ..Default::default()
        })
    }

    fn build(&self, quick: bool) -> Cache {
        let ops = Self::outbox_in(&self.lock_store());
        let known = lock(&self.known_attachments).clone();
        let mut cache = Cache::default();
        let mut meta: Vec<(Option<String>, Vec<String>)> = Vec::new();
        for (index, account) in self.accounts.iter().enumerate() {
            let stored = {
                let store = self.lock_store();
                StoreSink::new(&store, &account.id, true).folders().unwrap_or_default()
            };
            let account_ops: Vec<&Op> = ops.iter().map(|(_, op)| op).filter(|op| op.account() == account.id).collect();
            let shown_folders = with_pending(stored, &account_ops);
            let folders: Vec<Remote> = shown_folders.iter().map(|(_, f)| f.clone()).collect();
            cache.shape.extend(tree_shape(index, &folders));
            let mut pending: HashMap<(String, u32), String> = HashMap::new();
            let mut flag_ops: HashMap<(String, u32), (Option<bool>, Option<bool>)> = HashMap::new();
            for (_, op) in &ops {
                if let Op::Flags { account: a, folder, uid, seen, flagged, .. } = op {
                    if *a == account.id {
                        let e = flag_ops.entry((folder.clone(), *uid)).or_default();
                        e.0 = seen.or(e.0);
                        e.1 = flagged.or(e.1);
                    }
                }
            }
            for (_, op) in &ops {
                if let Op::Move { account: a, folder, uid, to, .. } = op {
                    if *a == account.id {
                        pending.insert((folder.clone(), *uid), to.clone());
                    }
                }
            }
            for (stored_name, folder) in shown_folders.iter().filter(|(_, f)| f.selectable) {
                let Some(stored_name) = stored_name else { continue };
                let ui = ui_folder(folder).unwrap_or_else(|| ms::Folder::Other(index, folder.name.clone()));
                let validity = {
                    let store = self.lock_store();
                    StoreSink::new(&store, &account.id, true).sync_token(stored_name).ok().flatten().map(|t| t.uid_validity).unwrap_or(0)
                };
                cache.validity.insert((index, folder.name.clone()), validity);
                let mut after: Option<String> = None;
                loop {
                let page = {
                    let store = self.lock_store();
                    let sink = StoreSink::new(&store, &account.id, true);
                    if quick { sink.newest_page(stored_name, None, QUICK_PAGE) } else { sink.messages_page(stored_name, after.as_deref(), 2000) }.unwrap_or_default()
                };
                let Some((last, _)) = page.last() else { break };
                after = if quick { None } else { Some(last.clone()) };
                for m in page.into_iter().map(|(_, m)| m) {
                    if m.flags.deleted {
                        continue;
                    }
                    let id = stable_id(&format!("r\0{}\0{}\0{}\0{}", account.id, stored_name, validity, m.uid));
                    let shown = pending
                        .get(&(stored_name.clone(), m.uid))
                        .map(|to| folder_for(index, &folders, to))
                        .unwrap_or_else(|| ui.clone());
                    let attachments = known.get(&id).copied().unwrap_or(false);
                    let from: String = m.from.first().map(display::person).unwrap_or_default();
                    let from_address: String = m.from.first().map(|a| a.address.clone()).unwrap_or_default();
                    let subject: String = m.subject.clone().unwrap_or_default();
                    let mut refs = m.references;
                    refs.extend(m.in_reply_to);
                    meta.push((m.message_id, refs));
                    cache.entries.push(Entry {
                        id,
                        account: index,
                        folder: shown,
                        server_folder: stored_name.clone(),
                        uid: m.uid,
                        validity,
                        local: None,
                        search_key: search_key(&from, &from_address, &subject),
                        to_key: people_key(&m.to, &m.cc),
                        from,
                        from_address,
                        to: display::people(&m.to),
                        cc: display::people(&m.cc),
                        subject,
                        date: m.date.unwrap_or(0),
                        unread: flag_ops.get(&(stored_name.clone(), m.uid)).and_then(|f| f.0).map(|s| !s).unwrap_or(!m.flags.seen),
                        flagged: flag_ops.get(&(stored_name.clone(), m.uid)).and_then(|f| f.1).unwrap_or(m.flags.flagged),
                        attachments,
                        thread: id,
                        head: true,
                        count: 1,
                        thread_unread: false,
                        thread_flagged: false,
                    });
                }
                if quick {
                    break;
                }
                }
            }
        }
        let store = self.lock_store();
        for key in store.list(LOCAL).unwrap_or_default() {
            let Some(raw) = store.get(LOCAL, &key).ok().flatten() else { continue };
            let Ok(msg) = serde_json::from_slice::<LocalMsg>(&raw) else { continue };
            let Some(index) = self.index_of(&msg.account) else { continue };
            let Some(folder) = folder_from_name(&msg.folder) else { continue };
            meta.push((None, Vec::new()));
            let local_id = stable_id(&format!("l\0{key}"));
            cache.entries.push(Entry {
                id: local_id,
                thread: local_id,
                head: true,
                count: 1,
                thread_unread: false,
                thread_flagged: false,
                account: index,
                folder,
                server_folder: String::new(),
                uid: 0,
                validity: 0,
                local: Some(key),
                from: "me".into(),
                from_address: self.accounts[index].address.clone(),
                to: vec![msg.to.clone()],
                cc: if msg.cc.is_empty() { Vec::new() } else { vec![msg.cc.clone()] },
                subject: msg.subject.clone(),
                search_key: search_key("me", &self.accounts[index].address, &msg.subject),
                to_key: format!("{} {}", msg.to, msg.cc),
                date: msg.date,
                unread: false,
                flagged: false,
                attachments: false,
            });
        }
        let inputs: Vec<threads::Input> = cache
            .entries
            .iter()
            .zip(&meta)
            .map(|(e, (id, refs))| threads::Input { message_id: id.as_deref(), refs, subject: &e.subject, date: e.date })
            .collect();
        let ids: Vec<u64> = cache.entries.iter().map(|e| e.id).collect();
        let assigned = threads::assign(&inputs, &ids);
        drop(inputs);
        for (e, t) in cache.entries.iter_mut().zip(assigned) {
            e.thread = t;
        }
        cache.entries.sort_by(|a, b| b.date.cmp(&a.date).then(b.id.cmp(&a.id)));
        cache.index();
        cache
    }

    fn with_cache<T>(&self, f: impl FnOnce(&Cache) -> T) -> T {
        let cache = lock(&self.cache).clone();
        f(&cache)
    }
}

fn builder(shared: Arc<Shared>) {
    let mut seen = 0;
    let mut first = true;
    while !shared.stop.load(Ordering::Relaxed) {
        let generation = shared.generation.load(Ordering::SeqCst);
        if generation == seen {
            let guard = lock(&shared.wake_lock);
            let _ = shared.wake.wait_timeout(guard, Duration::from_millis(250));
            continue;
        }
        let user = shared.user_gen.load(Ordering::SeqCst);
        let fresh = Arc::new(shared.build(first));
        {
            let mut current = lock(&shared.cache);
            if shared.user_gen.load(Ordering::SeqCst) != user {
                continue;
            }
            *current = fresh;
        }
        if !first {
            seen = generation;
        }
        first = false;
        if let Some(ctx) = lock(&shared.ctx).as_ref() {
            ctx.request_repaint();
        }
    }
}

pub fn purge_account(store: &Store, account: &str) -> Result<(), StoreError> {
    StoreSink::purge(store, account).map_err(backend_err)?;
    for (key, op) in Shared::outbox_in(store) {
        if op.account() == account {
            let _ = store.delete(OUTBOX, &key);
        }
    }
    for key in store.list(LOCAL).unwrap_or_default() {
        let raw = store.get(LOCAL, &key).ok().flatten();
        if let Some(msg) = raw.and_then(|r| serde_json::from_slice::<LocalMsg>(&r).ok()) {
            if msg.account == account {
                let _ = store.delete(LOCAL, &key);
            }
        }
    }
    Ok(())
}

pub struct RealStore {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}


fn summary_from(e: &Entry, now: i64, remote: bool) -> Summary {
    Summary {
        remote,
        id: MessageId(e.id),
        from: e.from.clone(),
        subject: e.subject.clone(),
        when: display::when(e.date, now),
        unread: e.unread,
        flagged: e.flagged,
        new: e.unread,
        attachments: e.attachments,
        account: e.account,
        thread: e.count,
    }
}

fn row(e: &Entry, now: i64, remote: bool) -> Summary {
    Summary { unread: e.thread_unread, new: e.thread_unread, flagged: e.thread_flagged, ..summary_from(e, now, remote) }
}

impl RealStore {
    pub fn open(store: Store, accounts: Vec<AccountSetup>) -> RealStore {
        let metas = accounts
            .iter()
            .map(|a| AccountMeta { id: a.id.clone(), name: a.name.clone(), address: a.address.clone(), sender: a.sender.clone() })
            .collect::<Vec<_>>();
        let shared = Arc::new(Shared {
            store: Mutex::new(store),
            states: Mutex::new(vec![(SyncState::Syncing, None); metas.len()]),
            accounts: metas,
            ctx: Mutex::new(None),
            stop: AtomicBool::new(false),
            generation: AtomicU64::new(1),
            user_gen: AtomicU64::new(0),
            cache: Mutex::new(Arc::new(Cache::default())),
            known_attachments: Mutex::new(HashMap::new()),
            fetch_state: Mutex::new(HashMap::new()),
            opened: Mutex::new(Vec::new()),
            parse_pending: Mutex::new(std::collections::HashSet::new()),
            parser: Mutex::new(None),
            fetchers: Mutex::new(Vec::new()),
            wake: Condvar::new(),
            wake_lock: Mutex::new(()),
            seq: AtomicU64::new(0),
            kicks: Mutex::new(Vec::new()),
            searchers: Mutex::new(Vec::new()),
            search: Mutex::new(search::SearchState::default()),
            search_tx: Mutex::new(None),
            index_epoch: AtomicU64::new(0),
            policies: Mutex::new(vec![Default::default(); accounts.len()]),
            progress: Mutex::new(vec![None; accounts.len()]),
            backfill_wake: Mutex::new(Vec::new()),
        });
        let mut threads = Vec::new();
        let mut tasks = Vec::new();
        let background = shared.clone();
        threads.push(std::thread::spawn(move || builder(background)));
        let (parse_tx, parse_rx) = channel();
        *lock(&shared.parser) = Some(parse_tx);
        let parsing = shared.clone();
        threads.push(std::thread::spawn(move || {
            for e in parse_rx {
                if parsing.stop.load(Ordering::Relaxed) {
                    break;
                }
                parsing.parse_one(&e);
            }
        }));
        let (job_tx, job_rx) = channel();
        *lock(&shared.search_tx) = Some(job_tx);
        let searching = shared.clone();
        threads.push(std::thread::spawn(move || search::serve(searching, job_rx)));
        for (index, setup) in accounts.into_iter().enumerate() {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            lock(&shared.kicks).push(tx);
            let (fetch_tx, fetch_rx) = tokio::sync::mpsc::unbounded_channel();
            lock(&shared.fetchers).push(fetch_tx);
            let wake = Arc::new(tokio::sync::Notify::new());
            lock(&shared.backfill_wake).push(wake.clone());
            if let Some(reader) = setup.backend.fork() {
                tasks.push(crate::runtime::spawn(worker::serve_fetches(shared.clone(), index, reader, fetch_rx)));
            }
            let (remote_tx, remote_rx) = tokio::sync::mpsc::unbounded_channel();
            lock(&shared.searchers).push(remote_tx);
            if let Some(seeker) = setup.backend.fork() {
                tasks.push(crate::runtime::spawn(worker::serve_searches(shared.clone(), index, seeker, remote_rx)));
            }
            if let Some(filler) = setup.backend.fork() {
                tasks.push(crate::runtime::spawn(backfill::run(shared.clone(), index, filler, wake)));
            }
            tasks.push(crate::runtime::spawn(worker::run(shared.clone(), index, setup.backend, rx)));
        }
        RealStore { shared, threads, tasks }
    }

    pub fn set_backfill(&self, account: &str, policy: crate::search::BackfillPolicy) {
        let Some(i) = self.shared.index_of(account) else { return };
        lock(&self.shared.policies)[i] = policy;
        if let Some(w) = lock(&self.shared.backfill_wake).get(i) {
            w.notify_one();
        }
    }

    pub fn attach(&self, ctx: &egui::Context) {
        *lock(&self.shared.ctx) = Some(ctx.clone());
        ctx.request_repaint();
    }

    pub fn pending(&self) -> usize {
        Shared::outbox_in(&self.shared.lock_store()).len()
    }

    pub fn sync_now(&self) {
        for i in 0..self.shared.accounts.len() {
            self.shared.kick(i);
        }
    }

    fn entry(&self, id: MessageId) -> Result<Entry, StoreError> {
        self.shared.with_cache(|c| c.by_id.get(&id.0).map(|i| c.entries[*i].clone())).ok_or(StoreError::NotFound)
    }
}

impl Drop for RealStore {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.wake.notify_all();
        lock(&self.shared.kicks).clear();
        lock(&self.shared.fetchers).clear();
        lock(&self.shared.searchers).clear();
        *lock(&self.shared.parser) = None;
        *lock(&self.shared.search_tx) = None;
        for t in self.tasks.drain(..) {
            t.abort();
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl MailStore for RealStore {
    fn accounts(&self) -> Vec<AccountInfo> {
        let states = lock(&self.shared.states).clone();
        self.shared.with_cache(|c| {
            self.shared
                .accounts
                .iter()
                .enumerate()
                .map(|(i, a)| AccountInfo {
                    name: a.name.clone(),
                    address: a.address.clone(),
                    unread: c.inbox_unread.get(i).copied().unwrap_or(0),
                    sync: states[i].0,
                    detail: states[i].1.clone(),
                    indexed: lock(&self.shared.progress)[i],
                })
                .collect()
        })
    }

    fn folders(&self) -> Vec<FolderInfo> {
        self.shared.with_cache(|c| {
            ms::Folder::UNIFIED
                .iter()
                .map(|f| {
                    let counts = c.folder_counts.get(f).copied().unwrap_or_default();
                    FolderInfo { folder: f.clone(), unread: counts.unread, total: counts.all }
                })
                .collect()
        })
    }

    fn labels(&self) -> Vec<LabelInfo> {
        Vec::new()
    }

    fn tree(&self) -> Vec<TreeFolder> {
        self.shared.with_cache(|c| c.tree())
    }

    fn counts(&self, query: &Query) -> Counts {
        let view = self.shared.search_view(query);
        self.shared.with_cache(|c| {
            if view.is_plain() {
                return c.folder_counts.get(&query.folder).copied().unwrap_or_default();
            }
            let base = Query { filter: Filter::All, ..query.clone() };
            let mut out = Counts::default();
            for e in c.by_folder.get(&query.folder).into_iter().flatten().map(|i| &c.entries[*i]).filter(|e| search::matches_row(c, e, &base, &view)) {
                out.all += 1;
                out.unread += e.thread_unread as usize;
                out.flagged += e.thread_flagged as usize;
            }
            out
        })
    }

    fn list(&self, query: &Query, offset: usize, limit: usize) -> Page {
        let now = now();
        let view = self.shared.search_view(query);
        self.shared.with_cache(|c| {
            let Some(index) = c.by_folder.get(&query.folder) else {
                return Page { offset, total: 0, items: Vec::new() };
            };
            if view.is_plain() && query.filter == Filter::All {
                let items = index.iter().skip(offset).take(limit).map(|i| row(&c.entries[*i], now, false)).collect();
                return Page { offset, total: index.len(), items };
            }
            let mut total = 0;
            let mut items = Vec::new();
            for e in index.iter().map(|i| &c.entries[*i]).filter(|e| search::matches_row(c, e, query, &view)) {
                if total >= offset && items.len() < limit {
                    items.push(row(e, now, view.is_remote(e.id)));
                }
                total += 1;
            }
            Page { offset, total, items }
        })
    }

    fn thread(&self, id: MessageId) -> Vec<ThreadItem> {
        let now = now();
        self.shared.with_cache(|c| {
            let Some(i) = c.by_id.get(&id.0) else { return Vec::new() };
            let Some(members) = c.by_thread.get(&c.entries[*i].thread) else { return Vec::new() };
            members
                .iter()
                .rev()
                .map(|m| ThreadItem { summary: summary(&c.entries[*m], now), folder: c.entries[*m].folder.clone() })
                .collect()
        })
    }

    fn searching(&self, query: &Query) -> bool {
        self.shared.searching(query)
    }

    fn get(&self, id: MessageId) -> Result<Message, StoreError> {
        let e = self.entry(id)?;
        let body = if let Some(local) = &e.local {
            let store = self.shared.lock_store();
            let msg = store
                .get(LOCAL, local)
                .ok()
                .flatten()
                .and_then(|r| serde_json::from_slice::<LocalMsg>(&r).ok())
                .ok_or(StoreError::NotFound)?;
            (
                display::Body {
                    paragraphs: msg.body.split("\n\n").map(|p| p.to_string()).collect(),
                    ..Default::default()
                },
                Headers::default(),
                BodyState::Ready,
            )
        } else {
            let cached = lock(&self.shared.opened).iter().find(|(i, _)| *i == id.0).map(|(_, m)| m.clone());
            if let Some(mut m) = cached {
                m.summary = summary(&e, now());
                m.folder = e.folder.clone();
                return Ok(m);
            }
            let state = if self.shared.body_failed(id.0) {
                BodyState::Failed
            } else {
                self.shared.queue_parse(&e);
                BodyState::Loading
            };
            (display::Body::default(), Headers::default(), state)
        };
        let (body, headers, body_state) = body;
        Ok(Message {
            summary: summary(&e, now()),
            folder: e.folder.clone(),
            from_address: e.from_address,
            to: e.to,
            cc: e.cc,
            sent: display::sent(e.date),
            paragraphs: body.paragraphs,
            html: body.html,
            quoted: body.quoted,
            attachments: body.attachments,
            remote_images: body.remote_images,
            headers,
            body_state,
        })
    }

    fn attachments(&self, id: MessageId) -> Result<Vec<AttachmentData>, StoreError> {
        let e = self.entry(id)?;
        if e.local.is_some() {
            return Ok(Vec::new());
        }
        let raw = {
            let store = self.shared.lock_store();
            StoreSink::new(&store, &self.shared.accounts[e.account].id, true)
                .body(&e.server_folder, e.uid)
                .map_err(backend_err)?
        };
        Ok(raw.map(|r| display::extract_attachments(&r)).unwrap_or_default())
    }

    fn set_flags(&mut self, id: MessageId, change: FlagChange) -> Result<(), StoreError> {
        let e = self.entry(id)?;
        if e.local.is_some() {
            return Ok(());
        }
        let seen = change.unread.map(|u| !u).filter(|s| *s == e.unread);
        let flagged = change.flagged.filter(|f| *f != e.flagged);
        if seen.is_none() && flagged.is_none() {
            return Ok(());
        }
        let account = self.shared.accounts[e.account].id.clone();
        let validity = self.shared.validity(&account, &e.server_folder).unwrap_or(e.validity);
        {
            let store = self.shared.lock_store();
            let sink = StoreSink::new(&store, &account, true);
            let base = sink
                .summary(&e.server_folder, e.uid)
                .map_err(backend_err)?
                .ok_or(StoreError::NotFound)?
                .flags;
            self.shared.enqueue(
                &store,
                &Op::Flags {
                    account,
                    folder: e.server_folder.clone(),
                    uid: e.uid,
                    uid_validity: validity,
                    seen,
                    flagged,
                    base_seen: base.seen,
                    base_flagged: base.flagged,
                },
            )?;
        }
        self.shared.user_gen.fetch_add(1, Ordering::SeqCst);
        self.shared.patch(e.id, |x| {
            if let Some(s) = seen {
                x.unread = !s;
            }
            if let Some(f) = flagged {
                x.flagged = f;
            }
        });
        self.shared.refresh();
        self.shared.kick(e.account);
        Ok(())
    }

    fn move_to(&mut self, id: MessageId, folder: ms::Folder) -> Result<(), StoreError> {
        let e = self.entry(id)?;
        if e.folder == folder {
            return Ok(());
        }
        if matches!(&folder, ms::Folder::Other(a, _) if *a != e.account) {
            return Err(StoreError::Backend("a message cannot move to another account".into()));
        }
        let account = self.shared.accounts[e.account].id.clone();
        if let Some(local) = &e.local {
            {
                let store = self.shared.lock_store();
                let raw = store.get(LOCAL, local).map_err(backend_err)?.ok_or(StoreError::NotFound)?;
                let mut msg: LocalMsg = serde_json::from_slice(&raw).map_err(backend_err)?;
                msg.folder = folder.name().to_string();
                store.put(LOCAL, local, &serde_json::to_vec(&msg).map_err(backend_err)?).map_err(backend_err)?;
            }
            self.shared.user_gen.fetch_add(1, Ordering::SeqCst);
            self.shared.patch(e.id, |x| x.folder = folder);
            self.shared.refresh();
            return Ok(());
        }
        let folders = self.shared.folders_of(&account);
        let validity = self.shared.validity(&account, &e.server_folder).unwrap_or(e.validity);
        let to = match &folder {
            ms::Folder::Other(_, name) => name.clone(),
            _ => server_folder(&folders, &folder).unwrap_or_else(|| default_name(&folder).to_string()),
        };
        {
            let store = self.shared.lock_store();
            self.shared.enqueue(
                &store,
                &Op::Move { account, folder: e.server_folder.clone(), uid: e.uid, uid_validity: validity, to },
            )?;
        }
        self.shared.user_gen.fetch_add(1, Ordering::SeqCst);
        self.shared.patch(e.id, |x| x.folder = folder);
        self.shared.refresh();
        self.shared.kick(e.account);
        Ok(())
    }

    fn create_folder(&mut self, account: usize, name: &str) -> Result<(), StoreError> {
        let id = self.shared.accounts.get(account).ok_or(StoreError::NotFound)?.id.clone();
        self.shared.queue_folder_op(account, Op::CreateFolder { account: id, name: name.to_string() })
    }

    fn rename_folder(&mut self, folder: &ms::Folder, name: &str) -> Result<(), StoreError> {
        let ms::Folder::Other(account, from) = folder else { return Err(StoreError::Backend("only your own folders can be renamed".into())) };
        let id = self.shared.accounts.get(*account).ok_or(StoreError::NotFound)?.id.clone();
        self.shared.queue_folder_op(*account, Op::RenameFolder { account: id, from: from.clone(), to: name.to_string() })
    }

    fn delete_folder(&mut self, folder: &ms::Folder) -> Result<(), StoreError> {
        let ms::Folder::Other(account, name) = folder else { return Err(StoreError::Backend("only your own folders can be deleted".into())) };
        let id = self.shared.accounts.get(*account).ok_or(StoreError::NotFound)?.id.clone();
        self.shared.queue_folder_op(*account, Op::DeleteFolder { account: id, name: name.clone() })
    }

    fn append(&mut self, folder: ms::Folder, message: Outgoing) -> Result<MessageId, StoreError> {
        let account = self.shared.accounts.get(message.account).ok_or(StoreError::NotFound)?.id.clone();
        let n = self.shared.seq.fetch_add(1, Ordering::SeqCst);
        let key = format!("{:x}{n:x}", SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        let msg = LocalMsg {
            account: account.clone(),
            folder: folder.name().to_string(),
            to: message.to,
            cc: message.cc,
            subject: message.subject,
            body: message.body,
            date: now(),
        };
        let id = {
            let store = self.shared.lock_store();
            store.put(LOCAL, &key, &serde_json::to_vec(&msg).map_err(backend_err)?).map_err(backend_err)?;
            if folder == ms::Folder::Sent {
                self.shared.enqueue(&store, &Op::Send { account, local: key.clone() })?;
            }
            stable_id(&format!("l\0{key}"))
        };
        let index = message.account;
        self.shared.user_gen.fetch_add(1, Ordering::SeqCst);
        self.shared.insert(Entry {
            id,
            account: index,
            folder,
            server_folder: String::new(),
            uid: 0,
            validity: 0,
            local: Some(key),
            from: "me".into(),
            from_address: self.shared.accounts[index].address.clone(),
            to: vec![msg.to.clone()],
            cc: if msg.cc.is_empty() { Vec::new() } else { vec![msg.cc.clone()] },
            search_key: search_key("me", &self.shared.accounts[index].address, &msg.subject),
            to_key: format!("{} {}", msg.to, msg.cc),
            subject: msg.subject.clone(),
            date: msg.date,
            unread: false,
            flagged: false,
            attachments: false,
            thread: id,
            head: true,
            count: 1,
            thread_unread: false,
            thread_flagged: false,
        });
        self.shared.refresh();
        self.shared.kick(message.account);
        Ok(MessageId(id))
    }
}
