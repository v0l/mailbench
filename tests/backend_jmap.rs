use mailbench::backend::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

mod common;

fn user() -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    assert!(common::stalwart(), "docker is required for stalwart tests");
    let name = format!("u{}x{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed));
    common::create_stalwart_user(&name);
    name
}

fn jmap_as(name: &str, password: &str) -> JmapBackend {
    let auth = Auth::Password { user: name.to_string(), password: password.into() };
    JmapBackend::new(JmapConfig::new(&format!("http://127.0.0.1:{}", common::SW_HTTP), auth))
}

fn jmap(name: &str) -> JmapBackend {
    jmap_as(name, "secret")
}

fn mail(from: &str, to: &str, subject: &str) -> OutgoingMessage {
    OutgoingMessage {
        from: Address { name: Some("Sender".into()), address: format!("{from}@example.org") },
        to: vec![Address { name: None, address: format!("{to}@example.org") }],
        subject: subject.into(),
        body: format!("body of {subject}"),
        ..Default::default()
    }
}

async fn wait_for(b: &mut JmapBackend, folder: &str, count: usize) -> Vec<u32> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let uids = b.search(folder, "ALL").await.unwrap();
        if uids.len() >= count {
            return uids;
        }
        assert!(Instant::now() < deadline, "mail did not arrive in {folder}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn session_discovery_and_capabilities() {
    let name = user();
    let mut b = jmap(&name);
    b.check().await.unwrap();
    let caps = b.capabilities().await.unwrap();
    assert!(caps.send && caps.idle && caps.move_command);
    assert!(caps.server_folders && caps.server_search && caps.server_flags);
    assert!(!caps.condstore && !caps.qresync && !caps.uidplus && !caps.gmail_ext);
    assert!(b.sent_copy_by_server());
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_password_is_an_auth_error() {
    let name = user();
    let mut b = jmap_as(&name, "wrong");
    assert!(matches!(b.check().await, Err(BackendError::Auth(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_folders_with_roles() {
    let name = user();
    let mut b = jmap(&name);
    let folders = b.list_folders().await.unwrap();
    let inbox = folders.iter().find(|f| f.role == Some(FolderRole::Inbox)).expect("inbox");
    assert_eq!(inbox.name, "Inbox");
    assert!(folders.iter().any(|f| f.role == Some(FolderRole::Sent)));
    assert!(folders.iter().any(|f| f.role == Some(FolderRole::Drafts)));
}

#[tokio::test(flavor = "multi_thread")]
async fn send_then_sync_into_sink() {
    let name = user();
    let mut b = jmap(&name);
    b.send(&mail(&name, &name, "first")).await.unwrap();
    b.send(&mail(&name, &name, "second")).await.unwrap();
    wait_for(&mut b, "Inbox", 2).await;

    let mut sink = MemorySink::default();
    let report = sync_account(&mut b, &mut sink).await.unwrap();
    assert!(report.added >= 4, "inbox and sent copies: {report:?}");
    let inbox = &sink.messages["Inbox"];
    let mut subjects: Vec<_> = inbox.values().map(|m| m.subject.clone().unwrap()).collect();
    subjects.sort();
    assert_eq!(subjects, ["first", "second"]);
    let m = inbox.values().next().unwrap();
    assert_eq!(m.from[0].address, format!("{name}@example.org"));
    assert_eq!(m.from[0].name.as_deref(), Some("Sender"));
    assert!(m.message_id.is_some());
    assert!(m.date.is_some());
    let sent_name = b.list_folders().await.unwrap().into_iter().find(|f| f.role == Some(FolderRole::Sent)).unwrap().name;
    assert_eq!(sink.messages[&sent_name].len(), 2, "sent copy is filed by the server");

    let report = sync_account(&mut b, &mut sink).await.unwrap();
    assert_eq!((report.added, report.flag_changes, report.resets), (0, 0, 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn bodies_download_as_rfc822() {
    let name = user();
    let mut b = jmap(&name);
    let mut m = mail(&name, &name, "with attachment");
    m.attachments.push(Attachment { name: "a.txt".into(), mime: "text/plain".into(), data: b"attached bytes".to_vec(), ..Default::default() });
    b.send(&m).await.unwrap();
    let uids = wait_for(&mut b, "Inbox", 1).await;
    let raw = b.fetch("Inbox", uids[0]).await.unwrap();
    let parsed = mail_parser::MessageParser::default().parse(&raw).unwrap();
    assert_eq!(parsed.subject(), Some("with attachment"));
    assert!(parsed.body_text(0).unwrap().contains("body of with attachment"));
    assert_eq!(parsed.attachments().len(), 1);
    assert_eq!(parsed.attachments().next().unwrap().decoded().as_ref(), b"attached bytes");
}

#[tokio::test(flavor = "multi_thread")]
async fn incremental_sync_uses_changes_for_new_mail_flags_and_destroys() {
    let name = user();
    let mut b = jmap(&name);
    b.send(&mail(&name, &name, "one")).await.unwrap();
    wait_for(&mut b, "Inbox", 1).await;
    let mut sink = MemorySink::default();
    sync_account(&mut b, &mut sink).await.unwrap();

    b.send(&mail(&name, &name, "two")).await.unwrap();
    wait_for(&mut b, "Inbox", 2).await;
    let report = sync_account(&mut b, &mut sink).await.unwrap();
    assert_eq!((report.added, report.resets), (2, 0), "new inbox and sent copy only");
    assert_eq!(sink.messages["Inbox"].len(), 2);

    let uids = b.search("Inbox", "ALL").await.unwrap();
    b.set_flags("Inbox", &uids[..1], &FlagUpdate::Add(Flags { seen: true, flagged: true, keywords: vec!["project".into()], ..Default::default() })).await.unwrap();
    let report = sync_account(&mut b, &mut sink).await.unwrap();
    assert_eq!((report.added, report.flag_changes), (0, 1));
    let flags = &sink.messages["Inbox"][&uids[0]].flags;
    assert!(flags.seen && flags.flagged);
    assert_eq!(flags.keywords, ["project"]);

    b.set_flags("Inbox", &uids[..1], &FlagUpdate::Remove(Flags { flagged: true, ..Default::default() })).await.unwrap();
    sync_account(&mut b, &mut sink).await.unwrap();
    assert!(!sink.messages["Inbox"][&uids[0]].flags.flagged);

    b.delete_messages("Inbox", &uids[..1]).await.unwrap();
    sync_account(&mut b, &mut sink).await.unwrap();
    assert_eq!(sink.messages["Inbox"].len(), 1);
    assert!(!sink.messages["Inbox"].contains_key(&uids[0]));
}

#[tokio::test(flavor = "multi_thread")]
async fn folders_move_search_and_append() {
    let name = user();
    let mut b = jmap(&name);
    b.create_folder("Projects").await.unwrap();
    b.create_folder("Projects/Deep").await.unwrap();
    let names: Vec<_> = b.list_folders().await.unwrap().into_iter().map(|f| f.name).collect();
    assert!(names.contains(&"Projects".to_string()) && names.contains(&"Projects/Deep".to_string()));

    b.send(&mail(&name, &name, "needle in haystack")).await.unwrap();
    b.send(&mail(&name, &name, "other")).await.unwrap();
    wait_for(&mut b, "Inbox", 2).await;
    let hits = b.search("Inbox", "needle").await.unwrap();
    assert_eq!(hits.len(), 1);

    b.move_messages("Inbox", &hits, "Projects").await.unwrap();
    assert_eq!(b.search("Inbox", "ALL").await.unwrap().len(), 1);
    assert_eq!(b.search("Projects", "ALL").await.unwrap().len(), 1);

    b.rename_folder("Projects/Deep", "Projects/Deeper").await.unwrap();
    b.delete_folder("Projects/Deeper").await.unwrap();

    let raw = b"From: a@example.org\r\nTo: b@example.org\r\nSubject: appended\r\nMessage-ID: <app@example.org>\r\n\r\nhello\r\n";
    b.append_flagged("Projects", &Flags { seen: true, ..Default::default() }, raw).await.unwrap();
    let mut sink = MemorySink::default();
    sync_account(&mut b, &mut sink).await.unwrap();
    let appended = sink.messages["Projects"].values().find(|m| m.subject.as_deref() == Some("appended")).expect("appended");
    assert!(appended.flags.seen);
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_reports_new_mail_over_event_source() {
    let name = user();
    let mut watcher = jmap(&name);
    let mut watch = watcher.watch("Inbox").await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut sender = jmap(&name);
    sender.send(&mail(&name, &name, "pushed")).await.unwrap();
    let event = tokio::time::timeout(Duration::from_secs(20), watch.events.recv()).await.expect("event in time").expect("channel open");
    assert_eq!(event, Event::FolderChanged { folder: "Inbox".into() });
}

#[tokio::test(flavor = "multi_thread")]
async fn sending_to_another_user_delivers_there() {
    let alice = user();
    let bob = user();
    jmap(&alice).send(&mail(&alice, &bob, "hello bob")).await.unwrap();
    let mut b = jmap(&bob);
    let uids = wait_for(&mut b, "Inbox", 1).await;
    let summaries = b.fetch_summaries("Inbox", &uids).await.unwrap();
    assert_eq!(summaries[0].subject.as_deref(), Some("hello bob"));
}
