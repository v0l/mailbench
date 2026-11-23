use mailbench::app::{Draft, Reply};
use mailbench::backend::*;
use mailbench::mailstore::{self as ms, MailStore, OutAttachment, Outgoing, Query};
use mailbench::real::{AccountSetup, RealStore};
use mailbench::runtime::block_on;
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

fn account(name: &str) -> AccountSetup {
    AccountSetup {
        id: "acct".into(),
        name: "test".into(),
        address: format!("{name}@localhost"),
        sender: "Me Myself".into(),
        backend: Box::new(Account { mail: Box::new(imap(name)), outbound: Box::new(smtp(name)) }),
    }
}

fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(40);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn users(prefix: &str) -> (String, String) {
    assert!(common::greenmail(), "docker is required for greenmail tests");
    let tag = format!("{prefix}{}", std::process::id());
    let (me, peer) = (format!("{tag}me"), format!("{tag}peer"));
    common::create_greenmail_user(&me);
    common::create_greenmail_user(&peer);
    (me, peer)
}

fn open(me: &str, dir: &tempfile::TempDir) -> RealStore {
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let real = RealStore::open(store, vec![account(me)]);
    real.set_send_hold(Duration::ZERO);
    real
}

fn server_messages(name: &str, folder: &str) -> Vec<Vec<u8>> {
    let mut b = imap(name);
    let uids = block_on(b.search(folder, "ALL")).unwrap_or_default();
    uids.into_iter().filter_map(|u| block_on(b.fetch(folder, u)).ok()).collect()
}

fn text(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).to_string()
}

fn seed(me: &str, peer: &str) {
    block_on(smtp(peer).send(&OutgoingMessage {
        from: Address { name: Some("Peer Person".into()), address: format!("{peer}@localhost") },
        to: vec![Address { name: None, address: format!("{me}@localhost") }],
        subject: "Quote this".into(),
        body: "first line\nsecond line".into(),
        message_id: Some("orig-123@peer.test".into()),
        ..Default::default()
    }))
    .unwrap();
}

fn first_inbox(real: &RealStore) -> ms::Message {
    until("inbox sync", || real.counts(&Query::new(ms::Folder::Inbox)).all == 1);
    let id = real.list(&Query::new(ms::Folder::Inbox), 0, 1).items[0].id;
    let mut got = None;
    until("body parsed", || {
        let m = real.get(id).unwrap();
        let ready = m.body_state == ms::BodyState::Ready && !m.paragraphs.is_empty();
        if ready {
            got = Some(m);
        }
        ready
    });
    got.unwrap()
}

#[test]
fn reply_arrives_quoted_with_threading_headers_and_a_copy_lands_in_sent() {
    let (me, peer) = users("rc");
    seed(&me, &peer);
    let dir = tempfile::tempdir().unwrap();
    let mut real = open(&me, &dir);
    let original = first_inbox(&real);
    assert_eq!(original.message_id.as_deref(), Some("orig-123@peer.test"));

    let mut draft = Draft::reply(&original, Reply::One, "", &[format!("{me}@localhost")], Vec::new(), None);
    draft.body = format!("my answer{}", draft.body);
    real.append(ms::Folder::Sent, draft.outgoing()).unwrap();

    until("reply delivered", || !server_messages(&peer, "INBOX").is_empty());
    let delivered = text(&server_messages(&peer, "INBOX")[0]);
    assert!(delivered.contains("In-Reply-To: <orig-123@peer.test>"), "{delivered}");
    assert!(delivered.contains("References: <orig-123@peer.test>"), "{delivered}");
    assert!(delivered.contains("Subject: Re: Quote this"), "{delivered}");
    assert!(delivered.contains("my answer"), "{delivered}");
    assert!(delivered.contains("wrote:"), "{delivered}");
    assert!(delivered.contains("> first line"), "{delivered}");
    assert!(delivered.contains("> second line"), "{delivered}");

    until("sent copy on the server", || !server_messages(&me, "Sent").is_empty());
    let copy = text(&server_messages(&me, "Sent")[0]);
    assert!(copy.contains("In-Reply-To: <orig-123@peer.test>"), "{copy}");
    assert!(copy.contains("> first line"), "{copy}");
}

