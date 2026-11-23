use mailbench::accounts::{self, AccountConfig, Credential};
use mailbench::autodetect::ServerSettings;
use mailbench::backend::Security;
use mailbench::store::{KdfParams, Store};

fn chat_config() -> AccountConfig {
    accounts::chat_config("fake").unwrap()
}

#[test]
fn a_chat_account_round_trips_and_is_not_a_mail_backend() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("a.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let cfg = chat_config();
    assert!(cfg.is_chat());
    assert!(!cfg.is_nostr() && !cfg.is_jmap());
    accounts::save(&store, &cfg).unwrap();
    let loaded = accounts::load_all(&store);
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].is_chat());
    assert!(accounts::setup_for(&loaded[0], &store).is_err());
}

#[test]
fn the_chat_account_is_created_once_and_hidden_from_mail() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("c.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let first = accounts::ensure_chat_account(&store).unwrap();
    let again = accounts::ensure_chat_account(&store).unwrap();
    assert_eq!(first.id, again.id);
    assert_eq!(accounts::load_all(&store).len(), 1);
    assert!(accounts::mail_accounts(&store).is_empty());
}

#[test]
fn only_the_fake_chat_backend_exists() {
    assert!(accounts::chat_config("fake").is_ok());
    assert!(accounts::chat_config("matrix").is_err());
    let backend = accounts::chat_backend(&chat_config()).unwrap();
    assert_eq!(backend.kind(), "fake");
    let mut mail = chat_config();
    mail.credential = Credential::Password("x".into());
    assert!(accounts::chat_backend(&mail).is_err());
}

#[test]
fn an_account_file_written_before_chat_existed_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("b.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let old = AccountConfig {
        id: "old".into(),
        name: "work".into(),
        display_name: String::new(),
        signature: String::new(),
        address: "me@example.org".into(),
        username: "me".into(),
        imap: ServerSettings::new("imap.example.org", 993, Security::Tls),
        smtp: ServerSettings::new("smtp.example.org", 587, Security::StartTls),
        credential: Credential::Password("x".into()),
        backfill: Default::default(),
        notify: Default::default(),
        sync: Default::default(),
    };
    accounts::save(&store, &old).unwrap();
    let loaded = accounts::load_all(&store);
    assert_eq!(loaded.len(), 1);
    assert!(!loaded[0].is_chat());
}
