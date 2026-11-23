use super::error::{BackendError, Result};
use super::smtp::render_message;
use super::types::*;
use super::{Backend, Watch};
use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::sync::Arc;
use std::time::Duration;

const CORE: &str = "urn:ietf:params:jmap:core";
const MAIL: &str = "urn:ietf:params:jmap:mail";
const SUBMISSION: &str = "urn:ietf:params:jmap:submission";
const PAGE: usize = 500;
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct JmapConfig {
    pub url: String,
    pub auth: Auth,
    pub timeout: Duration,
    pub event_ping: Duration,
}

impl JmapConfig {
    pub fn new(url: &str, auth: Auth) -> Self {
        Self { url: url.to_string(), auth, timeout: Duration::from_secs(60), event_ping: Duration::from_secs(30) }
    }

    fn session_url(&self) -> String {
        let url = self.url.trim().trim_end_matches('/');
        let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
        if after_scheme.contains('/') { url.to_string() } else { format!("{url}/.well-known/jmap") }
    }
}

#[derive(Clone)]
struct Http {
    agent: ureq::Agent,
    authorization: String,
}

fn http_error(e: ureq::Error) -> BackendError {
    match e {
        ureq::Error::Status(401 | 403, r) => BackendError::Auth(format!("http {}", r.status())),
        ureq::Error::Status(code, r) => BackendError::Protocol(format!("http {code}: {}", r.into_string().unwrap_or_default())),
        ureq::Error::Transport(t) => BackendError::Connection(t.to_string()),
    }
}

impl Http {
    fn new(auth: &Auth, timeout: Duration) -> Self {
        let authorization = match auth {
            Auth::Password { user, password } => {
                format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}")))
            }
            Auth::XOAuth2 { access_token, .. } => format!("Bearer {access_token}"),
        };
        let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).redirects(0).timeout(timeout).build();
        Self { agent, authorization }
    }

    fn get_json(&self, url: &str) -> Result<Value> {
        let mut url = url.to_string();
        for _ in 0..5 {
            let resp = self.agent.get(&url).set("Authorization", &self.authorization).call().map_err(http_error)?;
            if (300..400).contains(&resp.status()) {
                let target = resp.header("location").ok_or_else(|| BackendError::Protocol("redirect without location".into()))?;
                let base = url::Url::parse(&url).map_err(|e| BackendError::Protocol(e.to_string()))?;
                url = base.join(target).map_err(|e| BackendError::Protocol(e.to_string()))?.to_string();
                continue;
            }
            return serde_json::from_str(&resp.into_string()?).map_err(|e| BackendError::Protocol(e.to_string()));
        }
        Err(BackendError::Protocol("too many redirects".into()))
    }

    fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let resp = self.agent.get(url).set("Authorization", &self.authorization).call().map_err(http_error)?;
        let mut out = Vec::new();
        resp.into_reader().take(MAX_DOWNLOAD).read_to_end(&mut out)?;
        Ok(out)
    }

    fn post_json(&self, url: &str, body: &Value) -> Result<Value> {
        let resp = self
            .agent
            .post(url)
            .set("Authorization", &self.authorization)
            .set("Content-Type", "application/json")
            .send_string(&body.to_string())
            .map_err(http_error)?;
        serde_json::from_str(&resp.into_string()?).map_err(|e| BackendError::Protocol(e.to_string()))
    }

    fn post_bytes(&self, url: &str, content_type: &str, body: &[u8]) -> Result<Value> {
        let resp = self
            .agent
            .post(url)
            .set("Authorization", &self.authorization)
            .set("Content-Type", content_type)
            .send_bytes(body)
            .map_err(http_error)?;
        serde_json::from_str(&resp.into_string()?).map_err(|e| BackendError::Protocol(e.to_string()))
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| BackendError::Connection(e.to_string()))?
}

struct Session {
    api_url: String,
    download_url: String,
    upload_url: String,
    event_url: Option<String>,
    account_id: String,
    submission: bool,
}