#[test]
fn a_held_send_can_be_undone_and_is_not_delivered() {
    let (me, peer) = users("ru");
    let dir = tempfile::tempdir().unwrap();
    let mut real = open(&me, &dir);
    real.set_send_hold(Duration::from_secs(60));
    let out = Outgoing {
        account: 0,
        to: format!("{peer}@localhost"),
        subject: "never sent".into(),
        body: "oops".into(),
        ..Default::default()
    };
    let id = real.append(ms::Folder::Sent, out).unwrap();
    assert_eq!(real.counts(&Query::new(ms::Folder::Sent)).all, 1);
    let back = real.undo_send(id).unwrap().expect("still held");
    assert_eq!(back.subject, "never sent");
    assert_eq!(real.counts(&Query::new(ms::Folder::Sent)).all, 0);
    std::thread::sleep(Duration::from_secs(3));
    assert!(server_messages(&peer, "INBOX").is_empty());
}

#[test]
fn a_held_send_goes_out_once_the_hold_passes() {
    let (me, peer) = users("rh");
    let dir = tempfile::tempdir().unwrap();
    let mut real = open(&me, &dir);
    real.set_send_hold(Duration::from_secs(2));
    let out = Outgoing { account: 0, to: format!("{peer}@localhost"), subject: "held".into(), body: "later".into(), ..Default::default() };
    real.append(ms::Folder::Sent, out).unwrap();
    std::thread::sleep(Duration::from_millis(800));
    assert!(server_messages(&peer, "INBOX").is_empty());
    until("held send delivered", || !server_messages(&peer, "INBOX").is_empty());
}

#[test]
fn drafts_follow_the_server_and_are_replaced_in_place() {
    let (me, _peer) = users("rd");
    let dir = tempfile::tempdir().unwrap();
    let mut real = open(&me, &dir);
    let mut out = Outgoing { account: 0, to: "someone@example.test".into(), subject: "half written".into(), body: "v1".into(), ..Default::default() };
    let mid = real.save_draft(out.clone()).unwrap();
    until("draft on the server", || server_messages(&me, "Drafts").len() == 1);
    assert!(text(&server_messages(&me, "Drafts")[0]).contains("v1"));

    out.body = "v2 with more".into();
    out.message_id = Some(mid.clone());
    real.save_draft(out).unwrap();
    until("draft replaced", || {
        let all = server_messages(&me, "Drafts");
        all.len() == 1 && text(&all[0]).contains("v2 with more")
    });

    until("draft synced into the Drafts list", || {
        let page = real.list(&Query::new(ms::Folder::Drafts), 0, 10);
        page.total == 1 && page.items[0].subject == "half written"
    });
    let item = real.list(&Query::new(ms::Folder::Drafts), 0, 1).items[0].id;
    let mut reopened = None;
    until("draft body available", || {
        let item = real.list(&Query::new(ms::Folder::Drafts), 0, 1).items[0].id;
        let _ = real.get(item);
        reopened = real.open_draft(item).ok();
        reopened.is_some()
    });
    let reopened = reopened.unwrap();
    assert_eq!(reopened.subject, "half written");
    assert!(reopened.body.contains("v2 with more"));
    assert_eq!(reopened.message_id.as_deref(), Some(mid.as_str()));

    real.discard_draft(0, &mid);
    until("draft removed from the server", || server_messages(&me, "Drafts").is_empty());
}

