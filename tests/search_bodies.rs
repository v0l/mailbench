use mailbench::backend::*;
use mailbench::mailstore::{self as ms, MailStore, Query};
use mailbench::real::{AccountSetup, RealStore};
use mailbench::runtime::block_on;
use mailbench::search::BackfillPolicy;
use mailbench::store::{KdfParams, Store};
use std::time::{Duration, Instant};

mod common;

fn auth(name: &str) -> Auth {
    Auth::Password { user: name.to_string(), password: "secret".into() }
}

fn imap(name: &str) -> ImapBackend {
    ImapBackend::new(ImapConfig::new("127.0.0.1", 3143, Security::Plain, auth(name)))
}

fn smtp(name: &str) -> SmtpBackend {
    SmtpBackend::new(SmtpConfig::new("127.0.0.1", 3025, Security::Plain, auth(name)))
}

fn setup(name: &str) -> AccountSetup {
    AccountSetup {
        id: "acct".into(),
        name: "test".into(),
        address: format!("{name}@localhost"),
        sender: String::new(),
        backend: Box::new(Account { mail: Box::new(imap(name)), outbound: Box::new(smtp(name)) }),
    }
}

fn deliver(name: &str, subject: &str, body: &str) {
    block_on(smtp(name).send(&OutgoingMessage {
        from: Address { name: Some("Zed".into()), address: "zed@localhost".into() },
        to: vec![Address { name: None, address: format!("{name}@localhost") }],
        subject: subject.into(),
        body: body.into(),
        ..Default::default()
    }))
    .unwrap();
}

fn until(what: &str, cond: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(40);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn query(text: &str) -> Query {
    Query { text: text.into(), ..Query::new(ms::Folder::Inbox) }
}

fn fresh_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    (dir, store)
}

fn user(tag: &str) -> String {
    let name = format!("{tag}{}", std::process::id());
    common::create_greenmail_user(&name);
    name
}

#[test]
fn a_body_word_is_found_after_the_backfill() {
    if !common::greenmail() {
        return;
    }
    let name = user("bf");
    deliver(&name, "Plain subject", "the quarterly xylophone invoice is attached");
    deliver(&name, "Other subject", "nothing interesting here");
    let (_dir, store) = fresh_store();
    let real = RealStore::open(store, vec![setup(&name)]);
    until("both messages synced", || real.counts(&query("")).all == 2);
    real.set_backfill("acct", BackfillPolicy::on());
    until("body word found locally", || {
        let page = real.list(&query("xylophone"), 0, 10);
        page.total == 1 && !page.items[0].remote
    });
    assert_eq!(real.list(&query("xylophone"), 0, 10).items[0].subject, "Plain subject");
    until("progress shown", || real.accounts()[0].indexed == Some((2, 2)));
    until("phrase found", || real.list(&query("\"quarterly xylophone\""), 0, 10).total == 1);
    let reversed = query("\"xylophone quarterly\"");
    until("reversed phrase settled", || !real.searching(&reversed));
    assert_eq!(real.list(&reversed, 0, 10).total, 0);
    let mixed = query("xylophone subject:other");
    until("mixed settled", || !real.searching(&mixed));
    assert_eq!(real.list(&mixed, 0, 10).total, 0);
    assert_eq!(real.list(&query("subject:plain is:unread"), 0, 10).total, 1);
}

#[test]
fn the_server_fills_in_bodies_that_are_not_local_yet() {
    if !common::greenmail() {
        return;
    }
    let name = user("sf");
    deliver(&name, "Remote subject", "a gearbox rattles on the rover");
    deliver(&name, "Another subject", "nothing here");
    let (_dir, store) = fresh_store();
    let real = RealStore::open(store, vec![setup(&name)]);
    until("synced", || real.counts(&query("")).all == 2);
    let q = query("gearbox");
    until("server hit", || {
        let page = real.list(&q, 0, 10);
        page.total == 1 && page.items[0].remote
    });
    assert_eq!(real.list(&q, 0, 10).items[0].subject, "Remote subject");
    until("search settles", || !real.searching(&q));
}

#[test]
fn search_reaches_user_folders_and_the_server_there() {
    if !common::greenmail() {
        return;
    }
    let name = user("uf");
    block_on(imap(&name).create_folder("Receipts")).unwrap();
    let raw = b"From: shop@example.test\r\nTo: x@localhost\r\nSubject: Order\r\nDate: Mon, 13 Nov 2023 10:00:00 +0000\r\n\r\nyour sprocket shipped\r\n";
    block_on(imap(&name).append("Receipts", raw)).unwrap();
    let (_dir, store) = fresh_store();
    let real = RealStore::open(store, vec![setup(&name)]);
    let folder = ms::Folder::Other(0, "Receipts".into());
    let q = Query { text: "sprocket".into(), ..Query::new(folder.clone()) };
    until("user folder synced", || real.counts(&Query::new(folder.clone())).all == 1);
    until("server hit in user folder", || real.list(&q, 0, 10).total == 1);
    real.set_backfill("acct", BackfillPolicy::on());
    until("local hit in user folder", || {
        let page = real.list(&q, 0, 10);
        page.total == 1 && !page.items[0].remote
    });
    let elsewhere = Query { text: "sprocket".into(), ..Query::new(ms::Folder::Inbox) };
    assert_eq!(real.list(&elsewhere, 0, 10).total, 0);
}
