use super::worker::SearchRequest;
use super::{Entry, Shared, lock, stable_id, ui_folder};
use crate::backend::*;
use crate::mailstore::{Filter, Query};
use crate::search::Parsed;
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

const RERUN: Duration = Duration::from_millis(800);
const VERIFY_LIMIT: usize = 5000;

#[derive(Default)]
pub(super) struct SearchState {
    text: String,
    epoch: u64,
    started: Option<Instant>,
    running: bool,
    pending: usize,
    hits: Option<Arc<HashSet<u64>>>,
    remote: Arc<HashSet<u64>>,
    asked: HashSet<(usize, String)>,
}

pub(super) struct SearchJob {
    text: String,
}

pub(super) struct View {
    parsed: Parsed,
    plain: bool,
    hits: Option<Arc<HashSet<u64>>>,
    remote: Arc<HashSet<u64>>,
}

impl View {
    pub fn is_plain(&self) -> bool {
        self.parsed.is_empty()
    }

    pub fn is_remote(&self, id: u64) -> bool {
        self.remote.contains(&id) && !self.hits.as_ref().is_some_and(|h| h.contains(&id))
    }
}

pub(super) fn scope<'a>(c: &'a super::Cache, q: &Query, v: &View) -> Box<dyn Iterator<Item = &'a Entry> + 'a> {
    if v.parsed.folder.is_some() || v.parsed.label.is_some() {
        return Box::new(c.entries.iter());
    }
    Box::new(c.by_folder.get(&q.folder).into_iter().flatten().map(|i| &c.entries[*i]))
}

pub(super) fn matches_row(c: &super::Cache, e: &Entry, q: &Query, v: &View) -> bool {
    if !e.head {
        return false;
    }
    if q.account.is_some_and(|a| a != e.account) {
        return false;
    }
    if v.plain {
        return e.folder == q.folder
            && match q.filter {
                Filter::All => true,
                Filter::Unread => e.thread_unread,
                Filter::Flagged => e.thread_flagged,
            };
    }
    if e.count <= 1 {
        return matches(e, q, v);
    }
    match c.by_thread.get(&e.thread) {
        Some(members) => members.iter().any(|i| c.entries[*i].folder == e.folder && matches(&c.entries[*i], q, v)),
        None => matches(e, q, v),
    }
}

pub(super) fn matches(e: &Entry, q: &Query, v: &View) -> bool {
    let p = &v.parsed;
    if q.account.is_some_and(|a| a != e.account) {
        return false;
    }
    match &p.folder {
        Some(f) => {
            if e.folder.name().to_lowercase() != *f && e.server_folder.to_lowercase() != *f {
                return false;
            }
        }
        None => {
            if p.label.is_none() && e.folder != q.folder {
                return false;
            }
        }
    }
    if let Some(l) = &p.label {
        if !e.labels.iter().any(|x| x.to_lowercase() == *l) {
            return false;
        }
    }
    match q.filter {
        Filter::All => {}
        Filter::Unread if !e.unread => return false,
        Filter::Flagged if !e.flagged => return false,
        _ => {}
    }
    if p.unread.is_some_and(|u| e.unread != u) || p.flagged.is_some_and(|f| e.flagged != f) {
        return false;
    }
    if p.has_attachment && !e.attachments {
        return false;
    }
    if p.before.is_some_and(|t| e.date >= t) || p.after.is_some_and(|t| e.date < t) {
        return false;
    }
    if !p.from.is_empty() || !p.subject.is_empty() {
        let mut parts = e.search_key.splitn(3, '\0');
        let (name, address, subject) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        if !p.from.iter().all(|f| name.contains(f.as_str()) || address.contains(f.as_str())) {
            return false;
        }
        if !p.subject.iter().all(|s| subject.contains(s.as_str())) {
            return false;
        }
    }
    if !p.to.iter().all(|t| e.to_key.contains(t.as_str())) {
        return false;
    }
    if p.has_text() {
        let local = p.terms.iter().chain(&p.phrases).all(|t| e.search_key.contains(t.as_str()));
        if !local && !v.hits.as_ref().is_some_and(|h| h.contains(&e.id)) && !v.remote.contains(&e.id) {
            return false;
        }
    }
    true
}