#[test]
fn a_draft_from_the_server_reopens_in_the_composer() {
    let (me, _peer) = users("ro");
    let raw = format!(
        "From: {me}@localhost\r\nTo: Zed <zed@example.test>\r\nSubject: from another client\r\nMessage-ID: <other@client.test>\r\nIn-Reply-To: <parent@x.test>\r\nDate: Wed, 18 Nov 2026 10:31:00 +0000\r\n\r\ncontinue me\r\n"
    );
    block_on(imap(&me).append_flagged("Drafts", &Flags { draft: true, ..Default::default() }, raw.as_bytes())).ok();
    let mut b = imap(&me);
    if server_messages(&me, "Drafts").is_empty() {
        block_on(b.create_folder("Drafts")).unwrap();
        block_on(b.append_flagged("Drafts", &Flags { draft: true, ..Default::default() }, raw.as_bytes())).unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let mut real = open(&me, &dir);
    until("draft listed", || real.counts(&Query::new(ms::Folder::Drafts)).all == 1);
    let id = real.list(&Query::new(ms::Folder::Drafts), 0, 1).items[0].id;
    let mut out = None;
    until("draft body fetched", || {
        let _ = real.get(id);
        out = real.open_draft(id).ok();
        out.is_some()
    });
    let out = out.unwrap();
    assert_eq!(out.to, "Zed <zed@example.test>");
    assert_eq!(out.body, "continue me");
    assert_eq!(out.in_reply_to.as_deref(), Some("parent@x.test"));
    let d = Draft::from_outgoing(out, "");
    assert_eq!(d.message_id.as_deref(), Some("other@client.test"));
}

#[test]
fn an_attachment_round_trips_and_is_encrypted_while_queued() {
    let (me, peer) = users("ra");
    let dir = tempfile::tempdir().unwrap();
    let mut real = open(&me, &dir);
    real.set_send_hold(Duration::from_secs(60));
    let data: Vec<u8> = (0..5000u32).map(|n| (n % 251) as u8).collect();
    let marker = b"PLAINTEXT-ATTACHMENT-MARKER";
    let mut payload = marker.to_vec();
    payload.extend_from_slice(&data);
    let out = Outgoing {
        account: 0,
        to: format!("{peer}@localhost"),
        subject: "with file".into(),
        body: "see attached".into(),
        attachments: vec![OutAttachment { name: "board.bin".into(), mime: "application/octet-stream".into(), data: payload.clone(), cid: None }],
        ..Default::default()
    };
    let id = real.append(ms::Folder::Sent, out).unwrap();
    let on_disk = std::fs::read(dir.path().join("mail.db")).unwrap();
    let wal = std::fs::read(dir.path().join("mail.db-wal")).unwrap_or_default();
    assert!(!on_disk.windows(marker.len()).any(|w| w == marker), "attachment bytes are in the database file in clear");
    assert!(!wal.windows(marker.len()).any(|w| w == marker), "attachment bytes are in the WAL in clear");
    assert_eq!(real.get(id).unwrap().attachments[0].name, "board.bin");
    assert_eq!(real.attachments(id).unwrap()[0].data.as_slice(), payload.as_slice());

    let out = real.undo_send(id).unwrap().unwrap();
    real.set_send_hold(Duration::ZERO);
    real.append(ms::Folder::Sent, out).unwrap();

    until("attachment delivered", || !server_messages(&peer, "INBOX").is_empty());
    let raw = server_messages(&peer, "INBOX").remove(0);
    assert!(text(&raw).contains("multipart/mixed"));
    let parsed = mail_parser::MessageParser::default().parse(&raw).unwrap();
    let att: Vec<_> = parsed.attachments().collect();
    assert_eq!(att.len(), 1);
    assert_eq!(att[0].attachment_name(), Some("board.bin"));
    assert_eq!(att[0].decoded().as_ref(), payload.as_slice());
}

#[test]
fn addresses_seen_in_synced_mail_and_sent_to_are_suggested() {
    let (me, peer) = users("rs");
    seed(&me, &peer);
    let dir = tempfile::tempdir().unwrap();
    let real = open(&me, &dir);
    first_inbox(&real);
    let hits = real.suggest("peer pers", 5);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].address, format!("{peer}@localhost"));
    assert!(real.suggest("zzzz-nobody", 5).is_empty());
}

