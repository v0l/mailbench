use super::mail::{AccountRef, MailTools};
use super::prefs::{self, AgentPrefs};
use super::tools::Tools;
use super::{Server, ServerConfig, token};
use crate::accounts;
use crate::real::RealStore;
use crate::store::Store;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const LOCK_FILE: &str = "mcp.lock";

pub struct Running {
    pub server: Server,
    pub port: u16,
    _lock: File,
}

pub fn take_lock(dir: &Path) -> Result<File, String> {
    let path = dir.join(LOCK_FILE);
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&path).map_err(|e| format!("could not open {}: {e}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(format!(
            "another mailbench agent server already uses this mail store (lock {}). Stop it first: the app's Agents switch or `mailbench mcp serve` can run, but only one at a time.",
            path.display()
        )),
        Err(std::fs::TryLockError::Error(e)) => Err(format!("could not lock {}: {e}", path.display())),
    }
}

pub fn lock_held(dir: &Path) -> bool {
    let path = dir.join(LOCK_FILE);
    let Ok(file) = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path) else { return false };
    file.try_lock().is_err()
}

fn wait_settled(real: &RealStore, limit: Duration) {
    let deadline = std::time::Instant::now() + limit;
    while !real.settled() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn start(store: &Store, dir: &Path, port: Option<u16>, wait_for_sync: bool) -> Result<Running, String> {
    let lock = take_lock(dir)?;
    let configs = accounts::load_all(store);
    if configs.is_empty() {
        return Err("no accounts; add one in the app".into());
    }
    let setups: Vec<_> = configs.iter().filter_map(|c| accounts::setup_for(c, store).ok()).collect();
    let refs: Vec<AccountRef> = configs
        .iter()
        .filter(|c| accounts::setup_for(c, store).is_ok())
        .map(|c| AccountRef { id: c.id.clone(), name: c.name.clone(), address: c.address.clone(), nostr: c.is_nostr() })
        .collect();
    let clone = store.try_clone().map_err(|e| e.to_string())?;
    let real = RealStore::open(clone, setups);
    real.set_send_hold(Duration::ZERO);
    for c in &configs {
        real.set_signature(&c.id, &c.signature);
    }
    if wait_for_sync {
        wait_settled(&real, Duration::from_secs(120));
    }
    let prefs_store = Mutex::new(store.try_clone().map_err(|e| e.to_string())?);
    let reader: Box<dyn Fn() -> AgentPrefs + Send + Sync> = Box::new(move || prefs::load(&prefs_store.lock().unwrap_or_else(|e| e.into_inner())));
    let current = prefs::load(store);
    let token = token::load_or_create(store)?.0;
    let tools: Arc<dyn Tools> = Arc::new(MailTools::new(Box::new(real), refs, reader, Some(dir.to_path_buf())));
    let port = port.unwrap_or(current.port);
    let server = Server::start(ServerConfig { token, port, tools })?;
    let bound = server.port();
    Ok(Running { server, port: bound, _lock: lock })
}

pub fn data_dir_of(store: &Store) -> PathBuf {
    store.path().parent().map(Path::to_path_buf).unwrap_or_default()
}
