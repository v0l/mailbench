use crate::store::Store;
use pgp::composed::{Deserializable, Message, SignedSecretKey};
use pgp::ser::Serialize;
use pgp::types::{KeyDetails, Password};
use std::io::Cursor;

#[cfg(smime)]
pub use crate::smime::{cached_trust_store, import_pkcs12, os_trust_store, smime_decrypt, smime_verify, smime_verify_default, trust_store_with};
#[cfg(not(smime))]
pub use crate::smime_off::{import_pkcs12, smime_decrypt, smime_verify_default};

pub const PGP_KIND: &str = "pgpkey";
pub const SMIME_KIND: &str = "smimekey";

#[derive(Debug, Clone, PartialEq)]
pub enum CryptError {
    BadKey(String),
    NoKey,
    NeedsPassphrase,
    BadMessage(String),
    Store(String),
    Unsupported,
}

impl std::fmt::Display for CryptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptError::BadKey(e) => write!(f, "could not read the key: {e}"),
            CryptError::NoKey => write!(f, "no imported key can open this message"),
            CryptError::NeedsPassphrase => write!(f, "the key needs its passphrase"),
            CryptError::BadMessage(e) => write!(f, "could not read the message: {e}"),
            CryptError::Store(e) => write!(f, "the encrypted store failed: {e}"),
            CryptError::Unsupported => write!(f, "S/MIME is not available in this build"),
        }
    }
}

impl std::error::Error for CryptError {}