#[test]
fn kittest_formatted_message_reaches_the_server_with_both_parts() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    let (me, peer) = users("fm");
    seed(&me, &peer);
    let dir = tempfile::tempdir().unwrap();
    let real = open(&me, &dir);
    let app = mailbench::app::App::with_store(Box::new(real));
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_pixels_per_point(1.0).build_ui_state(
        |ui, (ready, app): &mut (bool, mailbench::app::App)| {
            if !*ready {
                mailbench::kit::setup(ui.ctx());
                *ready = true;
                ui.ctx().request_repaint();
                return;
            }
            app.show(ui);
        },
        (false, app),
    );
    h.run();
    h.run();
    h.key_press(egui::Key::C);
    h.run();
    h.run();
    h.state_mut().1.draft.as_mut().unwrap().to = format!("{peer}@localhost");
    h.state_mut().1.draft.as_mut().unwrap().subject = "formatted".into();
    h.run();
    h.get_by_label("message body").click();
    h.run();
    let body = h.get_by_label("message body");
    body.type_text("first");
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    h.get_by_label("message body").type_text("second");
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    h.get_by_label("message body").type_text("third word");
    h.run();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::B);
    h.run();
    h.get_by_label("message body").type_text("bold");
    h.run();
    h.state_mut().1.draft.as_mut().unwrap().body = "first\nsecond\nthird **word**\n\nsee [docs](https://example.test/docs)\n\n- one\n- two".into();
    h.run();
    h.run();
    let img = h.render().ok();
    if let Some(img) = img {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/shots");
        std::fs::create_dir_all(&dir).unwrap();
        img.save(dir.join("formatted-composer.png")).unwrap();
    }
    h.get_by_label("send").click();
    h.run();
    h.run();
    until("delivered", || !server_messages(&peer, "INBOX").is_empty());
    let raw = server_messages(&peer, "INBOX").remove(0);
    let parsed = mail_parser::MessageParser::default().parse(&raw).unwrap();
    let plain = parsed.body_text(0).unwrap().replace("\r\n", "\n");
    assert_eq!(plain.trim_end(), "first\nsecond\nthird **word**\n\nsee [docs](https://example.test/docs)\n\n- one\n- two");
    let html = parsed.body_html(0).unwrap().replace("\r\n", "\n");
    assert!(html.contains("<strong>word</strong>"), "{html}");
    assert!(html.contains("<a href=\"https://example.test/docs\">docs</a>"), "{html}");
    assert!(html.contains("<ul><li>one</li><li>two</li></ul>"), "{html}");
    assert!(html.contains("first<br>\nsecond<br>\nthird"), "{html}");
}

#[test]
fn kittest_reply_through_the_composer_reaches_the_server_quoted() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    let (me, peer) = users("rk");
    seed(&me, &peer);
    let dir = tempfile::tempdir().unwrap();
    let real = open(&me, &dir);
    let app = mailbench::app::App::with_store(Box::new(real));
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_pixels_per_point(1.0).build_ui_state(
        |ui, (ready, app): &mut (bool, mailbench::app::App)| {
            if !*ready {
                mailbench::kit::setup(ui.ctx());
                *ready = true;
                ui.ctx().request_repaint();
                return;
            }
            app.show(ui);
        },
        (false, app),
    );
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        h.run();
        let a = &h.state().1;
        let ready = a.store.counts(&Query::new(ms::Folder::Inbox)).all == 1
            && a.store.list(&Query::new(ms::Folder::Inbox), 0, 1).items.first().is_some_and(|s| a.store.get(s.id).is_ok_and(|m| !m.paragraphs.is_empty()));
        if ready {
            break;
        }
        assert!(Instant::now() < deadline, "inbox never loaded");
        std::thread::sleep(Duration::from_millis(100));
    }
    h.key_press(egui::Key::R);
    h.run();
    h.run();
    let body = h.state().1.draft.as_ref().unwrap().body.clone();
    assert!(body.contains("> first line"), "{body}");
    h.state_mut().1.draft.as_mut().unwrap().body = format!("typed reply{body}");
    h.run();
    h.get_by_label("send").click();
    h.run();
    h.run();
    until("delivered", || !server_messages(&peer, "INBOX").is_empty());
    let delivered = text(&server_messages(&peer, "INBOX")[0]);
    assert!(delivered.contains("In-Reply-To: <orig-123@peer.test>"), "{delivered}");
    assert!(delivered.contains("typed reply"), "{delivered}");
    assert!(delivered.contains("> second line"), "{delivered}");
    until("sent copy", || !server_messages(&me, "Sent").is_empty());
}

