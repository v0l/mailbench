use super::prefs::AgentPrefs;
use super::sanitize;
use super::tools::{Tools, definition, error_result, iso_date, text_result, with_items};
use crate::mailstore::{Filter, FlagChange, Folder, MailStore, Message, MessageId, Query, StoreError, Summary};
use base64::Engine;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const MAX_LIMIT: usize = 50;
pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_ATTACHMENT: usize = 1_000_000;
pub const MAX_IDS: usize = 100;
const BODY_WAIT: Duration = Duration::from_secs(15);
const SEARCH_WAIT: Duration = Duration::from_secs(20);
const THREAD_MAX: usize = 50;
const SNIPPET_CHARS: usize = 120;
const SNIPPET_WAIT: Duration = Duration::from_secs(4);

fn flag_words(unread: &Value, flagged: &Value, attachments: &Value) -> String {
    let mut words = Vec::new();
    words.push(if unread == &json!(true) { "unread" } else { "read" });
    if flagged == &json!(true) {
        words.push("flagged");
    }
    if attachments == &json!(true) {
        words.push("attachments");
    }
    words.join(" ")
}

fn listing_line(m: &Value) -> String {
    let field = |k: &str, max: usize| sanitize::cell(m[k].as_str().unwrap_or_default(), max);
    let line = format!(
        "{} | {} | {} | {} | {} | {} | {}",
        field("id", 40),
        m["date"].as_str().unwrap_or("no date"),
        field("from", 80),
        field("subject", 300),
        field("folder", 60),
        flag_words(&m["unread"], &m["flagged"], &m["has_attachments"]),
        field("snippet", SNIPPET_CHARS),
    );
    line.trim_end_matches([' ', '|']).to_string()
}

pub struct AccountRef {
    pub id: String,
    pub name: String,
    pub address: String,
    pub nostr: bool,
}

pub struct MailTools {
    pub(super) store: Mutex<Box<dyn MailStore + Send>>,
    pub(super) accounts: Vec<AccountRef>,
    pub(super) prefs: Box<dyn Fn() -> AgentPrefs + Send + Sync>,
    pub(super) dir: Option<PathBuf>,
    pub(super) sends: Mutex<Vec<(usize, i64)>>,
    pub(super) clock: Box<dyn Fn() -> i64 + Send + Sync>,
}

pub type Res<T> = Result<T, String>;

pub fn parse_id(text: &str) -> Res<MessageId> {
    u64::from_str_radix(text.trim().trim_start_matches("0x"), 16).map(MessageId).map_err(|_| format!("{text} is not a message id"))
}

