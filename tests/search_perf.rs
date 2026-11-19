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

const MESSAGES: u32 = 100_000;

fn budget(ms: u64) -> Duration {
    Duration::from_millis(if cfg!(debug_assertions) { ms * 5 } else { ms })
}

fn summary(uid: u32) -> MessageSummary {
    MessageSummary {
        uid,
        subject: Some(format!("Newsletter number {uid} about topic{}", uid % 97)),
        from: vec![Address { name: Some(format!("Sender {}", uid % 300)), address: format!("s{}@example{}.test", uid % 300, uid % 17) }],
        to: vec![Address { name: None, address: "me@example.test".into() }],
        date: Some(1_700_000_000 + uid as i64),
        flags: Flags { seen: uid % 3 != 0, ..Default::default() },
        ..Default::default()
    }
}

fn raw(uid: u32) -> Vec<u8> {
    let rare = if uid % 1000 == 0 { " zebracorn" } else { "" };
    format!("From: s@example.test\r\nSubject: n{uid}\r\n\r\nbody of message{} and common words{rare}\r\n", uid % 5000).into_bytes()
}

fn wait_for(what: &str, cond: impl Fn() -> bool) {
    let started = Instant::now();
    while !cond() {
        assert!(started.elapsed() < Duration::from_secs(120), "never {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn searching_100k_messages_with_bodies_stays_under_100ms() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    {
        let store = Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap();
        let mut sink = StoreSink::new(&store, "acct", true);
        sink.put_folders(&[Folder { name: "INBOX".into(), delimiter: None, role: Some(FolderRole::Inbox), selectable: true }]).unwrap();
        for batch in (1..=MESSAGES).collect::<Vec<_>>().chunks(5000) {
            let messages: Vec<_> = batch.iter().copied().map(summary).collect();
            sink.put_summaries("INBOX", &messages).unwrap();
            for uid in batch.iter().filter(|u| *u % 10 == 0) {
                sink.put_body("INBOX", *uid, &raw(*uid)).unwrap();
            }
        }
        sink.put_token("INBOX", &SyncToken { uid_validity: 7, uid_next: MESSAGES + 1, highest_mod_seq: None }).unwrap();
    }
    let store = Store::open(&path, &[Unlock::Password("pw")]).unwrap();
    let real = RealStore::open(
        store,
        vec![AccountSetup { id: "acct".into(), name: "test".into(), address: "me@example.test".into(), sender: String::new(), backend: Box::new(Offline) }],
    );
    let base = Query::new(ms::Folder::Inbox);
    wait_for("loaded", || real.counts(&base).all == MESSAGES as usize);

    let mut query = Query::new(ms::Folder::Inbox);
    query.text = "zebracorn".into();
    let started = Instant::now();
    wait_for("found", || real.list(&query, 0, 20).total == (MESSAGES / 1000) as usize);
    let first = started.elapsed();
    let frame = Instant::now();
    for _ in 0..10 {
        real.list(&query, 0, 30);
        real.counts(&query);
    }
    let per_frame = frame.elapsed() / 10;

    query.text = "common words message40".into();
    let started = Instant::now();
    wait_for("body words found", || real.list(&query, 0, 20).total == (MESSAGES / 5000) as usize);
    let second = started.elapsed();

    query.text = "from:sender is:unread topic5".into();
    let started = Instant::now();
    wait_for("operators", || real.list(&query, 0, 20).total > 0);
    let third = started.elapsed();
    eprintln!("body hit {first:?}, per frame {per_frame:?}, multi {second:?}, operators {third:?}");
    assert!(first < budget(100), "body search took {first:?}");
    assert!(second < budget(100), "multi-term search took {second:?}");
    assert!(third < budget(100), "operator search took {third:?}");
    assert!(per_frame < budget(30), "a frame of search list queries took {per_frame:?}");
}
