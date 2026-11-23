use mailbench::backend::*;
use mailbench::mailstore::{self as ms, MailStore, Query, ThreadItem};
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

fn addr(address: &str) -> Address {
    Address { name: None, address: address.into() }
}

fn mail(uid: u32, id: &str, refs: &[&str], subject: &str, to: &str) -> MessageSummary {
    MessageSummary {
        uid,
        subject: Some(subject.into()),
        from: vec![Address { name: Some("Kieran".into()), address: "kieran@harkin.me".into() }],
        to: if to.is_empty() { vec![] } else { vec![addr(to)] },
        date: Some(1000 + uid as i64),
        message_id: Some(id.into()),
        references: refs.iter().map(|r| r.to_string()).collect(),
        flags: Flags { seen: true, ..Default::default() },
        ..Default::default()
    }
}

fn folder(name: &str, role: FolderRole) -> Folder {
    Folder { name: name.into(), delimiter: None, role: Some(role), selectable: true }
}

fn open(folders: Vec<(&str, FolderRole, Vec<MessageSummary>)>) -> (RealStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    {
        let store = Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap();
        let mut sink = StoreSink::new(&store, "acct", true);
        let all: Vec<Folder> = folders.iter().map(|(n, r, _)| folder(n, r.clone())).collect();
        sink.put_folders(&all).unwrap();
        for (name, _, messages) in &folders {
            let n = messages.len() as u32;
            sink.put_summaries(name, messages).unwrap();
            sink.put_token(name, &SyncToken { uid_validity: 1, uid_next: n + 1, highest_mod_seq: None }).unwrap();
        }
    }
    let store = Store::open(&path, &[Unlock::Password("pw")]).unwrap();
    let real = RealStore::open(
        store,
        vec![AccountSetup { id: "acct".into(), name: "t".into(), address: "kieran@harkin.me".into(), sender: String::new(), backend: Box::new(Offline) }],
    );
    (real, dir)
}

fn first(real: &RealStore, folder: ms::Folder) -> ms::Summary {
    let query = Query::new(folder);
    let started = Instant::now();
    while real.counts(&query).all < 1 {
        assert!(started.elapsed() < Duration::from_secs(30), "never loaded");
        std::thread::sleep(Duration::from_millis(10));
    }
    real.list(&query, 0, 10).items.remove(0)
}

fn thread_of(real: &RealStore, folder: ms::Folder) -> (ms::Summary, Vec<ThreadItem>) {
    let row = first(real, folder);
    let items = real.thread(row.id);
    (row, items)
}

fn leftovers() -> Vec<(&'static str, FolderRole, Vec<MessageSummary>)> {
    vec![
        ("Sent", FolderRole::Sent, vec![mail(3, "<m@x>", &[], "", "info@owon.com.cc")]),
        ("Drafts", FolderRole::Drafts, vec![mail(1, "<m@x>", &[], "", ""), mail(2, "<m@x>", &[], "", "info@owon")]),
    ]
}

#[test]
fn leftover_autosaves_of_a_sent_message_are_not_shown_or_counted() {
    let (real, _dir) = open(leftovers());
    let (row, items) = thread_of(&real, ms::Folder::Sent);
    assert_eq!(items.len(), 1);
    assert!(!items[0].draft);
    assert_eq!(row.thread, 1);
}

#[test]
fn a_live_reply_draft_is_a_draft_card_and_is_not_counted() {
    let (real, _dir) = open(vec![
        ("Sent", FolderRole::Sent, vec![mail(1, "<a@x>", &[], "Plan", "info@owon.com.cc")]),
        ("Drafts", FolderRole::Drafts, vec![mail(2, "<d@x>", &["<a@x>"], "Re: Plan", "info@owon.com.cc")]),
    ]);
    let (row, items) = thread_of(&real, ms::Folder::Sent);
    assert_eq!(items.len(), 2);
    assert_eq!(items.iter().filter(|t| t.draft).count(), 1);
    assert_eq!(row.thread, 1);
}

#[test]
fn only_the_newest_draft_of_a_conversation_is_shown() {
    let (real, _dir) = open(vec![
        ("Sent", FolderRole::Sent, vec![mail(1, "<a@x>", &[], "Plan", "info@owon.com.cc")]),
        (
            "Drafts",
            FolderRole::Drafts,
            vec![mail(2, "<d1@x>", &["<a@x>"], "Re: Plan", ""), mail(3, "<d2@x>", &["<a@x>"], "Re: Plan", "info@owon.com.cc")],
        ),
    ]);
    let (_, items) = thread_of(&real, ms::Folder::Sent);
    assert_eq!(items.iter().filter(|t| t.draft).count(), 1);
}

#[test]
fn trash_members_are_hidden_outside_trash() {
    let (real, _dir) = open(vec![
        ("Sent", FolderRole::Sent, vec![mail(1, "<a@x>", &[], "Plan", "info@owon.com.cc")]),
        ("Trash", FolderRole::Trash, vec![mail(2, "<b@x>", &["<a@x>"], "Re: Plan", "info@owon.com.cc")]),
    ]);
    let (row, items) = thread_of(&real, ms::Folder::Sent);
    assert_eq!(items.len(), 1);
    assert_eq!(row.thread, 1);
    let (_, in_trash) = thread_of(&real, ms::Folder::Trash);
    assert_eq!(in_trash.len(), 2);
}

#[test]
fn drafts_without_a_subject_do_not_join_an_unrelated_sent_mail() {
    let (real, _dir) = open(vec![
        ("Sent", FolderRole::Sent, vec![mail(1, "<a@x>", &[], "", "info@owon.com.cc")]),
        ("Drafts", FolderRole::Drafts, vec![mail(2, "<d@x>", &[], "", "info@owon.com.cc")]),
    ]);
    let (_, items) = thread_of(&real, ms::Folder::Sent);
    assert_eq!(items.len(), 1);
}

#[test]
fn drafts_already_in_sent_are_found_for_repair() {
    let (real, _dir) = open(leftovers());
    first(&real, ms::Folder::Sent);
    assert_eq!(real.sent_drafts(0), vec!["<m@x>".to_string()]);
}
