mod error;
mod imap;
mod memory;
mod smtp;
mod sync;
mod types;

pub use error::{BackendError, Result};
pub use imap::{ImapBackend, ImapConfig};
pub use memory::MemorySink;
pub use smtp::{SmtpBackend, SmtpConfig, SmtpSecurity};
pub use sync::{SyncReport, SyncSink, sync_account};
pub use types::*;

use std::sync::mpsc::Receiver;

pub trait Backend: Send {
    fn kind(&self) -> &'static str;

    fn capabilities(&mut self) -> Result<Capabilities> {
        Ok(Capabilities::default())
    }

    fn list_folders(&mut self) -> Result<Vec<Folder>> {
        Err(BackendError::Unsupported("list_folders"))
    }

    fn create_folder(&mut self, _name: &str) -> Result<()> {
        Err(BackendError::Unsupported("create_folder"))
    }

    fn sync_folder(&mut self, _folder: &str, _since: Option<&SyncToken>) -> Result<FolderDelta> {
        Err(BackendError::Unsupported("sync_folder"))
    }

    fn fetch(&mut self, _folder: &str, _uid: u32) -> Result<Vec<u8>> {
        Err(BackendError::Unsupported("fetch"))
    }

    fn set_flags(&mut self, _folder: &str, _uids: &[u32], _update: &FlagUpdate) -> Result<()> {
        Err(BackendError::Unsupported("set_flags"))
    }

    fn move_messages(&mut self, _folder: &str, _uids: &[u32], _destination: &str) -> Result<()> {
        Err(BackendError::Unsupported("move_messages"))
    }

    fn delete_messages(&mut self, _folder: &str, _uids: &[u32]) -> Result<()> {
        Err(BackendError::Unsupported("delete_messages"))
    }

    fn search(&mut self, _folder: &str, _query: &str) -> Result<Vec<u32>> {
        Err(BackendError::Unsupported("search"))
    }

    fn send(&mut self, _message: &OutgoingMessage) -> Result<()> {
        Err(BackendError::Unsupported("send"))
    }

    fn watch(&mut self, _folder: &str) -> Result<Watch> {
        Err(BackendError::Unsupported("watch"))
    }
}

pub struct Account {
    pub mail: Box<dyn Backend>,
    pub outbound: Box<dyn Backend>,
}

impl Backend for Account {
    fn kind(&self) -> &'static str {
        self.mail.kind()
    }

    fn capabilities(&mut self) -> Result<Capabilities> {
        let mut caps = self.mail.capabilities()?;
        caps.send = self.outbound.capabilities()?.send;
        Ok(caps)
    }

    fn list_folders(&mut self) -> Result<Vec<Folder>> {
        self.mail.list_folders()
    }

    fn create_folder(&mut self, name: &str) -> Result<()> {
        self.mail.create_folder(name)
    }

    fn sync_folder(&mut self, folder: &str, since: Option<&SyncToken>) -> Result<FolderDelta> {
        self.mail.sync_folder(folder, since)
    }

    fn fetch(&mut self, folder: &str, uid: u32) -> Result<Vec<u8>> {
        self.mail.fetch(folder, uid)
    }

    fn set_flags(&mut self, folder: &str, uids: &[u32], update: &FlagUpdate) -> Result<()> {
        self.mail.set_flags(folder, uids, update)
    }

    fn move_messages(&mut self, folder: &str, uids: &[u32], destination: &str) -> Result<()> {
        self.mail.move_messages(folder, uids, destination)
    }

    fn delete_messages(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        self.mail.delete_messages(folder, uids)
    }

    fn search(&mut self, folder: &str, query: &str) -> Result<Vec<u32>> {
        self.mail.search(folder, query)
    }

    fn send(&mut self, message: &OutgoingMessage) -> Result<()> {
        self.outbound.send(message)
    }

    fn watch(&mut self, folder: &str) -> Result<Watch> {
        self.mail.watch(folder)
    }
}

pub struct Watch {
    pub events: Receiver<Event>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Watch {
    pub(crate) fn new(
        events: Receiver<Event>,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self { events, stop }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
