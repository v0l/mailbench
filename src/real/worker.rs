use super::model::*;
use super::{Outbound, Shared, lock};
use crate::backend::*;
use crate::mailstore::SyncState;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedReceiver;

const RETURNING: &str = "$Returning";
const POLL: Duration = Duration::from_secs(60);
const RETRY: Duration = Duration::from_secs(5);
const REFRESH: Duration = Duration::from_millis(1500);

struct LockedSink {
    shared: Arc<Shared>,
    account: String,
    index: usize,
    last_refresh: Instant,
    fresh: bool,
    inbox: String,
    reset: bool,
    keep_bodies: bool,
}

impl LockedSink {
    fn refresh_if_due(&mut self) {
        if self.fresh && self.last_refresh.elapsed() >= REFRESH {
            self.last_refresh = Instant::now();
            self.fresh = false;
            self.shared.refresh();
        }
    }

    fn with<T>(&self, f: impl FnOnce(&mut StoreSink<'_>) -> Result<T>) -> Result<T> {
        let guard = self.shared.lock_store();
        let mut sink = StoreSink::new(&guard, &self.account, self.keep_bodies);
        f(&mut sink)
    }
}

impl SyncSink for LockedSink {
    fn sync_token(&mut self, folder: &str) -> Result<Option<SyncToken>> {
        self.with(|s| s.sync_token(folder))
    }
    fn put_folders(&mut self, folders: &[Folder]) -> Result<()> {
        let changed = self.with(|s| Ok(s.folders()? != folders))?;
        self.with(|s| s.put_folders(folders))?;
        if changed {
            self.last_refresh = Instant::now();
            self.shared.refresh();
        }
        Ok(())
    }
    fn reset_folder(&mut self, folder: &str) -> Result<()> {
        self.fresh = true;
        self.reset |= folder == self.inbox;
        self.with(|s| s.reset_folder(folder))
    }
    fn known_uids(&mut self, folder: &str) -> Result<Vec<u32>> {
        self.with(|s| s.known_uids(folder))
    }
    fn put_summaries(&mut self, folder: &str, messages: &[MessageSummary]) -> Result<()> {
        self.fresh |= !messages.is_empty();
        self.with(|s| s.put_summaries(folder, messages))?;
        if folder == self.inbox && !self.reset {
            let validity = self.shared.validity(&self.account, folder).unwrap_or(0);
            self.shared.note_incoming(self.index, folder, validity, messages);
        }
        Ok(())
    }
    fn put_flags(&mut self, folder: &str, changes: &[(u32, Flags)]) -> Result<usize> {
        let n = self.with(|s| s.put_flags(folder, changes))?;
        self.fresh |= n > 0;
        Ok(n)
    }
    fn retain_uids(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        self.with(|s| s.retain_uids(folder, uids))
    }
    fn put_token(&mut self, folder: &str, token: &SyncToken) -> Result<()> {
        self.with(|s| s.put_token(folder, token))?;
        self.refresh_if_due();
        Ok(())
    }
    fn progress(&mut self, folder: &str, done: usize, total: usize) {
        let detail = (total > 0).then(|| format!("{folder}: {done} of {total}"));
        self.shared.set_state(self.index, SyncState::Syncing, detail);
        self.refresh_if_due();
    }
    fn cancelled(&self) -> bool {
        self.shared.stop.load(Ordering::Relaxed)
    }
}

fn retryable(e: &BackendError) -> bool {
    !matches!(e, BackendError::Protocol(_) | BackendError::Unsupported(_))
}

async fn push(shared: &Shared, backend: &mut dyn Backend, op: &Op, account: &str) -> Result<()> {
    match op {
        Op::Flags { folder, uid, uid_validity, seen, flagged, base_seen, base_flagged, .. } => {
            if shared.validity(account, folder) != Some(*uid_validity) {
                return Ok(());
            }
            let Some(current) = shared.stored_flags(account, folder, *uid) else {
                return Ok(());
            };
            let seen = seen.filter(|_| current.seen == *base_seen);
            let flagged = flagged.filter(|_| current.flagged == *base_flagged);
            let mut add = Flags::default();
            let mut remove = Flags::default();
            if let Some(v) = seen {
                if v { add.seen = true } else { remove.seen = true }
            }
            if let Some(v) = flagged {
                if v { add.flagged = true } else { remove.flagged = true }
            }
            if add != Flags::default() {
                backend.set_flags(folder, &[*uid], &FlagUpdate::Add(add)).await?;
            }
            if remove != Flags::default() {
                backend.set_flags(folder, &[*uid], &FlagUpdate::Remove(remove)).await?;
            }
            shared.apply_flags(account, folder, *uid, seen, flagged);
            Ok(())
        }
        Op::Move { folder, uid, uid_validity, to, .. } => {
            if shared.validity(account, folder) != Some(*uid_validity) {
                return Ok(());
            }
            match backend.move_messages(folder, &[*uid], to).await {
                Err(BackendError::Protocol(_)) => {
                    backend.create_folder(to).await?;
                    backend.move_messages(folder, &[*uid], to).await
                }
                other => other,
            }
        }
        Op::Wake { folder, uid, uid_validity, to, key, .. } => {
            if shared.validity(account, folder) != Some(*uid_validity) {
                shared.forget_snooze(account, key);
                return Ok(());
            }
            let returning = Flags { keywords: vec![RETURNING.into()], ..Default::default() };
            let _ = backend.set_flags(folder, &[*uid], &FlagUpdate::Add(returning)).await;
            let unseen = Flags { seen: true, ..Default::default() };
            backend.set_flags(folder, &[*uid], &FlagUpdate::Remove(unseen)).await?;
            backend.move_messages(folder, &[*uid], to).await?;
            shared.forget_snooze(account, key);
            Ok(())
        }
        Op::FlagsSet { folder, uids, uid_validity, seen, flagged, .. } => {
            if shared.validity(account, folder) != Some(*uid_validity) {
                return Ok(());
            }
            let mut add = Flags::default();
            let mut remove = Flags::default();
            if let Some(v) = seen {
                if *v { add.seen = true } else { remove.seen = true }
            }
            if let Some(v) = flagged {
                if *v { add.flagged = true } else { remove.flagged = true }
            }
            if add != Flags::default() {
                backend.set_flags(folder, uids, &FlagUpdate::Add(add)).await?;
            }
            if remove != Flags::default() {
                backend.set_flags(folder, uids, &FlagUpdate::Remove(remove)).await?;
            }
            for uid in uids {
                shared.apply_flags(account, folder, *uid, *seen, *flagged);
            }
            Ok(())
        }
        Op::MoveSet { folder, uids, uid_validity, to, .. } => {
            if shared.validity(account, folder) != Some(*uid_validity) {
                return Ok(());
            }
            match backend.move_messages(folder, uids, to).await {
                Err(BackendError::Protocol(_)) => {
                    backend.create_folder(to).await?;
                    backend.move_messages(folder, uids, to).await
                }
                other => other,
            }
        }
        Op::CreateFolder { name, .. } => backend.create_folder(name).await,
        Op::RenameFolder { from, to, .. } => backend.rename_folder(from, to).await,
        Op::DeleteFolder { name, .. } => backend.delete_folder(name).await,
        Op::Send { local, .. } => {
            let message = match shared.outgoing(local, false, true) {
                Outbound::Ready(m) => m,
                _ => return Ok(()),
            };
            if !shared.already_sent(local) {
                backend.send(&message).await?;
                shared.mark_sent(local, &message);
            }
            if !backend.sent_copy_by_server() {
                let sent = shared.sent_folder(account);
                let raw = render_message(&message, true)?;
                let flags = Flags { seen: true, ..Default::default() };
                match append_creating(backend, &sent, &flags, &raw).await {
                    Err(e) if retryable(&e) => return Err(e),
                    _ => {}
                }
            }
            Ok(())
        }
        Op::Draft { local, .. } => {
            let Some(rev) = shared.local_rev(local) else { return Ok(()) };
            let Outbound::Ready(message) = shared.outgoing(local, true, true) else { return Ok(()) };
            let Some(mid) = message.message_id.clone() else { return Ok(()) };
            let folder = shared.drafts_folder(account);
            let old = drafts_with(backend, &folder, &mid).await;
            let raw = render_message(&message, true)?;
            let flags = Flags { draft: true, seen: true, ..Default::default() };
            append_creating(backend, &folder, &flags, &raw).await?;
            if !old.is_empty() {
                backend.delete_messages(&folder, &old).await?;
            }
            shared.mark_pushed(local, rev);
            Ok(())
        }
        Op::DropDraft { message_id, .. } => {
            let folder = shared.drafts_folder(account);
            let old = drafts_with(backend, &folder, message_id).await;
            if !old.is_empty() {
                backend.delete_messages(&folder, &old).await?;
            }
            Ok(())
        }
    }
}

async fn drafts_with(backend: &mut dyn Backend, folder: &str, message_id: &str) -> Vec<u32> {
    let query = format!("HEADER Message-ID \"{}\"", message_id.trim_matches(['<', '>']));
    backend.search(folder, &query).await.unwrap_or_default()
}

async fn append_creating(backend: &mut dyn Backend, folder: &str, flags: &Flags, raw: &[u8]) -> Result<()> {
    match backend.append_flagged(folder, flags, raw).await {
        Err(BackendError::Protocol(_)) => {
            backend.create_folder(folder).await?;
            backend.append_flagged(folder, flags, raw).await
        }
        other => other,
    }
}

async fn push_outbox(shared: &Shared, account: &str, backend: &mut dyn Backend, done: &mut Vec<String>) -> Result<()> {
    for (key, op) in shared.outbox(account) {
        if let Op::Send { local, .. } = &op {
            if shared.is_held(local) {
                continue;
            }
        }
        match push(shared, backend, &op, account).await {
            Ok(()) => done.push(key),
            Err(e) if retryable(&e) => return Err(e),
            Err(e) => {
                done.push(key);
                if matches!(op, Op::Send { .. }) {
                    return Err(BackendError::Protocol(format!("send failed: {e}")));
                }
            }
        }
    }
    Ok(())
}

fn queue_wakes(shared: &Shared, account: &str) {
    let due = shared.snooze_due(account);
    if due.is_empty() {
        return;
    }
    let folders = shared.folders_of(account);
    let Some(snoozed) = server_folder(&folders, &crate::mailstore::Folder::Snoozed) else { return };
    let inbox = inbox_name(shared, account);
    let queued = shared.outbox(account);
    let moving = queued.iter().any(|(_, op)| matches!(op, Op::Move { .. }));
    for snooze in due {
        let key = snooze.key();
        if queued.iter().any(|(_, op)| matches!(op, Op::Wake { key: k, .. } if *k == key)) {
            continue;
        }
        match shared.find_stored(account, &snoozed, |m| snooze.matches(m)) {
            Some(uid) => {
                let Some(uid_validity) = shared.validity(account, &snoozed) else { continue };
                let op = Op::Wake { account: account.to_string(), folder: snoozed.clone(), uid, uid_validity, to: inbox.clone(), key };
                let _ = shared.enqueue_op(&op);
            }
            None if !moving => shared.forget_snooze(account, &key),
            None => {}
        }
    }
}

fn next_wait(shared: &Shared, account: &str, wait: Duration) -> Duration {
    match shared.next_wake(account) {
        Some(at) => wait.min(Duration::from_secs((at - shared.now_secs()).max(1) as u64)),
        None => wait,
    }
}

async fn cycle(shared: &Arc<Shared>, index: usize, account: &str, backend: &mut dyn Backend) -> Result<()> {
    let mut sink = LockedSink {
        shared: shared.clone(),
        account: account.to_string(),
        index,
        last_refresh: Instant::now(),
        fresh: false,
        inbox: inbox_name(shared, account),
        reset: false,
        keep_bodies: *lock(&shared.body_policy) != crate::prefs::BodyPolicy::HeadersOnly,
    };
    queue_wakes(shared, account);
    let mut done = Vec::new();
    let pushed = push_outbox(shared, account, backend, &mut done).await;
    let (folder_ops, done): (Vec<String>, Vec<String>) = done.into_iter().partition(|key| {
        shared.outbox(account).iter().any(|(k, op)| k == key && matches!(op, Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. }))
    });
    shared.remove_ops(&done);
    if let Err(e) = pushed {
        shared.remove_ops(&folder_ops);
        return Err(e);
    }
    let filter = lock(&shared.sync_opts)[index].filter(shared.now_secs());
    let synced = sync_account_with(backend, &mut sink, &filter).await;
    shared.remove_ops(&folder_ops);
    synced?;
    if sink.fresh {
        shared.refresh();
    }
    Ok(())
}