pub(crate) fn store_err(e: impl std::fmt::Debug) -> CryptError {
    CryptError::Store(format!("{e:?}"))
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyInfo {
    pub kind: &'static str,
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SignerCert {
    pub subject: String,
    pub issuer: String,
    pub emails: Vec<String>,
    pub fingerprint: String,
    pub not_before: String,
    pub not_after: String,
    pub expired: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SmimeState {
    Trusted,
    Untrusted(String),
    BadSignature(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SmimeVerdict {
    pub state: SmimeState,
    pub signer: Option<SignerCert>,
    pub content: Option<Vec<u8>>,
}

impl SmimeVerdict {
    pub fn text(&self) -> String {
        let who = self.signer.as_ref().map(|s| s.emails.first().cloned().unwrap_or_else(|| s.subject.clone())).unwrap_or_default();
        match &self.state {
            SmimeState::Trusted => format!("S/MIME signed by {who}"),
            SmimeState::Untrusted(_) => format!("S/MIME signed by {who}, certificate not trusted"),
            SmimeState::BadSignature(_) => "S/MIME signature is bad".to_string(),
        }
    }

    pub fn is_alarm(&self) -> bool {
        !matches!(self.state, SmimeState::Trusted)
    }

    pub fn detail_lines(&self) -> Vec<(String, String)> {
        let Some(s) = &self.signer else { return Vec::new() };
        let mut out = vec![("signer".to_string(), s.subject.clone()), ("issuer".to_string(), s.issuer.clone())];
        if !s.emails.is_empty() {
            out.push(("email".to_string(), s.emails.join(", ")));
        }
        out.push(("sha-256".to_string(), s.fingerprint.clone()));
        out.push(("valid".to_string(), format!("{} to {}{}", s.not_before, s.not_after, if s.expired { " (expired)" } else { "" })));
        out
    }
}

pub fn list_keys(store: &Store) -> Result<Vec<KeyInfo>, CryptError> {
    let mut out = Vec::new();
    for id in store.list(PGP_KIND).map_err(store_err)? {
        if let Some(raw) = store.get(PGP_KIND, &id).map_err(store_err)? {
            let label = pgp_key(&raw).map(|k| pgp_label(&k)).unwrap_or_default();
            out.push(KeyInfo { kind: PGP_KIND, id, label });
        }
    }
    for id in store.list(SMIME_KIND).map_err(store_err)? {
        if let Some(raw) = store.get(SMIME_KIND, &id).map_err(store_err)? {
            out.push(KeyInfo { kind: SMIME_KIND, id, label: smime_label(&raw) });
        }
    }
    Ok(out)
}

#[cfg(smime)]
fn smime_label(raw: &[u8]) -> String {
    crate::smime::label_of(raw)
}

#[cfg(not(smime))]
fn smime_label(_: &[u8]) -> String {
    String::new()
}

pub fn remove_key(store: &Store, kind: &str, id: &str) -> Result<(), CryptError> {
    store.delete(kind, id).map_err(store_err)
}

fn pgp_key(bytes: &[u8]) -> Result<SignedSecretKey, CryptError> {
    SignedSecretKey::from_reader_single(Cursor::new(bytes)).map(|(k, _)| k).map_err(|e| CryptError::BadKey(e.to_string()))
}

fn pgp_label(key: &SignedSecretKey) -> String {
    key.details.users.first().map(|u| String::from_utf8_lossy(u.id.id()).into_owned()).unwrap_or_default()
}

pub fn import_pgp(store: &Store, data: &[u8], passphrase: &str) -> Result<KeyInfo, CryptError> {
    let mut key = pgp_key(data)?;
    if key_is_locked(&key) {
        if passphrase.is_empty() {
            return Err(CryptError::NeedsPassphrase);
        }
        let pw = Password::from(passphrase);
        let bad = |e: pgp::errors::Error| CryptError::BadKey(format!("the passphrase did not unlock the key ({e})"));
        key.primary_key.remove_password(&pw).map_err(bad)?;
        for sub in &mut key.secret_subkeys {
            sub.key.remove_password(&pw).map_err(bad)?;
        }
    }
    let id = key.fingerprint().to_string().to_lowercase();
    let bytes = key.to_bytes().map_err(|e| CryptError::BadKey(e.to_string()))?;
    store.put(PGP_KIND, &id, &bytes).map_err(store_err)?;
    Ok(KeyInfo { kind: PGP_KIND, id, label: pgp_label(&key) })
}

pub struct PgpPlain {
    pub plain: Vec<u8>,
    pub key_id: String,
}

fn pgp_message(data: &[u8]) -> Result<Message<'_>, CryptError> {
    let armored = data.windows(10).take(512).any(|w| w == b"-----BEGIN");
    let parsed = if armored { Message::from_armor(data).map(|(m, _)| m) } else { Message::from_bytes(data) };
    parsed.map_err(|e| CryptError::BadMessage(e.to_string()))
}

pub fn pgp_decrypt(store: &Store, data: &[u8], passphrase: &str) -> Result<PgpPlain, CryptError> {
    let password = Password::from(passphrase);
    let mut locked = false;
    for id in store.list(PGP_KIND).map_err(store_err)? {
        let Some(raw) = store.get(PGP_KIND, &id).map_err(store_err)? else { continue };
        let Ok(key) = pgp_key(&raw) else { continue };
        let message = pgp_message(data)?;
        if !message.is_encrypted() {
            return Err(CryptError::BadMessage("the message is not encrypted".into()));
        }
        let Ok(mut opened) = message.decrypt(&password, &key) else {
            locked |= passphrase.is_empty() && key_is_locked(&key);
            continue;
        };
        if opened.is_compressed() {
            opened = opened.decompress().map_err(|e| CryptError::BadMessage(e.to_string()))?;
        }
        let plain = opened.as_data_vec().map_err(|e| CryptError::BadMessage(e.to_string()))?;
        return Ok(PgpPlain { plain, key_id: id });
    }
    if locked { Err(CryptError::NeedsPassphrase) } else { Err(CryptError::NoKey) }
}

fn key_is_locked(key: &SignedSecretKey) -> bool {
    key.primary_key.secret_params().is_encrypted() || key.secret_subkeys.iter().any(|s| s.key.secret_params().is_encrypted())
}

pub fn pgp_armored(data: &[u8]) -> bool {
    data.windows(27).any(|w| w == b"-----BEGIN PGP MESSAGE-----")
}

pub fn encrypted_kind(raw: &[u8]) -> Option<EncryptedKind> {
    let msg = mail_parser::MessageParser::default().parse(raw)?;
    let ctype = |p: &mail_parser::MessagePart, t: &str, s: &str| p.is_content_type(t, s);
    let root = msg.root_part();
    if let Some(ct) = root.content_type() {
        let sub = ct.subtype().unwrap_or("").to_ascii_lowercase();
        if ct.ctype().eq_ignore_ascii_case("multipart")
            && sub == "encrypted"
            && ct.attribute("protocol").is_some_and(|p| p.eq_ignore_ascii_case("application/pgp-encrypted"))
        {
            let payload = msg.parts().find(|p| ctype(p, "application", "octet-stream"))?;
            return Some(EncryptedKind::Pgp(payload.decoded().to_vec()));
        }
        if ct.ctype().eq_ignore_ascii_case("application") && (sub == "pkcs7-mime" || sub == "x-pkcs7-mime") {
            let smime_type = ct.attribute("smime-type").unwrap_or("").to_ascii_lowercase();
            if smime_type == "signed-data" {
                return None;
            }
            return Some(EncryptedKind::Smime(root.decoded().to_vec()));
        }
    }
    let text = msg.text_bodies().map(|t| t.into_owned()).collect::<Vec<_>>().join("\n");
    let start = text.find("-----BEGIN PGP MESSAGE-----")?;
    Some(EncryptedKind::Pgp(text[start..].as_bytes().to_vec()))
}

#[derive(Debug, Clone, PartialEq)]
pub enum EncryptedKind {
    Pgp(Vec<u8>),
    Smime(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Opened {
    Plain(Vec<u8>),
    Failed(String),
}

pub fn open_encrypted(store: &Store, raw: &[u8], passphrase: &str) -> Option<Opened> {
    let kind = encrypted_kind(raw)?;
    let result = match kind {
        EncryptedKind::Pgp(data) => pgp_decrypt(store, &data, passphrase).map(|p| p.plain),
        EncryptedKind::Smime(data) => smime_decrypt(store, &data),
    };
    Some(match result {
        Ok(plain) => Opened::Plain(plain),
        Err(e) => Opened::Failed(e.to_string()),
    })
}

pub fn as_entity(plain: &[u8]) -> Vec<u8> {
    let head = String::from_utf8_lossy(&plain[..plain.len().min(64)]).to_ascii_lowercase();
    if head.starts_with("content-") || head.starts_with("mime-version") {
        plain.to_vec()
    } else {
        let mut out = b"Content-Type: text/plain; charset=utf-8\r\n\r\n".to_vec();
        out.extend_from_slice(plain);
        out
    }
}

pub fn signed_smime(raw: &[u8]) -> Option<Vec<u8>> {
    let msg = mail_parser::MessageParser::default().parse(raw)?;
    let ct = msg.root_part().content_type()?;
    let sub = ct.subtype().unwrap_or("").to_ascii_lowercase();
    let multipart_signed = ct.ctype().eq_ignore_ascii_case("multipart")
        && sub == "signed"
        && ct.attribute("protocol").is_some_and(|p| p.to_ascii_lowercase().ends_with("pkcs7-signature"));
    let opaque = ct.ctype().eq_ignore_ascii_case("application")
        && (sub == "pkcs7-mime" || sub == "x-pkcs7-mime")
        && ct.attribute("smime-type").is_some_and(|t| t.eq_ignore_ascii_case("signed-data"));
    (multipart_signed || opaque).then(|| raw.to_vec())
}
