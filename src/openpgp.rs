use base64::Engine;
use mail_parser::MessageParser;
use pgp::composed::{CleartextSignedMessage, DetachedSignature, Deserializable, SignedPublicKey};
use pgp::packet::Signature;
use pgp::types::KeyDetails;
use std::io::Cursor;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PgpState {
    Good,
    Bad,
    UnknownKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PgpStatus {
    pub state: PgpState,
    pub signer: Option<String>,
    pub fingerprint: String,
}

impl PgpStatus {
    pub fn short_fingerprint(&self) -> String {
        let f = &self.fingerprint;
        let tail = &f[f.len().saturating_sub(16)..];
        tail.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join(" ")
    }

    pub fn text(&self) -> String {
        match self.state {
            PgpState::Good => match &self.signer {
                Some(s) => format!("signed by {s} {}", self.short_fingerprint()),
                None => format!("signed {}", self.short_fingerprint()),
            },
            PgpState::Bad => format!("bad signature {}", self.short_fingerprint()),
            PgpState::UnknownKey => format!("signed, key unknown {}", self.short_fingerprint()),
        }
    }

    pub fn is_alarm(&self) -> bool {
        self.state == PgpState::Bad
    }
}

fn parse_key(data: &[u8]) -> Option<SignedPublicKey> {
    let text = String::from_utf8_lossy(data);
    if text.contains("-----BEGIN PGP") {
        SignedPublicKey::from_string(&text).ok().map(|(k, _)| k)
    } else {
        SignedPublicKey::from_bytes(Cursor::new(data)).ok()
    }
}

pub fn embedded_keys(raw: &[u8]) -> Vec<SignedPublicKey> {
    let Some(msg) = MessageParser::default().parse(raw) else {
        return Vec::new();
    };
    let mut keys = Vec::new();
    for part in msg.parts() {
        if part.is_content_type("application", "pgp-keys") {
            keys.extend(parse_key(&part.decoded()));
        }
    }
    for h in msg.root_part().headers() {
        if h.name().as_str().eq_ignore_ascii_case("Autocrypt") {
            let value = String::from_utf8_lossy(h.raw_value()).into_owned();
            let data = value.split_once("keydata=").map(|(_, d)| d.split(';').next().unwrap_or("").split_whitespace().collect::<String>());
            if let Some(bytes) = data.and_then(|d| base64::engine::general_purpose::STANDARD.decode(d).ok()) {
                keys.extend(parse_key(&bytes));
            }
        }
    }
    keys
}

fn crlf(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16);
    let mut prev = 0u8;
    for &b in data {
        if b == b'\n' && prev != b'\r' {
            out.push(b'\r');
        }
        out.push(b);
        prev = b;
    }
    out
}

fn signer_name(key: &SignedPublicKey) -> Option<String> {
    let user = key.details.users.iter().find(|u| u.is_primary()).or_else(|| key.details.users.first())?;
    user.id.as_str().map(|s| s.to_string())
}

fn issuer_matches(sig: &Signature, key: &SignedPublicKey) -> bool {
    let fp_match = sig.issuer_fingerprint().iter().any(|f| {
        **f == key.primary_key.fingerprint() || key.public_subkeys.iter().any(|s| **f == s.fingerprint())
    });
    let id_match = sig.issuer_key_id().iter().any(|i| {
        **i == key.primary_key.legacy_key_id() || key.public_subkeys.iter().any(|s| **i == s.legacy_key_id())
    });
    fp_match || id_match
}

fn issuer_label(sig: &Signature) -> String {
    if let Some(f) = sig.issuer_fingerprint().first() {
        return f.to_string().to_uppercase();
    }
    sig.issuer_key_id().first().map(|i| format!("{i:?}").to_uppercase()).unwrap_or_default()
}

