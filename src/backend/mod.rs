mod error;
mod imap;
mod memory;
mod smtp;
mod store_sink;
mod sync;
mod types;

pub use error::{BackendError, Result};
pub use imap::{ImapBackend, ImapConfig};
pub use memory::MemorySink;
pub use smtp::{SmtpBackend, SmtpConfig, SmtpSecurity};
pub use store_sink::StoreSink;
pub use sync::{SyncReport, SyncSink, sync_account};
pub use types::*;

use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedReceiver;

#[async_trait]
pub trait Backend: Send {
    fn kind(&self) -> &'static str;

    fn fork(&self) -> Option<Box<dyn Backend>> {
        None
    }

    async fn capabilities(&mut self) -> Result<Capabilities> {
        Ok(Capabilities::default())
    }

    async fn check(&mut self) -> Result<()> {
        self.capabilities().await.map(|_| ())
    }

    async fn list_folders(&mut self) -> Result<Vec<Folder>> {
        Err(BackendError::Unsupported("list_folders"))
    }

    async fn create_folder(&mut self, _name: &str) -> Result<()> {
        Err(BackendError::Unsupported("create_folder"))
    }

    async fn rename_folder(&mut self, _from: &str, _to: &str) -> Result<()> {
        Err(BackendError::Unsupported("rename_folder"))
    }

    async fn open_folder(&mut self, _folder: &str) -> Result<FolderState> {
        Err(BackendError::Unsupported("open_folder"))
    }

    async fn fetch_summaries(&mut self, _folder: &str, _uids: &[u32]) -> Result<Vec<MessageSummary>> {
        Err(BackendError::Unsupported("fetch_summaries"))
    }

    async fn changed_flags(&mut self, _folder: &str, _below: u32, _since_mod_seq: Option<u64>) -> Result<Vec<(u32, Flags)>> {
        Err(BackendError::Unsupported("changed_flags"))
    }

    async fn sync_folder(&mut self, folder: &str, since: Option<&SyncToken>) -> Result<FolderDelta> {
        let state = self.open_folder(folder).await?;
        let since = since.filter(|t| t.uid_validity == state.token.uid_validity);
        let from = since.map_or(1, |t| t.uid_next);
        let fresh: Vec<u32> = state.uids.iter().copied().filter(|u| *u >= from).collect();
        let added = if fresh.is_empty() { Vec::new() } else { self.fetch_summaries(folder, &fresh).await? };
        let flag_changes = match since {
            Some(t) if state.uids.iter().any(|u| *u < t.uid_next) => {
                self.changed_flags(folder, t.uid_next, t.highest_mod_seq).await?
            }
            _ => Vec::new(),
        };
        Ok(FolderDelta { token: state.token, reset: since.is_none(), added, flag_changes, uids: state.uids, more: false })
    }

    async fn fetch(&mut self, _folder: &str, _uid: u32) -> Result<Vec<u8>> {
        Err(BackendError::Unsupported("fetch"))
    }

    async fn set_flags(&mut self, _folder: &str, _uids: &[u32], _update: &FlagUpdate) -> Result<()> {
        Err(BackendError::Unsupported("set_flags"))
    }

    async fn move_messages(&mut self, _folder: &str, _uids: &[u32], _destination: &str) -> Result<()> {
        Err(BackendError::Unsupported("move_messages"))
    }

    async fn delete_messages(&mut self, _folder: &str, _uids: &[u32]) -> Result<()> {
        Err(BackendError::Unsupported("delete_messages"))
    }

    async fn search(&mut self, _folder: &str, _query: &str) -> Result<Vec<u32>> {
        Err(BackendError::Unsupported("search"))
    }

    async fn append(&mut self, _folder: &str, _raw: &[u8]) -> Result<()> {
        Err(BackendError::Unsupported("append"))
    }

    async fn delete_folder(&mut self, _name: &str) -> Result<()> {
        Err(BackendError::Unsupported("delete_folder"))
    }

    async fn send(&mut self, _message: &OutgoingMessage) -> Result<()> {
        Err(BackendError::Unsupported("send"))
    }

    async fn watch(&mut self, _folder: &str) -> Result<Watch> {
        Err(BackendError::Unsupported("watch"))
    }
}

pub struct Account {
    pub mail: Box<dyn Backend>,
    pub outbound: Box<dyn Backend>,
}

#[async_trait]
impl Backend for Account {
    fn kind(&self) -> &'static str {
        self.mail.kind()
    }

    fn fork(&self) -> Option<Box<dyn Backend>> {
        Some(Box::new(Account { mail: self.mail.fork()?, outbound: self.outbound.fork()? }))
    }

    async fn capabilities(&mut self) -> Result<Capabilities> {
        let mut caps = self.mail.capabilities().await?;
        caps.send = self.outbound.capabilities().await?.send;
        Ok(caps)
    }

    async fn check(&mut self) -> Result<()> {
        self.mail.check().await?;
        self.outbound.check().await
    }

    async fn list_folders(&mut self) -> Result<Vec<Folder>> {
        self.mail.list_folders().await
    }

    async fn create_folder(&mut self, name: &str) -> Result<()> {
        self.mail.create_folder(name).await
    }

    async fn rename_folder(&mut self, from: &str, to: &str) -> Result<()> {
        self.mail.rename_folder(from, to).await
    }

    async fn open_folder(&mut self, folder: &str) -> Result<FolderState> {
        self.mail.open_folder(folder).await
    }

    async fn fetch_summaries(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<MessageSummary>> {
        self.mail.fetch_summaries(folder, uids).await
    }

    async fn changed_flags(&mut self, folder: &str, below: u32, since_mod_seq: Option<u64>) -> Result<Vec<(u32, Flags)>> {
        self.mail.changed_flags(folder, below, since_mod_seq).await
    }

    async fn sync_folder(&mut self, folder: &str, since: Option<&SyncToken>) -> Result<FolderDelta> {
        self.mail.sync_folder(folder, since).await
    }

    async fn fetch(&mut self, folder: &str, uid: u32) -> Result<Vec<u8>> {
        self.mail.fetch(folder, uid).await
    }

    async fn set_flags(&mut self, folder: &str, uids: &[u32], update: &FlagUpdate) -> Result<()> {
        self.mail.set_flags(folder, uids, update).await
    }

    async fn move_messages(&mut self, folder: &str, uids: &[u32], destination: &str) -> Result<()> {
        self.mail.move_messages(folder, uids, destination).await
    }

    async fn delete_messages(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        self.mail.delete_messages(folder, uids).await
    }

    async fn search(&mut self, folder: &str, query: &str) -> Result<Vec<u32>> {
        self.mail.search(folder, query).await
    }

    async fn append(&mut self, folder: &str, raw: &[u8]) -> Result<()> {
        self.mail.append(folder, raw).await
    }

    async fn delete_folder(&mut self, name: &str) -> Result<()> {
        self.mail.delete_folder(name).await
    }

    async fn send(&mut self, message: &OutgoingMessage) -> Result<()> {
        self.outbound.send(message).await
    }

    async fn watch(&mut self, folder: &str) -> Result<Watch> {
        self.mail.watch(folder).await
    }
}

pub struct Watch {
    pub events: UnboundedReceiver<Event>,
    task: tokio::task::AbortHandle,
}

impl Watch {
    pub(crate) fn new(events: UnboundedReceiver<Event>, task: tokio::task::AbortHandle) -> Self {
        Self { events, task }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.task.abort();
    }
}
