#![cfg(smime)]
use mailbench::crypt::*;
use mailbench::store::{KdfParams, Store};
use openssl::asn1::Asn1Time;
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkcs12::Pkcs12;
use openssl::pkcs7::{Pkcs7, Pkcs7Flags};
use openssl::pkey::{PKey, Private};
use openssl::rsa::Rsa;
use openssl::stack::Stack;
use openssl::x509::extension::{BasicConstraints, SubjectAlternativeName};
use openssl::x509::{X509, X509NameBuilder};
use pgp::composed::{Deserializable, EncryptionCaps, KeyType, MessageBuilder, SecretKeyParamsBuilder, SignedPublicKey, SignedSecretKey, SubkeyParamsBuilder};
use pgp::crypto::ecc_curve::ECCCurve;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::ser::Serialize;
use rand::thread_rng;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    (dir, store)
}

fn pgp_pair(uid: &str) -> (SignedSecretKey, SignedPublicKey) {
    let mut enc = SubkeyParamsBuilder::default();
    enc.key_type(KeyType::ECDH(ECCCurve::Curve25519Legacy)).can_encrypt(EncryptionCaps::All);
    let mut params = SecretKeyParamsBuilder::default();
    params
        .key_type(KeyType::Ed25519Legacy)
        .can_certify(true)
        .can_sign(true)
        .primary_user_id(uid.into())
        .subkeys(vec![enc.build().unwrap()]);
    let secret = params.build().unwrap().generate(thread_rng()).unwrap();
    let public = SignedPublicKey::from(secret.clone());
    (secret, public)
}

fn pgp_encrypt(public: &SignedPublicKey, text: &[u8]) -> Vec<u8> {
    let mut builder = MessageBuilder::from_bytes("", text.to_vec()).seipd_v1(thread_rng(), SymmetricKeyAlgorithm::AES256);
    builder.encrypt_to_key(thread_rng(), &public.public_subkeys[0]).unwrap();
    builder.to_vec(thread_rng()).unwrap()
}

#[test]
fn pgp_key_is_held_in_the_store_and_decrypts() {
    let (dir, store) = store();
    let (secret, public) = pgp_pair("Ada <ada@example.com>");
    let info = import_pgp(&store, &secret.to_bytes().unwrap(), "").unwrap();
    assert_eq!(info.label, "Ada <ada@example.com>");
    let cipher = pgp_encrypt(&public, b"meet at noon");
    let out = pgp_decrypt(&store, &cipher, "").unwrap();
    assert_eq!(out.plain, b"meet at noon");
    assert_eq!(out.key_id, info.id);
    let on_disk = std::fs::read(dir.path().join("mail.db")).unwrap();
    let needle = secret.to_bytes().unwrap();
    assert!(!on_disk.windows(32).any(|w| w == &needle[..32]));
}

#[test]
fn pgp_decrypt_fails_without_the_right_key() {
    let (_dir, store) = store();
    let (other, _) = pgp_pair("Eve <eve@example.com>");
    import_pgp(&store, &other.to_bytes().unwrap(), "").unwrap();
    let (_, public) = pgp_pair("Ada <ada@example.com>");
    let cipher = pgp_encrypt(&public, b"secret");
    assert_eq!(pgp_decrypt(&store, &cipher, "").err(), Some(CryptError::NoKey));
}

#[test]
fn pgp_decrypt_with_no_keys_imported_fails() {
    let (_dir, store) = store();
    let (_, public) = pgp_pair("Ada <ada@example.com>");
    let cipher = pgp_encrypt(&public, b"secret");
    assert_eq!(pgp_decrypt(&store, &cipher, "").err(), Some(CryptError::NoKey));
}

#[test]
fn garbage_is_not_a_key() {
    let (_dir, store) = store();
    assert!(matches!(import_pgp(&store, b"not a key", ""), Err(CryptError::BadKey(_))));
    assert!(matches!(import_pkcs12(&store, b"nope", "x"), Err(CryptError::BadKey(_))));
}

struct Ident {
    key: PKey<Private>,
    cert: X509,
}

fn make_cert(cn: &str, email: Option<&str>, issuer: Option<&Ident>, ca: bool, days: u32) -> Ident {
    let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_nid(Nid::COMMONNAME, cn).unwrap();
    let name = name.build();
    let mut b = X509::builder().unwrap();
    b.set_version(2).unwrap();
    let serial = openssl::bn::BigNum::from_u32(rand::random::<u32>() | 1).unwrap().to_asn1_integer().unwrap();
    b.set_serial_number(&serial).unwrap();
    b.set_subject_name(&name).unwrap();
    b.set_issuer_name(issuer.map(|i| i.cert.subject_name()).unwrap_or(&name)).unwrap();
    b.set_pubkey(&key).unwrap();
    b.set_not_before(&Asn1Time::days_from_now(0).unwrap()).unwrap();
    b.set_not_after(&Asn1Time::days_from_now(days).unwrap()).unwrap();
    if ca {
        b.append_extension(BasicConstraints::new().critical().ca().build().unwrap()).unwrap();
    }
    if let Some(e) = email {
        let ctx = b.x509v3_context(issuer.map(|i| i.cert.as_ref()), None);
        let san = SubjectAlternativeName::new().email(e).build(&ctx).unwrap();
        b.append_extension(san).unwrap();
    }
    b.sign(issuer.map(|i| &i.key).unwrap_or(&key), MessageDigest::sha256()).unwrap();
    Ident { key, cert: b.build() }
}