struct BlindSearch(Box<dyn Backend>);

#[async_trait::async_trait]
impl Backend for BlindSearch {
    fn kind(&self) -> &'static str {
        self.0.kind()
    }
    fn fork(&self) -> Option<Box<dyn Backend>> {
        Some(Box::new(BlindSearch(self.0.fork()?)))
    }
    async fn capabilities(&mut self) -> Result<Capabilities> {
        self.0.capabilities().await
    }
    async fn list_folders(&mut self) -> Result<Vec<Folder>> {
        self.0.list_folders().await
    }
    async fn create_folder(&mut self, name: &str) -> Result<()> {
        self.0.create_folder(name).await
    }
    async fn open_folder(&mut self, folder: &str) -> Result<FolderState> {
        self.0.open_folder(folder).await
    }
    async fn fetch_summaries(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<MessageSummary>> {
        self.0.fetch_summaries(folder, uids).await
    }
    async fn changed_flags(&mut self, folder: &str, below: u32, since: Option<u64>) -> Result<Vec<(u32, Flags)>> {
        self.0.changed_flags(folder, below, since).await
    }
    async fn fetch(&mut self, folder: &str, uid: u32) -> Result<Vec<u8>> {
        self.0.fetch(folder, uid).await
    }
    async fn set_flags(&mut self, folder: &str, uids: &[u32], update: &FlagUpdate) -> Result<()> {
        self.0.set_flags(folder, uids, update).await
    }
    async fn move_messages(&mut self, folder: &str, uids: &[u32], to: &str) -> Result<()> {
        self.0.move_messages(folder, uids, to).await
    }
    async fn delete_messages(&mut self, folder: &str, uids: &[u32]) -> Result<()> {
        self.0.delete_messages(folder, uids).await
    }
    async fn search(&mut self, folder: &str, query: &str) -> Result<Vec<u32>> {
        if query.contains("HEADER") {
            return Ok(Vec::new());
        }
        self.0.search(folder, query).await
    }
    async fn append_flagged(&mut self, folder: &str, flags: &Flags, raw: &[u8]) -> Result<()> {
        self.0.append_flagged(folder, flags, raw).await
    }
    async fn watch(&mut self, folder: &str) -> Result<Watch> {
        self.0.watch(folder).await
    }
}

fn open_blind(me: &str, dir: &tempfile::TempDir) -> RealStore {
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let mut setup = account(me);
    setup.backend = Box::new(Account { mail: Box::new(BlindSearch(Box::new(imap(me)))), outbound: Box::new(smtp(me)) });
    let real = RealStore::open(store, vec![setup]);
    real.set_send_hold(Duration::ZERO);
    real
}

