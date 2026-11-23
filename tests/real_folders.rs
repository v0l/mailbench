use mailbench::backend::*;
use mailbench::mailstore::{self as ms, MailStore};
use mailbench::real::{AccountSetup, RealStore};
use mailbench::store::{KdfParams, Store, Unlock};
use std::time::{Duration, Instant};

struct Listed {
    fail_open: bool,
    stall: bool,
}

fn folder(name: &str, role: Option<FolderRole>) -> Folder {
    Folder { name: name.into(), delimiter: Some("/".into()), role, selectable: true }
}

#[async_trait::async_trait]
impl Backend for Listed {
    fn kind(&self) -> &'static str {
        "listed"
    }

    async fn list_folders(&mut self) -> Result<Vec<Folder>> {
        tokio::time::sleep(Duration::from_millis(800)).await;
        Ok(vec![
            folder("INBOX", Some(FolderRole::Inbox)),
            folder("Sent", Some(FolderRole::Sent)),
            folder("Archive", Some(FolderRole::Archive)),
            folder("Projects", None),
        ])
    }

    async fn open_folder(&mut self, _folder: &str) -> Result<FolderState> {
        if self.stall {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
        if self.fail_open {
            return Err(BackendError::Connection("dropped".into()));
        }
        Ok(FolderState { token: SyncToken { uid_validity: 1, uid_next: 1, highest_mod_seq: None }, uids: Vec::new() })
    }
}

fn setup(id: &str, fail_open: bool) -> AccountSetup {
    AccountSetup { id: id.into(), name: id.into(), address: format!("{id}@x"), sender: String::new(), backend: Box::new(Listed { fail_open, stall: false }) }
}

fn stalled(id: &str) -> AccountSetup {
    AccountSetup { id: id.into(), name: id.into(), address: format!("{id}@x"), sender: String::new(), backend: Box::new(Listed { fail_open: false, stall: true }) }
}

fn store(dir: &tempfile::TempDir) -> Store {
    let path = dir.path().join("mail.db");
    if path.exists() {
        Store::open(&path, &[Unlock::Password("pw")]).unwrap()
    } else {
        Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap()
    }
}

fn wait_for_folders(real: &RealStore, account: usize) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let roles: Vec<_> = real.account_folders(account).into_iter().map(|f| f.folder).collect();
        let custom: Vec<_> = real.tree().into_iter().filter(|t| t.account == account).map(|t| t.path).collect();
        if roles == vec![ms::Folder::Inbox, ms::Folder::Sent, ms::Folder::Archive] && custom == vec!["Projects".to_string()] {
            return;
        }
        assert!(Instant::now() < deadline, "folders never listed: {roles:?} {custom:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_new_account_with_no_mail_lists_its_folders_after_the_first_sync() {
    let dir = tempfile::tempdir().unwrap();
    let real = RealStore::open(store(&dir), vec![setup("a", false)]);
    wait_for_folders(&real, 0);
}

#[test]
fn an_account_added_while_running_lists_its_folders() {
    let dir = tempfile::tempdir().unwrap();
    {
        let real = RealStore::open(store(&dir), vec![setup("a", false)]);
        wait_for_folders(&real, 0);
    }
    let real = RealStore::open(store(&dir), vec![setup("a", false), setup("b", false)]);
    wait_for_folders(&real, 1);
}

#[test]
fn folders_are_listed_while_the_first_sync_is_still_fetching() {
    let dir = tempfile::tempdir().unwrap();
    let real = RealStore::open(store(&dir), vec![stalled("a")]);
    wait_for_folders(&real, 0);
}

#[test]
fn a_first_sync_that_fails_after_listing_folders_still_lists_them() {
    let dir = tempfile::tempdir().unwrap();
    let real = RealStore::open(store(&dir), vec![setup("a", true)]);
    wait_for_folders(&real, 0);
}