fn sign(who: &Ident, body: &[u8]) -> Vec<u8> {
    let certs = Stack::new().unwrap();
    Pkcs7::sign(&who.cert, &who.key, &certs, body, Pkcs7Flags::DETACHED).unwrap().to_smime(body, Pkcs7Flags::DETACHED).unwrap()
}

#[test]
fn smime_signed_by_a_trusted_root_shows_the_signer() {
    let root = make_cert("Test Root", None, None, true, 30);
    let leaf = make_cert("Ada", Some("ada@example.com"), Some(&root), false, 30);
    let mail = sign(&leaf, b"Content-Type: text/plain\r\n\r\nhello\r\n");
    let trust = trust_store_with(&[root.cert.clone()]).unwrap();
    let verdict = smime_verify(&mail, &trust).unwrap();
    assert_eq!(verdict.state, SmimeState::Trusted);
    let signer = verdict.signer.unwrap();
    assert_eq!(signer.subject, "CN=Ada");
    assert_eq!(signer.issuer, "CN=Test Root");
    assert_eq!(signer.emails, vec!["ada@example.com".to_string()]);
    assert!(!signer.expired);
    assert_eq!(signer.fingerprint.len(), 64);
}

#[test]
fn smime_signed_by_an_unknown_root_is_untrusted_but_shows_the_signer() {
    let root = make_cert("Other Root", None, None, true, 30);
    let leaf = make_cert("Mallory", Some("m@example.com"), Some(&root), false, 30);
    let mail = sign(&leaf, b"Content-Type: text/plain\r\n\r\nhello\r\n");
    let trust = trust_store_with(&[]).unwrap();
    let verdict = smime_verify(&mail, &trust).unwrap();
    assert!(matches!(verdict.state, SmimeState::Untrusted(_)));
    assert_eq!(verdict.signer.unwrap().subject, "CN=Mallory");
}

#[test]
fn smime_tampered_body_is_a_bad_signature() {
    let root = make_cert("Test Root", None, None, true, 30);
    let leaf = make_cert("Ada", None, Some(&root), false, 30);
    let mail = sign(&leaf, b"Content-Type: text/plain\r\n\r\npay 100\r\n");
    assert!(String::from_utf8_lossy(&mail).contains("pay 100"));
    let tampered = String::from_utf8(mail).unwrap().replace("pay 100", "pay 999");
    let trust = trust_store_with(&[root.cert.clone()]).unwrap();
    let verdict = smime_verify(tampered.as_bytes(), &trust).unwrap();
    assert!(matches!(verdict.state, SmimeState::BadSignature(_)));
    assert!(verdict.content.is_none());
}

#[test]
fn pkcs12_import_is_held_in_the_store_and_decrypts_smime() {
    let (dir, store) = store();
    let leaf = make_cert("Ada", Some("ada@example.com"), None, false, 30);
    let p12 = Pkcs12::builder().name("ada").pkey(&leaf.key).cert(&leaf.cert).build2("p12pass").unwrap().to_der().unwrap();
    let info = import_pkcs12(&store, &p12, "p12pass").unwrap();
    assert_eq!(info.kind, SMIME_KIND);
    assert_eq!(info.label, "CN=Ada");
    assert!(matches!(import_pkcs12(&store, &p12, "wrong"), Err(CryptError::BadKey(_))));

    let mut certs = Stack::new().unwrap();
    certs.push(leaf.cert.clone()).unwrap();
    let body = b"Content-Type: text/plain\r\n\r\nsealed words\r\n";
    let enveloped = Pkcs7::encrypt(&certs, body, openssl::symm::Cipher::aes_256_cbc(), Pkcs7Flags::empty()).unwrap().to_smime(body, Pkcs7Flags::empty());
    let enveloped = enveloped.unwrap_or_else(|_| {
        let p = Pkcs7::encrypt(&certs, body, openssl::symm::Cipher::aes_256_cbc(), Pkcs7Flags::empty()).unwrap();
        p.to_smime(&[], Pkcs7Flags::empty()).unwrap()
    });
    let plain = smime_decrypt(&store, &enveloped).unwrap();
    assert!(String::from_utf8_lossy(&plain).contains("sealed words"));

    let listed = list_keys(&store).unwrap();
    assert_eq!(listed.len(), 1);
    let on_disk = std::fs::read(dir.path().join("mail.db")).unwrap();
    assert!(!on_disk.windows(10).any(|w| w == b"PRIVATE KE"));
    remove_key(&store, SMIME_KIND, &info.id).unwrap();
    assert!(list_keys(&store).unwrap().is_empty());
    assert_eq!(smime_decrypt(&store, &enveloped).err(), Some(CryptError::NoKey));
}