#[test]
fn snapshots_are_cleaned_even_when_the_server_search_by_message_id_finds_nothing() {
    let (me, _peer) = users("rb");
    let dir = tempfile::tempdir().unwrap();
    let mut real = open_blind(&me, &dir);
    let mut draft = Draft::blank();
    draft.subject = "OWON SPE6103".into();
    draft.body = "Hello,".into();
    for to in ["", "info@owon", "info@owon.com.cc"] {
        draft.to = to.into();
        let mid = real.save_draft(draft.outgoing()).unwrap();
        draft.message_id = Some(mid);
        until("draft pushed", || real.pending_ops() == 0);
    }
    assert_eq!(server_messages(&me, "Drafts").len(), 1, "an autosave left an older snapshot beside the new one");
    let mid = draft.message_id.clone().unwrap();
    real.discard_draft(0, &mid);
    until("discard pushed", || real.pending_ops() == 0);
    assert!(server_messages(&me, "Drafts").is_empty(), "discard left a snapshot on the server");
}

#[test]
fn the_sent_message_is_one_conversation_with_one_card_after_autosaves_and_a_held_send() {
    let (me, peer) = users("rz");
    let dir = tempfile::tempdir().unwrap();
    let mut real = open(&me, &dir);
    real.set_send_hold(Duration::from_secs(2));
    let mut draft = Draft::blank();
    draft.subject = "OWON SPE6103".into();
    draft.body = "Hello,".into();
    for to in ["", "info@owon"] {
        draft.to = to.into();
        let mid = real.save_draft(draft.outgoing()).unwrap();
        draft.message_id = Some(mid);
        until("draft pushed", || real.pending_ops() == 0);
    }
    draft.to = format!("{peer}@localhost");
    draft.body = "Hello, full text".into();
    let mid = draft.message_id.clone().unwrap();
    real.append(ms::Folder::Sent, draft.outgoing()).unwrap();
    real.discard_draft(0, &mid);
    until("sent copy on the server", || server_messages(&me, "Sent").len() == 1);
    until("sent listed", || real.counts(&Query::new(ms::Folder::Sent)).all == 1);
    std::thread::sleep(Duration::from_secs(3));
    let row = real.list(&Query::new(ms::Folder::Sent), 0, 10).items.remove(0);
    let cards = real.thread(row.id);
    let shown: Vec<_> = cards.iter().map(|c| (c.folder.clone(), c.to.clone())).collect();
    assert_eq!(cards.len(), 1, "{shown:?}");
    assert_eq!(row.thread, 1);
    assert_eq!(row.subject, "OWON SPE6103");
    let sent_copy = text(&server_messages(&me, "Sent")[0]);
    assert!(sent_copy.contains("Subject: OWON SPE6103"), "{sent_copy}");
    assert!(server_messages(&me, "Drafts").is_empty());
    assert_eq!(real.counts(&Query::new(ms::Folder::Drafts)).all, 0);
}

#[test]
fn autosaved_snapshots_leave_no_draft_behind_once_sent() {
    let (me, peer) = users("rq");
    let dir = tempfile::tempdir().unwrap();
    let mut real = open(&me, &dir);
    let mut draft = Draft::blank();
    draft.subject = "OWON SPE6103".into();
    draft.body = "Hello,".into();
    for to in ["", "info@owon", "info@owon.com.cc"] {
        draft.to = to.into();
        let mid = real.save_draft(draft.outgoing()).unwrap();
        draft.message_id = Some(mid);
        until("snapshot on the server", || {
            let all = server_messages(&me, "Drafts");
            all.len() == 1 && text(&all[0]).contains(&format!("To: {to}")) || (to.is_empty() && all.len() == 1)
        });
        std::thread::sleep(Duration::from_millis(1500));
    }
    draft.to = format!("{peer}@localhost");
    draft.body = "Hello, full text".into();
    let mid = draft.message_id.clone().unwrap();
    real.append(ms::Folder::Sent, draft.outgoing()).unwrap();
    real.discard_draft(0, &mid);
    until("sent copy", || server_messages(&me, "Sent").len() == 1);
    std::thread::sleep(Duration::from_secs(3));
    assert!(server_messages(&me, "Drafts").is_empty());
    assert!(server_messages(&me, "Trash").is_empty());
    assert_eq!(server_messages(&me, "Sent").len(), 1);
    assert_eq!(real.counts(&Query::new(ms::Folder::Drafts)).all, 0);
}
