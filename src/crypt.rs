use crate::store::Store;
use openssl::asn1::Asn1Time;
use openssl::hash::MessageDigest;
use openssl::pkcs12::Pkcs12;
use openssl::pkcs7::{Pkcs7, Pkcs7Flags};
use openssl::pkey::PKey;
use openssl::stack::Stack;
use openssl::ssl::SslFiletype;
use openssl::x509::store::{X509Lookup, X509Store, X509StoreBuilder};
use openssl::x509::{X509, X509NameRef};
use pgp::composed::{Deserializable, Message, SignedSecretKey};
use pgp::ser::Serialize;
use pgp::types::{KeyDetails, Password};
use serde::{Deserialize, Serialize as SerdeSerialize};
use std::io::Cursor;

pub const PGP_KIND: &str = "pgpkey";
pub const SMIME_KIND: &str = "smimekey";

#[derive(Debug, PartialEq)]
pub enum CryptError {
    BadKey(String),
    NoKey,
    BadMessage(String),
    Store(String),
}

impl std::fmt::Display for CryptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptError::BadKey(e) => write!(f, "could not read the key: {e}"),
            CryptError::NoKey => write!(f, "none of the imported keys can open this message"),
            CryptError::BadMessage(e) => write!(f, "could not read the message: {e}"),
            CryptError::Store(e) => write!(f, "the encrypted store failed: {e}"),
        }
    }
}

impl std::error::Error for CryptError {}

fn store_err(e: impl std::fmt::Debug) -> CryptError {
    CryptError::Store(format!("{e:?}"))
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyInfo {
    pub kind: &'static str,
    pub id: String,
    pub label: String,
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
            let label = serde_json::from_slice::<SmimeRecord>(&raw)
                .ok()
                .and_then(|r| X509::from_pem(r.cert_pem.as_bytes()).ok())
                .map(|c| name_text(c.subject_name()))
                .unwrap_or_default();
            out.push(KeyInfo { kind: SMIME_KIND, id, label });
        }
    }
    Ok(out)
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

pub fn import_pgp(store: &Store, data: &[u8]) -> Result<KeyInfo, CryptError> {
    let key = pgp_key(data)?;
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
    let mut saw_key = false;
    for id in store.list(PGP_KIND).map_err(store_err)? {
        let Some(raw) = store.get(PGP_KIND, &id).map_err(store_err)? else { continue };
        let Ok(key) = pgp_key(&raw) else { continue };
        saw_key = true;
        let message = pgp_message(data)?;
        if !message.is_encrypted() {
            return Err(CryptError::BadMessage("the message is not encrypted".into()));
        }
        let Ok(mut opened) = message.decrypt(&password, &key) else { continue };
        if opened.is_compressed() {
            opened = opened.decompress().map_err(|e| CryptError::BadMessage(e.to_string()))?;
        }
        let plain = opened.as_data_vec().map_err(|e| CryptError::BadMessage(e.to_string()))?;
        return Ok(PgpPlain { plain, key_id: id });
    }
    let _ = saw_key;
    Err(CryptError::NoKey)
}

#[derive(SerdeSerialize, Deserialize)]
struct SmimeRecord {
    key_pem: String,
    cert_pem: String,
    #[serde(default)]
    chain_pem: Vec<String>,
}

pub fn import_pkcs12(store: &Store, der: &[u8], password: &str) -> Result<KeyInfo, CryptError> {
    let bad = |e: openssl::error::ErrorStack| CryptError::BadKey(e.to_string());
    let parsed = Pkcs12::from_der(der).map_err(bad)?.parse2(password).map_err(bad)?;
    let key = parsed.pkey.ok_or_else(|| CryptError::BadKey("the file has no private key".into()))?;
    let cert = parsed.cert.ok_or_else(|| CryptError::BadKey("the file has no certificate".into()))?;
    let mut chain_pem = Vec::new();
    if let Some(ca) = parsed.ca {
        for c in ca.iter() {
            chain_pem.push(String::from_utf8_lossy(&c.to_pem().map_err(bad)?).into_owned());
        }
    }
    let record = SmimeRecord {
        key_pem: String::from_utf8_lossy(&key.private_key_to_pem_pkcs8().map_err(bad)?).into_owned(),
        cert_pem: String::from_utf8_lossy(&cert.to_pem().map_err(bad)?).into_owned(),
        chain_pem,
    };
    let id = fingerprint_text(&cert);
    let bytes = serde_json::to_vec(&record).map_err(store_err)?;
    store.put(SMIME_KIND, &id, &bytes).map_err(store_err)?;
    Ok(KeyInfo { kind: SMIME_KIND, id, label: name_text(cert.subject_name()) })
}