fn inbox_name(shared: &Shared, account: &str) -> String {
    shared
        .folders_of(account)
        .into_iter()
        .find(|f| f.role == Some(FolderRole::Inbox))
        .map(|f| f.name)
        .unwrap_or_else(|| "INBOX".into())
}

pub(super) struct FetchRequest {
    pub id: u64,
    pub folder: String,
    pub uid: u32,
    pub queued: std::time::Instant,
}

pub(super) async fn serve_fetches(
    shared: Arc<Shared>,
    index: usize,
    mut backend: Box<dyn Backend>,
    mut requests: UnboundedReceiver<FetchRequest>,
) {
    let account = shared.accounts[index].id.clone();
    while let Some(req) = requests.recv().await {
        if shared.stop.load(Ordering::Relaxed) {
            return;
        }
        let wait = req.queued.elapsed();
        let mut result = backend.fetch(&req.folder, req.uid).await;
        if matches!(result, Err(BackendError::Connection(_))) {
            result = backend.fetch(&req.folder, req.uid).await;
        }
        let round_trip = req.queued.elapsed() - wait;
        let outcome = result.ok().and_then(|raw| {
            let guard = shared.lock_store();
            StoreSink::new(&guard, &account, true)
                .put_body(&req.folder, req.uid, &raw)
                .ok()
                .map(|_| super::display::has_attachments(&raw))
        });
        if std::env::var_os("MAILBENCH_OPEN_LOG").is_some() {
            eprintln!("fetch id={} queue_wait={:?} server_round_trip={:?} stored={:?}", req.id, wait, round_trip, req.queued.elapsed() - wait - round_trip);
        }
        shared.finish_fetch(req.id, outcome);
    }
}

