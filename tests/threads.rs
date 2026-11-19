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

fn message(uid: u32, id: &str, refs: &[&str], subject: &str, from: &str, date: i64) -> MessageSummary {
    MessageSummary {
        uid,
        subject: Some(subject.into()),
        from: vec![Address { name: None, address: from.into() }],
        date: Some(date),
        message_id: Some(id.into()),
        references: refs.iter().map(|r| r.to_string()).collect(),
        flags: Flags { seen: uid != 2, ..Default::default() },
        ..Default::default()
    }
}

fn folder(name: &str, role: FolderRole) -> Folder {
    Folder { name: name.into(), delimiter: None, role: Some(role), selectable: true }
}

#[test]
fn a_reply_in_sent_joins_the_inbox_thread_as_one_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    {
        let store = Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap();
        let mut sink = StoreSink::new(&store, "acct", true);
        sink.put_folders(&[folder("INBOX", FolderRole::Inbox), folder("Sent", FolderRole::Sent)]).unwrap();
        sink.put_summaries(
            "INBOX",
            &[
                message(1, "<a@x>", &[], "Plan", "ann@x", 1000),
                message(2, "<c@x>", &["<a@x>", "<b@x>"], "Re: Plan", "ann@x", 3000),
                message(3, "<other@x>", &[], "Unrelated", "bob@x", 2000),
            ],
        )
        .unwrap();
        sink.put_summaries("Sent", &[message(1, "<b@x>", &["<a@x>"], "Re: Plan", "me@x", 2000)]).unwrap();
        sink.put_token("INBOX", &SyncToken { uid_validity: 1, uid_next: 4, highest_mod_seq: None }).unwrap();
        sink.put_token("Sent", &SyncToken { uid_validity: 1, uid_next: 2, highest_mod_seq: None }).unwrap();
    }
    let store = Store::open(&path, &[Unlock::Password("pw")]).unwrap();
    let real = RealStore::open(
        store,
        vec![AccountSetup { id: "acct".into(), name: "t".into(), address: "me@x".into(), sender: String::new(), backend: Box::new(Offline) }],
    );
    let inbox = Query::new(ms::Folder::Inbox);
    let started = Instant::now();
    while real.counts(&inbox).all < 2 || real.counts(&Query::new(ms::Folder::Sent)).all < 1 {
        assert!(started.elapsed() < Duration::from_secs(30), "never loaded");
        std::thread::sleep(Duration::from_millis(10));
    }
    let page = real.list(&inbox, 0, 10);
    assert_eq!(page.total, 2);
    assert_eq!(page.items[0].thread, 3);
    assert!(page.items[0].unread);
    assert_eq!(page.items[1].thread, 1);
    let thread = real.thread(page.items[0].id);
    assert_eq!(thread.len(), 3);
    let folders: Vec<_> = thread.iter().map(|t| t.folder.clone()).collect();
    assert_eq!(folders, vec![ms::Folder::Inbox, ms::Folder::Sent, ms::Folder::Inbox]);
}
