use super::error::{BackendError, Result};
use super::sync::SyncSink;
use super::types::*;
use crate::store::Store;
use serde::Serialize;
use std::collections::BTreeMap;
use serde::de::DeserializeOwned;

const FOLDERS: &str = "folders";
const MESSAGE: &str = "message";
const BODY: &str = "body";
const FOLDER_IDS: &str = "folderids";
const TOKEN: &str = "synctoken";

pub struct StoreSink<'a> {
    store: &'a Store,
    account: String,
    keep_bodies: bool,
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|e| BackendError::Store(e.to_string()))
}

fn decode<T: DeserializeOwned>(raw: &[u8]) -> Result<T> {
    serde_json::from_slice(raw).map_err(|e| BackendError::Store(e.to_string()))
}

impl<'a> StoreSink<'a> {
    pub fn new(store: &'a Store, account: &str, keep_bodies: bool) -> Self {
        Self {
            store,
            account: account.to_string(),
            keep_bodies,
        }
    }

    pub fn purge(store: &Store, account: &str) -> Result<()> {
        let prefix = format!("{account}\0");
        for kind in [FOLDERS, FOLDER_IDS, MESSAGE, BODY, TOKEN] {
            for id in store.list(kind)? {
                if id == account || id.starts_with(&prefix) {
                    store.delete(kind, &id)?;
                }
            }
        }
        Ok(())
    }

    fn folder_prefix(&self, folder: &str) -> Result<String> {
        let mut map: BTreeMap<String, u32> = match self.store.get(FOLDER_IDS, &self.account)? {
            Some(raw) => decode(&raw)?,
            None => BTreeMap::new(),
        };
        let id = match map.get(folder) {
            Some(id) => *id,
            None => {
                let id = map.values().max().map_or(1, |m| m + 1);
                map.insert(folder.to_string(), id);
                self.store.put(FOLDER_IDS, &self.account, &encode(&map)?)?;
                id
            }
        };
        Ok(format!("{}\0f{id}\0", self.account))
    }

    fn message_id(&self, folder: &str, uid: u32) -> Result<String> {
        Ok(format!("{}{:010}", self.folder_prefix(folder)?, uid))
    }

    fn uid_of(&self, folder: &str, id: &str) -> Result<Option<u32>> {
        Ok(id.strip_prefix(&self.folder_prefix(folder)?).and_then(|u| u.parse().ok()))
    }

    fn ids(&self, kind: &str, folder: &str) -> Result<Vec<String>> {
        let prefix = self.folder_prefix(folder)?;
        Ok(self
            .store
            .list(kind)?
            .into_iter()
            .filter(|id| id.starts_with(&prefix))
            .collect())
    }

    pub fn messages(&self, folder: &str) -> Result<Vec<MessageSummary>> {
        let mut out = Vec::new();
        for id in self.ids(MESSAGE, folder)? {
            if let Some(raw) = self.store.get(MESSAGE, &id)? {
                out.push(decode(&raw)?);
            }
        }
        Ok(out)
    }

    pub fn summary(&self, folder: &str, uid: u32) -> Result<Option<MessageSummary>> {
        match self.store.get(MESSAGE, &self.message_id(folder, uid)?)? {
            Some(raw) => Ok(Some(decode(&raw)?)),
            None => Ok(None),
        }
    }

    pub fn folders(&self) -> Result<Vec<Folder>> {
        match self.store.get(FOLDERS, &self.account)? {
            Some(raw) => decode(&raw),
            None => Ok(Vec::new()),
        }
    }

    pub fn body(&self, folder: &str, uid: u32) -> Result<Option<Vec<u8>>> {
        Ok(self
            .store
            .get(BODY, &self.message_id(folder, uid)?)?
            .map(|b| b.to_vec()))
    }

    pub fn search(&self, query: &str) -> Result<Vec<String>> {
        let prefix = format!("{}\0", self.account);
        Ok(self
            .store
            .search(query)?
            .into_iter()
            .filter(|id| id.starts_with(&prefix))
            .collect())
    }

    fn index(&self, id: &str, text: &str) -> Result<()> {
        self.store.index_text(id, text)?;
        Ok(())
    }
}

impl SyncSink for StoreSink<'_> {
    fn sync_token(&mut self, folder: &str) -> Result<Option<SyncToken>> {
        match self.store.get(TOKEN, &self.folder_prefix(folder)?)? {
            Some(raw) => Ok(Some(decode(&raw)?)),
            None => Ok(None),
        }
    }

    fn put_folders(&mut self, folders: &[Folder]) -> Result<()> {
        self.store.put(FOLDERS, &self.account, &encode(&folders)?)?;
        Ok(())
    }

    fn reset_folder(&mut self, folder: &str) -> Result<()> {
        for kind in [MESSAGE, BODY] {
            for id in self.ids(kind, folder)? {
                self.store.delete(kind, &id)?;
            }
        }
        self.store.delete(TOKEN, &self.folder_prefix(folder)?)?;
        Ok(())
    }

    fn put_summaries(&mut self, folder: &str, messages: &[MessageSummary]) -> Result<()> {
        for m in messages {
            let id = self.message_id(folder, m.uid)?;
            self.store.put(MESSAGE, &id, &encode(m)?)?;
            let mut text = m.subject.clone().unwrap_or_default();
            for a in &m.from {
                text.push(' ');
                text.push_str(a.name.as_deref().unwrap_or_default());
                text.push(' ');
                text.push_str(&a.address);
            }
            self.index(&id, &text)?;
        }
        Ok(())
    }

    fn put_flags(&mut self, folder: &str, changes: &[(u32, Flags)]) -> Result<usize> {
        let mut changed = 0;
        for (uid, flags) in changes {
            let id = self.message_id(folder, *uid)?;
            let Some(raw) = self.store.get(MESSAGE, &id)? else {
                continue;
            };
            let mut m: MessageSummary = decode(&raw)?;
            if m.flags != *flags {
                m.flags = flags.clone();
                self.store.put(MESSAGE, &id, &encode(&m)?)?;
                changed += 1;
            }
        }
        Ok(changed)
    }

    fn retain_uids(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        for kind in [MESSAGE, BODY] {
            for id in self.ids(kind, folder)? {
                if self.uid_of(folder, &id)?.is_some_and(|u| !uids.contains(&u)) {
                    self.store.delete(kind, &id)?;
                }
            }
        }
        Ok(())
    }

    fn put_token(&mut self, folder: &str, token: &SyncToken) -> Result<()> {
        self.store
            .put(TOKEN, &self.folder_prefix(folder)?, &encode(token)?)?;
        Ok(())
    }

    fn wants_bodies(&self) -> bool {
        self.keep_bodies
    }

    fn put_body(&mut self, folder: &str, uid: u32, raw: &[u8]) -> Result<()> {
        let id = self.message_id(folder, uid)?;
        self.store.put(BODY, &id, raw)?;
        if let Some(msg) = mail_parser::MessageParser::default().parse(raw) {
            let mut text = msg.subject().unwrap_or_default().to_string();
            for i in 0..msg.text_body().len() {
                text.push(' ');
                text.push_str(&msg.body_text(i).unwrap_or_default());
            }
            self.index(&id, &text)?;
        }
        Ok(())
    }
}
