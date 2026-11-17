use super::error::Result;
use super::sync::SyncSink;
use super::types::*;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct MemorySink {
    pub folders: Vec<Folder>,
    pub messages: BTreeMap<String, BTreeMap<u32, MessageSummary>>,
    pub tokens: BTreeMap<String, SyncToken>,
}

impl SyncSink for MemorySink {
    fn sync_token(&mut self, folder: &str) -> Result<Option<SyncToken>> {
        Ok(self.tokens.get(folder).cloned())
    }

    fn put_folders(&mut self, folders: &[Folder]) -> Result<()> {
        self.folders = folders.to_vec();
        Ok(())
    }

    fn reset_folder(&mut self, folder: &str) -> Result<()> {
        self.messages.remove(folder);
        self.tokens.remove(folder);
        Ok(())
    }

    fn put_summaries(&mut self, folder: &str, messages: &[MessageSummary]) -> Result<()> {
        let map = self.messages.entry(folder.to_string()).or_default();
        for m in messages {
            map.insert(m.uid, m.clone());
        }
        Ok(())
    }

    fn put_flags(&mut self, folder: &str, changes: &[(u32, Flags)]) -> Result<usize> {
        let mut changed = 0;
        if let Some(map) = self.messages.get_mut(folder) {
            for (uid, flags) in changes {
                if let Some(m) = map.get_mut(uid).filter(|m| m.flags != *flags) {
                    m.flags = flags.clone();
                    changed += 1;
                }
            }
        }
        Ok(changed)
    }

    fn retain_uids(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        if let Some(map) = self.messages.get_mut(folder) {
            map.retain(|uid, _| uids.contains(uid));
        }
        Ok(())
    }

    fn put_token(&mut self, folder: &str, token: &SyncToken) -> Result<()> {
        self.tokens.insert(folder.to_string(), token.clone());
        Ok(())
    }
}
