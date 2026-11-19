use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use mailbench::app::App;
use mailbench::backend::*;
use mailbench::mailstore::{self as ms, MailStore, Query};
use mailbench::real::{AccountSetup, RealStore};
use mailbench::store::{KdfParams, Store};
use nostr_relay_builder::MockRelay;
use std::time::{Duration, Instant};

type State = (bool, App);

fn secret(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

fn nostr(n: u8, relay: &str) -> NostrBackend {
    NostrBackend::new(NostrConfig::new(&secret(n), vec![relay.to_string()])).unwrap()
}

fn dm(from: &NostrBackend, to: &NostrBackend, subject: &str) -> OutgoingMessage {
    OutgoingMessage {
        from: Address { name: None, address: from.address() },
        to: vec![Address { name: None, address: to.address() }],
        subject: subject.into(),
        body: "hello over nostr".into(),
        ..Default::default()
    }
}

fn setup(b: NostrBackend) -> AccountSetup {
    AccountSetup { id: "nostr".into(), name: "dms".into(), address: b.address(), sender: String::new(), backend: Box::new(b) }
}

fn harness(real: RealStore) -> Harness<'static, State> {
    let app = App::with_store(Box::new(real));
    Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_pixels_per_point(1.0).build_ui_state(
        |ui, (ready, app): &mut State| {
            if !*ready {
                mailbench::kit::setup(ui.ctx());
                *ready = true;
                ui.ctx().request_repaint();
                return;
            }
            app.show(ui);
        },
        (false, app),
    )
}

fn until(h: &mut Harness<'static, State>, what: &str, cond: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        h.run();
        if cond(&h.state().1) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

async fn open() -> (Harness<'static, State>, tempfile::TempDir, MockRelay) {
    let relay = MockRelay::run().await.unwrap();
    let url = relay.url().await.to_string();
    let mut alice = nostr(1, &url);
    let bob = nostr(2, &url);
    alice.send(&dm(&alice, &bob, "Plans")).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let real = RealStore::open(store, vec![setup(bob)]);
    let mut h = harness(real);
    until(&mut h, "dm in inbox", |a| a.store.counts(&Query::new(ms::Folder::Inbox)).all == 1);
    (h, dir, relay)
}

#[test]
fn a_nostr_account_reports_no_server_features() {
    mailbench::runtime::block_on(async {
        let (h, _d, _r) = open().await;
        let info = &h.state().1.store.accounts()[0];
        assert!(!info.server_folders && !info.server_search && !info.server_flags);
    });
}

#[test]
fn a_nostr_account_has_no_new_folder_action() {
    mailbench::runtime::block_on(async {
        let (h, _d, _r) = open().await;
        assert!(h.query_by_label_contains("new folder in dms").is_none());
    });
}

#[test]
fn unread_is_local_and_survives_a_resync() {
    mailbench::runtime::block_on(async {
        let (mut h, _d, _r) = open().await;
        let q = Query::new(ms::Folder::Inbox);
        assert_eq!(h.state().1.store.counts(&q).unread, 1);
        let id = h.state().1.store.list(&q, 0, 10).items[0].id;
        h.state_mut().1.store.set_flags(id, ms::FlagChange { unread: Some(false), flagged: None }).unwrap();
        until(&mut h, "read", |a| a.store.counts(&q).unread == 0);
        std::thread::sleep(Duration::from_millis(1500));
        h.run();
        assert_eq!(h.state().1.store.counts(&q).unread, 0);
        assert_eq!(h.state().1.store.counts(&q).all, 1);
    });
}

#[test]
fn archiving_moves_a_dm_to_the_local_archive_folder() {
    mailbench::runtime::block_on(async {
        let (mut h, _d, _r) = open().await;
        let q = Query::new(ms::Folder::Inbox);
        let id = h.state().1.store.list(&q, 0, 10).items[0].id;
        h.state_mut().1.store.move_to(id, ms::Folder::Archive).unwrap();
        until(&mut h, "archived", |a| {
            a.store.counts(&q).all == 0 && a.store.counts(&Query::new(ms::Folder::Archive)).all == 1
        });
        std::thread::sleep(Duration::from_millis(1500));
        h.run();
        assert_eq!(h.state().1.store.counts(&Query::new(ms::Folder::Archive)).all, 1);
        assert_eq!(h.state().1.store.counts(&q).all, 0);
    });
}

#[test]
fn a_sender_picture_is_blocked_until_images_are_allowed_and_nip05_needs_proof() {
    mailbench::runtime::block_on(async {
        let relay = MockRelay::run().await.unwrap();
        let url = relay.url().await.to_string();
        let mut alice = nostr(1, &url);
        let bob = nostr(2, &url);
        let keys = nostr_sdk::Keys::parse(&secret(1)).unwrap();
        let client = nostr_sdk::Client::new(keys.clone());
        client.add_relay(url.as_str()).await.unwrap();
        client.connect().await;
        let meta = nostr_sdk::Metadata::new().name("Alice").picture(nostr_sdk::Url::parse("https://img.example.test/a.png").unwrap()).nip05("alice@127.0.0.1.invalid");
        client.send_event_builder(nostr_sdk::EventBuilder::metadata(&meta)).await.unwrap();
        alice.send(&dm(&alice, &bob, "Plans")).await.unwrap();

        let dir = tempfile::tempdir().unwrap();
        let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
        let real = RealStore::open(store, vec![setup(bob)]);
        let mut h = harness(real);
        until(&mut h, "dm in inbox", |a| a.store.counts(&Query::new(ms::Folder::Inbox)).all == 1);
        let id = h.state().1.store.list(&Query::new(ms::Folder::Inbox), 0, 10).items[0].id;
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            h.run();
            if let Ok(m) = h.state().1.store.get(id) {
                if m.body_state == ms::BodyState::Ready && m.remote_images.is_some() {
                    assert_eq!(m.remote_images.as_deref(), Some("img.example.test"));
                    assert!(m.headers.all.iter().any(|(k, v)| k == "X-Nostr-Picture" && v == "https://img.example.test/a.png"));
                    assert!(!m.headers.all.iter().any(|(k, _)| k == "X-Nostr-Nip05"));
                    assert!(!h.state().1.images_loaded.contains(&id));
                    return;
                }
            }
            assert!(Instant::now() < deadline, "picture header never appeared");
            std::thread::sleep(Duration::from_millis(100));
        }
    });
}
