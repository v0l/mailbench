use mailbench::backend::*;
use mailbench::mailstore::{self as ms, MailStore, Query};
use mailbench::real::{AccountSetup, RealStore};
use mailbench::store::{KdfParams, Store, Unlock};
use std::time::{Duration, Instant};

struct Offline;

#[async_trait::async_trait]
impl Backend for Offline {
    fn kind(&self) -> &'static str {
        "offline"
    }

    async fn list_folders(&mut self) -> Result<Vec<Folder>> {
        Err(BackendError::Connection("offline".into()))
    }
}

fn addr(name: Option<&str>, address: &str) -> Address {
    Address { name: name.map(str::to_string), address: address.into() }
}

fn sent(uid: u32, id: &str, refs: &[&str], to: Vec<Address>, cc: Vec<Address>) -> MessageSummary {
    MessageSummary {
        uid,
        subject: Some("Quote".into()),
        from: vec![addr(Some("Kieran"), "kieran@harkin.me")],
        to,
        cc,
        date: Some(1000 + uid as i64),
        message_id: Some(id.into()),
        references: refs.iter().map(|r| r.to_string()).collect(),
        flags: Flags { seen: true, ..Default::default() },
        ..Default::default()
    }
}

fn cards(messages: Vec<MessageSummary>) -> Vec<Vec<String>> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    {
        let store = Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap();
        let mut sink = StoreSink::new(&store, "acct", true);
        let n = messages.len() as u32;
        sink.put_folders(&[Folder { name: "Sent".into(), delimiter: None, role: Some(FolderRole::Sent), selectable: true }]).unwrap();
        sink.put_summaries("Sent", &messages).unwrap();
        sink.put_token("Sent", &SyncToken { uid_validity: 1, uid_next: n + 1, highest_mod_seq: None }).unwrap();
    }
    let store = Store::open(&path, &[Unlock::Password("pw")]).unwrap();
    let real = RealStore::open(
        store,
        vec![AccountSetup { id: "acct".into(), name: "t".into(), address: "kieran@harkin.me".into(), sender: String::new(), backend: Box::new(Offline) }],
    );
    let query = Query::new(ms::Folder::Sent);
    let started = Instant::now();
    while real.counts(&query).all < 1 {
        assert!(started.elapsed() < Duration::from_secs(30), "never loaded");
        std::thread::sleep(Duration::from_millis(10));
    }
    let page = real.list(&query, 0, 10);
    real.thread(page.items[0].id).into_iter().map(|t| t.to).collect()
}

#[test]
fn sent_cards_name_the_real_recipient_whatever_the_header_looked_like() {
    let target = "info@owon.com.cc";
    let thread = cards(vec![
        sent(1, "<a@x>", &[], vec![addr(None, target)], vec![]),
        sent(2, "<b@x>", &["<a@x>"], vec![addr(Some("info@owon"), target)], vec![]),
        sent(3, "<c@x>", &["<a@x>"], vec![], vec![addr(None, target)]),
    ]);
    assert_eq!(thread.len(), 3);
    for to in thread {
        assert_eq!(to, vec![target.to_string()]);
    }
}
