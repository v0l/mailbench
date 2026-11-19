use super::error::{BackendError, Result};
use super::types::*;
use super::{Backend, Watch};
use async_imap::extensions::idle::IdleResponse;
use async_imap::imap_proto::NameAttribute;
use async_imap::types::{Fetch, Flag, Mailbox, Name};
use async_imap::{Client, Session};
use async_trait::async_trait;
use futures::TryStreamExt;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_native_tls::TlsStream;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const HEADER_FIELDS: &str =
    "BODY.PEEK[HEADER.FIELDS (DATE FROM TO CC SUBJECT MESSAGE-ID IN-REPLY-TO REFERENCES)]";

#[derive(Clone, Debug)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub auth: Auth,
    pub accept_invalid_certs: bool,
    pub idle_timeout: Duration,
}

impl ImapConfig {
    pub fn new(host: &str, port: u16, security: Security, auth: Auth) -> Self {
        Self {
            host: host.to_string(),
            port,
            security,
            auth,
            accept_invalid_certs: false,
            idle_timeout: Duration::from_secs(29 * 60),
        }
    }
}

#[derive(Debug)]
pub enum Stream {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl AsyncRead for Stream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_flush(cx),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

type ImapSession = Session<Stream>;

struct XOAuth2 {
    user: String,
    token: String,
}

impl async_imap::Authenticator for XOAuth2 {
    type Response = String;

    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        format!("user={}\x01auth=Bearer {}\x01\x01", self.user, self.token)
    }
}

async fn tls_wrap(cfg: &ImapConfig, tcp: TcpStream) -> Result<Stream> {
    let connector = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(cfg.accept_invalid_certs)
        .danger_accept_invalid_hostnames(cfg.accept_invalid_certs)
        .build()?;
    let tls = tokio_native_tls::TlsConnector::from(connector).connect(&cfg.host, tcp).await?;
    Ok(Stream::Tls(Box::new(tls)))
}

async fn greet(client: &mut Client<Stream>) -> Result<()> {
    client
        .read_response()
        .await?
        .ok_or_else(|| BackendError::Connection("the server closed the connection".into()))?;
    Ok(())
}

async fn open_client(cfg: &ImapConfig) -> Result<Client<Stream>> {
    let tcp = timeout(CONNECT_TIMEOUT, TcpStream::connect((cfg.host.as_str(), cfg.port))).await??;
    tcp.set_nodelay(true)?;
    match cfg.security {
        Security::Plain => {
            let mut client = Client::new(Stream::Plain(tcp));
            greet(&mut client).await?;
            Ok(client)
        }
        Security::Tls => {
            let mut client = Client::new(tls_wrap(cfg, tcp).await?);
            greet(&mut client).await?;
            Ok(client)
        }
        Security::StartTls => {
            let mut client = Client::new(Stream::Plain(tcp));
            greet(&mut client).await?;
            client.run_command_and_check_ok("STARTTLS", None).await?;
            let Stream::Plain(tcp) = client.into_inner() else {
                return Err(BackendError::Protocol("unexpected stream".into()));
            };
            Ok(Client::new(tls_wrap(cfg, tcp).await?))
        }
    }
}

async fn connect(cfg: &ImapConfig) -> Result<ImapSession> {
    let client = timeout(CONNECT_TIMEOUT, open_client(cfg)).await??;
    let session = match &cfg.auth {
        Auth::Password { user, password } => client.login(user, password).await,
        Auth::XOAuth2 { user, access_token } => {
            client.authenticate("XOAUTH2", XOAuth2 { user: user.clone(), token: access_token.clone() }).await
        }
    };
    session.map_err(|(e, _)| BackendError::Auth(e.to_string()))
}

fn caps_of(raw: &async_imap::types::Capabilities) -> Capabilities {
    Capabilities {
        idle: raw.has_str("IDLE"),
        move_command: raw.has_str("MOVE"),
        condstore: raw.has_str("CONDSTORE") || raw.has_str("QRESYNC"),
        qresync: raw.has_str("QRESYNC"),
        uidplus: raw.has_str("UIDPLUS"),
        send: false,
    }
}

pub struct ImapBackend {
    cfg: ImapConfig,
    session: Option<ImapSession>,
    caps: Option<Capabilities>,
    selected: Option<String>,
}

impl ImapBackend {
    pub fn new(cfg: ImapConfig) -> Self {
        Self { cfg, session: None, caps: None, selected: None }
    }

    async fn ensure(&mut self) -> Result<Capabilities> {
        if self.session.is_none() {
            let mut session = connect(&self.cfg).await?;
            let caps = caps_of(&timeout(COMMAND_TIMEOUT, session.capabilities()).await??);
            if caps.condstore {
                timeout(COMMAND_TIMEOUT, session.run_command_and_check_ok("ENABLE CONDSTORE")).await??;
            }
            self.caps = Some(caps);
            self.selected = None;
            self.session = Some(session);
        }
        Ok(self.caps.clone().unwrap_or_default())
    }

