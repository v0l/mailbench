use crate::autodetect::{Provider, ServerSettings};
use crate::backend::*;
use crate::oauth::{self, OAuthTokens};
use crate::real::AccountSetup;
use crate::store::Store;
use serde::{Deserialize, Serialize};

pub const KIND: &str = "account";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Credential {
    Password(String),
    OAuth { provider: Provider, tokens: OAuthTokens },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountConfig {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub signature: String,
    pub address: String,
    pub username: String,
    pub imap: ServerSettings,
    pub smtp: ServerSettings,
    pub credential: Credential,
}

pub fn new_id() -> String {
    let bytes: [u8; 8] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn load_all(store: &Store) -> Vec<AccountConfig> {
    let mut out: Vec<AccountConfig> = store
        .list(KIND)
        .unwrap_or_default()
        .iter()
        .filter_map(|id| store.get(KIND, id).ok().flatten())
        .filter_map(|raw| serde_json::from_slice(&raw).ok())
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

pub fn save(store: &Store, cfg: &AccountConfig) -> Result<()> {
    let raw = serde_json::to_vec(cfg).map_err(|e| BackendError::Store(e.to_string()))?;
    store.put(KIND, &cfg.id, &raw)?;
    Ok(())
}

pub fn remove(store: &Store, id: &str) -> Result<()> {
    crate::real::purge_account(store, id).map_err(|e| BackendError::Store(format!("{e:?}")))?;
    store.delete(KIND, id)?;
    Ok(())
}

pub fn auth_of(cfg: &AccountConfig) -> Auth {
    match &cfg.credential {
        Credential::Password(p) => Auth::Password { user: cfg.username.clone(), password: p.clone() },
        Credential::OAuth { tokens, .. } => {
            Auth::XOAuth2 { user: cfg.username.clone(), access_token: tokens.access_token.clone() }
        }
    }
}

pub fn build_account(cfg: &AccountConfig) -> Account {
    let auth = auth_of(cfg);
    Account {
        mail: Box::new(ImapBackend::new(ImapConfig::new(&cfg.imap.host, cfg.imap.port, cfg.imap.security.clone(), auth.clone()))),
        outbound: Box::new(SmtpBackend::new(SmtpConfig::new(&cfg.smtp.host, cfg.smtp.port, cfg.smtp.security.clone(), auth))),
    }
}

struct Fresh {
    cfg: AccountConfig,
    store: Store,
    inner: Option<Account>,
}

impl Fresh {
    fn ready(&mut self) -> Result<&mut Account> {
        if let Credential::OAuth { provider, tokens } = &self.cfg.credential {
            if tokens.expires_at - now() < 120 {
                let client = oauth::client_for(*provider)
                    .ok_or_else(|| BackendError::Auth(oauth::missing_client_note(*provider)))?;
                let fresh = oauth::refresh(&oauth::endpoints(*provider), &client, &tokens.refresh_token)
                    .map_err(BackendError::Auth)?;
                self.cfg.credential = Credential::OAuth { provider: *provider, tokens: fresh };
                save(&self.store, &self.cfg)?;
                self.inner = None;
            }
        }
        if self.inner.is_none() {
            self.inner = Some(build_account(&self.cfg));
        }
        Ok(self.inner.as_mut().expect("just built"))
    }
}

impl Backend for Fresh {
    fn kind(&self) -> &'static str {
        "imap"
    }
    fn capabilities(&mut self) -> Result<Capabilities> {
        self.ready()?.capabilities()
    }
    fn check(&mut self) -> Result<()> {
        self.ready()?.check()
    }
    fn list_folders(&mut self) -> Result<Vec<Folder>> {
        self.ready()?.list_folders()
    }
    fn create_folder(&mut self, name: &str) -> Result<()> {
        self.ready()?.create_folder(name)
    }
    fn sync_folder(&mut self, folder: &str, since: Option<&SyncToken>) -> Result<FolderDelta> {
        self.ready()?.sync_folder(folder, since)
    }
    fn fetch(&mut self, folder: &str, uid: u32) -> Result<Vec<u8>> {
        self.ready()?.fetch(folder, uid)
    }
    fn set_flags(&mut self, folder: &str, uids: &[u32], update: &FlagUpdate) -> Result<()> {
        self.ready()?.set_flags(folder, uids, update)
    }
    fn move_messages(&mut self, folder: &str, uids: &[u32], destination: &str) -> Result<()> {
        self.ready()?.move_messages(folder, uids, destination)
    }
    fn delete_messages(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        self.ready()?.delete_messages(folder, uids)
    }
    fn search(&mut self, folder: &str, query: &str) -> Result<Vec<u32>> {
        self.ready()?.search(folder, query)
    }
    fn send(&mut self, message: &OutgoingMessage) -> Result<()> {
        self.ready()?.send(message)
    }
    fn watch(&mut self, folder: &str) -> Result<Watch> {
        self.ready()?.watch(folder)
    }
}

pub fn setup_for(cfg: &AccountConfig, store: &Store) -> Result<AccountSetup> {
    let backend: Box<dyn Backend> = match cfg.credential {
        Credential::Password(_) => Box::new(build_account(cfg)),
        Credential::OAuth { .. } => Box::new(Fresh { cfg: cfg.clone(), store: store.try_clone()?, inner: None }),
    };
    Ok(AccountSetup {
        id: cfg.id.clone(),
        name: cfg.name.clone(),
        address: cfg.address.clone(),
        sender: cfg.display_name.clone(),
        backend,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Imap,
    Smtp,
}

impl Step {
    pub const ALL: [Step; 2] = [Step::Imap, Step::Smtp];

    pub fn label(self) -> &'static str {
        match self {
            Step::Imap => "incoming mail (IMAP)",
            Step::Smtp => "outgoing mail (SMTP)",
        }
    }
}

pub fn run_checks(cfg: &AccountConfig, mut report: impl FnMut(Step, std::result::Result<(), String>)) {
    let mut cfg = cfg.clone();
    if let Credential::OAuth { provider, tokens } = &cfg.credential {
        if tokens.expires_at - now() < 60 {
            let fresh = oauth::client_for(*provider)
                .ok_or_else(|| oauth::missing_client_note(*provider))
                .and_then(|c| oauth::refresh(&oauth::endpoints(*provider), &c, &tokens.refresh_token));
            match fresh {
                Ok(t) => cfg.credential = Credential::OAuth { provider: *provider, tokens: t },
                Err(e) => {
                    report(Step::Imap, Err(e));
                    return;
                }
            }
        }
    }
    let mut account = build_account(&cfg);
    let imap = account.mail.check().map_err(|e| e.to_string());
    let imap_ok = imap.is_ok();
    report(Step::Imap, imap);
    if imap_ok {
        report(Step::Smtp, account.outbound.check().map_err(|e| e.to_string()));
    }
}

pub fn fix_hint(cfg: &AccountConfig, error: &str, oauth_offered: bool) -> String {
    let host = cfg.imap.host.to_lowercase();
    let lower = error.to_lowercase();
    let auth_problem = lower.contains("auth")
        || lower.contains("login")
        || lower.contains("password")
        || lower.contains("credentials")
        || lower.contains("invalid");
    if auth_problem {
        if host.contains("gmail") || host.contains("google") {
            return if oauth_offered {
                "Gmail needs an app password or Sign in with Google".into()
            } else {
                "Gmail needs an app password (Google account, Security, App passwords)".into()
            };
        }
        if host.contains("office365") || host.contains("outlook") || host.contains("hotmail") {
            return "Microsoft accounts need Sign in with Microsoft or an app password".into();
        }
        if host.contains("yahoo") || host.contains("icloud") || host.contains("me.com") || host.contains("aol") {
            return "This provider needs an app password, not your account password".into();
        }
        return "Check the user name and password. Some providers want the full address as the user name".into();
    }
    if lower.contains("certificate") || lower.contains("tls") || lower.contains("ssl") {
        return "The server certificate was not accepted. Check the host name and the security setting".into();
    }
    if lower.contains("connection") || lower.contains("refused") || lower.contains("resolve") || lower.contains("timed out") {
        return "Check the host, the port and the security setting, and that you are online".into();
    }
    "Check the server settings under edit server settings".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Security;

    fn cfg(host: &str) -> AccountConfig {
        AccountConfig {
            id: "1".into(),
            name: "n".into(),
            display_name: String::new(),
            signature: String::new(),
            address: "a@b.c".into(),
            username: "a@b.c".into(),
            imap: ServerSettings::new(host, 993, Security::Tls),
            smtp: ServerSettings::new(host, 465, Security::Tls),
            credential: Credential::Password("x".into()),
        }
    }

    #[test]
    fn gmail_auth_failure_points_to_app_passwords() {
        let hint = fix_hint(&cfg("imap.gmail.com"), "authentication failed: [AUTHENTICATIONFAILED] Invalid credentials", true);
        assert_eq!(hint, "Gmail needs an app password or Sign in with Google");
    }

    #[test]
    fn connection_failure_points_to_server_settings() {
        let hint = fix_hint(&cfg("mail.example.com"), "connection failed: Connection refused", false);
        assert!(hint.contains("host, the port"), "{hint}");
    }
}
