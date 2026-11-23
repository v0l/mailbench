use mailbench::backend::*;
use mailbench::mailstore::{self as ms, MailStore, Outgoing, Query};
use mailbench::real::{AccountSetup, RealStore};
use mailbench::runtime::block_on;
use mailbench::store::{KdfParams, Store};
use std::time::{Duration, Instant};

mod common;

fn imap(name: &str) -> ImapBackend {
    let mut cfg = ImapConfig::new("172.17.0.5", 143, Security::StartTls, Auth::Password { user: name.to_string(), password: "secret".into() });
    cfg.accept_invalid_certs = true;
    ImapBackend::new(cfg)
}

fn smtp(name: &str) -> SmtpBackend {
    SmtpBackend::new(SmtpConfig::new("127.0.0.1", 3025, Security::Plain, Auth::Password { user: name.to_string(), password: "secret".into() }))
}

fn open(name: &str, dir: &tempfile::TempDir) -> RealStore {
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let account = AccountSetup {
        id: "acct".into(),
        name: "test".into(),
        address: format!("{name}@example.org"),
        sender: "Me".into(),
        backend: Box::new(Account { mail: Box::new(imap(name)), outbound: Box::new(smtp(name)) }),
    };
    let real = RealStore::open(store, vec![account]);
    real.set_send_hold(Duration::ZERO);
    real
}

fn drafts_on_server(name: &str) -> usize {
    let mut b = imap(name);
    block_on(b.search("Drafts", "ALL")).unwrap_or_default().len()
}

fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(40);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(150));
    }
}

#[test]
fn an_autosave_replaces_a_copy_the_server_has_not_indexed_yet() {
    assert!(common::stalwart(), "docker is required for stalwart tests");
    let name = format!("dl{}", std::process::id());
    common::create_stalwart_user(&name);
    let dir = tempfile::tempdir().unwrap();
    let mut real = open(&name, &dir);
    let mid = "fresh-snapshot@example.org";
    let raw = format!("From: me@example.org\r\nSubject: OWON\r\nMessage-ID: <{mid}>\r\nDate: Mon, 1 Jan 2024 10:00:00 +0000\r\n\r\nHello,\r\n");
    let mut b = imap(&name);
    let _ = block_on(b.create_folder("Drafts"));
    block_on(b.append_flagged("Drafts", &Flags { draft: true, seen: true, ..Default::default() }, raw.as_bytes())).unwrap();
    let out = Outgoing { account: 0, to: "info@owon.com.cc".into(), subject: "OWON".into(), body: "Hello, more".into(), message_id: Some(mid.into()), ..Default::default() };
    real.save_draft(out).unwrap();
    until("the autosave pushed", || real.pending_ops() == 0);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(drafts_on_server(&name), 1, "the replaced snapshot stayed on the server next to the new copy");
}

#[test]
fn many_quick_autosaves_keep_one_copy_on_a_server_that_indexes_late() {
    assert!(common::stalwart(), "docker is required for stalwart tests");
    let name = format!("dm{}", std::process::id());
    common::create_stalwart_user(&name);
    let dir = tempfile::tempdir().unwrap();
    let mut real = open(&name, &dir);
    let mut out = Outgoing { account: 0, to: "info@owon".into(), subject: "OWON".into(), ..Default::default() };
    for n in 0..10 {
        out.body = format!("Hello {n}");
        let mid = real.save_draft(out.clone()).unwrap();
        out.message_id = Some(mid);
        until("the autosave pushed", || real.pending_ops() == 0);
    }
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(drafts_on_server(&name), 1, "quick autosaves left older snapshots on the server");
}

#[test]
fn a_discard_right_after_an_autosave_leaves_no_draft_on_a_server_that_indexes_late() {
    assert!(common::stalwart(), "docker is required for stalwart tests");
    let name = format!("dq{}", std::process::id());
    common::create_stalwart_user(&name);
    let dir = tempfile::tempdir().unwrap();
    let mut real = open(&name, &dir);
    let out = Outgoing { account: 0, to: "info@owon".into(), subject: "OWON SPE6103".into(), body: "Hello,".into(), ..Default::default() };
    let mid = real.save_draft(out).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while drafts_on_server(&name) == 0 {
        assert!(Instant::now() < deadline, "the autosave never reached the server");
    }
    real.discard_draft(0, &mid);
    until("the discard pushed", || real.pending_ops() == 0);
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(drafts_on_server(&name), 0, "a discard right after an autosave left the snapshot on the server");
}

#[test]
fn quick_autosaves_and_a_discard_leave_no_draft_on_a_server_that_indexes_late() {
    assert!(common::stalwart(), "docker is required for stalwart tests");
    let name = format!("dr{}", std::process::id());
    common::create_stalwart_user(&name);
    let dir = tempfile::tempdir().unwrap();
    let mut real = open(&name, &dir);
    let mut out = Outgoing { account: 0, subject: "OWON SPE6103".into(), body: "Hello,".into(), ..Default::default() };
    for to in ["", "info@owon", "info@owon.com.cc"] {
        out.to = to.into();
        let mid = real.save_draft(out.clone()).unwrap();
        out.message_id = Some(mid);
        until("the autosave pushed", || real.pending_ops() == 0);
    }
    assert_eq!(drafts_on_server(&name), 1, "autosaves pushed back to back left older copies on the server");
    real.discard_draft(0, out.message_id.as_deref().unwrap());
    until("the discard pushed", || real.pending_ops() == 0);
    assert_eq!(drafts_on_server(&name), 0, "discard left a snapshot on the server");
}
