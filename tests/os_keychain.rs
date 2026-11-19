use mailbench::store::{KdfParams, Keychain, OsKeychain, Store, Unlock};

#[test]
#[ignore = "needs a real OS keychain: Secret Service, Keychain or Credential Manager"]
fn os_keychain_round_trips_a_secret_and_unlocks_a_store() {
    let kc = OsKeychain;
    let account = format!("test-{}", std::process::id());
    kc.set(&account, b"s3cret-bytes").unwrap();
    assert_eq!(kc.get(&account).unwrap().as_deref(), Some(&b"s3cret-bytes"[..]));
    kc.delete(&account).unwrap();
    assert_eq!(kc.get(&account).unwrap(), None);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let kdf = KdfParams { memory_kib: 8, iterations: 1, lanes: 1 };
    let store = Store::create(&path, Some(&kc), Some(("hunter22hunter", kdf))).unwrap();
    store.put("k", "a", b"v").unwrap();
    drop(store);
    let reopened = Store::open(&path, &[Unlock::Keychain(&kc)]).unwrap();
    assert_eq!(reopened.get("k", "a").unwrap().unwrap().as_slice(), b"v");
    reopened.remove_keychain(&kc).unwrap();
}
