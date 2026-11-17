mod display;
mod model;
mod worker;

use crate::backend::*;
use crate::mailstore::{self as ms, *};
use crate::store::Store;
use model::*;
use std::collections::HashMap;
use std::result::Result;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

const OUTBOX: &str = "outbox";
const LOCAL: &str = "local";
const ID_FWD: &str = "idfwd";
const ID_REV: &str = "idrev";
const ID_NEXT: &str = "idnext";

pub struct AccountSetup {
    pub id: String,
    pub name: String,
    pub address: String,
    pub backend: Box<dyn Backend>,
}

pub(crate) struct AccountMeta {
    pub id: String,
    pub name: String,
    pub address: String,
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
}

#[derive(Default)]
struct Cache {
    generation: u64,
    built: bool,
    entries: Vec<Entry>,
    by_id: HashMap<u64, usize>,
    labels: Vec<LabelInfo>,
    attachments: HashMap<u64, bool>,
}

pub(crate) struct Shared {
    store: Mutex<Store>,
    pub accounts: Vec<AccountMeta>,
    states: Mutex<Vec<(SyncState, Option<String>)>>,
    ctx: Mutex<Option<egui::Context>>,
    pub stop: AtomicBool,
    generation: AtomicU64,
    cache: Mutex<Cache>,
    seq: AtomicU64,
    kicks: Mutex<Vec<Sender<()>>>,
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn folder_for(folders: &[crate::backend::Folder], name: &str) -> Option<ms::Folder> {
    folders.iter().find(|f| f.name == name).and_then(ui_folder).or_else(|| {
        [ms::Folder::Archive, ms::Folder::Trash, ms::Folder::Junk, ms::Folder::Sent, ms::Folder::Drafts, ms::Folder::Inbox]
            .into_iter()
            .find(|f| default_name(*f) == name)
    })
}

fn backend_err(e: impl std::fmt::Display) -> StoreError {
    StoreError::Backend(e.to_string())
}

fn assign_id(store: &Store, key: &str) -> u64 {
    if let Ok(Some(b)) = store.get(ID_FWD, key) {
        if let Ok(bytes) = <[u8; 8]>::try_from(&b[..]) {
            return u64::from_be_bytes(bytes);
        }
    }
    let next = store
        .get(ID_NEXT, "n")
        .ok()
        .flatten()
        .and_then(|b| <[u8; 8]>::try_from(&b[..]).ok())
        .map(u64::from_be_bytes)
        .unwrap_or(1);
    let _ = store.put(ID_NEXT, "n", &(next + 1).to_be_bytes());
    let _ = store.put(ID_FWD, key, &next.to_be_bytes());
    let _ = store.put(ID_REV, &next.to_string(), key.as_bytes());
    next
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
        if let Some(ctx) = lock(&self.ctx).as_ref() {
            ctx.request_repaint();
        }
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
            from: Address { name: Some(account.name.clone()), address: account.address.clone() },
            to: list(&msg.to),
            cc: list(&msg.cc),
            subject: msg.subject,
            body: msg.body,
            ..Default::default()
        })
    }

    fn build(&self, old: &Cache, generation: u64) -> Cache {
        let store = self.lock_store();
        let ops = Self::outbox_in(&store);
        let mut cache = Cache { generation, built: true, attachments: old.attachments.clone(), ..Default::default() };
        let mut label_counts: HashMap<String, usize> = HashMap::new();
        for (index, account) in self.accounts.iter().enumerate() {
            let mut sink = StoreSink::new(&store, &account.id, true);
            let folders = sink.folders().unwrap_or_default();
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
            for folder in folders.iter().filter(|f| f.selectable) {
                let ui = ui_folder(folder);
                let validity = sink.sync_token(&folder.name).ok().flatten().map(|t| t.uid_validity).unwrap_or(0);
                for m in sink.messages(&folder.name).unwrap_or_default() {
                    if m.flags.deleted {
                        continue;
                    }
                    let Some(ui) = ui else {
                        if !m.flags.seen {
                            *label_counts.entry(folder.name.clone()).or_default() += 1;
                        } else {
                            label_counts.entry(folder.name.clone()).or_default();
                        }
                        continue;
                    };
                    let key = format!("r\0{}\0{}\0{}\0{}", account.id, folder.name, validity, m.uid);
                    let id = assign_id(&store, &key);
                    let shown = pending
                        .get(&(folder.name.clone(), m.uid))
                        .map(|to| folder_for(&folders, to))
                        .unwrap_or(Some(ui))
                        .unwrap_or(ui);
                    let attachments = *cache.attachments.entry(id).or_insert_with(|| {
                        sink.body(&folder.name, m.uid).ok().flatten().is_some_and(|b| display::has_attachments(&b))
                    });
                    cache.entries.push(Entry {
                        id,
                        account: index,
                        folder: shown,
                        server_folder: folder.name.clone(),
                        uid: m.uid,
                        validity,
                        local: None,
                        from: m.from.first().map(display::person).unwrap_or_default(),
                        from_address: m.from.first().map(|a| a.address.clone()).unwrap_or_default(),
                        to: display::people(&m.to),
                        cc: display::people(&m.cc),
                        subject: m.subject.clone().unwrap_or_default(),
                        date: m.date.unwrap_or(0),
                        unread: flag_ops.get(&(folder.name.clone(), m.uid)).and_then(|f| f.0).map(|s| !s).unwrap_or(!m.flags.seen),
                        flagged: flag_ops.get(&(folder.name.clone(), m.uid)).and_then(|f| f.1).unwrap_or(m.flags.flagged),
                        attachments,
                    });
                }
            }
        }
        for key in store.list(LOCAL).unwrap_or_default() {
            let Some(raw) = store.get(LOCAL, &key).ok().flatten() else { continue };
            let Ok(msg) = serde_json::from_slice::<LocalMsg>(&raw) else { continue };
            let Some(index) = self.index_of(&msg.account) else { continue };
            let Some(folder) = folder_from_name(&msg.folder) else { continue };
            cache.entries.push(Entry {
                id: assign_id(&store, &format!("l\0{key}")),
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
                date: msg.date,
                unread: false,
                flagged: false,
                attachments: false,
            });
        }
        cache.entries.sort_by(|a, b| b.date.cmp(&a.date).then(b.id.cmp(&a.id)));
        cache.by_id = cache.entries.iter().enumerate().map(|(i, e)| (e.id, i)).collect();
        let mut labels: Vec<LabelInfo> =
            label_counts.into_iter().map(|(name, unread)| LabelInfo { name, unread }).collect();
        labels.sort_by(|a, b| a.name.cmp(&b.name));
        cache.labels = labels;
        cache
    }

    fn with_cache<T>(&self, f: impl FnOnce(&Cache) -> T) -> T {
        let mut cache = lock(&self.cache);
        let generation = self.generation.load(Ordering::SeqCst);
        if !cache.built || cache.generation != generation {
            let fresh = self.build(&cache, generation);
            *cache = fresh;
        }
        f(&cache)
    }
}