impl Session {
    fn parse(v: &Value) -> Result<Self> {
        let text = |key: &str| v[key].as_str().map(str::to_string);
        let missing = |key: &str| BackendError::Protocol(format!("session has no {key}"));
        let account_id = v["primaryAccounts"][MAIL].as_str().ok_or_else(|| BackendError::Protocol("no mail account in session".into()))?.to_string();
        Ok(Self {
            api_url: text("apiUrl").ok_or_else(|| missing("apiUrl"))?,
            download_url: text("downloadUrl").ok_or_else(|| missing("downloadUrl"))?,
            upload_url: text("uploadUrl").ok_or_else(|| missing("uploadUrl"))?,
            event_url: text("eventSourceUrl"),
            account_id,
            submission: v["capabilities"].get(SUBMISSION).is_some(),
        })
    }
}

#[derive(Clone, Debug)]
struct MailboxInfo {
    id: String,
    path: String,
    role: Option<FolderRole>,
}

#[derive(Default)]
struct Cache {
    state: Option<String>,
    emails: HashMap<String, (Vec<String>, Flags)>,
    uid_by_id: HashMap<String, u32>,
    id_by_uid: HashMap<u32, String>,
    touched: HashSet<String>,
}

fn fnv(id: &str) -> u32 {
    let mut h = 0x811c9dc5u32;
    for b in id.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h.max(1)
}

impl Cache {
    fn assign(&mut self, id: &str) -> u32 {
        if let Some(uid) = self.uid_by_id.get(id) {
            return *uid;
        }
        let mut uid = fnv(id);
        while self.id_by_uid.contains_key(&uid) {
            uid = uid.checked_add(1).unwrap_or(1);
        }
        self.uid_by_id.insert(id.to_string(), uid);
        self.id_by_uid.insert(uid, id.to_string());
        uid
    }

    fn put(&mut self, id: &str, mailboxes: Vec<String>, flags: Flags) {
        self.assign(id);
        self.emails.insert(id.to_string(), (mailboxes, flags));
        self.touched.insert(id.to_string());
    }

    fn remove(&mut self, id: &str) {
        self.emails.remove(id);
        self.touched.remove(id);
        if let Some(uid) = self.uid_by_id.remove(id) {
            self.id_by_uid.remove(&uid);
        }
    }

    fn clear(&mut self) {
        *self = Cache::default();
    }

    fn uids_in(&self, mailbox: &str) -> Vec<u32> {
        let mut uids: Vec<u32> = self
            .emails
            .iter()
            .filter(|(_, (boxes, _))| boxes.iter().any(|b| b == mailbox))
            .filter_map(|(id, _)| self.uid_by_id.get(id).copied())
            .collect();
        uids.sort_unstable();
        uids
    }

    fn flags_of(&self, uid: u32) -> Option<&Flags> {
        self.emails.get(self.id_by_uid.get(&uid)?).map(|(_, f)| f)
    }
}

fn flags_of_keywords(v: &Value) -> Flags {
    let mut flags = Flags::default();
    let Some(map) = v.as_object() else {
        return flags;
    };
    for (key, on) in map {
        if on.as_bool() != Some(true) {
            continue;
        }
        match key.as_str() {
            "$seen" => flags.seen = true,
            "$answered" => flags.answered = true,
            "$flagged" => flags.flagged = true,
            "$draft" => flags.draft = true,
            other => flags.keywords.push(other.to_string()),
        }
    }
    flags.keywords.sort();
    flags
}

fn keyword_patch(flags: &Flags, on: bool) -> Vec<(String, Value)> {
    let value = if on { json!(true) } else { Value::Null };
    let mut out = Vec::new();
    for (set, key) in [(flags.seen, "$seen"), (flags.answered, "$answered"), (flags.flagged, "$flagged"), (flags.draft, "$draft")] {
        if set {
            out.push((format!("keywords/{key}"), value.clone()));
        }
    }
    for k in &flags.keywords {
        out.push((format!("keywords/{k}"), value.clone()));
    }
    out
}

fn keyword_map(flags: &Flags) -> Value {
    let mut map = Map::new();
    for (key, value) in keyword_patch(flags, true) {
        map.insert(key.trim_start_matches("keywords/").to_string(), value);
    }
    Value::Object(map)
}

