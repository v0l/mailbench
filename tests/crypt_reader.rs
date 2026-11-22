#![cfg(smime)]
use mailbench::crypt::*;
use mailbench::store::{KdfParams, Store};
use openssl::asn1::Asn1Time;
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkcs7::{Pkcs7, Pkcs7Flags};
use openssl::pkey::{PKey, Private};
use openssl::rsa::Rsa;
use openssl::stack::Stack;
use openssl::x509::{X509, X509NameBuilder};
use pgp::composed::{EncryptionCaps, KeyType, MessageBuilder, SecretKeyParamsBuilder, SignedPublicKey, SignedSecretKey, SubkeyParamsBuilder};
use pgp::crypto::ecc_curve::ECCCurve;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::ser::Serialize;
use rand::thread_rng;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    (dir, store)
}

fn pgp_pair(uid: &str, passphrase: Option<&str>) -> (SignedSecretKey, SignedPublicKey) {
    let mut enc = SubkeyParamsBuilder::default();
    enc.key_type(KeyType::ECDH(ECCCurve::Curve25519Legacy)).can_encrypt(EncryptionCaps::All);
    let mut params = SecretKeyParamsBuilder::default();
    params.key_type(KeyType::Ed25519Legacy).can_certify(true).can_sign(true).primary_user_id(uid.into()).subkeys(vec![enc.build().unwrap()]);
    if let Some(p) = passphrase {
        params.passphrase(Some(p.into()));
    }
    let secret = params.build().unwrap().generate(thread_rng()).unwrap();
    let public = SignedPublicKey::from(secret.clone());
    (secret, public)
}

fn armored_cipher(public: &SignedPublicKey, text: &[u8]) -> String {
    let mut builder = MessageBuilder::from_bytes("", text.to_vec()).seipd_v1(thread_rng(), SymmetricKeyAlgorithm::AES256);
    builder.encrypt_to_key(thread_rng(), &public.public_subkeys[0]).unwrap();
    builder.to_armored_string(thread_rng(), Default::default()).unwrap()
}

fn pgp_mime(armored: &str) -> Vec<u8> {
    format!(
        "From: Ann <ann@example.com>\r\nTo: bob@example.com\r\nSubject: hi\r\nMIME-Version: 1.0\r\nContent-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n\r\n--b\r\nContent-Type: application/octet-stream\r\n\r\n{armored}\r\n--b--\r\n"
    )
    .into_bytes()
}

#[test]
fn reader_opens_pgp_mime_encrypted_to_a_stored_key() {
    let (_dir, store) = store();
    let (secret, public) = pgp_pair("Ada <ada@example.com>", None);
    import_pgp(&store, &secret.to_bytes().unwrap(), "").unwrap();
    let raw = pgp_mime(&armored_cipher(&public, b"Content-Type: text/plain\r\n\r\nlunch at one\r\n"));
    match open_encrypted(&store, &raw, "") {
        Some(Opened::Plain(p)) => assert!(String::from_utf8_lossy(&as_entity(&p)).contains("lunch at one")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn reader_says_there_is_no_key_when_none_matches() {
    let (_dir, store) = store();
    let (_, public) = pgp_pair("Ada <ada@example.com>", None);
    let raw = pgp_mime(&armored_cipher(&public, b"hidden"));
    match open_encrypted(&store, &raw, "") {
        Some(Opened::Failed(why)) => assert!(why.contains("no imported key"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn reader_opens_inline_pgp_in_a_plain_mail() {
    let (_dir, store) = store();
    let (secret, public) = pgp_pair("Ada <ada@example.com>", None);
    import_pgp(&store, &secret.to_bytes().unwrap(), "").unwrap();
    let raw = format!("From: a@example.com\r\nSubject: x\r\nContent-Type: text/plain\r\n\r\n{}\r\n", armored_cipher(&public, b"inline words"));
    match open_encrypted(&store, raw.as_bytes(), "") {
        Some(Opened::Plain(p)) => assert_eq!(p, b"inline words"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn ordinary_mail_is_not_treated_as_encrypted() {
    let (_dir, store) = store();
    assert!(open_encrypted(&store, b"From: a@example.com\r\nSubject: x\r\n\r\nhello\r\n", "").is_none());
}

#[test]
fn locked_pgp_key_needs_its_passphrase_to_import() {
    let (_dir, store) = store();
    let (secret, public) = pgp_pair("Locked <l@example.com>", Some("hunter2"));
    let bytes = secret.to_bytes().unwrap();
    assert_eq!(import_pgp(&store, &bytes, "").err(), Some(CryptError::NeedsPassphrase));
    assert!(matches!(import_pgp(&store, &bytes, "wrong"), Err(CryptError::BadKey(_))));
    import_pgp(&store, &bytes, "hunter2").unwrap();
    let out = pgp_decrypt(&store, armored_cipher(&public, b"unlocked").as_bytes(), "").unwrap();
    assert_eq!(out.plain, b"unlocked");
}

#[test]
fn signed_mail_is_detected_for_smime_verify() {
    let key: PKey<Private> = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_nid(Nid::COMMONNAME, "Ada").unwrap();
    let name = name.build();
    let mut b = X509::builder().unwrap();
    b.set_version(2).unwrap();
    b.set_subject_name(&name).unwrap();
    b.set_issuer_name(&name).unwrap();
    b.set_pubkey(&key).unwrap();
    b.set_not_before(&Asn1Time::days_from_now(0).unwrap()).unwrap();
    b.set_not_after(&Asn1Time::days_from_now(2).unwrap()).unwrap();
    b.sign(&key, MessageDigest::sha256()).unwrap();
    let cert = b.build();
    let body = b"Content-Type: text/plain\r\n\r\nhello\r\n";
    let certs = Stack::new().unwrap();
    let smime = Pkcs7::sign(&cert, &key, &certs, body, Pkcs7Flags::DETACHED).unwrap().to_smime(body, Pkcs7Flags::DETACHED).unwrap();
    let raw = [b"From: ada@example.com\r\nSubject: s\r\n".to_vec(), smime].concat();
    assert!(signed_smime(&raw).is_some());
    assert!(signed_smime(b"From: a@b.c\r\n\r\nhi").is_none());
    let verdict = smime_verify_default(&raw).unwrap();
    assert!(verdict.is_alarm());
    assert!(verdict.text().contains("not trusted"));
}