impl Shared {
    pub(super) fn search_view(&self, query: &Query) -> View {
        let parsed = Parsed::parse(&query.text);
        if !parsed.has_text() {
            let plain = parsed.is_empty();
            return View { parsed, plain, hits: None, remote: Default::default() };
        }
        let epoch = self.index_epoch.load(Ordering::SeqCst);
        let mut st = lock(&self.search);
        let fresh = st.text != query.text;
        let stale = !fresh && st.epoch != epoch && !st.running && st.started.is_none_or(|t| t.elapsed() >= RERUN);
        if fresh {
            *st = SearchState { text: query.text.clone(), ..Default::default() };
        }
        if fresh || stale {
            st.epoch = epoch;
            st.running = true;
            st.started = Some(Instant::now());
            let sent = lock(&self.search_tx).as_ref().is_some_and(|tx| tx.send(SearchJob { text: query.text.clone() }).is_ok());
            if !sent {
                st.running = false;
            }
        }
        View { parsed, plain: false, hits: st.hits.clone(), remote: st.remote.clone() }
    }

    pub(super) fn searching(&self, query: &Query) -> bool {
        if !Parsed::parse(&query.text).has_text() {
            return false;
        }
        let st = lock(&self.search);
        st.text == query.text && (st.running || st.pending > 0)
    }

    pub(super) fn finish_remote(&self, index: usize, text: &str, folder: &str, uids: Vec<u32>) {
        let validity = self.with_cache(|c| c.validity.get(&(index, folder.to_string())).copied()).unwrap_or(0);
        let account = &self.accounts[index].id;
        {
            let mut st = lock(&self.search);
            if st.text != text {
                return;
            }
            st.pending = st.pending.saturating_sub(1);
            let remote = Arc::make_mut(&mut st.remote);
            for uid in uids {
                remote.insert(stable_id(&format!("r\0{account}\0{folder}\0{validity}\0{uid}")));
            }
        }
        self.repaint();
    }

    fn search_current(&self, text: &str) -> bool {
        lock(&self.search).text == text
    }
}

pub(super) fn serve(shared: Arc<Shared>, jobs: Receiver<SearchJob>) {
    while let Ok(mut job) = jobs.recv() {
        while let Ok(next) = jobs.try_recv() {
            job = next;
        }
        if shared.stop.load(Ordering::Relaxed) {
            return;
        }
        run(&shared, &job.text);
        let mut st = lock(&shared.search);
        if st.text == job.text {
            st.running = false;
        }
        drop(st);
        shared.repaint();
    }
}

fn run(shared: &Arc<Shared>, text: &str) {
    let parsed = Parsed::parse(text);
    let Some(query) = parsed.index_query() else { return };
    let validity = shared.with_cache(|c| c.validity.clone());
    let mut hits: HashSet<u64> = HashSet::new();
    let mut folders_hit: HashSet<(usize, String)> = HashSet::new();
    for (index, account) in shared.accounts.iter().enumerate() {
        if shared.stop.load(Ordering::Relaxed) || !shared.search_current(text) {
            return;
        }
        let found = {
            let store = shared.lock_store();
            let sink = StoreSink::new(&store, &account.id, true);
            let mut found = sink.search_hits(&query).unwrap_or_default();
            if !parsed.phrases.is_empty() {
                let mut checked = 0;
                found.retain(|(folder, uid)| {
                    checked += 1;
                    if checked > VERIFY_LIMIT {
                        return true;
                    }
                    let haystack = sink.searchable_text(folder, *uid).unwrap_or_default();
                    parsed.phrases.iter().all(|p| haystack.contains(p.as_str()))
                });
            }
            found
        };
        for (folder, uid) in found {
            let v = validity.get(&(index, folder.clone())).copied().unwrap_or(0);
            hits.insert(stable_id(&format!("r\0{}\0{}\0{}\0{}", account.id, folder, v, uid)));
            folders_hit.insert((index, folder));
        }
        {
            let mut st = lock(&shared.search);
            if st.text != text {
                return;
            }
            st.hits = Some(Arc::new(hits.clone()));
        }
        shared.repaint();
    }
    ask_servers(shared, text, &parsed, &folders_hit);
}

