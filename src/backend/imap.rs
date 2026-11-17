use super::error::{BackendError, Result};
use super::types::*;
use super::{Backend, Watch};
use imap::types::{Fetch, Flag, Mailbox, UnsolicitedResponse};
use imap::{Client, ClientBuilder, Connection, ConnectionMode, Session};
use imap_proto::NameAttribute;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::time::Duration;

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

struct XOAuth2<'a> {
    user: &'a str,
    token: &'a str,
}

impl imap::Authenticator for XOAuth2<'_> {
    type Response = String;

    fn process(&self, _challenge: &[u8]) -> Self::Response {
        format!("user={}\x01auth=Bearer {}\x01\x01", self.user, self.token)
    }
}

fn connect(cfg: &ImapConfig) -> Result<Session<Connection>> {
    let mode = match cfg.security {
        Security::Tls => ConnectionMode::Tls,
        Security::StartTls => ConnectionMode::StartTls,
        Security::Plain => ConnectionMode::Plaintext,
    };
    let client: Client<Connection> = ClientBuilder::new(cfg.host.as_str(), cfg.port)
        .mode(mode)
        .danger_skip_tls_verify(cfg.accept_invalid_certs)
        .connect()?;
    let session = match &cfg.auth {
        Auth::Password { user, password } => client.login(user, password),
        Auth::XOAuth2 { user, access_token } => client.authenticate(
            "XOAUTH2",
            &XOAuth2 {
                user,
                token: access_token,
            },
        ),
    };
    session.map_err(|(e, _)| BackendError::Auth(e.to_string()))
}

pub struct ImapBackend {
    cfg: ImapConfig,
    session: Option<Session<Connection>>,
    caps: Option<Capabilities>,
}

impl ImapBackend {
    pub fn new(cfg: ImapConfig) -> Self {
        Self {
            cfg,
            session: None,
            caps: None,
        }
    }

    fn session(&mut self) -> Result<&mut Session<Connection>> {
        if self.session.is_none() {
            self.session = Some(connect(&self.cfg)?);
        }
        Ok(self.session.as_mut().unwrap())
    }

    fn run<T>(
        &mut self,
        op: impl FnOnce(&mut Session<Connection>, &Capabilities) -> Result<T>,
    ) -> Result<T> {
        let caps = self.capabilities()?;
        let result = op(self.session()?, &caps);
        if let Err(BackendError::Connection(_)) = result {
            self.session = None;
        }
        result
    }
}

fn uid_set(uids: &[u32]) -> String {
    uids.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
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

fn parse_flags(flags: &[Flag<'_>]) -> Flags {
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
            .filter_map(|m| {
                Some(Address {
                    name: m.name().map(str::to_string),
                    address: m.address()?.to_string(),
                })
            })
            .collect()
    })
    .unwrap_or_default()
}

fn text_list(list: Option<mail_parser::TextList<'_>>) -> Vec<String> {
    list.map(|l| l.iter().map(str::to_string).collect())
        .unwrap_or_default()
}

pub(crate) fn summarize(uid: u32, flags: Flags, size: Option<u32>, header: &[u8]) -> MessageSummary {
    let mut summary = MessageSummary {
        uid,
        flags,
        size,
        ..Default::default()
    };
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
        uid_validity: mailbox
            .uid_validity
            .ok_or_else(|| BackendError::Protocol("no UIDVALIDITY".into()))?,
        uid_next: mailbox
            .uid_next
            .ok_or_else(|| BackendError::Protocol("no UIDNEXT".into()))?,
        highest_mod_seq: mailbox.highest_mod_seq,
    })
}

fn summaries(fetches: &imap::types::Fetches, min_uid: u32) -> Vec<MessageSummary> {
    let mut out: Vec<MessageSummary> = fetches
        .iter()
        .filter_map(|f: &Fetch| {
            let uid = f.uid.filter(|u| *u >= min_uid)?;
            Some(summarize(
                uid,
                parse_flags(f.flags()),
                f.size,
                f.header().unwrap_or_default(),
            ))
        })
        .collect();
    out.sort_by_key(|m| m.uid);
    out
}

const SUMMARY_QUERY: &str = "(UID FLAGS RFC822.SIZE BODY.PEEK[HEADER])";

