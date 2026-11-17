use mailbench::backend::*;
use mailbench::store::{KdfParams, Store, Unlock};
use std::time::{Duration, Instant};

mod common;

const SUBJECT: &str = "Quarterly zebracorn budget";
const BODY_WORD: &str = "marmalade-hovercraft-payload";

fn all_bytes(dir: &std::path::Path) -> Vec<u8> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        out.extend(std::fs::read(entry.unwrap().path()).unwrap());
    }
    out
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle.as_bytes())
}

#[test]
fn sync_into_encrypted_store_leaves_no_plaintext_on_disk() {
    if !common::greenmail() {
        return;
    }
    let name = format!("st{}", std::process::id());
    common::create_greenmail_user(&name);

    let auth = Auth::Password { user: name.clone(), password: "secret".into() };
    let mut smtp = SmtpBackend::new(SmtpConfig::new("127.0.0.1", 3025, Security::Plain, auth.clone()));
    smtp.send(&OutgoingMessage {
        from: Address { name: Some("Zed".into()), address: "zed@localhost".into() },
        to: vec![Address { name: None, address: format!("{name}@localhost") }],
        subject: SUBJECT.into(),
        body: format!("this carries {BODY_WORD}"),
        ..Default::default()
    })
    .unwrap();
    let mut imap = ImapBackend::new(ImapConfig::new("127.0.0.1", 3143, Security::Plain, auth));
    let deadline = Instant::now() + Duration::from_secs(15);
    while imap.search("INBOX", "ALL").unwrap().is_empty() {
        assert!(Instant::now() < deadline, "mail did not arrive");
        std::thread::sleep(Duration::from_millis(200));
    }

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let store = Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap();
    {
        let mut sink = StoreSink::new(&store, "acct", true);
        let report = sync_account(&mut imap, &mut sink).unwrap();
        assert_eq!(report.added, 1);
        let messages = sink.messages("INBOX").unwrap();
        assert_eq!(messages[0].subject.as_deref(), Some(SUBJECT));
        let raw = sink.body("INBOX", messages[0].uid).unwrap().unwrap();
        assert!(contains(&raw, BODY_WORD));
        assert_eq!(sink.search("zebracorn").unwrap().len(), 1);
        assert_eq!(sink.search(BODY_WORD.split('-').next().unwrap()).unwrap().len(), 1);
        let again = sync_account(&mut imap, &mut sink).unwrap();
        assert_eq!((again.added, again.flag_changes, again.resets), (0, 0, 0));
    }
    drop(store);

    let bytes = all_bytes(dir.path());
    assert!(!bytes.is_empty());
    for needle in [SUBJECT, "zebracorn", BODY_WORD, "zed@localhost", "Subject:", "INBOX"] {
        assert!(!contains(&bytes, needle), "plaintext {needle:?} found on disk");
    }

    let reopened = Store::open(&path, &[Unlock::Password("pw")]).unwrap();
    let sink = StoreSink::new(&reopened, "acct", true);
    assert_eq!(sink.messages("INBOX").unwrap().len(), 1);
}
