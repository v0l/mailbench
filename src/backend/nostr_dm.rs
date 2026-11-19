use super::error::{BackendError, Result};
use super::types::*;
use super::Backend;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use async_trait::async_trait;
use base64::Engine;
use nostr_sdk::prelude::*;
use nostr_sdk::Event as NostrEvent;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};

pub const DOMAIN: &str = "nostr";
pub const INBOX: &str = "Inbox";
pub const SENT: &str = "Sent";
pub const ARCHIVE: &str = "Archive";
pub const DEFAULT_RELAYS: [&str; 2] = ["wss://relay.damus.io", "wss://nos.lol"];
pub const BACKFILL_CAP: usize = 500;
pub const WRAP_SKEW_SECS: u64 = 2 * 24 * 3600;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const PROFILE_TTL: Duration = Duration::from_secs(24 * 3600);
const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct NostrConfig {
    pub secret: String,
    pub relays: Vec<String>,
    pub publish_inbox: bool,
}

impl NostrConfig {
    pub fn new(secret: &str, relays: Vec<String>) -> Self {
        Self { secret: secret.trim().to_string(), relays, publish_inbox: false }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRef {
    pub url: String,
    pub mime: String,
    pub key: Vec<u8>,
    pub nonce: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dm {
    pub id: EventId,
    pub sender: PublicKey,
    pub to: Vec<PublicKey>,
    pub created_at: i64,
    pub subject: Option<String>,
    pub content: String,
    pub reply_to: Option<EventId>,
    pub file: Option<FileRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejected {
    NotGiftWrap,
    Undecryptable,
    SenderMismatch,
    WrongKind,
}

pub fn address_of(key: &PublicKey) -> String {
    format!("{}@{DOMAIN}", key.to_bech32().unwrap_or_else(|_| key.to_hex()))
}

pub fn parse_key(text: &str) -> Option<PublicKey> {
    let text = text.trim().trim_start_matches("nostr:");
    let text = text.strip_suffix(&format!("@{DOMAIN}")).unwrap_or(text);
    PublicKey::parse(text).ok()
}

pub fn thread_key(participants: &[PublicKey], subject: Option<&str>) -> String {
    let set: BTreeSet<String> = participants.iter().map(|p| p.to_hex()).collect();
    let (subject, _) = crate::real::threads::normalise(subject.unwrap_or(""));
    let mut hash = Sha256::new();
    for p in &set {
        hash.update(p.as_bytes());
        hash.update([0]);
    }
    hash.update(subject.as_bytes());
    let digest = hash.finalize();
    let short: String = digest.iter().take(10).map(|b| format!("{b:02x}")).collect();
    format!("nostr-thread-{short}@{DOMAIN}")
}

pub fn message_id_of(id: &EventId) -> String {
    format!("{}@{DOMAIN}", id.to_hex())
}

pub fn uid_of(id: &EventId) -> u32 {
    let b = id.as_bytes();
    u32::from_be_bytes([b[0], b[1], b[2], b[3]]).max(1)
}

pub fn tag_value(tags: &Tags, name: &str) -> Option<String> {
    tags.iter().find(|t| t.as_slice().first().map(String::as_str) == Some(name)).and_then(|t| t.as_slice().get(1).cloned())
}

fn file_ref(content: &str, tags: &Tags) -> Option<FileRef> {
    let key = hex_decode(&tag_value(tags, "decryption-key")?)?;
    let nonce = hex_decode(&tag_value(tags, "decryption-nonce")?)?;
    Some(FileRef {
        url: content.trim().to_string(),
        mime: tag_value(tags, "file-type").unwrap_or_else(|| "application/octet-stream".into()),
        key,
        nonce,
    })
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

pub fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn decrypt_file(file: &FileRef, cipher: &[u8]) -> Result<Vec<u8>> {
    let aes = Aes256Gcm::new_from_slice(&file.key).map_err(|e| BackendError::Protocol(format!("file key: {e}")))?;
    if file.nonce.len() != 12 {
        return Err(BackendError::Protocol("file nonce must be 12 bytes".into()));
    }
    aes.decrypt(Nonce::from_slice(&file.nonce), cipher).map_err(|_| BackendError::Protocol("file did not decrypt".into()))
}

pub fn encrypt_file(key: &[u8; 32], nonce: &[u8; 12], plain: &[u8]) -> Result<Vec<u8>> {
    let aes = Aes256Gcm::new_from_slice(key).map_err(|e| BackendError::Protocol(e.to_string()))?;
    aes.encrypt(Nonce::from_slice(nonce), plain).map_err(|_| BackendError::Protocol("file did not encrypt".into()))
}

pub async fn open_wrap(keys: &Keys, wrap: &NostrEvent) -> std::result::Result<Dm, Rejected> {
    if wrap.kind != Kind::GiftWrap {
        return Err(Rejected::NotGiftWrap);
    }
    let gift = match UnwrappedGift::from_gift_wrap(keys, wrap).await {
        Ok(g) => g,
        Err(nostr_sdk::nips::nip59::Error::SenderMismatch) => return Err(Rejected::SenderMismatch),
        Err(_) => return Err(Rejected::Undecryptable),
    };
    let mut rumor = gift.rumor;
    if rumor.pubkey != gift.sender {
        return Err(Rejected::SenderMismatch);
    }
    rumor.ensure_id();
    let id = rumor.id.ok_or(Rejected::Undecryptable)?;
    let is_file = rumor.kind == Kind::from_u16(15);
    if rumor.kind != Kind::PrivateDirectMessage && !is_file {
        return Err(Rejected::WrongKind);
    }
    let mut to: Vec<PublicKey> = rumor.tags.public_keys().copied().collect();
    to.dedup();
    let reply_to = rumor.tags.event_ids().next().copied();
    Ok(Dm {
        id,
        sender: gift.sender,
        to,
        created_at: rumor.created_at.as_secs() as i64,
        subject: tag_value(&rumor.tags, "subject").filter(|s| !s.trim().is_empty()),
        file: if is_file { file_ref(&rumor.content, &rumor.tags) } else { None },
        content: rumor.content,
        reply_to,
    })
}

impl Dm {
    pub fn participants(&self) -> Vec<PublicKey> {
        let mut all = vec![self.sender];
        all.extend(self.to.iter().copied());
        all
    }

    pub fn thread(&self) -> String {
        thread_key(&self.participants(), self.subject.as_deref())
    }

    pub fn display_subject(&self) -> String {
        if let Some(s) = &self.subject {
            return s.clone();
        }
        let first = self.content.lines().next().unwrap_or("").trim();
        if self.file.is_some() {
            return "File".into();
        }
        first.chars().take(60).collect()
    }

    pub fn references(&self) -> Vec<String> {
        let mut refs = vec![self.thread()];
        if let Some(r) = &self.reply_to {
            refs.push(message_id_of(r));
        }
        refs
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profile {
    pub name: Option<String>,
    pub picture: Option<String>,
    pub nip05: Option<String>,
}

#[derive(Default)]
pub struct Names {
    map: HashMap<PublicKey, (Profile, Instant)>,
}

impl Names {
    fn fresh(&self, key: &PublicKey) -> Option<&Profile> {
        self.map.get(key).filter(|(_, at)| at.elapsed() < PROFILE_TTL).map(|(p, _)| p)
    }

    fn get(&self, key: &PublicKey) -> Option<Option<String>> {
        self.fresh(key).map(|p| p.name.clone())
    }

    pub fn set(&mut self, key: PublicKey, profile: Profile) {
        self.map.insert(key, (profile, Instant::now()));
    }
}

pub fn nip05_matches(body: &[u8], identifier: &str, key: &PublicKey) -> bool {
    let Some((name, _)) = identifier.trim().split_once('@') else { return false };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(body) else { return false };
    json.get("names").and_then(|n| n.get(name)).and_then(|v| v.as_str()).and_then(|v| PublicKey::parse(v).ok()) == Some(*key)
}

fn summary_of(dm: &Dm, me: &PublicKey, names: &Names) -> MessageSummary {
    let name = |k: &PublicKey| Address { name: names.get(k).flatten(), address: address_of(k) };
    let to: Vec<Address> = dm.to.iter().filter(|k| **k != dm.sender || dm.to.len() == 1).map(name).collect();
    MessageSummary {
        uid: uid_of(&dm.id),
        flags: Flags { seen: dm.sender == *me, ..Default::default() },
        subject: Some(dm.display_subject()),
        from: vec![name(&dm.sender)],
        to,
        cc: Vec::new(),
        date: Some(dm.created_at),
        message_id: Some(message_id_of(&dm.id)),
        in_reply_to: dm.reply_to.iter().map(message_id_of).collect(),
        references: dm.references(),
        size: Some(dm.content.len() as u32),
    }
}

fn q_encode(text: &str) -> String {
    if text.is_ascii() && !text.contains(['\r', '\n']) {
        return text.to_string();
    }
    let flat = text.replace(['\r', '\n'], " ");
    format!("=?UTF-8?B?{}?=", base64::engine::general_purpose::STANDARD.encode(flat))
}

fn wrap_base64(data: &[u8]) -> String {
    let all = base64::engine::general_purpose::STANDARD.encode(data);
    all.as_bytes().chunks(76).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("\r\n")
}

fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub fn rfc2822(ts: i64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let days = ts.div_euclid(86400);
    let secs = ts.rem_euclid(86400);
    let (y, m, d) = civil(days);
    format!(
        "{}, {d:02} {} {y} {:02}:{:02}:{:02} +0000",
        DAYS[days.rem_euclid(7) as usize],
        MONTHS[(m - 1) as usize],
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

pub(crate) fn render(dm: &Dm, names: &Names, file: Option<(&FileRef, Vec<u8>)>) -> Vec<u8> {
    let summary = summary_of(dm, &dm.sender, names);
    let addr = |a: &Address| match &a.name {
        Some(n) => format!("{} <{}>", q_encode_phrase(n), a.address),
        None => format!("<{}>", a.address),
    };
    let mut h = String::new();
    h.push_str(&format!("From: {}\r\n", summary.from.iter().map(addr).collect::<Vec<_>>().join(", ")));
    h.push_str(&format!("To: {}\r\n", summary.to.iter().map(addr).collect::<Vec<_>>().join(", ")));
    h.push_str(&format!("Subject: {}\r\n", q_encode(&dm.display_subject())));
    h.push_str(&format!("Date: {}\r\n", rfc2822(dm.created_at)));
    h.push_str(&format!("Message-ID: <{}>\r\n", message_id_of(&dm.id)));
    if let Some(r) = &dm.reply_to {
        h.push_str(&format!("In-Reply-To: <{}>\r\n", message_id_of(r)));
    }
    h.push_str(&format!("References: {}\r\n", dm.references().iter().map(|r| format!("<{r}>")).collect::<Vec<_>>().join(" ")));
    if let Some(profile) = names.fresh(&dm.sender) {
        if let Some(pic) = profile.picture.as_deref().filter(|p| p.starts_with("https://") && !p.contains(['\r', '\n', ' '])) {
            h.push_str(&format!("X-Nostr-Picture: {pic}\r\n"));
        }
        if let Some(id) = profile.nip05.as_deref().filter(|p| !p.contains(['\r', '\n'])) {
            h.push_str(&format!("X-Nostr-Nip05: {id}\r\n"));
        }
    }
    h.push_str("MIME-Version: 1.0\r\n");
    let text = if dm.file.is_some() { String::new() } else { dm.content.clone() };
    match file {
        None => {
            h.push_str("Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n");
            h.push_str(&wrap_base64(text.as_bytes()));
            h.push_str("\r\n");
        }
        Some((f, data)) => {
            let boundary = format!("nostr-{}", &dm.id.to_hex()[..16]);
            h.push_str(&format!("Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n"));
            h.push_str(&format!("--{boundary}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n"));
            h.push_str(&wrap_base64(text.as_bytes()));
            let name = f.url.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("file").split(['?', '#']).next().unwrap_or("file");
            h.push_str(&format!(
                "\r\n--{boundary}\r\nContent-Type: {}; name=\"{name}\"\r\nContent-Disposition: attachment; filename=\"{name}\"\r\nContent-Transfer-Encoding: base64\r\n\r\n",
                f.mime
            ));
            h.push_str(&wrap_base64(&data));
            h.push_str(&format!("\r\n--{boundary}--\r\n"));
        }
    }
    h.into_bytes()
}

fn q_encode_phrase(name: &str) -> String {
    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ') {
        return name.to_string();
    }
    let encoded = q_encode(&name.replace('"', ""));
    if encoded.starts_with("=?") { encoded } else { format!("\"{encoded}\"") }
}

fn fetch_url(url: &str) -> Result<Vec<u8>> {
    let response = ureq::get(url).timeout(Duration::from_secs(30)).call().map_err(|e| BackendError::Connection(e.to_string()))?;
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(response.into_reader(), MAX_FILE_BYTES), &mut out)?;
    Ok(out)
}

pub async fn verify_nip05(identifier: &str, key: &PublicKey) -> bool {
    let Some((name, domain)) = identifier.trim().split_once('@') else { return false };
    let url = format!("https://{domain}/.well-known/nostr.json?name={name}");
    match tokio::task::spawn_blocking(move || fetch_url(&url)).await {
        Ok(Ok(body)) => nip05_matches(&body, identifier, key),
        _ => false,
    }
}

pub async fn resolve_nip05(identifier: &str) -> Option<PublicKey> {
    let (name, domain) = identifier.trim().split_once('@')?;
    let url = format!("https://{domain}/.well-known/nostr.json?name={name}");
    let body = tokio::task::spawn_blocking(move || fetch_url(&url)).await.ok()?.ok()?;
    let json: serde_json::Value = serde_json::from_slice(&body).ok()?;
    PublicKey::parse(json.get("names")?.get(name)?.as_str()?).ok()
}

pub struct NostrBackend {
    cfg: NostrConfig,
    keys: Keys,
    client: Option<Client>,
    msgs: BTreeMap<u32, Dm>,
    seen_wraps: HashSet<EventId>,
    newest_wrap: Option<u64>,
    names: Names,
    relays: Vec<String>,
}

fn proto<E: std::fmt::Display>(e: E) -> BackendError {
    BackendError::Protocol(e.to_string())
}

fn conn<E: std::fmt::Display>(e: E) -> BackendError {
    BackendError::Connection(e.to_string())
}

impl NostrBackend {
    pub fn new(cfg: NostrConfig) -> Result<Self> {
        let keys = Keys::parse(&cfg.secret).map_err(|e| BackendError::Auth(format!("bad nostr key: {e}")))?;
        let relays = if cfg.relays.is_empty() { DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect() } else { cfg.relays.clone() };
        Ok(Self { cfg, keys, client: None, msgs: BTreeMap::new(), seen_wraps: HashSet::new(), newest_wrap: None, names: Names::default(), relays })
    }

    pub fn public_key(&self) -> PublicKey {
        self.keys.public_key()
    }

    pub fn address(&self) -> String {
        address_of(&self.public_key())
    }

    async fn client(&mut self) -> Result<Client> {
        if let Some(c) = &self.client {
            return Ok(c.clone());
        }
        let client = Client::new(self.keys.clone());
        for url in &self.relays {
            client.add_relay(url.as_str()).await.map_err(conn)?;
        }
        client.connect().await;
        client.wait_for_connection(Duration::from_secs(8)).await;
        let own = self.lookup_relays(&client, &self.public_key()).await?;
        for url in own.iter().filter(|u| !self.relays.contains(u)) {
            client.add_relay(url.as_str()).await.map_err(conn)?;
        }
        client.connect().await;
        client.wait_for_connection(Duration::from_secs(8)).await;
        self.relays = own;
        if self.cfg.publish_inbox && !self.has_inbox_list(&client).await {
            self.publish_inbox_list(&client).await?;
        }
        self.client = Some(client.clone());
        Ok(client)
    }

    async fn has_inbox_list(&self, client: &Client) -> bool {
        let filter = Filter::new().author(self.public_key()).kind(Kind::InboxRelays).limit(1);
        client.fetch_events_from(self.relays.clone(), filter, FETCH_TIMEOUT).await.map(|e| !e.is_empty()).unwrap_or(false)
    }

    async fn publish_inbox_list(&self, client: &Client) -> Result<()> {
        let tags: Vec<Tag> = self.relays.iter().filter_map(|r| Tag::parse(["relay", r.as_str()]).ok()).collect();
        let event = EventBuilder::new(Kind::InboxRelays, "").tags(tags).sign_with_keys(&self.keys).map_err(proto)?;
        client.send_event_to(self.relays.clone(), &event).await.map_err(conn)?;
        Ok(())
    }

    pub async fn lacks_inbox_list(&mut self) -> Result<bool> {
        let client = self.client().await?;
        Ok(!self.has_inbox_list(&client).await)
    }

    pub async fn relays_for(&mut self, who: &PublicKey) -> Result<Vec<String>> {
        let client = self.client().await?;
        self.lookup_relays(&client, who).await
    }

    async fn lookup_relays(&self, client: &Client, who: &PublicKey) -> Result<Vec<String>> {
        let ask = |kind: Kind| Filter::new().author(*who).kind(kind).limit(1);
        let found = client.fetch_events_from(self.relays.clone(), ask(Kind::InboxRelays), FETCH_TIMEOUT).await.map_err(conn)?;
        if let Some(ev) = found.first() {
            let list: Vec<String> = nostr_sdk::nips::nip17::extract_relay_list(ev).map(|u| u.to_string()).collect();
            if !list.is_empty() {
                return Ok(list);
            }
        }
        let found = client.fetch_events_from(self.relays.clone(), ask(Kind::RelayList), FETCH_TIMEOUT).await.map_err(conn)?;
        if let Some(ev) = found.first() {
            let list: Vec<String> = nostr_sdk::nips::nip65::extract_relay_list(ev)
                .filter(|(_, m)| !matches!(m, Some(RelayMetadata::Write)))
                .map(|(u, _)| u.to_string())
                .collect();
            if !list.is_empty() {
                return Ok(list);
            }
        }
        Ok(self.relays.clone())
    }

    async fn refresh_names(&mut self, client: &Client, keys: Vec<PublicKey>) {
        let stale: Vec<PublicKey> = keys.into_iter().filter(|k| self.names.get(k).is_none()).collect();
        if stale.is_empty() {
            return;
        }
        let filter = Filter::new().authors(stale.clone()).kind(Kind::Metadata);
        let events = client.fetch_events_from(self.relays.clone(), filter, FETCH_TIMEOUT).await;
        let mut found: HashMap<PublicKey, Profile> = HashMap::new();
        if let Ok(events) = events {
            for ev in events.iter() {
                let Ok(meta) = Metadata::from_json(&ev.content) else { continue };
                let non_empty = |s: Option<String>| s.filter(|s| !s.trim().is_empty());
                let mut profile = Profile {
                    name: non_empty(meta.display_name).or(non_empty(meta.name)),
                    picture: non_empty(meta.picture),
                    nip05: None,
                };
                if let Some(id) = non_empty(meta.nip05) {
                    if verify_nip05(&id, &ev.pubkey).await {
                        profile.nip05 = Some(id);
                    }
                }
                found.insert(ev.pubkey, profile);
            }
        }
        for k in stale {
            self.names.set(k, found.remove(&k).unwrap_or_default());
        }
    }

    pub async fn refresh(&mut self) -> Result<usize> {
        let client = self.client().await?;
        let mut filter = Filter::new().kind(Kind::GiftWrap).pubkey(self.public_key()).limit(BACKFILL_CAP);
        if let Some(newest) = self.newest_wrap {
            filter = filter.since(Timestamp::from_secs(newest.saturating_sub(WRAP_SKEW_SECS)));
        }
        let events = client.fetch_events_from(self.relays.clone(), filter, FETCH_TIMEOUT).await.map_err(conn)?;
        let mut added = 0;
        let mut people = Vec::new();
        for wrap in events.iter() {
            if !self.seen_wraps.insert(wrap.id) {
                continue;
            }
            self.newest_wrap = Some(self.newest_wrap.unwrap_or(0).max(wrap.created_at.as_secs()));
            if let Ok(dm) = open_wrap(&self.keys, wrap).await {
                if self.insert(dm.clone()) {
                    added += 1;
                    people.extend(dm.participants());
                }
            }
        }
        people.sort();
        people.dedup();
        self.refresh_names(&client, people).await;
        Ok(added)
    }

    pub fn insert(&mut self, dm: Dm) -> bool {
        let uid = uid_of(&dm.id);
        if self.msgs.get(&uid).is_some_and(|m| m.id != dm.id) {
            return false;
        }
        self.msgs.insert(uid, dm).is_none()
    }

    fn folder_of(&self, dm: &Dm) -> &'static str {
        if dm.sender == self.public_key() { SENT } else { INBOX }
    }

    fn uids_in(&self, folder: &str) -> Vec<u32> {
        self.msgs.iter().filter(|(_, m)| self.folder_of(m) == folder).map(|(u, _)| *u).collect()
    }

    pub async fn resolve(&self, text: &str) -> Result<PublicKey> {
        if let Some(k) = parse_key(text) {
            return Ok(k);
        }
        let bare = text.trim().strip_suffix(&format!("@{DOMAIN}")).unwrap_or(text.trim()).to_string();
        resolve_nip05(&bare).await.ok_or_else(|| BackendError::Protocol(format!("cannot find a nostr key for {text}")))
    }
}

#[async_trait]
impl Backend for NostrBackend {
    fn kind(&self) -> &'static str {
        "nostr"
    }

    fn fork(&self) -> Option<Box<dyn Backend>> {
        NostrBackend::new(self.cfg.clone()).ok().map(|b| Box::new(b) as Box<dyn Backend>)
    }

    async fn capabilities(&mut self) -> Result<Capabilities> {
        Ok(Capabilities { send: true, ..Default::default() })
    }

    async fn check(&mut self) -> Result<()> {
        let client = self.client().await?;
        let connected = client.relays().await.values().any(|r| r.is_connected());
        if connected { Ok(()) } else { Err(BackendError::Connection("no relay reachable".into())) }
    }

    async fn list_folders(&mut self) -> Result<Vec<Folder>> {
        let folder = |name: &str, role| Folder { name: name.into(), delimiter: None, role: Some(role), selectable: true };
        Ok(vec![folder(INBOX, FolderRole::Inbox), folder(SENT, FolderRole::Sent), folder(ARCHIVE, FolderRole::Archive)])
    }

    async fn open_folder(&mut self, folder: &str) -> Result<FolderState> {
        if folder != ARCHIVE {
            self.refresh().await?;
        }
        let uids = self.uids_in(folder);
        let next = uids.iter().max().map_or(1, |m| m + 1);
        Ok(FolderState { token: SyncToken { uid_validity: 1, uid_next: next, highest_mod_seq: None }, uids })
    }

    async fn fetch_summaries(&mut self, _folder: &str, uids: &[u32]) -> Result<Vec<MessageSummary>> {
        let me = self.public_key();
        Ok(uids.iter().filter_map(|u| self.msgs.get(u)).map(|m| summary_of(m, &me, &self.names)).collect())
    }

    async fn changed_flags(&mut self, _folder: &str, _below: u32, _since: Option<u64>) -> Result<Vec<(u32, Flags)>> {
        Ok(Vec::new())
    }

    async fn fetch(&mut self, _folder: &str, uid: u32) -> Result<Vec<u8>> {
        if !self.msgs.contains_key(&uid) {
            self.refresh().await?;
        }
        let dm = self.msgs.get(&uid).cloned().ok_or_else(|| BackendError::Protocol("message not found".into()))?;
        let file = match &dm.file {
            Some(f) => {
                let (url, f2) = (f.url.clone(), f.clone());
                let cipher = tokio::task::spawn_blocking(move || fetch_url(&url)).await.map_err(conn)??;
                Some((f2.clone(), decrypt_file(&f2, &cipher)?))
            }
            None => None,
        };
        Ok(render(&dm, &self.names, file.as_ref().map(|(f, d)| (f, d.clone()))))
    }

    async fn append(&mut self, _folder: &str, _raw: &[u8]) -> Result<()> {
        Ok(())
    }

    fn sent_copy_by_server(&self) -> bool {
        true
    }

    async fn send(&mut self, message: &OutgoingMessage) -> Result<()> {
        if !message.attachments.is_empty() {
            return Err(BackendError::Protocol("nostr messages cannot carry attachments yet".into()));
        }
        let client = self.client().await?;
        let me = self.public_key();
        let mut recipients = Vec::new();
        for a in message.to.iter().chain(&message.cc).chain(&message.bcc) {
            recipients.push(self.resolve(&a.address).await?);
        }
        recipients.dedup();
        if recipients.is_empty() {
            return Err(BackendError::Protocol("no recipients".into()));
        }
        let mut tags: Vec<Tag> = recipients.iter().map(|k| Tag::public_key(*k)).collect();
        if !message.subject.trim().is_empty() {
            tags.push(Tag::parse(["subject", message.subject.trim()]).map_err(proto)?);
        }
        let parent = message
            .in_reply_to
            .as_deref()
            .map(|s| s.trim().trim_matches(['<', '>']))
            .and_then(|s| s.strip_suffix(&format!("@{DOMAIN}")))
            .and_then(|h| EventId::from_hex(h).ok());
        if let Some(p) = parent {
            tags.push(Tag::event(p));
        }
        let mut rumor = EventBuilder::new(Kind::PrivateDirectMessage, message.body.clone()).tags(tags).build(me);
        rumor.ensure_id();
        let mut targets: Vec<(PublicKey, Vec<String>)> = Vec::new();
        for r in recipients.iter().filter(|r| **r != me) {
            targets.push((*r, self.relays_for(r).await?));
        }
        targets.push((me, self.relays.clone()));
        for (who, relays) in targets {
            let wrap = EventBuilder::gift_wrap(&self.keys, &who, rumor.clone(), []).await.map_err(proto)?;
            let out = client.send_event_to(relays, &wrap).await.map_err(conn)?;
            if out.success.is_empty() {
                return Err(BackendError::Connection("no relay accepted the message".into()));
            }
        }
        if let Some(id) = rumor.id {
            self.msgs.entry(uid_of(&id)).or_insert_with(|| Dm {
                id,
                sender: me,
                to: recipients.clone(),
                created_at: rumor.created_at.as_secs() as i64,
                subject: Some(message.subject.trim().to_string()).filter(|s| !s.is_empty()),
                content: message.body.clone(),
                reply_to: parent,
                file: None,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(n: u8) -> Keys {
        Keys::parse(&format!("{n:02x}").repeat(32)).unwrap()
    }

    async fn wrap(from: &Keys, to: &Keys, rumor: EventBuilder) -> NostrEvent {
        EventBuilder::gift_wrap(from, &to.public_key(), rumor.build(from.public_key()), []).await.unwrap()
    }

    #[tokio::test]
    async fn wrap_round_trips_and_carries_subject_and_reply() {
        let (a, b) = (keys(1), keys(2));
        let parent = EventId::all_zeros();
        let rumor = EventBuilder::new(Kind::PrivateDirectMessage, "hello")
            .tags([Tag::public_key(b.public_key()), Tag::parse(["subject", "Plans"]).unwrap(), Tag::event(parent)]);
        let w = wrap(&a, &b, rumor).await;
        assert_eq!(w.kind, Kind::GiftWrap);
        assert_ne!(w.pubkey, a.public_key());
        let dm = open_wrap(&b, &w).await.unwrap();
        assert_eq!(dm.sender, a.public_key());
        assert_eq!(dm.content, "hello");
        assert_eq!(dm.subject.as_deref(), Some("Plans"));
        assert_eq!(dm.reply_to, Some(parent));
        assert_eq!(dm.to, vec![b.public_key()]);
        assert_eq!(open_wrap(&a, &w).await.unwrap_err(), Rejected::Undecryptable);
    }

    #[tokio::test]
    async fn seal_pubkey_mismatch_is_rejected() {
        let (a, b, c) = (keys(1), keys(2), keys(3));
        let rumor = EventBuilder::new(Kind::PrivateDirectMessage, "forged").tags([Tag::public_key(b.public_key())]).build(c.public_key());
        let seal = EventBuilder::seal(&a, &b.public_key(), rumor).await.unwrap().sign_with_keys(&a).unwrap();
        let w = EventBuilder::gift_wrap_from_seal(&b.public_key(), &seal, []).unwrap();
        assert_eq!(open_wrap(&b, &w).await.unwrap_err(), Rejected::SenderMismatch);
    }

    #[tokio::test]
    async fn other_kinds_are_not_messages() {
        let (a, b) = (keys(1), keys(2));
        let w = wrap(&a, &b, EventBuilder::text_note("note")).await;
        assert_eq!(open_wrap(&b, &w).await.unwrap_err(), Rejected::WrongKind);
        let plain = EventBuilder::text_note("x").sign_with_keys(&a).unwrap();
        assert_eq!(open_wrap(&b, &plain).await.unwrap_err(), Rejected::NotGiftWrap);
    }

    #[tokio::test]
    async fn two_wraps_of_one_rumor_dedupe_by_rumor_id() {
        let (a, b) = (keys(1), keys(2));
        let mut rumor = EventBuilder::new(Kind::PrivateDirectMessage, "once").tags([Tag::public_key(b.public_key())]).build(a.public_key());
        rumor.ensure_id();
        let w1 = EventBuilder::gift_wrap(&a, &b.public_key(), rumor.clone(), []).await.unwrap();
        let w2 = EventBuilder::gift_wrap(&a, &b.public_key(), rumor, []).await.unwrap();
        assert_ne!(w1.id, w2.id);
        let mut backend = NostrBackend::new(NostrConfig::new(&b.secret_key().to_secret_hex(), vec![])).unwrap();
        assert!(backend.insert(open_wrap(&b, &w1).await.unwrap()));
        assert!(!backend.insert(open_wrap(&b, &w2).await.unwrap()));
        assert_eq!(backend.msgs.len(), 1);
    }

    #[test]
    fn thread_key_ignores_order_and_reply_prefixes_but_not_people() {
        let (a, b, c) = (keys(1).public_key(), keys(2).public_key(), keys(3).public_key());
        assert_eq!(thread_key(&[a, b], Some("Plans")), thread_key(&[b, a, a], Some("Re: plans")));
        assert_ne!(thread_key(&[a, b], Some("Plans")), thread_key(&[a, c], Some("Plans")));
        assert_ne!(thread_key(&[a, b], Some("Plans")), thread_key(&[a, b], Some("Other")));
        assert_ne!(thread_key(&[a, b], None), thread_key(&[a, b], Some("Plans")));
    }

    #[test]
    fn file_encryption_round_trips_and_rejects_tampering() {
        let (key, nonce) = ([7u8; 32], [9u8; 12]);
        let cipher = encrypt_file(&key, &nonce, b"secret bytes").unwrap();
        let file = FileRef { url: "http://x/f.txt".into(), mime: "text/plain".into(), key: key.to_vec(), nonce: nonce.to_vec() };
        assert_eq!(decrypt_file(&file, &cipher).unwrap(), b"secret bytes");
        let mut bad = cipher.clone();
        bad[0] ^= 1;
        assert!(decrypt_file(&file, &bad).is_err());
    }

    #[test]
    fn addresses_parse_in_every_form() {
        let k = keys(5).public_key();
        assert_eq!(parse_key(&address_of(&k)), Some(k));
        assert_eq!(parse_key(&k.to_hex()), Some(k));
        assert_eq!(parse_key(&format!("nostr:{}", k.to_bech32().unwrap())), Some(k));
        assert_eq!(parse_key("alice@example.com"), None);
    }

    #[test]
    fn nip05_only_verifies_the_matching_key() {
        let (a, b) = (keys(1).public_key(), keys(2).public_key());
        let body = format!("{{\"names\":{{\"bob\":\"{}\"}}}}", a.to_hex());
        assert!(nip05_matches(body.as_bytes(), "bob@example.com", &a));
        assert!(!nip05_matches(body.as_bytes(), "bob@example.com", &b));
        assert!(!nip05_matches(body.as_bytes(), "eve@example.com", &a));
        assert!(!nip05_matches(b"not json", "bob@example.com", &a));
    }

    #[tokio::test]
    async fn profile_picture_and_verified_nip05_become_headers() {
        let (a, b) = (keys(1), keys(2));
        let rumor = EventBuilder::new(Kind::PrivateDirectMessage, "hi").tags([Tag::public_key(b.public_key())]);
        let dm = open_wrap(&b, &wrap(&a, &b, rumor).await).await.unwrap();
        let mut names = Names::default();
        names.set(a.public_key(), Profile { name: Some("Alice".into()), picture: Some("https://img.example/a.png".into()), nip05: Some("alice@example.com".into()) });
        let raw = String::from_utf8(render(&dm, &names, None)).unwrap();
        assert!(raw.contains("X-Nostr-Picture: https://img.example/a.png\r\n"));
        assert!(raw.contains("X-Nostr-Nip05: alice@example.com\r\n"));
        let plain = String::from_utf8(render(&dm, &Names::default(), None)).unwrap();
        assert!(!plain.contains("X-Nostr-"));
    }

    #[test]
    fn dates_render_as_rfc2822() {
        assert_eq!(rfc2822(0), "Thu, 01 Jan 1970 00:00:00 +0000");
        assert_eq!(rfc2822(1_700_000_000), "Tue, 14 Nov 2023 22:13:20 +0000");
    }

    #[tokio::test]
    async fn rendered_message_parses_with_thread_references() {
        let (a, b) = (keys(1), keys(2));
        let rumor = EventBuilder::new(Kind::PrivateDirectMessage, "héllo there").tags([Tag::public_key(b.public_key()), Tag::parse(["subject", "Plans"]).unwrap()]);
        let dm = open_wrap(&b, &wrap(&a, &b, rumor).await).await.unwrap();
        let raw = render(&dm, &Names::default(), None);
        let parsed = mail_parser::MessageParser::default().parse(&raw).unwrap();
        assert_eq!(parsed.subject(), Some("Plans"));
        assert_eq!(parsed.body_text(0).unwrap().trim(), "héllo there");
        assert!(parsed.references().unwrap().iter().any(|r| r == dm.thread()));
    }
}