impl Backend for ImapBackend {
    fn kind(&self) -> &'static str {
        "imap"
    }

    fn capabilities(&mut self) -> Result<Capabilities> {
        if let Some(caps) = &self.caps {
            return Ok(caps.clone());
        }
        let raw = self.session()?.capabilities()?;
        let caps = Capabilities {
            idle: raw.has_str("IDLE"),
            move_command: raw.has_str("MOVE"),
            condstore: raw.has_str("CONDSTORE") || raw.has_str("QRESYNC"),
            qresync: raw.has_str("QRESYNC"),
            uidplus: raw.has_str("UIDPLUS"),
            send: false,
        };
        self.caps = Some(caps.clone());
        Ok(caps)
    }

    fn list_folders(&mut self) -> Result<Vec<Folder>> {
        self.run(|s, _| {
            let names = s.list(Some(""), Some("*"))?;
            Ok(names
                .iter()
                .map(|n| {
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
                })
                .collect())
        })
    }

    fn create_folder(&mut self, name: &str) -> Result<()> {
        self.run(|s, _| Ok(s.create(name)?))
    }

    fn sync_folder(&mut self, folder: &str, since: Option<&SyncToken>) -> Result<FolderDelta> {
        self.run(|s, caps| {
            let mailbox = s.select(folder)?;
            let token = token_of(&mailbox)?;
            let uids = {
                let mut v: Vec<u32> = s.uid_search("ALL")?.into_iter().collect();
                v.sort_unstable();
                v
            };
            let since = since.filter(|t| t.uid_validity == token.uid_validity);
            let Some(since) = since else {
                let added = if uids.is_empty() {
                    Vec::new()
                } else {
                    summaries(&s.uid_fetch("1:*", SUMMARY_QUERY)?, 1)
                };
                return Ok(FolderDelta {
                    token,
                    reset: true,
                    added,
                    flag_changes: Vec::new(),
                    uids,
                });
            };
            let added = if token.uid_next > since.uid_next {
                summaries(
                    &s.uid_fetch(format!("{}:*", since.uid_next), SUMMARY_QUERY)?,
                    since.uid_next,
                )
            } else {
                Vec::new()
            };
            let known_below = uids.iter().any(|u| *u < since.uid_next);
            let mut flag_changes = Vec::new();
            if known_below {
                let range = format!("1:{}", since.uid_next.saturating_sub(1));
                let changed_only = caps.condstore && since.highest_mod_seq.is_some();
                let query = match (changed_only, since.highest_mod_seq) {
                    (true, Some(m)) => format!("(UID FLAGS) (CHANGEDSINCE {m})"),
                    _ => "(UID FLAGS)".to_string(),
                };
                for f in s.uid_fetch(range, query)?.iter() {
                    if let Some(uid) = f.uid {
                        flag_changes.push((uid, parse_flags(f.flags())));
                    }
                }
                flag_changes.sort_by_key(|(u, _)| *u);
            }
            Ok(FolderDelta {
                token,
                reset: false,
                added,
                flag_changes,
                uids,
            })
        })
    }

    fn fetch(&mut self, folder: &str, uid: u32) -> Result<Vec<u8>> {
        self.run(|s, _| {
            s.select(folder)?;
            let fetches = s.uid_fetch(uid.to_string(), "(UID BODY.PEEK[])")?;
            fetches
                .iter()
                .find(|f| f.uid == Some(uid))
                .and_then(|f| f.body().map(<[u8]>::to_vec))
                .ok_or_else(|| BackendError::Protocol(format!("message {uid} not found")))
        })
    }

    fn set_flags(&mut self, folder: &str, uids: &[u32], update: &FlagUpdate) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        self.run(|s, _| {
            s.select(folder)?;
            let query = match update {
                FlagUpdate::Add(f) => format!("+FLAGS.SILENT {}", flag_list(f)),
                FlagUpdate::Remove(f) => format!("-FLAGS.SILENT {}", flag_list(f)),
                FlagUpdate::Set(f) => format!("FLAGS.SILENT {}", flag_list(f)),
            };
            s.uid_store(uid_set(uids), query)?;
            Ok(())
        })
    }

    fn move_messages(&mut self, folder: &str, uids: &[u32], destination: &str) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        self.run(|s, caps| {
            s.select(folder)?;
            let set = uid_set(uids);
            if caps.move_command {
                s.uid_mv(&set, destination)?;
            } else {
                s.uid_copy(&set, destination)?;
                s.uid_store(&set, "+FLAGS.SILENT (\\Deleted)")?;
                expunge(s, caps, &set)?;
            }
            Ok(())
        })
    }

    fn delete_messages(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        self.run(|s, caps| {
            s.select(folder)?;
            let set = uid_set(uids);
            s.uid_store(&set, "+FLAGS.SILENT (\\Deleted)")?;
            expunge(s, caps, &set)
        })
    }

    fn search(&mut self, folder: &str, query: &str) -> Result<Vec<u32>> {
        self.run(|s, _| {
            s.select(folder)?;
            let mut v: Vec<u32> = s.uid_search(query)?.into_iter().collect();
            v.sort_unstable();
            Ok(v)
        })
    }

    fn watch(&mut self, folder: &str) -> Result<Watch> {
        if !self.capabilities()?.idle {
            return Err(BackendError::Unsupported("watch"));
        }
        let mut session = connect(&self.cfg)?;
        session.select(folder)?;
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let folder = folder.to_string();
        let timeout = self.cfg.idle_timeout;
        std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                let mut changed = false;
                let outcome = session
                    .idle()
                    .timeout(timeout)
                    .keepalive(false)
                    .wait_while(|r| {
                        if matches!(
                            r,
                            UnsolicitedResponse::Exists(_)
                                | UnsolicitedResponse::Expunge(_)
                                | UnsolicitedResponse::Fetch { .. }
                        ) {
                            changed = true;
                            return false;
                        }
                        true
                    });
                match outcome {
                    Ok(_) => {
                        if changed
                            && tx
                                .send(Event::FolderChanged {
                                    folder: folder.clone(),
                                })
                                .is_err()
                        {
                            return;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Event::Disconnected {
                            reason: e.to_string(),
                        });
                        return;
                    }
                }
            }
        });
        Ok(Watch::new(rx, stop))
    }
}

fn expunge(s: &mut Session<Connection>, caps: &Capabilities, set: &str) -> Result<()> {
    if caps.uidplus {
        s.uid_expunge(set)?;
    } else {
        s.expunge()?;
    }
    Ok(())
}
