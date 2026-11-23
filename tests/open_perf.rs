use mailbench::backend::*;
use mailbench::mailstore::{self as ms, BodyState, MailStore, Query};
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

fn raw_body(uid: u32) -> Vec<u8> {
    let text = format!("Hello number {uid}, here is the news.\r\n").repeat(200);
    format!(
        "From: Sender {s} <s{s}@example{d}.test>\r\nTo: me@example.test\r\nSubject: Newsletter number {uid}\r\nMessage-ID: <{uid}@example.test>\r\nDate: Tue, 14 Nov 2023 22:13:20 +0000\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}",
        s = uid % 300,
        d = uid % 17
    )
    .into_bytes()
}

const MESSAGES: u32 = 100_000;
const OPENED: u32 = 100;

fn stored_uid(n: u32) -> u32 {
    MESSAGES - n * 997 - 3
}

#[test]
fn opening_a_stored_message_in_a_large_mailbox_is_fast() {
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
        for n in 0..OPENED {
            let uid = stored_uid(n);
            sink.put_body("INBOX", uid, &raw_body(uid)).unwrap();
        }
    }
    let store = Store::open(&path, &[Unlock::Password("pw")]).unwrap();
    let real = RealStore::open(
        store,
        vec![AccountSetup { id: "acct".into(), name: "test".into(), address: "me@example.test".into(), sender: String::new(), backend: Box::new(Offline) }],
    );
    let query = Query::new(ms::Folder::Inbox);
    let started = Instant::now();
    while real.counts(&query).all < MESSAGES as usize {
        assert!(started.elapsed() < Duration::from_secs(120), "never finished loading");
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(500));
    let ids: Vec<_> = (0..OPENED).map(|n| real.list(&query, (MESSAGES - stored_uid(n)) as usize, 1).items[0].id).collect();
    let mut p95s = Vec::new();
    for round in 0..5 {
        let mut times = Vec::new();
        for id in ids.iter().skip(round * 20).take(20) {
            let open = Instant::now();
            loop {
                let m = real.get(*id).unwrap();
                if m.body_state == BodyState::Ready {
                    assert!(!m.paragraphs.is_empty());
                    break;
                }
                assert!(open.elapsed() < Duration::from_secs(30), "message never opened");
                std::thread::sleep(Duration::from_micros(200));
            }
            times.push(open.elapsed());
        }
        times.sort();
        p95s.push(times[times.len() * 95 / 100]);
    }
    let best = p95s.iter().min().copied().unwrap();
    eprintln!("open p95 per round {p95s:?}");
    assert!(best < Duration::from_millis(50), "opening a stored message took {best:?} at p95");
}
