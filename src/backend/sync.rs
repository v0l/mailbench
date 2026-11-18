use super::error::{BackendError, Result};
use super::types::*;
use super::Backend;

pub const HEADER_BATCH: usize = 1000;

pub trait SyncSink {
    fn sync_token(&mut self, folder: &str) -> Result<Option<SyncToken>>;
    fn put_folders(&mut self, folders: &[Folder]) -> Result<()>;
    fn reset_folder(&mut self, folder: &str) -> Result<()>;
    fn known_uids(&mut self, folder: &str) -> Result<Vec<u32>>;
    fn put_summaries(&mut self, folder: &str, messages: &[MessageSummary]) -> Result<()>;
    fn put_flags(&mut self, folder: &str, changes: &[(u32, Flags)]) -> Result<usize>;
    fn retain_uids(&mut self, folder: &str, uids: &[u32]) -> Result<()>;
    fn put_token(&mut self, folder: &str, token: &SyncToken) -> Result<()>;

    fn progress(&mut self, _folder: &str, _done: usize, _total: usize) {}

    fn cancelled(&self) -> bool {
        false
    }

    fn wants_bodies(&self) -> bool {
        false
    }

    fn put_body(&mut self, _folder: &str, _uid: u32, _raw: &[u8]) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub folders: usize,
    pub added: usize,
    pub flag_changes: usize,
    pub resets: usize,
}

fn stopped() -> BackendError {
    BackendError::Connection("sync stopped".into())
}

fn sync_order(folders: &[Folder]) -> Vec<&Folder> {
    let mut out: Vec<&Folder> = folders.iter().filter(|f| f.selectable).collect();
    out.sort_by_key(|f| match f.role {
        Some(FolderRole::Inbox) => 0,
        Some(FolderRole::Sent) | Some(FolderRole::Drafts) => 1,
        Some(FolderRole::All) => 3,
        _ => 2,
    });
    out
}

pub async fn sync_folder_into<S: SyncSink + ?Sized>(
    backend: &mut dyn Backend,
    sink: &mut S,
    folder: &str,
    report: &mut SyncReport,
) -> Result<()> {
    let state = backend.open_folder(folder).await?;
    let stored = sink.sync_token(folder)?;
    let valid = stored.filter(|t| t.uid_validity == state.token.uid_validity);
    if valid.is_none() {
        sink.reset_folder(folder)?;
        sink.put_token(folder, &SyncToken { uid_validity: state.token.uid_validity, uid_next: 0, highest_mod_seq: None })?;
        report.resets += 1;
    }
    let known: std::collections::HashSet<u32> =
        if valid.is_some() { sink.known_uids(folder)?.into_iter().collect() } else { Default::default() };
    let mut missing: Vec<u32> = state.uids.iter().copied().filter(|u| !known.contains(u)).collect();
    missing.sort_unstable_by(|a, b| b.cmp(a));
    let total = missing.len();
    let mut done = 0;
    sink.progress(folder, done, total);
    for chunk in missing.chunks(HEADER_BATCH) {
        if sink.cancelled() {
            return Err(stopped());
        }
        let summaries = backend.fetch_summaries(folder, chunk).await?;
        sink.put_summaries(folder, &summaries)?;
        if sink.wants_bodies() {
            for m in &summaries {
                let raw = backend.fetch(folder, m.uid).await?;
                sink.put_body(folder, m.uid, &raw)?;
            }
        }
        done += chunk.len();
        report.added += summaries.len();
        sink.progress(folder, done, total);
    }
    let flag_scan = match valid {
        Some(t) if t.uid_next > 0 => known.iter().any(|u| *u < t.uid_next).then_some((t.uid_next, t.highest_mod_seq)),
        Some(_) => known.iter().max().map(|m| (m + 1, None)),
        None => None,
    };
    if let Some((below, since)) = flag_scan {
        let changes = backend.changed_flags(folder, below, since).await?;
        report.flag_changes += sink.put_flags(folder, &changes)?;
    }
    sink.retain_uids(folder, &state.uids)?;
    sink.put_token(folder, &state.token)?;
    Ok(())
}

pub async fn sync_account<S: SyncSink + ?Sized>(backend: &mut dyn Backend, sink: &mut S) -> Result<SyncReport> {
    let folders = backend.list_folders().await?;
    sink.put_folders(&folders)?;
    let mut report = SyncReport { folders: folders.len(), ..Default::default() };
    for folder in sync_order(&folders) {
        if sink.cancelled() {
            return Err(stopped());
        }
        sync_folder_into(backend, sink, &folder.name, &mut report).await?;
    }
    Ok(report)
}
