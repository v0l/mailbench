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

fn summary(uid: u32) -> MessageSummary {
    MessageSummary {
        uid,
        subject: Some(format!("Newsletter number {uid} about topic{} and offers", uid % 97)),
        from: vec![Address { name: Some(format!("Sender {}", uid % 300)), address: format!("s{}@example{}.test", uid % 300, uid % 17) }],
        to: vec![Address { name: None, address: "me@example.test".into() }],
        date: Some(1_700_000_000 + uid as i64),
        flags: Flags { seen: uid % 3 != 0, ..Default::default() },
        ..Default::default()
    }
}

const MESSAGES: u32 = 70_000;

#[test]
fn a_large_mailbox_shows_its_first_page_quickly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    {
        let store = Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap();
        let mut sink = StoreSink::new(&store, "acct", true);
        sink.put_folders(&[Folder { name: "INBOX".into(), delimiter: None, role: Some(FolderRole::Inbox), selectable: true }]).unwrap();
        for batch in (1..=MESSAGES).collect::<Vec<_>>().chunks(5000) {
            let messages: Vec<_> = batch.iter().copied().map(summary).collect();
            sink.put_summaries("INBOX", &messages).unwrap();
        }
        sink.put_token("INBOX", &SyncToken { uid_validity: 7, uid_next: MESSAGES + 1, highest_mod_seq: None }).unwrap();
    }
    let store = Store::open(&path, &[Unlock::Password("pw")]).unwrap();
    let started = Instant::now();
    let real = RealStore::open(
        store,
        vec![AccountSetup { id: "acct".into(), name: "test".into(), address: "me@example.test".into(), sender: String::new(), backend: Box::new(Offline) }],
    );
    let query = Query::new(ms::Folder::Inbox);
    while real.list(&query, 0, 20).items.is_empty() {
        assert!(started.elapsed() < Duration::from_secs(60), "never loaded");
        std::thread::sleep(Duration::from_millis(5));
    }
    let first = started.elapsed();
    let top = real.list(&query, 0, 1).items[0].subject.clone();
    assert_eq!(top, format!("Newsletter number {MESSAGES} about topic{} and offers", MESSAGES % 97));
    while real.counts(&query).all < MESSAGES as usize {
        assert!(started.elapsed() < Duration::from_secs(60), "never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    let all = started.elapsed();
    let frame = Instant::now();
    for _ in 0..10 {
        real.counts(&query);
        real.list(&query, 0, 30);
        real.accounts();
    }
    let per_frame = frame.elapsed() / 10;
    eprintln!("first page {first:?}, all {all:?}, per frame {per_frame:?}");
    assert!(first < Duration::from_millis(500), "first page took {first:?}");
    assert!(per_frame < Duration::from_millis(5), "a frame of list queries took {per_frame:?}");
}
