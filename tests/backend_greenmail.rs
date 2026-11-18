use mailbench::backend::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

mod common;
use common::{GM_IMAP as IMAP, GM_IMAPS as IMAPS, GM_SMTP as SMTP, GM_SMTPS as SMTPS};

fn user() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    assert!(common::greenmail(), "docker is required for greenmail tests");
    let name = format!("u{}x{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed));
    common::create_greenmail_user(&name);
    name
}

fn auth(name: &str) -> Auth {
    Auth::Password { user: name.to_string(), password: "secret".into() }
}

fn imap(name: &str) -> ImapBackend {
    ImapBackend::new(ImapConfig::new("127.0.0.1", IMAP, Security::Plain, auth(name)))
}

fn smtp(name: &str) -> SmtpBackend {
    SmtpBackend::new(SmtpConfig::new("127.0.0.1", SMTP, Security::Plain, auth(name)))
}

fn mail(name: &str, subject: &str) -> OutgoingMessage {
    OutgoingMessage {
        from: Address { name: Some("Sender".into()), address: "sender@localhost".into() },
        to: vec![Address { name: None, address: format!("{name}@localhost") }],
        subject: subject.into(),
        body: format!("body of {subject}"),
        ..Default::default()
    }
}

async fn wait_for(imap: &mut dyn Backend, count: usize) -> Vec<u32> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let uids = imap.search("INBOX", "ALL").await.unwrap();
        if uids.len() >= count {
            return uids;
        }
        assert!(Instant::now() < deadline, "mail did not arrive");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn send_then_sync_into_sink() {
    let name = user();
    let mut s = smtp(&name);
    s.send(&mail(&name, "first")).await.unwrap();
    s.send(&mail(&name, "second")).await.unwrap();
    let mut i = imap(&name);
    wait_for(&mut i, 2).await;

    let mut sink = MemorySink::default();
    let report = sync_account(&mut i, &mut sink).await.unwrap();
    assert_eq!(report.added, 2);
    assert_eq!(report.resets, 1);
    let inbox = &sink.messages["INBOX"];
    let subjects: Vec<_> = inbox.values().map(|m| m.subject.clone().unwrap()).collect();
    assert_eq!(subjects, ["first", "second"]);
    let m = inbox.values().next().unwrap();
    assert_eq!(m.from[0].address, "sender@localhost");
    assert_eq!(m.from[0].name.as_deref(), Some("Sender"));
    assert!(m.message_id.is_some());
    assert!(m.date.is_some());
    assert!(sink.tokens["INBOX"].uid_next > 2);

    let report = sync_account(&mut i, &mut sink).await.unwrap();
    assert_eq!((report.added, report.flag_changes, report.resets), (0, 0, 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn incremental_sync_picks_up_new_mail_flags_and_expunges() {
    let name = user();
    let mut s = smtp(&name);
    let mut i = imap(&name);
    s.send(&mail(&name, "one")).await.unwrap();
    s.send(&mail(&name, "two")).await.unwrap();
    let uids = wait_for(&mut i, 2).await;

    let mut sink = MemorySink::default();
    sync_account(&mut i, &mut sink).await.unwrap();

    s.send(&mail(&name, "three")).await.unwrap();
    wait_for(&mut i, 3).await;
    let seen = Flags { seen: true, ..Default::default() };
    i.set_flags("INBOX", &uids[..1], &FlagUpdate::Add(seen)).await.unwrap();
    i.delete_messages("INBOX", &uids[1..2]).await.unwrap();

    let report = sync_account(&mut i, &mut sink).await.unwrap();
    assert_eq!(report.resets, 0);
    assert_eq!(report.added, 1);
    let inbox = &sink.messages["INBOX"];
    assert_eq!(inbox.len(), 2);
    assert!(inbox[&uids[0]].flags.seen);
    assert!(!inbox.contains_key(&uids[1]));
    assert_eq!(inbox.values().last().unwrap().subject.as_deref(), Some("three"));
}

#[tokio::test(flavor = "multi_thread")]
async fn interrupted_first_sync_resumes_without_refetching() {
    let name = user();
    let mut s = smtp(&name);
    for n in 0..5 {
        s.send(&mail(&name, &format!("m{n}"))).await.unwrap();
    }
    let mut i = imap(&name);
    let uids = wait_for(&mut i, 5).await;

    let mut sink = MemorySink::default();
    let state = i.open_folder("INBOX").await.unwrap();
    sink.put_token("INBOX", &SyncToken { uid_validity: state.token.uid_validity, uid_next: 0, highest_mod_seq: None }).unwrap();
    let partial = i.fetch_summaries("INBOX", &uids[3..]).await.unwrap();
    sink.put_summaries("INBOX", &partial).unwrap();

    let report = sync_account(&mut i, &mut sink).await.unwrap();
    assert_eq!(report.resets, 0);
    assert_eq!(report.added, 3);
    assert_eq!(sink.messages["INBOX"].len(), 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_uidvalidity_resets_folder() {
    let name = user();
    let mut s = smtp(&name);
    let mut i = imap(&name);
    s.send(&mail(&name, "a")).await.unwrap();
    wait_for(&mut i, 1).await;
    let stale = SyncToken { uid_validity: 1, uid_next: 50, highest_mod_seq: None };
    let delta = i.sync_folder("INBOX", Some(&stale)).await.unwrap();
    assert!(delta.reset);
    assert_eq!(delta.added.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_returns_full_message() {
    let name = user();
    smtp(&name).send(&mail(&name, "fetch me")).await.unwrap();
    let mut i = imap(&name);
    let uid = wait_for(&mut i, 1).await[0];
    let raw = i.fetch("INBOX", uid).await.unwrap();
    let text = String::from_utf8(raw).unwrap();
    assert!(text.contains("Subject: fetch me"));
    assert!(text.contains("body of fetch me"));
}

#[tokio::test(flavor = "multi_thread")]
async fn move_creates_message_in_destination_and_removes_source() {
    let name = user();
    smtp(&name).send(&mail(&name, "mover")).await.unwrap();
    let mut i = imap(&name);
    let uid = wait_for(&mut i, 1).await[0];
    i.create_folder("Archive").await.unwrap();
    i.move_messages("INBOX", &[uid], "Archive").await.unwrap();
    assert!(i.search("INBOX", "ALL").await.unwrap().is_empty());
    assert_eq!(i.search("Archive", "ALL").await.unwrap().len(), 1);
    let folders = i.list_folders().await.unwrap();
    assert!(folders.iter().any(|f| f.name == "Archive"));
    assert!(folders.iter().any(|f| f.role == Some(FolderRole::Inbox)));
}

#[tokio::test(flavor = "multi_thread")]
async fn search_passes_query_through() {
    let name = user();
    let mut s = smtp(&name);
    s.send(&mail(&name, "needle")).await.unwrap();
    s.send(&mail(&name, "hay")).await.unwrap();
    let mut i = imap(&name);
    wait_for(&mut i, 2).await;
    assert_eq!(i.search("INBOX", "SUBJECT needle").await.unwrap().len(), 1);
    assert_eq!(i.search("INBOX", "UNSEEN").await.unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_reports_new_mail_via_idle() {
    let name = user();
    let mut i = imap(&name);
    assert!(i.capabilities().await.unwrap().idle);
    let mut watch = i.watch("INBOX").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    smtp(&name).send(&mail(&name, "push")).await.unwrap();
    let event = tokio::time::timeout(Duration::from_secs(15), watch.events.recv()).await.unwrap().unwrap();
    assert_eq!(event, Event::FolderChanged { folder: "INBOX".into() });
}

#[tokio::test(flavor = "multi_thread")]
async fn tls_connections_work_for_imap_and_smtp() {
    let name = user();
    let mut cfg = SmtpConfig::new("127.0.0.1", SMTPS, Security::Tls, auth(&name));
    cfg.accept_invalid_certs = true;
    SmtpBackend::new(cfg).send(&mail(&name, "tls")).await.unwrap();
    let mut cfg = ImapConfig::new("127.0.0.1", IMAPS, Security::Tls, auth(&name));
    cfg.accept_invalid_certs = true;
    let mut i = ImapBackend::new(cfg);
    wait_for(&mut i, 1).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tls_rejects_untrusted_certificate_by_default() {
    let name = user();
    let cfg = ImapConfig::new("127.0.0.1", IMAPS, Security::Tls, auth(&name));
    assert!(ImapBackend::new(cfg).list_folders().await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_password_is_an_auth_error() {
    let name = user();
    let bad = Auth::Password { user: name, password: "wrong".into() };
    let mut i = ImapBackend::new(ImapConfig::new("127.0.0.1", IMAP, Security::Plain, bad));
    assert!(matches!(i.list_folders().await, Err(BackendError::Auth(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn account_routes_send_to_smtp_and_reads_to_imap() {
    let name = user();
    let mut account = Account { mail: Box::new(imap(&name)), outbound: Box::new(smtp(&name)) };
    assert!(account.capabilities().await.unwrap().send);
    account.send(&mail(&name, "via account")).await.unwrap();
    wait_for(&mut account, 1).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn smtp_backend_has_no_mail_access() {
    let name = user();
    assert!(matches!(smtp(&name).list_folders().await, Err(BackendError::Unsupported(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn smtp_rejects_message_without_recipients() {
    let name = user();
    let mut m = mail(&name, "x");
    m.to.clear();
    assert!(smtp(&name).send(&m).await.is_err());
}
