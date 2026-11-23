use mailbench::backend::*;
use mailbench::real::{AccountSetup, RealStore};
use mailbench::store::{KdfParams, Store};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

fn open() -> (RealStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(&dir.path().join("mail.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
    let real = RealStore::open(
        store,
        vec![AccountSetup { id: "acct".into(), name: "test".into(), address: "me@example.test".into(), sender: String::new(), backend: Box::new(Offline) }],
    );
    (real, dir)
}

fn settle(real: &RealStore) -> u64 {
    let mut last = real.builds();
    let mut stable = Instant::now();
    let started = Instant::now();
    while stable.elapsed() < Duration::from_millis(1500) {
        assert!(started.elapsed() < Duration::from_secs(30), "builds never settled");
        std::thread::sleep(Duration::from_millis(20));
        let now = real.builds();
        if now != last {
            last = now;
            stable = Instant::now();
        }
    }
    last
}

#[test]
fn a_burst_of_refreshes_causes_one_build() {
    let (real, _dir) = open();
    let before = settle(&real);
    for _ in 0..100 {
        real.request_rebuild();
        std::thread::sleep(Duration::from_millis(2));
    }
    let after = settle(&real);
    assert_eq!(after - before, 1, "100 refreshes in 200 ms made {} builds", after - before);
}

#[test]
fn a_user_change_during_a_build_causes_exactly_one_more_build() {
    let (real, _dir) = open();
    let after_plain = settle(&real);
    let once = Arc::new(AtomicBool::new(false));
    let guard = once.clone();
    let handle = real.bump_handle();
    real.on_build(Arc::new(move || {
        if !guard.swap(true, Ordering::SeqCst) {
            handle();
        }
    }));
    real.request_rebuild();
    let after = settle(&real);
    assert_eq!(after - after_plain, 2, "expected the build plus exactly one more, got {}", after - after_plain);
}

#[test]
fn a_refresh_during_a_running_build_causes_exactly_one_more_build() {
    let (real, _dir) = open();
    let before = settle(&real);
    let entered = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let (e, r) = (entered.clone(), release.clone());
    real.on_build(Arc::new(move || {
        e.store(true, Ordering::SeqCst);
        while !r.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
    }));
    real.request_rebuild();
    let started = Instant::now();
    while !entered.load(Ordering::SeqCst) {
        assert!(started.elapsed() < Duration::from_secs(10), "build never started");
        std::thread::sleep(Duration::from_millis(5));
    }
    real.request_rebuild();
    release.store(true, Ordering::SeqCst);
    let after = settle(&real);
    assert_eq!(after - before, 2, "expected the running build plus exactly one more, got {}", after - before);
}

#[test]
fn an_idle_store_builds_nothing() {
    let (real, _dir) = open();
    let before = settle(&real);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(real.builds(), before);
}