    fn forget<T>(&mut self, result: Result<T>) -> Result<T> {
        if let Err(BackendError::Connection(_)) = result {
            self.session = None;
            self.selected = None;
        }
        result
    }

    async fn select(&mut self, folder: &str) -> Result<Option<Mailbox>> {
        self.ensure().await?;
        if self.selected.as_deref() == Some(folder) {
            return Ok(None);
        }
        let condstore = self.caps.as_ref().is_some_and(|c| c.condstore);
        let session = self.session.as_mut().expect("connected");
        let result = if condstore {
            timeout(COMMAND_TIMEOUT, session.select_condstore(folder)).await
        } else {
            timeout(COMMAND_TIMEOUT, session.select(folder)).await
        };
        let mailbox = self.forget(result.map_err(BackendError::from).and_then(|r| r.map_err(BackendError::from)))?;
        self.selected = Some(folder.to_string());
        Ok(Some(mailbox))
    }

    async fn reselect(&mut self, folder: &str) -> Result<Mailbox> {
        self.selected = None;
        self.select(folder).await?.ok_or_else(|| BackendError::Protocol("select failed".into()))
    }

    fn session(&mut self) -> &mut ImapSession {
        self.session.as_mut().expect("connected")
    }
}

pub(crate) fn uid_set(uids: &[u32]) -> String {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut out = String::new();
    let mut i = 0;
    while i < sorted.len() {
        let start = sorted[i];
        let mut end = start;
        while i + 1 < sorted.len() && sorted[i + 1] == end + 1 {
            i += 1;
            end = sorted[i];
        }
        if !out.is_empty() {
            out.push(',');
        }
        if start == end {
            out.push_str(&start.to_string());
        } else {
            out.push_str(&format!("{start}:{end}"));
        }
        i += 1;
    }
    out
}

fn flag_list(flags: &Flags) -> String {
    let mut out = Vec::new();
    if flags.seen {
        out.push("\\Seen".to_string());
    }
    if flags.answered {
        out.push("\\Answered".to_string());
    }
    if flags.flagged {
        out.push("\\Flagged".to_string());
    }
    if flags.deleted {
        out.push("\\Deleted".to_string());
    }
    if flags.draft {
        out.push("\\Draft".to_string());
    }
    out.extend(flags.keywords.iter().cloned());
    format!("({})", out.join(" "))
}

fn parse_flags<'a>(flags: impl Iterator<Item = Flag<'a>>) -> Flags {
    let mut out = Flags::default();
    for flag in flags {
        match flag {
            Flag::Seen => out.seen = true,
            Flag::Answered => out.answered = true,
            Flag::Flagged => out.flagged = true,
            Flag::Deleted => out.deleted = true,
            Flag::Draft => out.draft = true,
            Flag::Custom(name) => out.keywords.push(name.to_string()),
            _ => {}
        }
    }
    out
}

fn addresses(list: Option<mail_parser::AddressList<'_>>) -> Vec<Address> {
    list.map(|l| {
        l.mailboxes()
            .filter_map(|m| Some(Address { name: m.name().map(str::to_string), address: m.address()?.to_string() }))
            .collect()
    })
    .unwrap_or_default()
}

fn text_list(list: Option<mail_parser::TextList<'_>>) -> Vec<String> {
    list.map(|l| l.iter().map(str::to_string).collect()).unwrap_or_default()
}

pub(crate) fn summarize(uid: u32, flags: Flags, size: Option<u32>, header: &[u8]) -> MessageSummary {
    let mut summary = MessageSummary { uid, flags, size, ..Default::default() };
    if let Some(msg) = mail_parser::MessageParser::default().parse_headers(header) {
        summary.subject = msg.subject().map(str::to_string);
        summary.from = addresses(msg.from());
        summary.to = addresses(msg.to());
        summary.cc = addresses(msg.cc());
        summary.date = msg.date().map(|d| d.to_timestamp());
        summary.message_id = msg.message_id().map(str::to_string);
        summary.in_reply_to = text_list(msg.in_reply_to());
        summary.references = text_list(msg.references());
    }
    summary
}

fn token_of(mailbox: &Mailbox) -> Result<SyncToken> {
    Ok(SyncToken {
        uid_validity: mailbox.uid_validity.ok_or_else(|| BackendError::Protocol("no UIDVALIDITY".into()))?,
        uid_next: mailbox.uid_next.ok_or_else(|| BackendError::Protocol("no UIDNEXT".into()))?,
        highest_mod_seq: mailbox.highest_modseq,
    })
}

fn header_bytes(f: &Fetch) -> &[u8] {
    f.header().unwrap_or_default()
}

