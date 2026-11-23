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

fn summary_text(m: &MessageSummary) -> String {
    let mut text = m.subject.clone().unwrap_or_default();
    for a in &m.from {
        text.push(' ');
        text.push_str(a.name.as_deref().unwrap_or_default());
        text.push(' ');
        text.push_str(&a.address);
    }
    text
}

fn plain_text(raw: &[u8]) -> String {
    let Some(msg) = mail_parser::MessageParser::default().parse(raw) else { return String::new() };
    let mut text = String::new();
    for i in 0..msg.text_body().len() {
        text.push(' ');
        text.push_str(&msg.body_text(i).unwrap_or_default());
    }
    text
}

impl<'a> StoreSink<'a> {
    pub fn new(store: &'a Store, account: &str, keep_bodies: bool) -> Self {
        Self {
            store,
            account: account.to_string(),
            keep_bodies,
        }
    }

    pub fn local_counts(store: &Store, account: &str) -> (u64, u64) {
        let prefix = format!("{account}\0");
        (store.count_prefix(MESSAGE, &prefix).unwrap_or(0), store.count_prefix(BODY, &prefix).unwrap_or(0))
    }

    pub fn purge(store: &Store, account: &str) -> Result<()> {
        let prefix = format!("{account}\0");
        store.transaction(|| {
            for kind in [FOLDERS, FOLDER_IDS, MESSAGE, BODY, TOKEN] {
                store.delete(kind, account)?;
                store.delete_prefix(kind, &prefix)?;
            }
            Ok(())
        })
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

    pub fn messages(&self, folder: &str) -> Result<Vec<MessageSummary>> {
        let prefix = self.folder_prefix(folder)?;
        self.store.scan_prefix(MESSAGE, &prefix)?.iter().map(|(_, raw)| decode(raw)).collect()
    }

    pub fn messages_page(&self, folder: &str, after: Option<&str>, limit: usize) -> Result<Vec<(String, MessageSummary)>> {
        let prefix = self.folder_prefix(folder)?;
        let rows = self.store.scan_prefix_page(MESSAGE, &prefix, after, limit)?;
        rows.iter().map(|(id, raw)| Ok((id.clone(), decode(raw)?))).collect()
    }

    pub fn newest_page(&self, folder: &str, before: Option<&str>, limit: usize) -> Result<Vec<(String, MessageSummary)>> {
        let prefix = self.folder_prefix(folder)?;
        let rows = self.store.scan_prefix_page_desc(MESSAGE, &prefix, before, limit)?;
        rows.iter().map(|(id, raw)| Ok((id.clone(), decode(raw)?))).collect()
    }

    pub fn summary(&self, folder: &str, uid: u32) -> Result<Option<MessageSummary>> {
        match self.store.get(MESSAGE, &self.message_id(folder, uid)?)? {
            Some(raw) => Ok(Some(decode(&raw)?)),
            None => Ok(None),
        }
    }

    pub fn sync_token_for(&self, folder: &str) -> Option<SyncToken> {
        SyncSink::sync_token(&mut Self { store: self.store, account: self.account.clone(), keep_bodies: false }, folder).ok().flatten()
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

    pub fn body_uids(&self, folder: &str) -> Result<Vec<u32>> {
        let prefix = self.folder_prefix(folder)?;
        Ok(self
            .store
            .list_prefix(BODY, &prefix)?
            .iter()
            .filter_map(|id| id.strip_prefix(&prefix).and_then(|u| u.parse().ok()))
            .collect())
    }

    pub fn body_size(&self, folder: &str, uid: u32) -> Result<u64> {
        Ok(self.store.size_of(BODY, &self.message_id(folder, uid)?)?)
    }

    pub fn drop_body(&self, folder: &str, uid: u32) -> Result<()> {
        let id = self.message_id(folder, uid)?;
        self.store.delete(BODY, &id)?;
        if let Some(raw) = self.store.get(MESSAGE, &id)? {
            self.index(&id, &summary_text(&decode(&raw)?))?;
        }
        Ok(())
    }

    pub fn search_hits(&self, query: &str) -> Result<Vec<(String, u32)>> {
        let prefix = format!("{}\0", self.account);
        let names: BTreeMap<u32, String> = match self.store.get(FOLDER_IDS, &self.account)? {
            Some(raw) => decode::<BTreeMap<String, u32>>(&raw)?.into_iter().map(|(n, i)| (i, n)).collect(),
            None => return Ok(Vec::new()),
        };
        let mut out = Vec::new();
        for id in self.store.search(query)? {
            let Some(rest) = id.strip_prefix(&prefix) else { continue };
            let Some((folder, uid)) = rest.split_once('\0') else { continue };
            let number = folder.strip_prefix('f').and_then(|n| n.parse::<u32>().ok());
            if let (Some(name), Ok(uid)) = (number.and_then(|n| names.get(&n)), uid.parse::<u32>()) {
                out.push((name.clone(), uid));
            }
        }
        Ok(out)
    }

    pub fn searchable_text(&self, folder: &str, uid: u32) -> Result<String> {
        let mut text = match self.summary(folder, uid)? {
            Some(m) => summary_text(&m),
            None => String::new(),
        };
        if let Some(raw) = self.body(folder, uid)? {
            text.push(' ');
            text.push_str(&plain_text(&raw));
        }
        Ok(text.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" "))
    }

    pub fn body_text(&self, folder: &str, uid: u32) -> Result<Option<String>> {
        Ok(self.body(folder, uid)?.map(|raw| plain_text(&raw)))
    }

    fn index_body(&self, id: &str, raw: &[u8]) -> Result<()> {
        let mut text = match self.store.get(MESSAGE, id)? {
            Some(m) => summary_text(&decode(&m)?),
            None => String::new(),
        };
        text.push_str(&plain_text(raw));
        if let Some(msg) = mail_parser::MessageParser::default().parse(raw) {
            for p in msg.attachments() {
                text.push(' ');
                text.push_str(p.attachment_name().unwrap_or_default());
            }
        }
        self.index(id, &text)
    }

    pub fn reindex(&self) -> Result<usize> {
        let prefix = format!("{}\0", self.account);
        let mut done = 0;
        self.store.transaction(|| {
            for (id, raw) in self.store.scan_prefix(MESSAGE, &prefix)? {
                match self.store.get(BODY, &id)? {
                    Some(body) => self.index_body(&id, &body)?,
                    None => self.index(&id, &summary_text(&decode(&raw)?))?,
                }
                done += 1;
            }
            Ok(done)
        })
    }

    pub fn drop_bodies_before(&self, cutoff: i64) -> Result<usize> {
        let mut dropped = 0;
        for folder in self.folders()? {
            for uid in self.body_uids(&folder.name)? {
                let old = self.summary(&folder.name, uid)?.and_then(|m| m.date).is_some_and(|d| d < cutoff);
                if old {
                    self.drop_body(&folder.name, uid)?;
                    dropped += 1;
                }
            }
        }
        Ok(dropped)
    }

    pub fn drop_all_bodies(&self) -> Result<usize> {
        self.drop_bodies_before(i64::MAX)
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
        let prefix = self.folder_prefix(folder)?;
        self.store.transaction(|| {
            self.store.delete_prefix(MESSAGE, &prefix)?;
            self.store.delete_prefix(BODY, &prefix)?;
            self.store.delete(TOKEN, &prefix)?;
            Ok(())
        })
    }

    fn known_uids(&mut self, folder: &str) -> Result<Vec<u32>> {
        let prefix = self.folder_prefix(folder)?;
        Ok(self
            .store
            .list_prefix(MESSAGE, &prefix)?
            .iter()
            .filter_map(|id| id.strip_prefix(&prefix).and_then(|u| u.parse().ok()))
            .collect())
    }

    fn put_summaries(&mut self, folder: &str, messages: &[MessageSummary]) -> Result<()> {
        let prefix = self.folder_prefix(folder)?;
        self.store.transaction(|| {
            for m in messages {
                let id = format!("{prefix}{:010}", m.uid);
                self.store.put(MESSAGE, &id, &encode(m)?)?;
                self.index(&id, &summary_text(m))?;
            }
            Ok(())
        })
    }

    fn put_flags(&mut self, folder: &str, changes: &[(u32, Flags)]) -> Result<usize> {
        let prefix = self.folder_prefix(folder)?;
        self.store.transaction(|| {
            let mut changed = 0;
            for (uid, flags) in changes {
                let id = format!("{prefix}{:010}", uid);
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
        })
    }

    fn retain_uids(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        let prefix = self.folder_prefix(folder)?;
        let keep: std::collections::HashSet<u32> = uids.iter().copied().collect();
        self.store.transaction(|| {
            for kind in [MESSAGE, BODY] {
                for id in self.store.list_prefix(kind, &prefix)? {
                    let uid = id.strip_prefix(&prefix).and_then(|u| u.parse::<u32>().ok());
                    if uid.is_some_and(|u| !keep.contains(&u)) {
                        self.store.delete(kind, &id)?;
                    }
                }
            }
            Ok(())
        })
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
        self.index_body(&id, raw)
    }
}