fn addresses(v: &Value) -> Vec<Address> {
    v.as_array()
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    Some(Address {
                        name: a["name"].as_str().filter(|n| !n.is_empty()).map(str::to_string),
                        address: a["email"].as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|l| l.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn timestamp(v: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(v.as_str()?).ok().map(|d| d.timestamp())
}

fn role_of(v: &Value) -> Option<FolderRole> {
    Some(match v.as_str()? {
        "inbox" => FolderRole::Inbox,
        "sent" => FolderRole::Sent,
        "drafts" => FolderRole::Drafts,
        "trash" => FolderRole::Trash,
        "junk" => FolderRole::Junk,
        "archive" => FolderRole::Archive,
        "all" => FolderRole::All,
        "flagged" => FolderRole::Flagged,
        _ => return None,
    })
}

const SUMMARY_PROPERTIES: [&str; 11] =
    ["keywords", "size", "receivedAt", "sentAt", "subject", "from", "to", "cc", "messageId", "inReplyTo", "references"];

pub struct JmapBackend {
    cfg: JmapConfig,
    http: Http,
    session: Option<Arc<Session>>,
    mailboxes: Vec<MailboxInfo>,
    cache: Cache,
    identity: Option<(String, String)>,
}

impl JmapBackend {
    pub fn new(cfg: JmapConfig) -> Self {
        let http = Http::new(&cfg.auth, cfg.timeout);
        Self { cfg, http, session: None, mailboxes: Vec::new(), cache: Cache::default(), identity: None }
    }

    async fn session(&mut self) -> Result<Arc<Session>> {
        if let Some(s) = &self.session {
            return Ok(s.clone());
        }
        let http = self.http.clone();
        let url = self.cfg.session_url();
        let raw = blocking(move || http.get_json(&url)).await?;
        let session = Arc::new(Session::parse(&raw)?);
        self.session = Some(session.clone());
        Ok(session)
    }

    async fn request(&mut self, calls: Vec<(&str, Value)>) -> Result<Vec<Value>> {
        let session = self.session().await?;
        let mut using = vec![CORE, MAIL];
        if session.submission {
            using.push(SUBMISSION);
        }
        let method_calls: Vec<Value> = calls.iter().enumerate().map(|(i, (name, args))| json!([name, args, format!("c{i}")])).collect();
        let body = json!({ "using": using, "methodCalls": method_calls });
        let http = self.http.clone();
        let url = session.api_url.clone();
        let reply = blocking(move || http.post_json(&url, &body)).await.inspect_err(|e| {
            if matches!(e, BackendError::Auth(_)) {
                self.session = None;
            }
        })?;
        let responses = reply["methodResponses"].as_array().ok_or_else(|| BackendError::Protocol("no methodResponses".into()))?;
        let mut out = vec![Value::Null; calls.len()];
        for r in responses {
            let (name, args, id) = (r[0].as_str().unwrap_or(""), &r[1], r[2].as_str().unwrap_or(""));
            if name == "error" {
                let kind = args["type"].as_str().unwrap_or("unknown");
                let detail = args["description"].as_str().unwrap_or("");
                return Err(BackendError::Protocol(format!("{kind}: {detail}")));
            }
            if let Some(i) = id.strip_prefix('c').and_then(|n| n.parse::<usize>().ok()).filter(|i| *i < out.len()) {
                out[i] = args.clone();
            }
        }
        Ok(out)
    }

    async fn account(&mut self) -> Result<String> {
        Ok(self.session().await?.account_id.clone())
    }

    async fn load_mailboxes(&mut self) -> Result<()> {
        let account = self.account().await?;
        let r = self.request(vec![("Mailbox/get", json!({"accountId": account, "ids": null, "properties": ["id", "name", "parentId", "role"]}))]).await?;
        let list = r[0]["list"].as_array().cloned().unwrap_or_default();
        let by_id: HashMap<&str, &Value> = list.iter().filter_map(|m| Some((m["id"].as_str()?, m))).collect();
        let path_of = |m: &Value| {
            let mut m = m;
            let mut parts = vec![m["name"].as_str().unwrap_or("").to_string()];
            while let Some(parent) = m["parentId"].as_str().and_then(|p| by_id.get(p)) {
                parts.push(parent["name"].as_str().unwrap_or("").to_string());
                m = parent;
            }
            parts.reverse();
            parts.join("/")
        };
        self.mailboxes = list
            .iter()
            .filter_map(|m| Some(MailboxInfo { id: m["id"].as_str()?.to_string(), path: path_of(m), role: role_of(&m["role"]) }))
            .collect();
        Ok(())
    }

    async fn mailbox(&mut self, folder: &str) -> Result<MailboxInfo> {
        if !self.mailboxes.iter().any(|m| m.path == folder) {
            self.load_mailboxes().await?;
        }
        self.mailboxes.iter().find(|m| m.path == folder).cloned().ok_or_else(|| BackendError::Protocol(format!("no such folder: {folder}")))
    }

    async fn mailbox_with_role(&mut self, role: FolderRole) -> Result<Option<String>> {
        if self.mailboxes.is_empty() {
            self.load_mailboxes().await?;
        }
        Ok(self.mailboxes.iter().find(|m| m.role == Some(role)).map(|m| m.id.clone()))
    }

    async fn get_chunked(&mut self, ids: &[String], properties: &[&str]) -> Result<(Vec<Value>, Vec<String>)> {
        let account = self.account().await?;
        let (mut list, mut missing) = (Vec::new(), Vec::new());
        for chunk in ids.chunks(PAGE) {
            let r = self.request(vec![("Email/get", json!({"accountId": account, "ids": chunk, "properties": properties}))]).await?;
            list.extend(r[0]["list"].as_array().cloned().unwrap_or_default());
            missing.extend(strings(&r[0]["notFound"]));
        }
        Ok((list, missing))
    }

    fn cache_entry(&mut self, e: &Value) {
        let Some(id) = e["id"].as_str() else {
            return;
        };
        let boxes = e["mailboxIds"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
        self.cache.put(id, boxes, flags_of_keywords(&e["keywords"]));
    }

    async fn full_load(&mut self) -> Result<()> {
        let account = self.account().await?;
        self.cache.clear();
        let mut position = 0;
        let mut state: Option<String> = None;
        loop {
            let r = self
                .request(vec![
                    ("Email/query", json!({"accountId": account, "position": position, "limit": PAGE, "sort": [{"property": "receivedAt", "isAscending": true}]})),
                    ("Email/get", json!({"accountId": account, "#ids": {"resultOf": "c0", "name": "Email/query", "path": "/ids"}, "properties": ["mailboxIds", "keywords"]})),
                ])
                .await?;
            let n = r[0]["ids"].as_array().map_or(0, Vec::len);
            if state.is_none() {
                state = r[1]["state"].as_str().map(str::to_string);
            }
            for e in r[1]["list"].as_array().cloned().unwrap_or_default() {
                self.cache_entry(&e);
            }
            if n < PAGE {
                break;
            }
            position += n;
        }
        self.cache.state = state;
        Ok(())
    }

    async fn apply_changes(&mut self, since: String) -> Result<()> {
        let account = self.account().await?;
        let mut since = since;
        loop {
            let r = self.request(vec![("Email/changes", json!({"accountId": account, "sinceState": since, "maxChanges": PAGE}))]).await?;
            let c = &r[0];
            for id in strings(&c["destroyed"]) {
                self.cache.remove(&id);
            }
            let mut ids = strings(&c["created"]);
            ids.extend(strings(&c["updated"]));
            let (list, missing) = self.get_chunked(&ids, &["mailboxIds", "keywords"]).await?;
            for e in &list {
                self.cache_entry(e);
            }
            for id in missing {
                self.cache.remove(&id);
            }
            since = c["newState"].as_str().unwrap_or(&since).to_string();
            self.cache.state = Some(since.clone());
            if c["hasMoreChanges"].as_bool() != Some(true) {
                return Ok(());
            }
        }
    }

    async fn refresh(&mut self) -> Result<()> {
        self.cache.touched.clear();
        match self.cache.state.clone() {
            None => self.full_load().await,
            Some(state) => match self.apply_changes(state).await {
                Err(BackendError::Protocol(m)) if m.contains("cannotCalculateChanges") => self.full_load().await,
                other => other,
            },
        }
    }

    async fn id_of(&mut self, uid: u32) -> Result<String> {
        if !self.cache.id_by_uid.contains_key(&uid) {
            self.refresh().await?;
        }
        self.cache.id_by_uid.get(&uid).cloned().ok_or_else(|| BackendError::Protocol("message not found".into()))
    }

    async fn ids_of(&mut self, uids: &[u32]) -> Result<Vec<String>> {
        let mut out = Vec::with_capacity(uids.len());
        for uid in uids {
            out.push(self.id_of(*uid).await?);
        }
        Ok(out)
    }

    async fn email_set(&mut self, args: Value) -> Result<()> {
        let account = self.account().await?;
        let mut args = args;
        args["accountId"] = json!(account);
        let r = self.request(vec![("Email/set", args)]).await?;
        for key in ["notUpdated", "notDestroyed", "notCreated"] {
            if let Some(map) = r[0][key].as_object().filter(|m| !m.is_empty()) {
                return Err(BackendError::Protocol(format!("Email/set {key}: {}", Value::Object(map.clone()))));
            }
        }
        Ok(())
    }

    async fn upload(&mut self, raw: &[u8]) -> Result<String> {
        let session = self.session().await?;
        let url = session.upload_url.replace("{accountId}", &session.account_id);
        let http = self.http.clone();
        let body = raw.to_vec();
        let reply = blocking(move || http.post_bytes(&url, "message/rfc822", &body)).await?;
        reply["blobId"].as_str().map(str::to_string).ok_or_else(|| BackendError::Protocol("upload returned no blobId".into()))
    }

    async fn identity_for(&mut self, address: &str) -> Result<String> {
        if let Some((id, _)) = self.identity.as_ref().filter(|(_, email)| email.eq_ignore_ascii_case(address)) {
            return Ok(id.clone());
        }
        let account = self.account().await?;
        let r = self.request(vec![("Identity/get", json!({"accountId": account, "ids": null}))]).await?;
        let list = r[0]["list"].as_array().cloned().unwrap_or_default();
        let pick = list
            .iter()
            .find(|i| i["email"].as_str().is_some_and(|e| e.eq_ignore_ascii_case(address)))
            .or_else(|| list.iter().find(|i| i["email"].as_str().is_some_and(|e| e.starts_with('*'))))
            .or_else(|| list.first());
        let id = match pick {
            Some(p) => p["id"].as_str().ok_or_else(|| BackendError::Protocol("identity without id".into()))?.to_string(),
            None => {
                let r = self.request(vec![("Identity/set", json!({"accountId": account, "create": {"i": {"name": "", "email": address}}}))]).await?;
                r[0]["created"]["i"]["id"].as_str().map(str::to_string).ok_or_else(|| BackendError::Protocol(format!("no sending identity: {}", r[0]["notCreated"])))?
            }
        };
        self.identity = Some((id.clone(), address.to_string()));
        Ok(id)
    }

    async fn query_ids(&mut self, filter: Value) -> Result<Vec<String>> {
        let account = self.account().await?;
        let mut out = Vec::new();
        loop {
            let r = self.request(vec![("Email/query", json!({"accountId": account, "filter": filter, "position": out.len(), "limit": PAGE}))]).await?;
            let ids = strings(&r[0]["ids"]);
            let n = ids.len();
            out.extend(ids);
            if n < PAGE {
                return Ok(out);
            }
        }
    }

    async fn import(&mut self, raw: &[u8], mailbox: &str, flags: &Flags) -> Result<()> {
        let blob = self.upload(raw).await?;
        let account = self.account().await?;
        let args = json!({"accountId": account, "emails": {"i": {"blobId": blob, "mailboxIds": {mailbox: true}, "keywords": keyword_map(flags)}}});
        let r = self.request(vec![("Email/import", args)]).await?;
        if let Some(err) = r[0]["notCreated"].as_object().filter(|m| !m.is_empty()) {
            return Err(BackendError::Protocol(format!("Email/import failed: {}", Value::Object(err.clone()))));
        }
        Ok(())
    }

    async fn parent_and_leaf(&mut self, path: &str) -> Result<(Value, String)> {
        match path.rsplit_once('/') {
            Some((parent, leaf)) => Ok((json!(self.mailbox(parent).await?.id), leaf.to_string())),
            None => Ok((Value::Null, path.to_string())),
        }
    }
}

fn stream_events(http: Http, url: String, folder: String, tx: tokio::sync::mpsc::UnboundedSender<Event>, read_timeout: Duration) -> std::result::Result<(), String> {
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).timeout_read(read_timeout).build();
    let resp = agent
        .get(&url)
        .set("Authorization", &http.authorization)
        .set("Accept", "text/event-stream")
        .call()
        .map_err(|e| http_error(e).to_string())?;
    let mut reader = BufReader::new(resp.into_reader());
    let mut event = String::new();
    let mut last: Option<String> = None;
    let mut line = String::new();
    loop {
        if tx.is_closed() {
            return Ok(());
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return Err("event stream closed".into()),
            Ok(_) => {}
            Err(e) => return Err(e.to_string()),
        }
        let text = line.trim_end();
        if text.is_empty() {
            event.clear();
        } else if let Some(name) = text.strip_prefix("event:") {
            event = name.trim().to_string();
        } else if let Some(data) = text.strip_prefix("data:").filter(|_| event == "state") {
            let Ok(v) = serde_json::from_str::<Value>(data.trim()) else {
                continue;
            };
            let state = v["changed"].as_object().into_iter().flat_map(|m| m.values()).find_map(|a| a["Email"].as_str()).map(str::to_string);
            if state.is_some() && state != last {
                last = state;
                if tx.send(Event::FolderChanged { folder: folder.clone() }).is_err() {
                    return Ok(());
                }
            }
        }
    }
}

#[async_trait]
impl Backend for JmapBackend {
    fn kind(&self) -> &'static str {
        "jmap"
    }

    fn fork(&self) -> Option<Box<dyn Backend>> {
        Some(Box::new(JmapBackend::new(self.cfg.clone())))
    }

    async fn capabilities(&mut self) -> Result<Capabilities> {
        let session = self.session().await?;
        Ok(Capabilities {
            idle: session.event_url.is_some(),
            move_command: true,
            send: session.submission,
            server_folders: true,
            server_search: true,
            server_flags: true,
            ..Default::default()
        })
    }

    async fn list_folders(&mut self) -> Result<Vec<Folder>> {
        self.load_mailboxes().await?;
        let mut folders: Vec<Folder> = self
            .mailboxes
            .iter()
            .map(|m| Folder { name: m.path.clone(), delimiter: Some("/".into()), role: m.role, selectable: true })
            .collect();
        folders.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(folders)
    }

    async fn create_folder(&mut self, name: &str) -> Result<()> {
        let account = self.account().await?;
        let (parent, leaf) = self.parent_and_leaf(name).await?;
        let r = self.request(vec![("Mailbox/set", json!({"accountId": account, "create": {"m": {"name": leaf, "parentId": parent}}}))]).await?;
        if let Some(err) = r[0]["notCreated"].as_object().filter(|m| !m.is_empty()) {
            return Err(BackendError::Protocol(format!("Mailbox/set failed: {}", Value::Object(err.clone()))));
        }
        self.load_mailboxes().await
    }

    async fn rename_folder(&mut self, from: &str, to: &str) -> Result<()> {
        let account = self.account().await?;
        let id = self.mailbox(from).await?.id;
        let (parent, leaf) = self.parent_and_leaf(to).await?;
        let r = self.request(vec![("Mailbox/set", json!({"accountId": account, "update": {id: {"name": leaf, "parentId": parent}}}))]).await?;
        if let Some(err) = r[0]["notUpdated"].as_object().filter(|m| !m.is_empty()) {
            return Err(BackendError::Protocol(format!("Mailbox/set failed: {}", Value::Object(err.clone()))));
        }
        self.load_mailboxes().await
    }

    async fn delete_folder(&mut self, name: &str) -> Result<()> {
        let account = self.account().await?;
        let id = self.mailbox(name).await?.id;
        let r = self.request(vec![("Mailbox/set", json!({"accountId": account, "destroy": [id], "onDestroyRemoveEmails": true}))]).await?;
        if let Some(err) = r[0]["notDestroyed"].as_object().filter(|m| !m.is_empty()) {
            return Err(BackendError::Protocol(format!("Mailbox/set failed: {}", Value::Object(err.clone()))));
        }
        self.load_mailboxes().await
    }

    async fn open_folder(&mut self, folder: &str) -> Result<FolderState> {
        let mailbox = self.mailbox(folder).await?;
        self.refresh().await?;
        let uids = self.cache.uids_in(&mailbox.id);
        let uid_next = uids.last().map_or(1, |m| m.saturating_add(1));
        Ok(FolderState { token: SyncToken { uid_validity: 1, uid_next, highest_mod_seq: None }, uids })
    }

    async fn fetch_summaries(&mut self, _folder: &str, uids: &[u32]) -> Result<Vec<MessageSummary>> {
        let ids = self.ids_of(uids).await?;
        let (list, _) = self.get_chunked(&ids, &SUMMARY_PROPERTIES).await?;
        Ok(list
            .iter()
            .filter_map(|e| {
                let uid = *self.cache.uid_by_id.get(e["id"].as_str()?)?;
                Some(MessageSummary {
                    uid,
                    flags: flags_of_keywords(&e["keywords"]),
                    subject: e["subject"].as_str().map(str::to_string),
                    from: addresses(&e["from"]),
                    to: addresses(&e["to"]),
                    cc: addresses(&e["cc"]),
                    bcc: addresses(&e["bcc"]),
                    date: timestamp(&e["sentAt"]).or_else(|| timestamp(&e["receivedAt"])),
                    message_id: strings(&e["messageId"]).into_iter().next(),
                    in_reply_to: strings(&e["inReplyTo"]),
                    references: strings(&e["references"]),
                    size: e["size"].as_u64().map(|s| s as u32),
                })
            })
            .collect())
    }

    async fn changed_flags(&mut self, folder: &str, below: u32, _since_mod_seq: Option<u64>) -> Result<Vec<(u32, Flags)>> {
        let mailbox = self.mailbox(folder).await?;
        Ok(self
            .cache
            .uids_in(&mailbox.id)
            .into_iter()
            .filter(|u| *u < below)
            .filter_map(|u| Some((u, self.cache.flags_of(u)?.clone())))
            .collect())
    }

    async fn sync_folder(&mut self, folder: &str, since: Option<&SyncToken>) -> Result<FolderDelta> {
        let mailbox = self.mailbox(folder).await?;
        let warm = self.cache.state.is_some();
        let state = self.open_folder(folder).await?;
        let fresh: Vec<u32> = if warm && since.is_some() {
            state.uids.iter().copied().filter(|u| self.cache.id_by_uid.get(u).is_some_and(|id| self.cache.touched.contains(id))).collect()
        } else {
            state.uids.clone()
        };
        let added = if fresh.is_empty() { Vec::new() } else { self.fetch_summaries(folder, &fresh).await? };
        let _ = mailbox;
        Ok(FolderDelta { token: state.token, reset: since.is_none(), added, flag_changes: Vec::new(), uids: state.uids, more: false })
    }

    async fn fetch(&mut self, _folder: &str, uid: u32) -> Result<Vec<u8>> {
        let id = self.id_of(uid).await?;
        let (list, _) = self.get_chunked(&[id], &["blobId"]).await?;
        let blob = list.first().and_then(|e| e["blobId"].as_str()).ok_or_else(|| BackendError::Protocol("message not found".into()))?.to_string();
        let session = self.session().await?;
        let url = session
            .download_url
            .replace("{accountId}", &session.account_id)
            .replace("{blobId}", &blob)
            .replace("{name}", "message.eml")
            .replace("{type}", "message%2Frfc822");
        let http = self.http.clone();
        blocking(move || http.get_bytes(&url)).await
    }

    async fn set_flags(&mut self, _folder: &str, uids: &[u32], update: &FlagUpdate) -> Result<()> {
        let ids = self.ids_of(uids).await?;
        let patch: Value = match update {
            FlagUpdate::Add(f) => Value::Object(keyword_patch(f, true).into_iter().collect()),
            FlagUpdate::Remove(f) => Value::Object(keyword_patch(f, false).into_iter().collect()),
            FlagUpdate::Set(f) => json!({"keywords": keyword_map(f)}),
        };
        let updates: Map<String, Value> = ids.into_iter().map(|id| (id, patch.clone())).collect();
        self.email_set(json!({"update": updates})).await
    }

    async fn move_messages(&mut self, folder: &str, uids: &[u32], destination: &str) -> Result<()> {
        let from = self.mailbox(folder).await?.id;
        let to = self.mailbox(destination).await?.id;
        let ids = self.ids_of(uids).await?;
        let patch = json!({format!("mailboxIds/{from}"): null, format!("mailboxIds/{to}"): true});
        let updates: Map<String, Value> = ids.into_iter().map(|id| (id, patch.clone())).collect();
        self.email_set(json!({"update": updates})).await
    }

    async fn delete_messages(&mut self, _folder: &str, uids: &[u32]) -> Result<()> {
        let ids = self.ids_of(uids).await?;
        self.email_set(json!({"destroy": ids})).await?;
        for id in &ids {
            self.cache.remove(id);
        }
        Ok(())
    }

    async fn search(&mut self, folder: &str, query: &str) -> Result<Vec<u32>> {
        let mailbox = self.mailbox(folder).await?;
        let mut filter = json!({"inMailbox": mailbox.id});
        let text = query.trim();
        if !text.is_empty() && !text.eq_ignore_ascii_case("all") {
            filter = json!({"operator": "AND", "conditions": [filter, {"text": text}]});
        }
        let ids = self.query_ids(filter).await?;
        let mut uids: Vec<u32> = ids.iter().map(|id| self.cache.assign(id)).collect();
        uids.sort_unstable();
        Ok(uids)
    }

    async fn append(&mut self, folder: &str, raw: &[u8]) -> Result<()> {
        self.append_flagged(folder, &Flags::default(), raw).await
    }

    async fn append_flagged(&mut self, folder: &str, flags: &Flags, raw: &[u8]) -> Result<()> {
        let mailbox = self.mailbox(folder).await?;
        self.import(raw, &mailbox.id, flags).await
    }

    fn sent_copy_by_server(&self) -> bool {
        true
    }

    async fn send(&mut self, message: &OutgoingMessage) -> Result<()> {
        let recipients: Vec<&Address> = message.to.iter().chain(&message.cc).chain(&message.bcc).collect();
        if recipients.is_empty() {
            return Err(BackendError::Protocol("no recipients".into()));
        }
        let raw = render_message(message, false)?;
        let identity = self.identity_for(&message.from.address).await?;
        let drafts = self.mailbox_with_role(FolderRole::Drafts).await?;
        let sent = self.mailbox_with_role(FolderRole::Sent).await?;
        let staging = drafts.clone().or_else(|| sent.clone()).ok_or_else(|| BackendError::Protocol("no Drafts or Sent mailbox".into()))?;
        let blob = self.upload(&raw).await?;
        let account = self.account().await?;
        let mut after = Map::new();
        after.insert("keywords/$draft".into(), Value::Null);
        if let (Some(d), Some(s)) = (&drafts, &sent) {
            after.insert(format!("mailboxIds/{d}"), Value::Null);
            after.insert(format!("mailboxIds/{s}"), json!(true));
        }
        let import = json!({"accountId": account, "emails": {"msg": {"blobId": blob, "mailboxIds": {staging: true}, "keywords": {"$draft": true, "$seen": true}}}});
        let submit = json!({
            "accountId": account,
            "create": {"sub": {
                "identityId": identity,
                "emailId": "#msg",
                "envelope": {
                    "mailFrom": {"email": message.from.address},
                    "rcptTo": recipients.iter().map(|a| json!({"email": a.address})).collect::<Vec<_>>(),
                },
            }},
            "onSuccessUpdateEmail": {"#sub": Value::Object(after)},
        });
        let r = self.request(vec![("Email/import", import), ("EmailSubmission/set", submit)]).await?;
        if let Some(err) = r[0]["notCreated"].as_object().filter(|m| !m.is_empty()) {
            return Err(BackendError::Protocol(format!("Email/import failed: {}", Value::Object(err.clone()))));
        }
        if let Some(err) = r[1]["notCreated"].as_object().filter(|m| !m.is_empty()) {
            return Err(BackendError::Protocol(format!("EmailSubmission/set failed: {}", Value::Object(err.clone()))));
        }
        Ok(())
    }

    async fn check(&mut self) -> Result<()> {
        self.session = None;
        self.session().await.map(|_| ())
    }

    async fn watch(&mut self, folder: &str) -> Result<Watch> {
        let session = self.session().await?;
        let template = session.event_url.clone().ok_or(BackendError::Unsupported("watch"))?;
        let ping = self.cfg.event_ping.as_secs().max(1);
        let url = template.replace("{types}", "Email").replace("{closeafter}", "no").replace("{ping}", &ping.to_string());
        let http = self.http.clone();
        let folder = folder.to_string();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let notify = tx.clone();
            let outcome = tokio::task::spawn_blocking(move || stream_events(http, url, folder, tx, Duration::from_secs(ping * 3 + 5))).await;
            let reason = match outcome {
                Ok(Ok(())) => return,
                Ok(Err(reason)) => reason,
                Err(e) => e.to_string(),
            };
            let _ = notify.send(Event::Disconnected { reason });
        });
        Ok(Watch::new(rx, task.abort_handle()))
    }
}