fn folder_of(n: &Name) -> Folder {
    let role = if n.name().eq_ignore_ascii_case("INBOX") {
        Some(FolderRole::Inbox)
    } else {
        n.attributes().iter().find_map(|a| match a {
            NameAttribute::Sent => Some(FolderRole::Sent),
            NameAttribute::Drafts => Some(FolderRole::Drafts),
            NameAttribute::Trash => Some(FolderRole::Trash),
            NameAttribute::Junk => Some(FolderRole::Junk),
            NameAttribute::Archive => Some(FolderRole::Archive),
            NameAttribute::All => Some(FolderRole::All),
            NameAttribute::Flagged => Some(FolderRole::Flagged),
            _ => None,
        })
    };
    Folder {
        name: n.name().to_string(),
        delimiter: n.delimiter().map(str::to_string),
        role,
        selectable: !n.attributes().contains(&NameAttribute::NoSelect),
    }
}

macro_rules! timed {
    ($self:ident, $body:expr) => {{
        let result: Result<_> = async { timeout(COMMAND_TIMEOUT, $body).await? }.await;
        $self.forget(result)
    }};
}

#[async_trait]
impl Backend for ImapBackend {
    fn kind(&self) -> &'static str {
        "imap"
    }

    fn fork(&self) -> Option<Box<dyn Backend>> {
        Some(Box::new(ImapBackend::new(self.cfg.clone())))
    }

    async fn capabilities(&mut self) -> Result<Capabilities> {
        self.ensure().await
    }

    async fn list_folders(&mut self) -> Result<Vec<Folder>> {
        self.ensure().await?;
        let s = self.session();
        timed!(self, async {
            let names: Vec<Name> = s.list(Some(""), Some("*")).await?.try_collect().await?;
            Ok::<_, BackendError>(names.iter().map(folder_of).collect())
        })
    }

    async fn create_folder(&mut self, name: &str) -> Result<()> {
        self.ensure().await?;
        let s = self.session();
        timed!(self, async { s.create(name).await.map_err(BackendError::from) })
    }

    async fn rename_folder(&mut self, from: &str, to: &str) -> Result<()> {
        self.ensure().await?;
        if self.selected.as_deref() == Some(from) {
            self.selected = None;
        }
        let s = self.session();
        timed!(self, async { s.rename(from, to).await.map_err(BackendError::from) })
    }

    async fn delete_folder(&mut self, name: &str) -> Result<()> {
        self.ensure().await?;
        if self.selected.as_deref() == Some(name) {
            self.selected = None;
        }
        let s = self.session();
        timed!(self, async { s.delete(name).await.map_err(BackendError::from) })
    }

    async fn open_folder(&mut self, folder: &str) -> Result<FolderState> {
        let mailbox = self.reselect(folder).await?;
        let token = token_of(&mailbox)?;
        let uids = if mailbox.exists == 0 {
            Vec::new()
        } else {
            let s = self.session();
            timed!(self, async {
                let mut v: Vec<u32> = s.uid_search("ALL").await?.into_iter().collect();
                v.sort_unstable();
                Ok::<_, BackendError>(v)
            })?
        };
        Ok(FolderState { token, uids })
    }

    async fn fetch_summaries(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<MessageSummary>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        self.select(folder).await?;
        let set = uid_set(uids);
        let s = self.session();
        timed!(self, async {
            let query = format!("(UID FLAGS RFC822.SIZE {HEADER_FIELDS})");
            let stream = s.uid_fetch(&set, &query).await?;
            let fetches: Vec<Fetch> = stream.try_collect().await?;
            let mut out: Vec<MessageSummary> = fetches
                .iter()
                .filter_map(|f| Some(summarize(f.uid?, parse_flags(f.flags()), f.size, header_bytes(f))))
                .collect();
            out.sort_by_key(|m| m.uid);
            Ok::<_, BackendError>(out)
        })
    }

    async fn changed_flags(&mut self, folder: &str, below: u32, since_mod_seq: Option<u64>) -> Result<Vec<(u32, Flags)>> {
        if below <= 1 {
            return Ok(Vec::new());
        }
        let caps = self.ensure().await?;
        self.select(folder).await?;
        let range = format!("1:{}", below - 1);
        let query = match (caps.condstore, since_mod_seq) {
            (true, Some(m)) => format!("(UID FLAGS) (CHANGEDSINCE {m})"),
            _ => "(UID FLAGS)".to_string(),
        };
        let s = self.session();
        timed!(self, async {
            let fetches: Vec<Fetch> = s.uid_fetch(&range, &query).await?.try_collect().await?;
            let mut out: Vec<(u32, Flags)> =
                fetches.iter().filter_map(|f| Some((f.uid?, parse_flags(f.flags())))).filter(|(u, _)| *u < below).collect();
            out.sort_by_key(|(u, _)| *u);
            Ok::<_, BackendError>(out)
        })
    }

    async fn fetch(&mut self, folder: &str, uid: u32) -> Result<Vec<u8>> {
        self.select(folder).await?;
        let s = self.session();
        timed!(self, async {
            let fetches: Vec<Fetch> = s.uid_fetch(uid.to_string(), "(UID BODY.PEEK[])").await?.try_collect().await?;
            fetches
                .iter()
                .find(|f| f.uid == Some(uid))
                .and_then(|f| f.body().map(<[u8]>::to_vec))
                .ok_or_else(|| BackendError::Protocol(format!("message {uid} not found")))
        })
    }

    async fn set_flags(&mut self, folder: &str, uids: &[u32], update: &FlagUpdate) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        self.select(folder).await?;
        let query = match update {
            FlagUpdate::Add(f) => format!("+FLAGS.SILENT {}", flag_list(f)),
            FlagUpdate::Remove(f) => format!("-FLAGS.SILENT {}", flag_list(f)),
            FlagUpdate::Set(f) => format!("FLAGS.SILENT {}", flag_list(f)),
        };
        let set = uid_set(uids);
        let s = self.session();
        timed!(self, async {
            let _: Vec<Fetch> = s.uid_store(&set, &query).await?.try_collect().await?;
            Ok::<_, BackendError>(())
        })
    }

    async fn move_messages(&mut self, folder: &str, uids: &[u32], destination: &str) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        let caps = self.ensure().await?;
        self.select(folder).await?;
        let set = uid_set(uids);
        let s = self.session();
        timed!(self, async {
            if caps.move_command {
                s.uid_mv(&set, destination).await?;
            } else {
                s.uid_copy(&set, destination).await?;
                let _: Vec<Fetch> = s.uid_store(&set, "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
                expunge(s, &caps, &set).await?;
            }
            Ok::<_, BackendError>(())
        })
    }

    async fn delete_messages(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        let caps = self.ensure().await?;
        self.select(folder).await?;
        let set = uid_set(uids);
        let s = self.session();
        timed!(self, async {
            let _: Vec<Fetch> = s.uid_store(&set, "+FLAGS.SILENT (\\Deleted)").await?.try_collect().await?;
            expunge(s, &caps, &set).await
        })
    }

    async fn search(&mut self, folder: &str, query: &str) -> Result<Vec<u32>> {
        self.reselect(folder).await?;
        let s = self.session();
        timed!(self, async {
            let mut v: Vec<u32> = s.uid_search(query).await?.into_iter().collect();
            v.sort_unstable();
            Ok::<_, BackendError>(v)
        })
    }

    async fn append(&mut self, folder: &str, raw: &[u8]) -> Result<()> {
        self.ensure().await?;
        let s = self.session();
        timed!(self, async { s.append(folder, None, None, raw).await.map_err(BackendError::from) })
    }

    async fn watch(&mut self, folder: &str) -> Result<Watch> {
        if !self.ensure().await?.idle {
            return Err(BackendError::Unsupported("watch"));
        }
        let mut session = connect(&self.cfg).await?;
        timeout(COMMAND_TIMEOUT, session.select(folder)).await??;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let folder = folder.to_string();
        let wait = self.cfg.idle_timeout;
        let task = tokio::spawn(async move {
            let mut session = Some(session);
            loop {
                let mut idle = session.take().expect("session").idle();
                if let Err(e) = idle.init().await {
                    let _ = tx.send(Event::Disconnected { reason: e.to_string() });
                    return;
                }
                let (fut, _stop) = idle.wait_with_timeout(wait);
                let outcome = fut.await;
                match outcome {
                    Ok(IdleResponse::NewData(_)) => {
                        if tx.send(Event::FolderChanged { folder: folder.clone() }).is_err() {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        let _ = tx.send(Event::Disconnected { reason: e.to_string() });
                        return;
                    }
                }
                match idle.done().await {
                    Ok(s) => session = Some(s),
                    Err(e) => {
                        let _ = tx.send(Event::Disconnected { reason: e.to_string() });
                        return;
                    }
                }
            }
        });
        Ok(Watch::new(rx, task.abort_handle()))
    }
}

async fn expunge(s: &mut ImapSession, caps: &Capabilities, set: &str) -> Result<()> {
    if caps.uidplus {
        let _: Vec<u32> = s.uid_expunge(set).await?.try_collect().await?;
    } else {
        let _: Vec<u32> = s.expunge().await?.try_collect().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uid_sets_collapse_runs_into_ranges() {
        assert_eq!(uid_set(&[5, 1, 2, 3, 9, 10, 12]), "1:3,5,9:10,12");
        assert_eq!(uid_set(&[7]), "7");
        assert_eq!(uid_set(&[]), "");
    }
}
