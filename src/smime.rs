use crate::crypt::{CryptError, KeyInfo, SMIME_KIND, SignerCert, SmimeState, SmimeVerdict};
use crate::store::Store;
use openssl::asn1::Asn1Time;
use openssl::hash::MessageDigest;
use openssl::pkcs12::Pkcs12;
use openssl::pkcs7::{Pkcs7, Pkcs7Flags};
use openssl::pkey::PKey;
use openssl::ssl::SslFiletype;
use openssl::stack::Stack;
use openssl::x509::store::{X509Lookup, X509Store, X509StoreBuilder};
use openssl::x509::{X509, X509NameRef};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Serialize, Deserialize)]
struct SmimeRecord {
    key_pem: String,
    cert_pem: String,
    #[serde(default)]
    chain_pem: Vec<String>,
}

fn store_err(e: impl std::fmt::Debug) -> CryptError {
    CryptError::Store(format!("{e:?}"))
}

pub fn label_of(raw: &[u8]) -> String {
    serde_json::from_slice::<SmimeRecord>(raw)
        .ok()
        .and_then(|r| X509::from_pem(r.cert_pem.as_bytes()).ok())
        .map(|c| name_text(c.subject_name()))
        .unwrap_or_default()
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

fn parse_pkcs7(data: &[u8]) -> Result<(Pkcs7, Option<Vec<u8>>), CryptError> {
    Pkcs7::from_smime(data)
        .or_else(|_| Pkcs7::from_der(data).map(|p| (p, None)))
        .map_err(|e| CryptError::BadMessage(e.to_string()))
}

pub fn smime_decrypt(store: &Store, data: &[u8]) -> Result<Vec<u8>, CryptError> {
    let (pkcs7, _) = parse_pkcs7(data)?;
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

pub fn cached_trust_store() -> Result<&'static X509Store, CryptError> {
    static STORE: OnceLock<Result<X509Store, CryptError>> = OnceLock::new();
    STORE.get_or_init(os_trust_store).as_ref().map_err(|e| e.clone())
}

pub fn trust_store_with(roots: &[X509]) -> Result<X509Store, CryptError> {
    let mut builder = X509StoreBuilder::new().map_err(store_err)?;
    for r in roots {
        builder.add_cert(r.clone()).map_err(store_err)?;
    }
    Ok(builder.build())
}

pub fn smime_verify_default(data: &[u8]) -> Result<SmimeVerdict, CryptError> {
    smime_verify(data, cached_trust_store()?)
}

pub fn smime_verify(data: &[u8], trust: &X509Store) -> Result<SmimeVerdict, CryptError> {
    let (pkcs7, detached) = parse_pkcs7(data)?;
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