pub fn smime_decrypt(store: &Store, data: &[u8]) -> Result<Vec<u8>, CryptError> {
    let (pkcs7, _) = Pkcs7::from_smime(data).or_else(|_| Pkcs7::from_der(data).map(|p| (p, None))).map_err(|e| CryptError::BadMessage(e.to_string()))?;
    for id in store.list(SMIME_KIND).map_err(store_err)? {
        let Some(raw) = store.get(SMIME_KIND, &id).map_err(store_err)? else { continue };
        let Ok(record) = serde_json::from_slice::<SmimeRecord>(&raw) else { continue };
        let (Ok(key), Ok(cert)) = (PKey::private_key_from_pem(record.key_pem.as_bytes()), X509::from_pem(record.cert_pem.as_bytes())) else {
            continue;
        };
        if let Ok(plain) = pkcs7.decrypt(&key, &cert, Pkcs7Flags::empty()) {
            return Ok(plain);
        }
    }
    Err(CryptError::NoKey)
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

pub fn os_trust_store() -> Result<X509Store, CryptError> {
    let mut builder = X509StoreBuilder::new().map_err(store_err)?;
    let probe = openssl_probe::probe();
    if let Some(file) = &probe.cert_file {
        if let Ok(lookup) = builder.add_lookup(X509Lookup::file()) {
            let _ = lookup.load_cert_file(file, SslFiletype::PEM);
        }
    }
    if let Some(dir) = &probe.cert_dir {
        if let (Ok(lookup), Some(d)) = (builder.add_lookup(X509Lookup::hash_dir()), dir.to_str()) {
            let _ = lookup.add_dir(d, SslFiletype::PEM);
        }
    }
    builder.set_default_paths().map_err(store_err)?;
    Ok(builder.build())
}

pub fn smime_verify(data: &[u8], trust: &X509Store) -> Result<SmimeVerdict, CryptError> {
    let (pkcs7, detached) = Pkcs7::from_smime(data).map_err(|e| CryptError::BadMessage(e.to_string()))?;
    let no_certs = Stack::<X509>::new().map_err(store_err)?;
    let signer = pkcs7
        .signers(&no_certs, Pkcs7Flags::empty())
        .ok()
        .and_then(|s| s.iter().next().map(|c| c.to_owned()))
        .map(|c| describe_cert(&c));
    let mut content = Vec::new();
    let strict = pkcs7.verify(&no_certs, trust, detached.as_deref(), Some(&mut content), Pkcs7Flags::empty());
    let state = match strict {
        Ok(()) => SmimeState::Trusted,
        Err(strict_err) => {
            let mut loose = Vec::new();
            match pkcs7.verify(&no_certs, trust, detached.as_deref(), Some(&mut loose), Pkcs7Flags::NOVERIFY) {
                Ok(()) => {
                    content = loose;
                    SmimeState::Untrusted(strict_err.to_string())
                }
                Err(e) => SmimeState::BadSignature(e.to_string()),
            }
        }
    };
    let content = if matches!(state, SmimeState::BadSignature(_)) { None } else { Some(content) };
    Ok(SmimeVerdict { state, signer, content })
}

pub fn trust_store_with(roots: &[X509]) -> Result<X509Store, CryptError> {
    let mut builder = X509StoreBuilder::new().map_err(store_err)?;
    for r in roots {
        builder.add_cert(r.clone()).map_err(store_err)?;
    }
    Ok(builder.build())
}

fn describe_cert(c: &X509) -> SignerCert {
    let emails = c.subject_alt_names().map(|n| n.iter().filter_map(|g| g.email().map(str::to_string)).collect()).unwrap_or_default();
    let expired = Asn1Time::days_from_now(0).map(|now| c.not_after() < now).unwrap_or(false);
    SignerCert {
        subject: name_text(c.subject_name()),
        issuer: name_text(c.issuer_name()),
        emails,
        fingerprint: fingerprint_text(c),
        not_before: c.not_before().to_string(),
        not_after: c.not_after().to_string(),
        expired,
    }
}

fn name_text(n: &X509NameRef) -> String {
    n.entries()
        .filter_map(|e| {
            let key = e.object().nid().short_name().ok()?;
            let value = e.data().as_utf8().ok()?.to_string();
            Some(format!("{key}={value}"))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn fingerprint_text(c: &X509) -> String {
    c.digest(MessageDigest::sha256()).map(|d| d.iter().map(|b| format!("{b:02x}")).collect()).unwrap_or_default()
}