pub(super) struct SearchRequest {
    pub text: String,
    pub folder: String,
    pub query: String,
}

pub(super) async fn serve_searches(
    shared: Arc<Shared>,
    index: usize,
    mut backend: Box<dyn Backend>,
    mut requests: UnboundedReceiver<SearchRequest>,
) {
    while let Some(req) = requests.recv().await {
        if shared.stop.load(Ordering::Relaxed) {
            return;
        }
        let found = match backend.search(&req.folder, &req.query).await {
            Err(BackendError::Connection(_)) => backend.search(&req.folder, &req.query).await,
            other => other,
        };
        shared.finish_remote(index, &req.text, &req.folder, found.unwrap_or_default());
    }
}

pub(super) async fn run(
    shared: Arc<Shared>,
    index: usize,
    mut backend: Box<dyn Backend>,
    mut kick: UnboundedReceiver<()>,
) {
    let account = shared.accounts[index].id.clone();
    while !shared.stop.load(Ordering::Relaxed) {
        shared.set_state(index, SyncState::Syncing, None);
        if let Ok(caps) = backend.capabilities().await {
            shared.set_caps(index, caps);
        }
        let result = cycle(&shared, index, &account, backend.as_mut()).await;
        shared.flush_notifications(index);
        if result.is_ok() {
            shared.arm_notifications(index);
        }
        let mut watch = None;
        match &result {
            Ok(()) => {
                watch = backend.watch(&inbox_name(&shared, &account)).await.ok();
                shared.set_state(index, SyncState::Idle, None);
            }
            Err(e) => shared.set_state(index, SyncState::Failed, Some(e.to_string())),
        }
        shared.refresh();
        let polling = lock(&shared.sync_opts)[index].poll();
        let wait = next_wait(&shared, &account, match (&result, watch.is_some()) {
            (Ok(()), true) => POLL,
            (Ok(()), false) => polling,
            _ => RETRY,
        });
        let changed = async {
            match watch.as_mut() {
                Some(w) => {
                    w.events.recv().await;
                }
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            k = kick.recv() => if k.is_none() { return },
            _ = changed => {}
        }
    }
}
