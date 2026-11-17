use super::model::*;
use super::Shared;
use crate::backend::*;
use crate::mailstore::SyncState;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_secs(60);
const RETRY: Duration = Duration::from_secs(5);

struct LockedSink<'a> {
    shared: &'a Shared,
    account: String,
}

impl LockedSink<'_> {
    fn with<T>(&self, f: impl FnOnce(&mut StoreSink<'_>) -> Result<T>) -> Result<T> {
        let guard = self.shared.lock_store();
        let mut sink = StoreSink::new(&guard, &self.account, true);
        f(&mut sink)
    }
}

impl SyncSink for LockedSink<'_> {
    fn sync_token(&mut self, folder: &str) -> Result<Option<SyncToken>> {
        self.with(|s| s.sync_token(folder))
    }
    fn put_folders(&mut self, folders: &[Folder]) -> Result<()> {
        self.with(|s| s.put_folders(folders))
    }
    fn reset_folder(&mut self, folder: &str) -> Result<()> {
        self.with(|s| s.reset_folder(folder))
    }
    fn put_summaries(&mut self, folder: &str, messages: &[MessageSummary]) -> Result<()> {
        self.with(|s| s.put_summaries(folder, messages))
    }
    fn put_flags(&mut self, folder: &str, changes: &[(u32, Flags)]) -> Result<usize> {
        self.with(|s| s.put_flags(folder, changes))
    }
    fn retain_uids(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        self.with(|s| s.retain_uids(folder, uids))
    }
    fn put_token(&mut self, folder: &str, token: &SyncToken) -> Result<()> {
        self.with(|s| s.put_token(folder, token))
    }
    fn wants_bodies(&self) -> bool {
        true
    }
    fn put_body(&mut self, folder: &str, uid: u32, raw: &[u8]) -> Result<()> {
        self.with(|s| s.put_body(folder, uid, raw))
    }
}

fn retryable(e: &BackendError) -> bool {
    !matches!(e, BackendError::Protocol(_) | BackendError::Unsupported(_))
}

fn push(shared: &Shared, backend: &mut dyn Backend, op: &Op, account: &str) -> Result<()> {
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
            for (value, on_set) in [(&seen, 0), (&flagged, 1)] {
                let Some(value) = value else { continue };
                let target = if *value { &mut add } else { &mut remove };
                if on_set == 0 {
                    target.seen = true;
                } else {
                    target.flagged = true;
                }
            }
            if add != Flags::default() {
                backend.set_flags(folder, &[*uid], &FlagUpdate::Add(add))?;
            }
            if remove != Flags::default() {
                backend.set_flags(folder, &[*uid], &FlagUpdate::Remove(remove))?;
            }
            shared.apply_flags(account, folder, *uid, seen, flagged);
            Ok(())
        }
        Op::Move { folder, uid, uid_validity, to, .. } => {
            if shared.validity(account, folder) != Some(*uid_validity) {
                return Ok(());
            }
            let first = backend.move_messages(folder, &[*uid], to);
            match first {
                Err(BackendError::Protocol(_)) => {
                    backend.create_folder(to)?;
                    backend.move_messages(folder, &[*uid], to)
                }
                other => other,
            }
        }
        Op::Send { local, .. } => {
            let Some(message) = shared.outgoing(local) else { return Ok(()) };
            backend.send(&message)
        }
    }
}

fn push_outbox(shared: &Shared, account: &str, backend: &mut dyn Backend, done: &mut Vec<String>) -> Result<()> {
    for (key, op) in shared.outbox(account) {
        match push(shared, backend, &op, account) {
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

fn cycle(shared: &Shared, account: &str, backend: &mut dyn Backend) -> Result<()> {
    let mut sink = LockedSink { shared, account: account.to_string() };
    sync_account(backend, &mut sink)?;
    let mut done = Vec::new();
    let pushed = push_outbox(shared, account, backend, &mut done);
    shared.remove_ops(&done);
    pushed?;
    if !done.is_empty() {
        sync_account(backend, &mut sink)?;
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

pub(super) fn run(shared: Arc<Shared>, index: usize, mut backend: Box<dyn Backend>, kick: Receiver<()>) {
    let account = shared.accounts[index].id.clone();
    while !shared.stop.load(Ordering::Relaxed) {
        shared.set_state(index, SyncState::Syncing, None);
        let result = cycle(&shared, &account, backend.as_mut());
        let mut watch = None;
        match &result {
            Ok(()) => {
                watch = backend.watch(&inbox_name(&shared, &account)).ok();
                shared.set_state(index, SyncState::Idle, None);
            }
            Err(e) => shared.set_state(index, SyncState::Failed, Some(e.to_string())),
        }
        shared.refresh();
        let wait = if result.is_ok() { POLL } else { RETRY };
        let deadline = Instant::now() + wait;
        loop {
            if shared.stop.load(Ordering::Relaxed) || Instant::now() >= deadline {
                break;
            }
            match kick.recv_timeout(Duration::from_millis(50)) {
                Ok(()) => break,
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if let Some(w) = &watch {
                match w.events.try_recv() {
                    Ok(Event::FolderChanged { .. }) => break,
                    Ok(Event::Disconnected { .. }) => break,
                    Err(_) => {}
                }
            }
        }
    }
}
