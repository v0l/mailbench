use super::{Shared, lock, now};
use crate::backend::*;
use crate::mailstore::SyncState;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::Notify;

const PAUSE: Duration = Duration::from_millis(40);
const IDLE_WAIT: Duration = Duration::from_secs(5);
const RECHECK: Duration = Duration::from_secs(30);
const EPOCH_EVERY: usize = 25;
const SEAL_OVERHEAD: u64 = 64;

struct Candidate {
    folder: String,
    uid: u32,
    date: i64,
}

fn busy(shared: &Shared, index: usize) -> bool {
    lock(&shared.states)[index].0 != SyncState::Idle
        || !lock(&shared.fetch_state).is_empty()
        || !lock(&shared.parse_pending).is_empty()
}

fn plan(shared: &Shared, index: usize, cutoff: Option<i64>) -> (Vec<Candidate>, usize) {
    let account = &shared.accounts[index].id;
    let entries: Vec<(String, u32, i64)> = shared.with_cache(|c| {
        c.entries
            .iter()
            .filter(|e| e.account == index && e.local.is_none() && cutoff.is_none_or(|t| e.date >= t))
            .map(|e| (e.server_folder.clone(), e.uid, e.date))
            .collect()
    });
    let mut have: HashSet<(String, u32)> = HashSet::new();
    {
        let store = shared.lock_store();
        let sink = StoreSink::new(&store, account, true);
        let folders: HashSet<&String> = entries.iter().map(|e| &e.0).collect();
        for f in folders {
            for uid in sink.body_uids(f).unwrap_or_default() {
                have.insert((f.clone(), uid));
            }
        }
    }
    let total = entries.len();
    let mut todo: Vec<Candidate> = entries
        .into_iter()
        .filter(|(f, u, _)| !have.contains(&(f.clone(), *u)))
        .map(|(folder, uid, date)| Candidate { folder, uid, date })
        .collect();
    todo.sort_by(|a, b| b.date.cmp(&a.date));
    (todo, total)
}

fn prune(shared: &Shared, index: usize, cutoff: Option<i64>, cap: Option<u64>) {
    let account = &shared.accounts[index].id;
    let mut aged: Vec<(i64, String, u32)> = shared.with_cache(|c| {
        c.entries
            .iter()
            .filter(|e| e.account == index && e.local.is_none())
            .map(|e| (e.date, e.server_folder.clone(), e.uid))
            .collect()
    });
    aged.sort();
    let store = shared.lock_store();
    let sink = StoreSink::new(&store, account, true);
    let mut used = store.bytes_of("body").unwrap_or(0);
    for (date, folder, uid) in aged {
        let old = cutoff.is_some_and(|t| date < t);
        let over = cap.is_some_and(|c| used > c);
        if !old && !over {
            break;
        }
        let size = sink.body_size(&folder, uid).unwrap_or(0);
        if size > 0 && sink.drop_body(&folder, uid).is_ok() {
            used = used.saturating_sub(size);
        }
    }
}

fn set_progress(shared: &Shared, index: usize, value: Option<(usize, usize)>) {
    lock(&shared.progress)[index] = value;
    shared.repaint();
}

pub(super) async fn run(shared: Arc<Shared>, index: usize, mut backend: Box<dyn Backend>, wake: Arc<Notify>) {
    let account = shared.accounts[index].id.clone();
    while !shared.stop.load(Ordering::Relaxed) {
        let policy = lock(&shared.policies)[index];
        if !policy.enabled {
            set_progress(&shared, index, None);
            tokio::select! {
                _ = wake.notified() => {}
                _ = tokio::time::sleep(RECHECK) => {}
            }
            continue;
        }
        if busy(&shared, index) {
            tokio::time::sleep(IDLE_WAIT).await;
            continue;
        }
        let cutoff = policy.cutoff(now());
        if cutoff.is_some() || policy.byte_cap().is_some() {
            prune(&shared, index, cutoff, policy.byte_cap());
        }
        let (todo, total) = plan(&shared, index, cutoff);
        let mut done = total - todo.len();
        set_progress(&shared, index, Some((done, total)));
        let mut fetched = 0;
        let mut failed = false;
        for c in todo {
            if shared.stop.load(Ordering::Relaxed) || lock(&shared.policies)[index] != policy {
                break;
            }
            while busy(&shared, index) && !shared.stop.load(Ordering::Relaxed) {
                tokio::time::sleep(PAUSE * 5).await;
            }
            let raw = match backend.fetch(&c.folder, c.uid).await {
                Ok(raw) => raw,
                Err(BackendError::Connection(_)) => {
                    failed = true;
                    break;
                }
                Err(_) => continue,
            };
            if let Some(cap) = policy.byte_cap() {
                let used = shared.lock_store().bytes_of("body").unwrap_or(0);
                if used + raw.len() as u64 + SEAL_OVERHEAD > cap {
                    break;
                }
            }
            {
                let store = shared.lock_store();
                let mut sink = StoreSink::new(&store, &account, true);
                if sink.put_body(&c.folder, c.uid, &raw).is_err() {
                    continue;
                }
            }
            done += 1;
            fetched += 1;
            if let Some(id) = shared.entry_id(index, &c.folder, c.uid) {
                shared.note_attachments(id, super::display::has_attachments(&raw));
            }
            if fetched % EPOCH_EVERY == 0 {
                shared.index_epoch.fetch_add(1, Ordering::SeqCst);
                set_progress(&shared, index, Some((done, total)));
            }
            let _ = c.date;
            tokio::time::sleep(PAUSE).await;
        }
        shared.index_epoch.fetch_add(1, Ordering::SeqCst);
        set_progress(&shared, index, Some((done, total)));
        let wait = if failed { IDLE_WAIT } else { RECHECK };
        tokio::select! {
            _ = wake.notified() => {}
            _ = tokio::time::sleep(wait) => {}
        }
    }
}