pub struct RealStore {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

fn matches(e: &Entry, q: &Query) -> bool {
    if e.folder != q.folder {
        return false;
    }
    match q.filter {
        Filter::All => {}
        Filter::Unread if !e.unread => return false,
        Filter::Flagged if !e.flagged => return false,
        _ => {}
    }
    if q.text.is_empty() {
        return true;
    }
    let needle = q.text.to_lowercase();
    e.from.to_lowercase().contains(&needle)
        || e.from_address.to_lowercase().contains(&needle)
        || e.subject.to_lowercase().contains(&needle)
}

fn summary(e: &Entry, now: i64) -> Summary {
    Summary {
        id: MessageId(e.id),
        from: e.from.clone(),
        subject: e.subject.clone(),
        when: display::when(e.date, now),
        unread: e.unread,
        flagged: e.flagged,
        new: e.unread,
        attachments: e.attachments,
        account: e.account,
        thread: 1,
    }
}

impl RealStore {
    pub fn open(store: Store, accounts: Vec<AccountSetup>) -> RealStore {
        let metas = accounts
            .iter()
            .map(|a| AccountMeta { id: a.id.clone(), name: a.name.clone(), address: a.address.clone() })
            .collect::<Vec<_>>();
        let shared = Arc::new(Shared {
            store: Mutex::new(store),
            states: Mutex::new(vec![(SyncState::Syncing, None); metas.len()]),
            accounts: metas,
            ctx: Mutex::new(None),
            stop: AtomicBool::new(false),
            generation: AtomicU64::new(1),
            cache: Mutex::new(Cache::default()),
            seq: AtomicU64::new(0),
            kicks: Mutex::new(Vec::new()),
        });
        let mut threads = Vec::new();
        for (index, setup) in accounts.into_iter().enumerate() {
            let (tx, rx) = channel();
            lock(&shared.kicks).push(tx);
            let shared = shared.clone();
            threads.push(std::thread::spawn(move || worker::run(shared, index, setup.backend, rx)));
        }
        RealStore { shared, threads }
    }

