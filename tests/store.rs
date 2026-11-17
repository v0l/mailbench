use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use mailbench::store::{Keychain, KdfParams, Store, StoreError, Unlock};

#[derive(Default)]
struct MemKeychain {
    map: RefCell<HashMap<String, Vec<u8>>>,
}

impl Keychain for MemKeychain {
    fn get(&self, a: &str) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self.map.borrow().get(a).cloned())
    }
    fn set(&self, a: &str, s: &[u8]) -> Result<(), StoreError> {
        self.map.borrow_mut().insert(a.into(), s.to_vec());
        Ok(())
    }
    fn delete(&self, a: &str) -> Result<(), StoreError> {
        self.map.borrow_mut().remove(a);
        Ok(())
    }
}

struct DeadKeychain;

impl Keychain for DeadKeychain {
    fn get(&self, _: &str) -> Result<Option<Vec<u8>>, StoreError> {
        Err(StoreError::Keychain("no secret service".into()))
    }
    fn set(&self, _: &str, _: &[u8]) -> Result<(), StoreError> {
        Err(StoreError::Keychain("no secret service".into()))
    }
    fn delete(&self, _: &str) -> Result<(), StoreError> {
        Err(StoreError::Keychain("no secret service".into()))
    }
}

const SUBJECT: &str = "Quarterly zebracorn budget";
const BODY: &str = "Please review the xylophone invoice attached";

fn path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("mail.db")
}

fn fill(s: &Store) {
    s.put("message", "m1", format!("Subject: {SUBJECT}\r\n\r\n{BODY}").as_bytes()).unwrap();
    s.index_text("m1", &format!("{SUBJECT} {BODY}")).unwrap();
}

#[test]
fn round_trip_and_search() {
    let dir = tempfile::tempdir().unwrap();
    let kc = MemKeychain::default();
    let s = Store::create(&path(&dir), Some(&kc), None).unwrap();
    fill(&s);
    drop(s);
    let s = Store::open(&path(&dir), &[Unlock::Keychain(&kc)]).unwrap();
    let got = s.get("message", "m1").unwrap().unwrap();
    assert!(String::from_utf8_lossy(&got).contains(BODY));
    assert_eq!(s.search("XYLOPHONE zebracorn").unwrap(), vec!["m1"]);
    assert!(s.search("missing").unwrap().is_empty());
    assert_eq!(s.list("message").unwrap(), vec!["m1"]);
    s.delete("message", "m1").unwrap();
    assert!(s.get("message", "m1").unwrap().is_none());
    assert!(s.search("xylophone").unwrap().is_empty());
}

#[test]
fn wrong_password_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::create(&path(&dir), None, Some(("right", KdfParams::fast()))).unwrap();
    fill(&s);
    drop(s);
    assert!(matches!(Store::open(&path(&dir), &[Unlock::Password("wrong")]), Err(StoreError::WrongKey)));
    assert!(Store::open(&path(&dir), &[Unlock::Password("right")]).is_ok());
}

#[test]
fn tampered_record_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let kc = MemKeychain::default();
    let s = Store::create(&path(&dir), Some(&kc), None).unwrap();
    fill(&s);
    drop(s);
    let conn = rusqlite::Connection::open(path(&dir)).unwrap();
    let mut blob: Vec<u8> = conn.query_row("SELECT blob FROM records", [], |r| r.get(0)).unwrap();
    let last = blob.len() - 1;
    blob[last] ^= 1;
    conn.execute("UPDATE records SET blob = ?1", [blob]).unwrap();
    drop(conn);
    let s = Store::open(&path(&dir), &[Unlock::Keychain(&kc)]).unwrap();
    assert!(matches!(s.get("message", "m1"), Err(StoreError::Tampered(_))));
}

#[test]
fn record_moved_to_another_id_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let kc = MemKeychain::default();
    let s = Store::create(&path(&dir), Some(&kc), None).unwrap();
    fill(&s);
    drop(s);
    let conn = rusqlite::Connection::open(path(&dir)).unwrap();
    conn.execute("UPDATE records SET id = 'm2'", []).unwrap();
    drop(conn);
    let s = Store::open(&path(&dir), &[Unlock::Keychain(&kc)]).unwrap();
    assert!(matches!(s.get("message", "m2"), Err(StoreError::Tampered(_))));
}

#[test]
fn store_files_hold_no_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let kc = MemKeychain::default();
    let s = Store::create(&path(&dir), Some(&kc), Some(("pw", KdfParams::fast()))).unwrap();
    fill(&s);
    drop(s);
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        let text = String::from_utf8_lossy(&bytes).to_lowercase();
        for needle in ["zebracorn", "xylophone", "quarterly", "invoice"] {
            assert!(!text.contains(needle), "{needle} leaked");
        }
    }
}

#[test]
fn dead_keychain_falls_back_to_password() {
    let dir = tempfile::tempdir().unwrap();
    let kc = MemKeychain::default();
    let s = Store::create(&path(&dir), Some(&kc), Some(("pw", KdfParams::fast()))).unwrap();
    fill(&s);
    drop(s);
    let s = Store::open(&path(&dir), &[Unlock::Keychain(&DeadKeychain), Unlock::Password("pw")]).unwrap();
    assert!(s.get("message", "m1").unwrap().is_some());
    let empty = MemKeychain::default();
    assert!(Store::open(&path(&dir), &[Unlock::Keychain(&empty), Unlock::Password("pw")]).is_ok());
    assert!(Store::open(&path(&dir), &[Unlock::Keychain(&DeadKeychain)]).is_err());
}

#[test]
fn rekey_keeps_data_readable_without_touching_records() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::create(&path(&dir), None, Some(("old", KdfParams::fast()))).unwrap();
    fill(&s);
    let before: Vec<u8> = rusqlite::Connection::open(path(&dir))
        .unwrap()
        .query_row("SELECT blob FROM records", [], |r| r.get(0))
        .unwrap();
    s.set_password("new", KdfParams::fast()).unwrap();
    drop(s);
    let after: Vec<u8> = rusqlite::Connection::open(path(&dir))
        .unwrap()
        .query_row("SELECT blob FROM records", [], |r| r.get(0))
        .unwrap();
    assert_eq!(before, after);
    assert!(Store::open(&path(&dir), &[Unlock::Password("old")]).is_err());
    let s = Store::open(&path(&dir), &[Unlock::Password("new")]).unwrap();
    assert!(s.get("message", "m1").unwrap().is_some());
}

#[test]
fn cannot_remove_the_last_unlock_method() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::create(&path(&dir), None, Some(("pw", KdfParams::fast()))).unwrap();
    assert!(s.remove_password().is_err());
    let kc = MemKeychain::default();
    s.enroll_keychain(&kc).unwrap();
    s.remove_password().unwrap();
    assert!(!s.has_password().unwrap());
}
