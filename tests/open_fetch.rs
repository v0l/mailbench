use mailbench::backend::*;
use mailbench::mailstore::{self as ms, BodyState, MailStore, Query};
use mailbench::real::{AccountSetup, RealStore};
use mailbench::runtime::block_on;
use mailbench::store::{KdfParams, Store};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

mod common;

fn dovecot_imap(user: &str) -> ImapBackend {
    let mut cfg = ImapConfig::new("127.0.0.1", common::DC_IMAP, Security::StartTls, Auth::Password { user: user.into(), password: "secret".into() });
    cfg.accept_invalid_certs = true;
    ImapBackend::new(cfg)
}

fn message(n: u32) -> String {
    let text = format!("Hello number {n}, here is the news.\r\n").repeat(200);
    format!(
        "From: Sender {n} <s{n}@example.test>\r\nTo: me@example.test\r\nSubject: Fetch {n}\r\nMessage-ID: <fetch{n}@example.test>\r\nDate: Tue, 14 Nov 2023 22:13:20 +0000\r\n\r\n{text}"
    )
}

fn setup(id: &str, backend: Box<dyn Backend>) -> AccountSetup {
    AccountSetup { id: id.into(), name: "test".into(), address: "me@example.test".into(), sender: String::new(), backend }
}

fn wait_for(what: &str, cond: impl Fn() -> bool) {
    let started = Instant::now();
    while !cond() {
        assert!(started.elapsed() < Duration::from_secs(60), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn opening_a_message_that_needs_a_fetch_shows_headers_at_once_and_fills_in_quickly() {
    if !common::dovecot() {
        return;
    }
    let user = format!("of{}", std::process::id());
    let mut raw = dovecot_imap(&user);
    for n in 0..24 {
        block_on(raw.append("INBOX", message(n).as_bytes())).unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let real = RealStore::open(store, vec![setup("acct", Box::new(dovecot_imap(&user)))]);
    let query = Query::new(ms::Folder::Inbox);
    wait_for("sync", || real.counts(&query).all >= 24);
    let mut first_get = Vec::new();
    let mut total = Vec::new();
    for row in 0..20 {
        let id = real.list(&query, row, 1).items[0].id;
        let started = Instant::now();
        let m = real.get(id).unwrap();
        first_get.push(started.elapsed());
        if m.body_state == BodyState::Loading {
            assert!(m.summary.subject.starts_with("Fetch"));
        }
        wait_for("body", || real.get(id).unwrap().body_state == BodyState::Ready);
        total.push(started.elapsed());
    }
    first_get.sort();
    total.sort();
    eprintln!("first get median {:?} worst {:?}; filled median {:?} worst {:?}", first_get[10], first_get[19], total[10], total[19]);
    assert!(first_get[10] < Duration::from_millis(5), "headers took {:?}", first_get[10]);
    assert!(total[10] < Duration::from_millis(250), "fetch open median {:?}", total[10]);
}

struct Counting {
    fetched: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Backend for Counting {
    fn kind(&self) -> &'static str {
        "counting"
    }

    fn fork(&self) -> Option<Box<dyn Backend>> {
        Some(Box::new(Counting { fetched: self.fetched.clone() }))
    }

    async fn list_folders(&mut self) -> Result<Vec<Folder>> {
        Ok(vec![Folder { name: "INBOX".into(), delimiter: None, role: Some(FolderRole::Inbox), selectable: true }])
    }

    async fn fetch(&mut self, _folder: &str, uid: u32) -> Result<Vec<u8>> {
        self.fetched.fetch_add(1, Ordering::SeqCst);
        Ok(message(uid).into_bytes())
    }
}

#[test]
fn the_app_prefetches_the_next_message_when_the_selection_lands_on_a_row() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    {
        let mut sink = StoreSink::new(&store, "acct", true);
        sink.put_folders(&[Folder { name: "INBOX".into(), delimiter: None, role: Some(FolderRole::Inbox), selectable: true }]).unwrap();
        let messages: Vec<_> = (1..=5u32)
            .map(|uid| MessageSummary {
                uid,
                subject: Some(format!("Fetch {uid}")),
                from: vec![Address { name: None, address: format!("s{uid}@example.test") }],
                date: Some(1_700_000_000 + uid as i64),
                ..Default::default()
            })
            .collect();
        sink.put_summaries("INBOX", &messages).unwrap();
        sink.put_token("INBOX", &SyncToken { uid_validity: 7, uid_next: 6, highest_mod_seq: None }).unwrap();
    }
    let fetched = Arc::new(AtomicUsize::new(0));
    let real = RealStore::open(store, vec![setup("acct", Box::new(Counting { fetched: fetched.clone() }))]);
    let app = mailbench::app::App::with_store(Box::new(real));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_pixels_per_point(1.0).build_ui_state(
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
    let started = Instant::now();
    while fetched.load(Ordering::SeqCst) < 2 {
        h.run();
        assert!(started.elapsed() < Duration::from_secs(10), "only {} fetches started", fetched.load(Ordering::SeqCst));
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn prefetch_requests_bodies_of_the_given_messages() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    {
        let mut sink = StoreSink::new(&store, "acct", true);
        sink.put_folders(&[Folder { name: "INBOX".into(), delimiter: None, role: Some(FolderRole::Inbox), selectable: true }]).unwrap();
        let messages: Vec<_> = (1..=5u32)
            .map(|uid| MessageSummary {
                uid,
                subject: Some(format!("Fetch {uid}")),
                from: vec![Address { name: None, address: format!("s{uid}@example.test") }],
                date: Some(1_700_000_000 + uid as i64),
                ..Default::default()
            })
            .collect();
        sink.put_summaries("INBOX", &messages).unwrap();
        sink.put_token("INBOX", &SyncToken { uid_validity: 7, uid_next: 6, highest_mod_seq: None }).unwrap();
    }
    let fetched = Arc::new(AtomicUsize::new(0));
    let real = RealStore::open(store, vec![setup("acct", Box::new(Counting { fetched: fetched.clone() }))]);
    let query = Query::new(ms::Folder::Inbox);
    wait_for("list", || real.counts(&query).all == 5);
    let ids: Vec<_> = (1..=2).map(|row| real.list(&query, row, 1).items[0].id).collect();
    real.prefetch(&ids);
    wait_for("prefetch", || fetched.load(Ordering::SeqCst) == 2);
}