pub fn id_text(id: MessageId) -> String {
    format!("{:x}", id.0)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn until(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while !cond() {
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    true
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn bool_arg(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn ids_arg(args: &Value) -> Res<Vec<MessageId>> {
    let list = args.get("ids").and_then(Value::as_array).ok_or("ids must be a list of message ids")?;
    if list.is_empty() {
        return Err("ids is empty".into());
    }
    if list.len() > MAX_IDS {
        return Err(format!("at most {MAX_IDS} ids per call"));
    }
    list.iter().map(|v| v.as_str().ok_or_else(|| "ids must be strings".to_string()).and_then(parse_id)).collect()
}

fn folder_named(name: &str) -> Option<Folder> {
    let lower = name.to_lowercase();
    Folder::UNIFIED.iter().find(|f| f.name() == lower).cloned()
}

impl MailTools {
    pub fn new(
        store: Box<dyn MailStore + Send>,
        accounts: Vec<AccountRef>,
        prefs: Box<dyn Fn() -> AgentPrefs + Send + Sync>,
        dir: Option<PathBuf>,
    ) -> MailTools {
        MailTools { store: Mutex::new(store), accounts, prefs, dir, sends: Mutex::new(Vec::new()), clock: Box::new(now_secs) }
    }

    pub fn with_clock(mut self, clock: Box<dyn Fn() -> i64 + Send + Sync>) -> MailTools {
        self.clock = clock;
        self
    }

    pub fn store_for_tests(&self) -> std::sync::MutexGuard<'_, Box<dyn MailStore + Send>> {
        self.store()
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Box<dyn MailStore + Send>> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn allowed(&self, prefs: &AgentPrefs, index: usize) -> bool {
        self.accounts.get(index).is_some_and(|a| prefs.allows(&a.id))
    }

    pub(super) fn resolve_account(&self, prefs: &AgentPrefs, wanted: &str) -> Res<usize> {
        let w = wanted.to_lowercase();
        let found = self.accounts.iter().position(|a| a.id.to_lowercase() == w || a.name.to_lowercase() == w || a.address.to_lowercase() == w);
        match found {
            None => Err(format!("no account matches {wanted}")),
            Some(i) if !self.allowed(prefs, i) => Err("not allowed".into()),
            Some(i) => Ok(i),
        }
    }

    pub(super) fn message(&self, prefs: &AgentPrefs, text: &str) -> Res<Message> {
        let id = parse_id(text)?;
        let m = self.store().get(id).map_err(|_| format!("no message {text}"))?;
        if !self.allowed(prefs, m.summary.account) {
            return Err("not allowed".into());
        }
        Ok(m)
    }

    fn ready(&self, id: MessageId, first: Message) -> Res<Message> {
        let mut message = first;
        until(BODY_WAIT, || {
            message = self.store().get(id).unwrap_or_else(|_| message.clone());
            message.body_state != crate::mailstore::BodyState::Loading
        });
        match message.body_state {
            crate::mailstore::BodyState::Ready => Ok(message),
            _ => Err("the message body could not be fetched".into()),
        }
    }

    fn account_id(&self, index: usize) -> String {
        self.accounts.get(index).map(|a| a.id.clone()).unwrap_or_default()
    }

    fn summary_json(&self, s: &Summary, folder: &str, snippet: &str) -> Value {
        json!({
            "id": id_text(s.id),
            "from": sanitize::display(&s.from),
            "subject": sanitize::display(&s.subject),
            "when": s.when,
            "date": iso_date(s.date),
            "unread": s.unread,
            "flagged": s.flagged,
            "has_attachments": s.attachments,
            "account": self.account_id(s.account),
            "folder": folder,
            "thread_size": s.thread,
            "snippet": snippet,
        })
    }

    fn snippet_of(&self, store: &dyn MailStore, id: MessageId) -> String {
        match store.get(id) {
            Ok(m) if m.body_state == crate::mailstore::BodyState::Ready && m.folder != Folder::Junk => sanitize::capped(&crate::thread::snippet(&m.paragraphs), SNIPPET_CHARS),
            _ => String::new(),
        }
    }

    fn list_accounts(&self, prefs: &AgentPrefs) -> Value {
        let store = self.store();
        let allowed: Vec<(usize, &AccountRef)> = self.accounts.iter().enumerate().filter(|(_, a)| prefs.allows(&a.id)).collect();
        let items: Vec<Value> = allowed
            .iter()
            .map(|(i, a)| {
                let tree = store.tree().into_iter().filter(|t| t.account == *i && t.selectable).count();
                let folders = if tree > 0 { tree } else { store.account_folders(*i).len() };
                json!({"id": a.id, "name": a.name, "address": a.address, "kind": if a.nostr { "nostr" } else { "mail" }, "can_send": prefs.may_send(&a.id, a.nostr), "folders": folders})
            })
            .collect();
        drop(store);
        let lines: Vec<String> = items
            .iter()
            .map(|a| {
                format!(
                    "{} | {} | {} | {} folders | {}",
                    sanitize::cell(a["id"].as_str().unwrap_or_default(), 80),
                    sanitize::cell(a["name"].as_str().unwrap_or_default(), 80),
                    sanitize::cell(a["address"].as_str().unwrap_or_default(), 120),
                    a["folders"],
                    if a["can_send"] == json!(true) { "send allowed" } else { "send not allowed" }
                )
            })
            .collect();
        let summary = format!("{} accounts agents may use", items.len());
        let text = with_items(&summary, &lines, "\n", |shown, left| format!("... {left} more accounts omitted after {shown}"));
        text_result(&text, json!({"accounts": items}))
    }

    fn list_folders(&self, prefs: &AgentPrefs, args: &Value) -> Res<Value> {
        let index = self.resolve_account(prefs, str_arg(args, "account").ok_or("account is required")?)?;
        let store = self.store();
        let tree: Vec<_> = store.tree().into_iter().filter(|t| t.account == index && t.selectable).collect();
        let folders: Vec<Value> = if tree.is_empty() {
            store.account_folders(index).iter().map(|f| json!({"name": f.folder.name(), "unread": f.unread, "total": f.total})).collect()
        } else {
            tree.iter().map(|t| json!({"name": t.folder.name(), "path": t.path, "unread": t.unread, "total": t.total})).collect()
        };
        let roles: Vec<bool> = if tree.is_empty() { store.account_folders(index).iter().map(|f| !matches!(f.folder, Folder::Other(..))).collect() } else { tree.iter().map(|t| !matches!(t.folder, Folder::Other(..))).collect() };
        drop(store);
        let lines: Vec<String> = folders
            .iter()
            .zip(roles)
            .map(|(f, has_role)| {
                let name = f["name"].as_str().unwrap_or_default();
                let path = f["path"].as_str().filter(|p| *p != name).map(|p| format!(" | path {}", sanitize::cell(p, 200))).unwrap_or_default();
                format!("{} | {} | {} unread | {} total{path}", sanitize::cell(name, 200), if has_role { sanitize::cell(name, 40) } else { "-".into() }, f["unread"], f["total"])
            })
            .collect();
        let summary = format!("{} folders in {}", folders.len(), self.accounts[index].name);
        let text = with_items(&summary, &lines, "\n", |shown, left| format!("... {left} more folders omitted after {shown}"));
        Ok(text_result(&text, json!({"account": self.account_id(index), "folders": folders})))
    }

    fn search_folders(&self, folder: Option<&str>) -> Vec<(Folder, String, String)> {
        match folder {
            None => [Folder::Inbox, Folder::Archive, Folder::Sent, Folder::Drafts].into_iter().map(|f| (f.clone(), f.name().to_string(), String::new())).collect(),
            Some(name) => match folder_named(name) {
                Some(f) => vec![(f.clone(), f.name().to_string(), String::new())],
                None => vec![(Folder::Inbox, name.to_lowercase(), format!(" in:{}", name.replace(['"', ' '], "")))],
            },
        }
    }

    fn search(&self, prefs: &AgentPrefs, args: &Value) -> Res<Value> {
        let mut text = str_arg(args, "query").unwrap_or_default().to_string();
        if let Some(from) = str_arg(args, "from") {
            text.push_str(&format!(" from:\"{}\"", from.replace('"', "")));
        }
        if let Some(since) = str_arg(args, "since") {
            if crate::search::parse_date(since).is_none() {
                return Err("since must be a date like 2025-01-31".into());
            }
            text.push_str(&format!(" after:{since}"));
        }
        let account = match str_arg(args, "account") {
            Some(a) => Some(self.resolve_account(prefs, a)?),
            None => None,
        };
        let filter = if bool_arg(args, "unread") { Filter::Unread } else { Filter::All };
        let limit = args.get("limit").and_then(Value::as_u64).map(|n| (n as usize).clamp(1, MAX_LIMIT)).unwrap_or(DEFAULT_LIMIT);
        let offset = match str_arg(args, "cursor") {
            None => 0,
            Some(c) => c.strip_prefix("o").and_then(|n| n.parse::<usize>().ok()).ok_or("bad cursor")?,
        };
        let store = self.store();
        let mut seen = std::collections::HashSet::new();
        let mut all: Vec<Summary> = Vec::new();
        let mut labels: Vec<String> = Vec::new();
        for (folder, label, extra) in self.search_folders(str_arg(args, "folder")) {
            let q = Query { folder, filter, text: format!("{text}{extra}").trim().to_string(), account };
            store.list(&q, 0, 1000);
            until(SEARCH_WAIT, || !store.searching(&q));
            for s in store.list(&q, 0, 1000).items {
                if self.allowed(prefs, s.account) && seen.insert(s.id) {
                    all.push(s);
                    labels.push(label.clone());
                }
            }
        }
        let wanted: Vec<MessageId> = all.iter().skip(offset).take(limit).map(|s| s.id).collect();
        until(SNIPPET_WAIT, || wanted.iter().all(|id| store.get(*id).map(|m| m.body_state != crate::mailstore::BodyState::Loading).unwrap_or(true)));
        let total = all.len();
        let page: Vec<Value> = all
            .iter()
            .zip(&labels)
            .skip(offset)
            .take(limit)
            .map(|(s, label)| self.summary_json(s, label, &self.snippet_of(&**store, s.id)))
            .collect();
        let next = (offset + page.len() < total).then(|| format!("o{}", offset + page.len()));
        let summary = format!("{} of {total} messages", page.len());
        let lines: Vec<String> = page.iter().map(listing_line).collect();
        let text = with_items(&summary, &lines, "\n", |shown, left| format!("... {left} more items omitted, pass cursor o{} to continue", offset + shown));
        Ok(text_result(&text, json!({"messages": page, "total": total, "nextCursor": next})))
    }

    fn attachments_json(&self, m: &Message) -> Vec<Value> {
        m.attachments
            .iter()
            .enumerate()
            .map(|(i, a)| json!({"id": i.to_string(), "name": sanitize::display(&a.name), "size": a.size, "mime": a.mime}))
            .collect()
    }

    fn render(&self, m: &Message, id: MessageId, include_html: bool, include_junk: bool, nonce: &str) -> Value {
        let from = format!("{} <{}>", m.summary.from, m.from_address);
        let mut out = json!({
            "id": id_text(id),
            "account": self.account_id(m.summary.account),
            "folder": m.folder.name(),
            "from": sanitize::display(&m.summary.from),
            "from_address": sanitize::display(&m.from_address),
            "to": m.to.iter().map(|t| sanitize::display(t)).collect::<Vec<_>>(),
            "cc": m.cc.iter().map(|t| sanitize::display(t)).collect::<Vec<_>>(),
            "subject": sanitize::display(&m.summary.subject),
            "sent": m.sent,
            "unread": m.summary.unread,
            "flagged": m.summary.flagged,
            "message_id": m.message_id,
            "attachments": self.attachments_json(m),
        });
        let withheld = m.folder == Folder::Junk && !include_junk;
        if withheld {
            out["text"] = json!(null);
            out["withheld"] = json!("This message is in Junk. Ask again with include_junk true to read it.");
            return out;
        }
        out["text"] = json!(sanitize::block(&from, &m.paragraphs.join("\n\n"), nonce));
        if include_html {
            out["html"] = json!(sanitize::block(&from, &m.html.html, nonce));
        }
        out
    }

    fn message_text(v: &Value) -> String {
        let attachments = v["attachments"].as_array().filter(|a| !a.is_empty()).map(|list| {
            let lines: Vec<String> = list.iter().map(|a| format!("{} | {} | {} | {}", a["id"].as_str().unwrap_or_default(), a["name"].as_str().unwrap_or_default(), a["size"].as_str().unwrap_or_default(), a["mime"].as_str().unwrap_or_default())).collect();
            format!("\n\nAttachments (id | name | size | type):\n{}", lines.join("\n"))
        });
        let head = format!(
            "Id: {}\nFrom: {} <{}>\nTo: {}\nCc: {}\nSubject: {}\nDate: {}\nFolder: {}\nStatus: {}\n",
            v["id"].as_str().unwrap_or_default(),
            v["from"].as_str().unwrap_or_default(),
            v["from_address"].as_str().unwrap_or_default(),
            v["to"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
            v["cc"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
            v["subject"].as_str().unwrap_or_default(),
            v["sent"].as_str().unwrap_or_default(),
            v["folder"].as_str().unwrap_or_default(),
            flag_words(&v["unread"], &v["flagged"], &json!(!v["attachments"].as_array().map(Vec::is_empty).unwrap_or(true))),
        );
        let body = match (v["text"].as_str(), v["withheld"].as_str()) {
            (Some(t), _) => format!("{head}\n{t}"),
            (None, Some(w)) => format!("{head}\n{w}"),
            _ => head,
        };
        format!("{body}{}", attachments.unwrap_or_default())
    }

    pub(super) fn read_message(&self, prefs: &AgentPrefs, args: &Value) -> Res<Value> {
        let m = self.message(prefs, str_arg(args, "id").ok_or("id is required")?)?;
        let id = m.summary.id;
        let include_junk = bool_arg(args, "include_junk");
        let m = if m.folder == Folder::Junk && !include_junk { m } else { self.ready(id, m)? };
        let v = self.render(&m, id, bool_arg(args, "include_html"), include_junk, &sanitize::nonce());
        let text = format!("Message {} in {}\n\n{}", id_text(id), m.folder.name(), Self::message_text(&v));
        Ok(text_result(&text, v))
    }

    pub(super) fn read_thread(&self, prefs: &AgentPrefs, args: &Value) -> Res<Value> {
        let first = self.message(prefs, str_arg(args, "id").ok_or("id is required")?)?;
        let include_junk = bool_arg(args, "include_junk");
        let items = self.store().thread(first.summary.id);
        let nonce = sanitize::nonce();
        let mut messages = Vec::new();
        let mut parts: Vec<String> = Vec::new();
        for item in items.iter().take(THREAD_MAX) {
            if !self.allowed(prefs, item.summary.account) {
                continue;
            }
            let id = item.summary.id;
            let Ok(m) = self.store().get(id) else { continue };
            let m = if m.folder == Folder::Junk && !include_junk { m } else { self.ready(id, m.clone()).unwrap_or(m) };
            let v = self.render(&m, id, false, include_junk, &nonce);
            parts.push(Self::message_text(&v));
            messages.push(v);
        }
        let truncated = items.len() > THREAD_MAX;
        let summary = format!("{} messages in the thread of {}", messages.len(), id_text(first.summary.id));
        let text = with_items(&summary, &parts, "\n\n", |shown, left| format!("... {left} more messages omitted after {shown}, fetch them with get_message"));
        Ok(text_result(&text, json!({"messages": messages, "truncated": truncated})))
    }

    fn attachment(&self, prefs: &AgentPrefs, args: &Value) -> Res<Value> {
        let m = self.message(prefs, str_arg(args, "id").ok_or("id is required")?)?;
        if m.folder == Folder::Junk {
            return Err("attachments of Junk messages are not returned".into());
        }
        let which: usize = str_arg(args, "attachment_id").and_then(|s| s.parse().ok()).ok_or("attachment_id is required")?;
        let all = self.store().attachments(m.summary.id).map_err(|_| "the attachments could not be read".to_string())?;
        let a = all.get(which).ok_or("no such attachment")?;
        if a.data.len() > MAX_ATTACHMENT {
            return Err(format!("attachment is {} bytes; only attachments up to {MAX_ATTACHMENT} bytes are returned", a.data.len()));
        }
        let mime = a.mime.to_lowercase();
        let name = sanitize::display(&a.name);
        if mime.starts_with("image/") && mime != "image/svg+xml" {
            let data = base64::engine::general_purpose::STANDARD.encode(a.data.as_slice());
            let mut r = text_result(&format!("image attachment {name} (attachment {which} of message {}, {} bytes, {mime})", id_text(m.summary.id), a.data.len()), json!({"id": id_text(m.summary.id), "attachment_id": which.to_string(), "name": name, "mime": mime, "size": a.data.len(), "encoding": "base64", "data": data}));
            r["content"].as_array_mut().map(|c| c.push(json!({"type": "image", "data": data, "mimeType": mime})));
            return Ok(r);
        }
        let texty = mime.starts_with("text/") || matches!(mime.as_str(), "application/json" | "application/xml" | "application/x-yaml" | "application/toml") || mime == "image/svg+xml";
        if !texty {
            return Err(format!("{mime} attachments are not returned; only text and images up to {MAX_ATTACHMENT} bytes"));
        }
        let content = String::from_utf8(a.data.to_vec()).map_err(|_| "the attachment is not valid UTF-8 text".to_string())?;
        let from = format!("{} <{}>", m.summary.from, m.from_address);
        let wrapped = sanitize::block(&from, &content, &sanitize::nonce());
        let text = format!("Attachment {which} of message {}: {name} ({mime}, {} bytes)\n\n{wrapped}", id_text(m.summary.id), a.data.len());
        Ok(text_result(&text, json!({"id": id_text(m.summary.id), "attachment_id": which.to_string(), "name": name, "mime": mime, "size": a.data.len(), "encoding": "text", "text": wrapped})))
    }

    fn check_all(&self, prefs: &AgentPrefs, ids: &[MessageId]) -> Res<Vec<Message>> {
        let mut out = Vec::new();
        for id in ids {
            out.push(self.message(prefs, &id_text(*id))?);
        }
        Ok(out)
    }

    fn change_flags(&self, prefs: &AgentPrefs, args: &Value, key: &str) -> Res<Value> {
        let ids = ids_arg(args)?;
        let value = args.get(key).and_then(Value::as_bool).ok_or_else(|| format!("{key} must be true or false"))?;
        self.check_all(prefs, &ids)?;
        let change = if key == "read" { FlagChange { unread: Some(!value), flagged: None } } else { FlagChange { unread: None, flagged: Some(value) } };
        self.store().bulk_flags(&ids, change).map_err(store_error)?;
        let list = ids.iter().map(|i| id_text(*i)).collect::<Vec<_>>();
        let what = match (key, value) {
            ("read", true) => "marked read",
            ("read", false) => "marked unread",
            (_, true) => "flagged",
            (_, false) => "unflagged",
        };
        let text = format!("{what} {} messages\n\n{}", ids.len(), list.join("\n"));
        Ok(text_result(&text, json!({"updated": list, key: value})))
    }

    fn move_messages(&self, prefs: &AgentPrefs, args: &Value) -> Res<Value> {
        let ids = ids_arg(args)?;
        let name = str_arg(args, "folder").ok_or("folder is required")?;
        let messages = self.check_all(prefs, &ids)?;
        let mut targets = Vec::new();
        for m in &messages {
            targets.push(self.move_target(m.summary.account, name)?);
        }
        let mut store = self.store();
        for (m, target) in messages.iter().zip(targets) {
            store.move_to(m.summary.id, target).map_err(store_error)?;
        }
        let list = ids.iter().map(|i| id_text(*i)).collect::<Vec<_>>();
        let text = format!("moved {} messages to {name}\n\n{}", ids.len(), list.join("\n"));
        Ok(text_result(&text, json!({"moved": list, "folder": name})))
    }

    fn move_target(&self, account: usize, name: &str) -> Res<Folder> {
        match folder_named(name) {
            Some(Folder::Inbox) => Ok(Folder::Inbox),
            Some(Folder::Archive) => Ok(Folder::Archive),
            Some(Folder::Trash) => Ok(Folder::Trash),
            Some(Folder::Junk) => Ok(Folder::Junk),
            Some(_) => Err(format!("messages cannot be moved into {name}")),
            None => {
                let lower = name.to_lowercase();
                self.store()
                    .tree()
                    .into_iter()
                    .find(|t| t.account == account && t.selectable && (t.path.to_lowercase() == lower || t.leaf.to_lowercase() == lower))
                    .map(|t| t.folder)
                    .ok_or_else(|| format!("no folder named {name}"))
            }
        }
    }

    fn uri_id(uri: &str, kind: &str) -> Option<String> {
        uri.strip_prefix(&format!("mailbench://{kind}/")).map(str::to_string).filter(|s| !s.is_empty())
    }
}

pub(super) fn store_error(e: StoreError) -> String {
    match e {
        StoreError::NotFound => "not found".into(),
        StoreError::Backend(m) => m,
    }
}

pub fn read_tool_definitions() -> Vec<Value> {
    let mut defs = super::tools::read_definitions();
    defs.push(definition(
        "mark_read",
        "Mark messages read or unread.",
        json!({"ids": {"type": "array", "items": {"type": "string"}, "maxItems": MAX_IDS}, "read": {"type": "boolean"}}),
        &["ids", "read"],
    ));
    defs.push(definition(
        "flag",
        "Flag or unflag messages.",
        json!({"ids": {"type": "array", "items": {"type": "string"}, "maxItems": MAX_IDS}, "flagged": {"type": "boolean"}}),
        &["ids", "flagged"],
    ));
    defs.push(definition(
        "move_messages",
        "Move messages to inbox, archive, trash, junk or one of the account's own folders. Permanent deletion is not possible.",
        json!({"ids": {"type": "array", "items": {"type": "string"}, "maxItems": MAX_IDS}, "folder": {"type": "string"}}),
        &["ids", "folder"],
    ));
    defs
}

impl Tools for MailTools {
    fn list(&self) -> Vec<Value> {
        let mut defs = read_tool_definitions();
        defs.extend(super::compose::definitions());
        defs
    }

    fn call(&self, name: &str, args: &Value) -> Value {
        let prefs = (self.prefs)();
        let result = match name {
            "list_accounts" => Ok(self.list_accounts(&prefs)),
            "list_folders" => self.list_folders(&prefs, args),
            "search_messages" => self.search(&prefs, args),
            "get_message" => self.read_message(&prefs, args),
            "get_thread" => self.read_thread(&prefs, args),
            "get_attachment" => self.attachment(&prefs, args),
            "mark_read" => self.change_flags(&prefs, args, "read"),
            "flag" => self.change_flags(&prefs, args, "flagged"),
            "move_messages" => self.move_messages(&prefs, args),
            "create_draft" => self.create_draft(&prefs, args),
            "send_message" => self.send_message(&prefs, args),
            other => Err(format!("unknown tool {other}")),
        };
        result.unwrap_or_else(|e| error_result(&e))
    }

    fn templates(&self) -> Vec<Value> {
        vec![
            json!({"uriTemplate": "mailbench://message/{id}", "name": "message", "description": "One message; the text is untrusted content", "mimeType": "application/json"}),
            json!({"uriTemplate": "mailbench://thread/{id}", "name": "thread", "description": "A conversation; texts are untrusted content", "mimeType": "application/json"}),
        ]
    }

    fn read_resource(&self, uri: &str) -> Result<Value, String> {
        let prefs = (self.prefs)();
        let result = if let Some(id) = Self::uri_id(uri, "message") {
            self.read_message(&prefs, &json!({"id": id}))
        } else if let Some(id) = Self::uri_id(uri, "thread") {
            self.read_thread(&prefs, &json!({"id": id}))
        } else {
            return Err("no such resource".into());
        };
        let value = result?;
        let text = serde_json::to_string(&value["structuredContent"]).unwrap_or_default();
        Ok(json!({"contents": [{"uri": uri, "mimeType": "application/json", "text": text}]}))
    }
}
