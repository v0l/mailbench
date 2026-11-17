use super::error::Result;
use super::types::*;
use super::Backend;

pub trait SyncSink {
    fn sync_token(&mut self, folder: &str) -> Result<Option<SyncToken>>;
    fn put_folders(&mut self, folders: &[Folder]) -> Result<()>;
    fn reset_folder(&mut self, folder: &str) -> Result<()>;
    fn put_summaries(&mut self, folder: &str, messages: &[MessageSummary]) -> Result<()>;
    fn put_flags(&mut self, folder: &str, changes: &[(u32, Flags)]) -> Result<usize>;
    fn retain_uids(&mut self, folder: &str, uids: &[u32]) -> Result<()>;
    fn put_token(&mut self, folder: &str, token: &SyncToken) -> Result<()>;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub folders: usize,
    pub added: usize,
    pub flag_changes: usize,
    pub resets: usize,
}

pub fn sync_account(backend: &mut dyn Backend, sink: &mut dyn SyncSink) -> Result<SyncReport> {
    let folders = backend.list_folders()?;
    sink.put_folders(&folders)?;
    let mut report = SyncReport {
        folders: folders.len(),
        ..Default::default()
    };
    for folder in folders.iter().filter(|f| f.selectable) {
        let since = sink.sync_token(&folder.name)?;
        let delta = backend.sync_folder(&folder.name, since.as_ref())?;
        if delta.reset {
            sink.reset_folder(&folder.name)?;
            report.resets += 1;
        }
        sink.put_summaries(&folder.name, &delta.added)?;
        report.flag_changes += sink.put_flags(&folder.name, &delta.flag_changes)?;
        sink.retain_uids(&folder.name, &delta.uids)?;
        sink.put_token(&folder.name, &delta.token)?;
        report.added += delta.added.len();
    }
    Ok(report)
}