fn judge(sig: &Signature, keys: &[SignedPublicKey], check: &dyn Fn(&SignedPublicKey) -> bool) -> PgpStatus {
    let candidates: Vec<&SignedPublicKey> = keys.iter().filter(|k| issuer_matches(sig, k)).collect();
    if candidates.is_empty() {
        return PgpStatus { state: PgpState::UnknownKey, signer: None, fingerprint: issuer_label(sig) };
    }
    for key in &candidates {
        if check(key) {
            return PgpStatus {
                state: PgpState::Good,
                signer: signer_name(key),
                fingerprint: key.primary_key.fingerprint().to_string().to_uppercase(),
            };
        }
    }
    let key = candidates[0];
    PgpStatus { state: PgpState::Bad, signer: signer_name(key), fingerprint: key.primary_key.fingerprint().to_string().to_uppercase() }
}

fn verify_detached(signed: &[u8], sig_bytes: &[u8], keys: &[SignedPublicKey]) -> Option<PgpStatus> {
    let text = String::from_utf8_lossy(sig_bytes);
    let sig = if text.contains("-----BEGIN PGP") {
        DetachedSignature::from_string(&text).ok()?.0
    } else {
        DetachedSignature::from_bytes(Cursor::new(sig_bytes)).ok()?
    };
    let data = crlf(signed);
    Some(judge(&sig.signature, keys, &|k| {
        sig.signature.verify(&k.primary_key, data.as_slice()).is_ok()
            || k.public_subkeys.iter().any(|s| sig.signature.verify(s, data.as_slice()).is_ok())
    }))
}

fn verify_inline(text: &str, keys: &[SignedPublicKey]) -> Option<PgpStatus> {
    let start = text.find("-----BEGIN PGP SIGNED MESSAGE-----")?;
    let (msg, _) = CleartextSignedMessage::from_string(&text[start..]).ok()?;
    let sig = msg.signatures().first()?;
    Some(judge(sig, keys, &|k| msg.verify(&k.primary_key).is_ok() || k.public_subkeys.iter().any(|s| msg.verify(s).is_ok())))
}