    pub fn attach(&self, ctx: &egui::Context) {
        *lock(&self.shared.ctx) = Some(ctx.clone());
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
        lock(&self.shared.kicks).clear();
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
                    unread: c
                        .entries
                        .iter()
                        .filter(|e| e.account == i && e.folder == ms::Folder::Inbox && e.unread)
                        .count(),
                    sync: states[i].0,
                    detail: states[i].1.clone(),
                })
                .collect()
        })
    }

    fn folders(&self) -> Vec<FolderInfo> {
        self.shared.with_cache(|c| {
            ms::Folder::UNIFIED
                .iter()
                .map(|f| {
                    let inside = c.entries.iter().filter(|e| e.folder == *f);
                    FolderInfo { folder: *f, unread: inside.clone().filter(|e| e.unread).count(), total: inside.count() }
                })
                .collect()
        })
    }

    fn labels(&self) -> Vec<LabelInfo> {
        self.shared.with_cache(|c| c.labels.clone())
    }

    fn counts(&self, query: &Query) -> Counts {
        let base = Query { filter: Filter::All, ..query.clone() };
        self.shared.with_cache(|c| {
            let mut out = Counts::default();
            for e in c.entries.iter().filter(|e| matches(e, &base)) {
                out.all += 1;
                out.unread += e.unread as usize;
                out.flagged += e.flagged as usize;
            }
            out
        })
    }

    fn list(&self, query: &Query, offset: usize, limit: usize) -> Page {
        let now = now();
        self.shared.with_cache(|c| {
            let mut total = 0;
            let mut items = Vec::new();
            for e in c.entries.iter().filter(|e| matches(e, query)) {
                if total >= offset && items.len() < limit {
                    items.push(summary(e, now));
                }
                total += 1;
            }
            Page { offset, total, items }
        })
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
            display::Body {
                paragraphs: msg.body.split("\n\n").map(|p| p.to_string()).collect(),
                ..Default::default()
            }
        } else {
            let store = self.shared.lock_store();
            let account = &self.shared.accounts[e.account].id;
            let raw = StoreSink::new(&store, account, true).body(&e.server_folder, e.uid).ok().flatten();
            raw.map(|r| display::parse_body(&r)).unwrap_or_default()
        };
        Ok(Message {
            summary: summary(&e, now()),
            folder: e.folder,
            from_address: e.from_address,
            to: e.to,
            cc: e.cc,
            sent: display::sent(e.date),
            paragraphs: body.paragraphs,
            quoted: body.quoted,
            attachments: body.attachments,
            remote_images: body.remote_images,
        })
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
                    uid_validity: e.validity,
                    seen,
                    flagged,
                    base_seen: base.seen,
                    base_flagged: base.flagged,
                },
            )?;
        }
        self.shared.refresh();
        self.shared.kick(e.account);
        Ok(())
    }

    fn move_to(&mut self, id: MessageId, folder: ms::Folder) -> Result<(), StoreError> {
        let e = self.entry(id)?;
        if e.folder == folder {
            return Ok(());
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
            self.shared.refresh();
            return Ok(());
        }
        let folders = self.shared.folders_of(&account);
        let to = server_folder(&folders, folder).unwrap_or_else(|| default_name(folder).to_string());
        {
            let store = self.shared.lock_store();
            self.shared.enqueue(
                &store,
                &Op::Move { account, folder: e.server_folder.clone(), uid: e.uid, uid_validity: e.validity, to },
            )?;
        }
        self.shared.refresh();
        self.shared.kick(e.account);
        Ok(())
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
            assign_id(&store, &format!("l\0{key}"))
        };
        self.shared.refresh();
        self.shared.kick(message.account);
        Ok(MessageId(id))
    }
}
