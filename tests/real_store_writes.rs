use mailbench::backend::*;
use mailbench::mailstore::{self as ms, MailStore, Query, SyncState};
use mailbench::real::{AccountSetup, RealStore};
use mailbench::runtime::block_on;
use mailbench::store::{KdfParams, Store, Unlock};
use std::path::Path;
use std::time::{Duration, Instant};

mod common;

fn auth(name: &str) -> Auth {
    Auth::Password { user: name.to_string(), password: "secret".into() }
}

fn imap_at(name: &str, port: u16) -> ImapBackend {
    ImapBackend::new(ImapConfig::new("127.0.0.1", port, Security::Plain, auth(name)))
}

fn setup(name: &str, port: u16) -> AccountSetup {
    AccountSetup {
        id: "acct".into(),
        name: "test".into(),
        address: format!("{name}@localhost"),
        sender: String::new(),
        backend: Box::new(imap_at(name, port)),
    }
}

fn open(path: &Path, name: &str, port: u16) -> RealStore {
    let store = Store::open(path, &[Unlock::Password("pw")]).unwrap();
    RealStore::open(store, vec![setup(name, port)])
}

fn until(what: &str, cond: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn count(store: &RealStore, folder: ms::Folder) -> usize {
    store.counts(&Query::new(folder)).all
}

fn seeded(prefix: &str) -> (String, tempfile::TempDir, std::path::PathBuf) {
    assert!(common::greenmail(), "docker is required for greenmail tests");
    let name = format!("{prefix}{}", std::process::id());
    common::create_greenmail_user(&name);
    let mut smtp = SmtpBackend::new(SmtpConfig::new("127.0.0.1", 3025, Security::Plain, auth(&name)));
    block_on(smtp.send(&OutgoingMessage {
        from: Address { name: Some("Zed".into()), address: "zed@localhost".into() },
        to: vec![Address { name: None, address: format!("{name}@localhost") }],
        subject: "Queued writes".into(),
        body: "body".into(),
        ..Default::default()
    }))
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap();
    let online = open(&path, &name, 3143);
    until("first sync", || count(&online, ms::Folder::Inbox) == 1 && online.accounts()[0].sync == SyncState::Idle);
    drop(online);
    (name, dir, path)
}

fn flagged_on_server(name: &str, folder: &str) -> Option<bool> {
    let mut b = imap_at(name, 3143);
    let delta = block_on(b.sync_folder(folder, None)).ok()?;
    delta.added.first().map(|m| m.flags.flagged)
}

#[test]
fn queued_writes_show_at_once_survive_restart_and_drain_when_the_server_returns() {
    if !common::greenmail() {
        return;
    }
    let (name, _dir, path) = seeded("rq");

    let mut offline = open(&path, &name, 1);
    until("offline failure", || offline.accounts()[0].sync == SyncState::Failed);
    let first = offline.list(&Query::new(ms::Folder::Inbox), 0, 1).items[0].id;
    offline.set_flags(first, ms::FlagChange { unread: None, flagged: Some(true) }).unwrap();
    offline.move_to(first, ms::Folder::Archive).unwrap();
    assert_eq!(count(&offline, ms::Folder::Inbox), 0);
    let archived = offline.list(&Query::new(ms::Folder::Archive), 0, 10).items;
    assert_eq!(archived.len(), 1);
    assert!(archived[0].flagged);
    assert_eq!(offline.pending(), 2);
    drop(offline);

    let offline = open(&path, &name, 1);
    assert_eq!(offline.pending(), 2);
    until("archive shown", || count(&offline, ms::Folder::Archive) == 1);
    drop(offline);

    let online = open(&path, &name, 3143);
    until("outbox to drain", || online.pending() == 0);
    until("archive synced", || {
        count(&online, ms::Folder::Archive) == 1 && count(&online, ms::Folder::Inbox) == 0
    });
    until("archive shown", || count(&online, ms::Folder::Archive) == 1 && count(&online, ms::Folder::Inbox) == 0);
    until("archived flag shown", || {
        online.list(&Query::new(ms::Folder::Archive), 0, 1).items.first().is_some_and(|i| i.flagged)
    });
    assert_eq!(flagged_on_server(&name, "Archive"), Some(true));
    assert_eq!(flagged_on_server(&name, "INBOX"), None);
}

#[test]
fn sync_keeps_headers_only_and_opening_a_message_fetches_its_body() {
    if !common::greenmail() {
        return;
    }
    let (name, _dir, path) = seeded("rh");
    let online = open(&path, &name, 3143);
    until("synced", || count(&online, ms::Folder::Inbox) == 1);
    let id = online.list(&Query::new(ms::Folder::Inbox), 0, 1).items[0].id;
    let first = online.get(id).unwrap();
    assert_eq!(first.body_state, ms::BodyState::Loading);
    assert!(first.paragraphs.is_empty());
    until("body fetched on open", || {
        let m = online.get(id).unwrap();
        m.body_state == ms::BodyState::Ready && m.paragraphs.join(" ").contains("body")
    });
}

#[test]
fn server_wins_when_the_message_was_removed_before_the_push() {
    if !common::greenmail() {
        return;
    }
    let (name, _dir, path) = seeded("rw");

    let mut offline = open(&path, &name, 1);
    until("offline failure", || offline.accounts()[0].sync == SyncState::Failed);
    let first = offline.list(&Query::new(ms::Folder::Inbox), 0, 1).items[0].id;
    offline.set_flags(first, ms::FlagChange { unread: None, flagged: Some(true) }).unwrap();
    offline.move_to(first, ms::Folder::Archive).unwrap();
    assert_eq!(count(&offline, ms::Folder::Archive), 1);
    drop(offline);

    let mut other = imap_at(&name, 3143);
    let uids = block_on(other.search("INBOX", "ALL")).unwrap();
    block_on(other.delete_messages("INBOX", &uids)).unwrap();

    let online = open(&path, &name, 3143);
    until("outbox to drain", || online.pending() == 0);
    until("server state shown", || {
        count(&online, ms::Folder::Inbox) == 0 && count(&online, ms::Folder::Archive) == 0
    });
    assert_eq!(flagged_on_server(&name, "INBOX"), None);
}

#[test]
fn server_changes_made_before_the_push_are_kept_and_local_changes_still_land() {
    if !common::greenmail() {
        return;
    }
    let (name, _dir, path) = seeded("rf");

    let mut offline = open(&path, &name, 1);
    until("offline failure", || offline.accounts()[0].sync == SyncState::Failed);
    let first = offline.list(&Query::new(ms::Folder::Inbox), 0, 1).items[0].id;
    offline.set_flags(first, ms::FlagChange { unread: None, flagged: Some(true) }).unwrap();
    drop(offline);

    let mut other = imap_at(&name, 3143);
    let uids = block_on(other.search("INBOX", "ALL")).unwrap();
    block_on(other.set_flags("INBOX", &uids, &FlagUpdate::Add(Flags { seen: true, ..Default::default() }))).unwrap();

    let online = open(&path, &name, 3143);
    until("outbox to drain", || online.pending() == 0);
    until("server state shown", || {
        let item = &online.list(&Query::new(ms::Folder::Inbox), 0, 1).items[0];
        !item.unread && item.flagged
    });
    assert_eq!(flagged_on_server(&name, "INBOX"), Some(true));
}

#[test]
fn archiving_without_an_archive_folder_creates_it() {
    if !common::greenmail() {
        return;
    }
    let (name, _dir, path) = seeded("rc");
    let mut b = imap_at(&name, 3143);
    assert!(!block_on(b.list_folders()).unwrap().iter().any(|f| f.name == "Archive"));

    let mut online = open(&path, &name, 3143);
    until("synced", || count(&online, ms::Folder::Inbox) == 1);
    let first = online.list(&Query::new(ms::Folder::Inbox), 0, 1).items[0].id;
    online.move_to(first, ms::Folder::Archive).unwrap();
    until("outbox to drain", || online.pending() == 0);
    until("archive synced", || count(&online, ms::Folder::Archive) == 1);
    assert!(block_on(b.list_folders()).unwrap().iter().any(|f| f.name == "Archive"));
    assert_eq!(block_on(b.search("Archive", "ALL")).unwrap().len(), 1);
    assert!(block_on(b.search("INBOX", "ALL")).unwrap().is_empty());
}
