use super::model::*;
use super::Shared;
use crate::backend::*;
use crate::mailstore::SyncState;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedReceiver;

const POLL: Duration = Duration::from_secs(60);
const RETRY: Duration = Duration::from_secs(5);
const REFRESH: Duration = Duration::from_millis(1500);

struct LockedSink {
    shared: Arc<Shared>,
    account: String,
    index: usize,
    last_refresh: Instant,
    fresh: bool,
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
        let mut sink = StoreSink::new(&guard, &self.account, true);
        f(&mut sink)
    }
}

impl SyncSink for LockedSink {
    fn sync_token(&mut self, folder: &str) -> Result<Option<SyncToken>> {
        self.with(|s| s.sync_token(folder))
    }
    fn put_folders(&mut self, folders: &[Folder]) -> Result<()> {
        self.with(|s| s.put_folders(folders))
    }
    fn reset_folder(&mut self, folder: &str) -> Result<()> {
        self.fresh = true;
        self.with(|s| s.reset_folder(folder))
    }
    fn known_uids(&mut self, folder: &str) -> Result<Vec<u32>> {
        self.with(|s| s.known_uids(folder))
    }
    fn put_summaries(&mut self, folder: &str, messages: &[MessageSummary]) -> Result<()> {
        self.fresh |= !messages.is_empty();
        self.with(|s| s.put_summaries(folder, messages))
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
        Op::CreateFolder { name, .. } => backend.create_folder(name).await,
        Op::RenameFolder { from, to, .. } => backend.rename_folder(from, to).await,
        Op::DeleteFolder { name, .. } => backend.delete_folder(name).await,
        Op::Send { local, .. } => {
            let Some(message) = shared.outgoing(local) else { return Ok(()) };
            backend.send(&message).await
        }
    }
}

async fn push_outbox(shared: &Shared, account: &str, backend: &mut dyn Backend, done: &mut Vec<String>) -> Result<()> {
    for (key, op) in shared.outbox(account) {
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

async fn cycle(shared: &Arc<Shared>, index: usize, account: &str, backend: &mut dyn Backend) -> Result<()> {
    let mut sink =
        LockedSink { shared: shared.clone(), account: account.to_string(), index, last_refresh: Instant::now(), fresh: false };
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
    let synced = sync_account(backend, &mut sink).await;
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
        let mut result = backend.fetch(&req.folder, req.uid).await;
        if matches!(result, Err(BackendError::Connection(_))) {
            result = backend.fetch(&req.folder, req.uid).await;
        }
        let outcome = result.ok().and_then(|raw| {
            let guard = shared.lock_store();
            StoreSink::new(&guard, &account, true)
                .put_body(&req.folder, req.uid, &raw)
                .ok()
                .map(|_| super::display::has_attachments(&raw))
        });
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
        let result = cycle(&shared, index, &account, backend.as_mut()).await;
        let mut watch = None;
        match &result {
            Ok(()) => {
                watch = backend.watch(&inbox_name(&shared, &account)).await.ok();
                shared.set_state(index, SyncState::Idle, None);
            }
            Err(e) => shared.set_state(index, SyncState::Failed, Some(e.to_string())),
        }
        shared.refresh();
        let wait = if result.is_ok() { POLL } else { RETRY };
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