pub fn verify(raw: &[u8], extra_keys: &[SignedPublicKey]) -> Option<PgpStatus> {
    let msg = MessageParser::default().parse(raw)?;
    let mut keys = embedded_keys(raw);
    keys.extend(extra_keys.iter().cloned());
    let signed_part = msg.parts().find(|p| {
        p.content_type().is_some_and(|c| {
            c.ctype().eq_ignore_ascii_case("multipart")
                && c.subtype().is_some_and(|s| s.eq_ignore_ascii_case("signed"))
                && c.attribute("protocol").is_some_and(|p| p.eq_ignore_ascii_case("application/pgp-signature"))
        })
    });
    if let Some(root) = signed_part {
        let children: Vec<_> = root.children().collect();
        if let [content, signature, ..] = children.as_slice() {
            return verify_detached(content.raw(), &signature.decoded(), &keys);
        }
        return None;
    }
    let text = msg.text_bodies().map(|t| t.into_owned()).collect::<Vec<_>>().join("\n");
    verify_inline(&text, &keys)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pgp::composed::{KeyType, SecretKeyParamsBuilder, SignedSecretKey};
    use pgp::crypto::hash::HashAlgorithm;
    use pgp::types::Password;
    use pgp::composed::ArmorOptions;
    use rand::thread_rng;

    fn keypair(uid: &str) -> (SignedSecretKey, SignedPublicKey) {
        let params = SecretKeyParamsBuilder::default()
            .key_type(KeyType::Ed25519Legacy)
            .can_certify(true)
            .can_sign(true)
            .primary_user_id(uid.into())
            .build()
            .unwrap();
        let secret = params.generate(thread_rng()).unwrap();
        let public = secret.to_public_key();
        (secret, public)
    }

    fn armored_key(public: &SignedPublicKey) -> String {
        public.to_armored_string(ArmorOptions::default()).unwrap()
    }

    fn mime(content: &str, signature: &str, key: Option<&str>) -> Vec<u8> {
        let mut out = String::new();
        out.push_str("From: Ann <ann@example.com>\r\nTo: bob@example.com\r\nSubject: hi\r\nMIME-Version: 1.0\r\n");
        out.push_str("Content-Type: multipart/mixed; boundary=\"outer\"\r\n\r\n--outer\r\n");
        out.push_str("Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; micalg=pgp-sha256; boundary=\"b1\"\r\n\r\n");
        out.push_str("--b1\r\n");
        out.push_str(content);
        out.push_str("\r\n--b1\r\nContent-Type: application/pgp-signature\r\n\r\n");
        out.push_str(signature);
        out.push_str("\r\n--b1--\r\n");
        if let Some(k) = key {
            out.push_str("--outer\r\nContent-Type: application/pgp-keys\r\n\r\n");
            out.push_str(k);
            out.push_str("\r\n");
        }
        out.push_str("--outer--\r\n");
        out.into_bytes()
    }

    const CONTENT: &str = "Content-Type: text/plain; charset=utf-8\r\n\r\nthe deploy window is friday\r\n";

    fn sign(secret: &SignedSecretKey, content: &str) -> String {
        DetachedSignature::sign_binary_data(thread_rng(), &secret.primary_key, &Password::empty(), HashAlgorithm::Sha256, content.as_bytes())
            .unwrap()
            .to_armored_string(ArmorOptions::default())
            .unwrap()
    }

    #[test]
    fn pgp_mime_signature_verifies_with_embedded_key() {
        let (secret, public) = keypair("Ann <ann@example.com>");
        let raw = mime(CONTENT, &sign(&secret, CONTENT), Some(&armored_key(&public)));
        let status = verify(&raw, &[]).unwrap();
        assert_eq!(status.state, PgpState::Good);
        assert_eq!(status.signer.as_deref(), Some("Ann <ann@example.com>"));
        assert_eq!(status.fingerprint, public.primary_key.fingerprint().to_string().to_uppercase());
        assert!(status.text().starts_with("signed by Ann <ann@example.com> "));
    }

    #[test]
    fn pgp_mime_tampered_body_is_bad() {
        let (secret, public) = keypair("Ann <ann@example.com>");
        let tampered = CONTENT.replace("friday", "monday");
        let raw = mime(&tampered, &sign(&secret, CONTENT), Some(&armored_key(&public)));
        let status = verify(&raw, &[]).unwrap();
        assert_eq!(status.state, PgpState::Bad);
        assert!(status.is_alarm());
    }

    #[test]
    fn pgp_mime_without_key_is_unknown_key_with_fingerprint() {
        let (secret, public) = keypair("Ann <ann@example.com>");
        let raw = mime(CONTENT, &sign(&secret, CONTENT), None);
        let status = verify(&raw, &[]).unwrap();
        assert_eq!(status.state, PgpState::UnknownKey);
        assert!(!status.fingerprint.is_empty());
        assert!(public.primary_key.fingerprint().to_string().to_uppercase().ends_with(&status.fingerprint[status.fingerprint.len() - 16..]));
        let with_key = verify(&raw, &[public]).unwrap();
        assert_eq!(with_key.state, PgpState::Good);
    }

    #[test]
    fn inline_cleartext_signature_verifies() {
        let (secret, public) = keypair("Ann <ann@example.com>");
        let signed = CleartextSignedMessage::sign(thread_rng(), "meet at noon\n", &secret.primary_key, &Password::empty())
            .unwrap()
            .to_armored_string(ArmorOptions::default())
            .unwrap();
        let raw = format!("From: ann@example.com\r\nSubject: x\r\nContent-Type: text/plain\r\n\r\n{signed}");
        let status = verify(raw.as_bytes(), &[public.clone()]).unwrap();
        assert_eq!(status.state, PgpState::Good);
        let forged = raw.replace("noon", "nine");
        assert_eq!(verify(forged.as_bytes(), &[public]).unwrap().state, PgpState::Bad);
    }

    #[test]
    fn unsigned_message_has_no_status() {
        let raw = b"From: a@b.c\r\nSubject: x\r\n\r\nhello\r\n";
        assert!(verify(raw, &[]).is_none());
    }

    #[test]
    fn short_fingerprint_groups_last_sixteen() {
        let s = PgpStatus { state: PgpState::Good, signer: None, fingerprint: "AABBCCDDEEFF00112233445566778899AABBCCDD".into() };
        assert_eq!(s.short_fingerprint(), "6677 8899 AABB CCDD");
    }
}
