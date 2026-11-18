use mailbench::backend::*;
use mailbench::store::{KdfParams, Store};
use std::time::{Duration, Instant};

fn summary(uid: u32) -> MessageSummary {
    MessageSummary {
        uid,
        subject: Some(format!("Newsletter number {uid} about topic{} and offers", uid % 97)),
        from: vec![Address { name: Some(format!("Sender {}", uid % 300)), address: format!("s{}@example{}.test", uid % 300, uid % 17) }],
        date: Some(1_700_000_000 + uid as i64),
        ..Default::default()
    }
}

#[test]
fn storing_thousands_of_summaries_stays_fast() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let mut sink = StoreSink::new(&store, "acct", true);
    let started = Instant::now();
    for batch in 0..20u32 {
        let messages: Vec<_> = (batch * 500 + 1..=batch * 500 + 500).map(summary).collect();
        sink.put_summaries("INBOX", &messages).unwrap();
    }
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(5), "10000 summaries took {elapsed:?}");
    assert_eq!(sink.messages("INBOX").unwrap().len(), 10_000);
}