fn ask_servers(shared: &Arc<Shared>, text: &str, parsed: &Parsed, folders_hit: &HashSet<(usize, String)>) {
    let Some(server_query) = parsed.server_query() else { return };
    for (index, account) in shared.accounts.iter().enumerate() {
        if !shared.caps_of(index).server_search {
            continue;
        }
        let folders = shared.folders_of(&account.id);
        for folder in folders.iter().filter(|f| f.selectable) {
            let ui = ui_folder(folder).unwrap_or_else(|| crate::mailstore::Folder::Other(index, folder.name.clone()));
            if let Some(wanted) = &parsed.folder {
                if ui.name().to_lowercase() != *wanted && folder.name.to_lowercase() != *wanted {
                    continue;
                }
            }
            if folders_hit.contains(&(index, folder.name.clone())) {
                continue;
            }
            let complete = {
                let store = shared.lock_store();
                let mut sink = StoreSink::new(&store, &account.id, true);
                let known = sink.known_uids(&folder.name).unwrap_or_default().len();
                known == 0 || sink.body_uids(&folder.name).map(|b| b.len()).unwrap_or(0) >= known
            };
            if complete {
                continue;
            }
            {
                let mut st = lock(&shared.search);
                if st.text != text || !st.asked.insert((index, folder.name.clone())) {
                    continue;
                }
                st.pending += 1;
            }
            let sent = lock(&shared.searchers).get(index).is_some_and(|tx| {
                tx.send(SearchRequest { text: text.to_string(), folder: folder.name.clone(), query: server_query.clone() }).is_ok()
            });
            if !sent {
                let mut st = lock(&shared.search);
                st.pending = st.pending.saturating_sub(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mailstore::Folder;

    fn entry(folder: Folder, labels: &[&str]) -> Entry {
        Entry {
            id: 1,
            account: 0,
            folder,
            server_folder: String::new(),
            uid: 1,
            validity: 1,
            local: None,
            from: String::new(),
            from_address: String::new(),
            to: Vec::new(),
            cc: Vec::new(),
            subject: String::new(),
            date: 0,
            unread: false,
            flagged: false,
            labels: labels.iter().map(|l| l.to_string()).collect(),
            attachments: false,
            search_key: String::new(),
            thread: 1,
            head: true,
            count: 1,
            thread_unread: false,
            thread_flagged: false,
            to_key: String::new(),
            message_id: None,
            refs: Vec::new(),
            wake: None,
        }
    }

    fn view(text: &str) -> View {
        View { parsed: Parsed::parse(text), plain: false, hits: None, remote: Default::default() }
    }

    #[test]
    fn a_label_query_matches_messages_in_any_folder_by_label_case_insensitively() {
        let q = Query { folder: Folder::Inbox, filter: Filter::All, text: "label:\"bench night\"".into(), account: None };
        let v = view(&q.text);
        assert!(matches(&entry(Folder::Inbox, &["Bench Night"]), &q, &v));
        assert!(matches(&entry(Folder::Archive, &["Receipts", "Bench Night"]), &q, &v));
        assert!(!matches(&entry(Folder::Inbox, &["Receipts"]), &q, &v));
        assert!(!matches(&entry(Folder::Inbox, &[]), &q, &v));
    }
}
