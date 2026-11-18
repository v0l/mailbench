use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use rand::RngCore;
use rand::rngs::OsRng;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const KEYCHAIN_SERVICE: &str = "mailbench";
const NONCE_LEN: usize = 24;
const WRAP_KEYCHAIN: &str = "keychain";
const WRAP_PASSWORD: &str = "password";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("wrong password or key")]
    WrongKey,
    #[error("record {0} failed authentication")]
    Tampered(String),
    #[error("no unlock method available")]
    Locked,
    #[error("store has no such unlock method")]
    NoSuchWrap,
    #[error("keychain: {0}")]
    Keychain(String),
    #[error("kdf: {0}")]
    Kdf(String),
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub trait Keychain {
    fn get(&self, account: &str) -> Result<Option<Vec<u8>>>;
    fn set(&self, account: &str, secret: &[u8]) -> Result<()>;
    fn delete(&self, account: &str) -> Result<()>;
    fn available(&self) -> bool {
        true
    }
}

pub struct OsKeychain;

impl OsKeychain {
    fn entry(account: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(KEYCHAIN_SERVICE, account).map_err(|e| StoreError::Keychain(e.to_string()))
    }
}

impl Keychain for OsKeychain {
    fn available(&self) -> bool {
        self.get("availability-probe").is_ok()
    }

    fn get(&self, account: &str) -> Result<Option<Vec<u8>>> {
        match Self::entry(account)?.get_secret() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(StoreError::Keychain(e.to_string())),
        }
    }

    fn set(&self, account: &str, secret: &[u8]) -> Result<()> {
        Self::entry(account)?
            .set_secret(secret)
            .map_err(|e| StoreError::Keychain(e.to_string()))
    }

    fn delete(&self, account: &str) -> Result<()> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(StoreError::Keychain(e.to_string())),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct KdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub lanes: u32,
}

impl KdfParams {
    pub fn calibrate(target: Duration) -> Self {
        let mut p = KdfParams { memory_kib: 64 * 1024, iterations: 1, lanes: 4 };
        let salt = [0u8; 16];
        let start = Instant::now();
        let _ = derive(b"calibrate", &salt, p);
        let one = start.elapsed().as_secs_f64().max(0.001);
        p.iterations = ((target.as_secs_f64() / one).round() as u32).clamp(1, 64);
        p
    }

    pub fn fast() -> Self {
        KdfParams { memory_kib: 8, iterations: 1, lanes: 1 }
    }

    fn encode(self) -> String {
        format!("{},{},{}", self.memory_kib, self.iterations, self.lanes)
    }

    fn decode(s: &str) -> Option<Self> {
        let mut it = s.split(',').map(|x| x.parse::<u32>().ok());
        Some(KdfParams { memory_kib: it.next()??, iterations: it.next()??, lanes: it.next()?? })
    }
}

fn derive(password: &[u8], salt: &[u8], p: KdfParams) -> Result<Zeroizing<[u8; 32]>> {
    let params = Params::new(p.memory_kib, p.iterations, p.lanes, Some(32))
        .map_err(|e| StoreError::Kdf(e.to_string()))?;
    let mut out = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password, salt, &mut *out)
        .map_err(|e| StoreError::Kdf(e.to_string()))?;
    Ok(out)
}

#[derive(Zeroize, ZeroizeOnDrop)]
struct DataKey([u8; 32]);

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    OsRng.fill_bytes(&mut b);
    b
}

fn seal(key: &[u8; 32], aad: &[u8], plain: &[u8]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let nonce = random::<NONCE_LEN>();
    let mut out = nonce.to_vec();
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plain, aad })
        .expect("encryption cannot fail");
    out.extend(ct);
    out
}

fn open(key: &[u8; 32], aad: &[u8], blob: &[u8]) -> Option<Vec<u8>> {
    if blob.len() < NONCE_LEN {
        return None;
    }
    let (nonce, ct) = blob.split_at(NONCE_LEN);
    XChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad })
        .ok()
}

fn record_aad(kind: &str, id: &str) -> Vec<u8> {
    let mut a = b"record\0".to_vec();
    a.extend(kind.as_bytes());
    a.push(0);
    a.extend(id.as_bytes());
    a
}

fn wrap_aad(name: &str) -> Vec<u8> {
    format!("wrap\0{name}").into_bytes()
}

pub fn tokenize(text: &str) -> Vec<String> {
    let mut terms: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect();
    terms.sort();
    terms.dedup();
    terms
}

pub struct Store {
    conn: Connection,
    key: DataKey,
    index_key: DataKey,
    store_id: String,
    path: PathBuf,
}

fn index_key_for(key: &[u8; 32]) -> DataKey {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("any key length");
    mac.update(b"mailbench index key v1");
    let mut out = [0u8; 32];
    out.copy_from_slice(&mac.finalize().into_bytes());
    DataKey(out)
}

fn prefix_end(prefix: &str) -> String {
    let mut end = prefix.to_string();
    end.push(char::MAX);
    end
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnlockMethods {
    pub keychain: bool,
    pub password: bool,
}

pub enum Unlock<'a> {
    Keychain(&'a dyn Keychain),
    Password(&'a str),
}

impl Store {
    pub fn create(path: &Path, keychain: Option<&dyn Keychain>, password: Option<(&str, KdfParams)>) -> Result<Self> {
        if keychain.is_none() && password.is_none() {
            return Err(StoreError::Locked);
        }
        let conn = Connection::open(path)?;
        tune(&conn)?;
        init_schema(&conn)?;
        let store_id = hex(&random::<16>());
        conn.execute("INSERT INTO meta (k, v) VALUES ('store_id', ?1)", params![store_id.as_bytes()])?;
        let key = DataKey(random::<32>());
        let store = Store { conn, index_key: index_key_for(&key.0), key, store_id, path: path.to_path_buf() };
        if let Some(kc) = keychain {
            store.enroll_keychain(kc)?;
        }
        if let Some((pw, kdf)) = password {
            store.set_password(pw, kdf)?;
        }
        Ok(store)
    }

    pub fn open(path: &Path, methods: &[Unlock]) -> Result<Self> {
        let conn = Connection::open(path)?;
        tune(&conn)?;
        init_schema(&conn)?;
        let store_id: Vec<u8> = conn
            .query_row("SELECT v FROM meta WHERE k = 'store_id'", [], |r| r.get(0))
            .optional()?
            .ok_or(StoreError::NoSuchWrap)?;
        let store_id = String::from_utf8_lossy(&store_id).into_owned();
        let mut last = StoreError::Locked;
        for m in methods {
            let attempt = match m {
                Unlock::Keychain(kc) => unwrap_with_keychain(&conn, *kc, &store_id),
                Unlock::Password(pw) => unwrap_with_password(&conn, pw),
            };
            match attempt {
                Ok(key) => {
                    return Ok(Store { conn, index_key: index_key_for(&key.0), key, store_id, path: path.to_path_buf() });
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    pub fn methods(path: &Path) -> Result<UnlockMethods> {
        let conn = Connection::open(path)?;
        let count = |name: &str| -> Result<bool> {
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM wraps WHERE name = ?1", params![name], |r| r.get(0))
                .unwrap_or(0);
            Ok(n > 0)
        };
        Ok(UnlockMethods { keychain: count(WRAP_KEYCHAIN)?, password: count(WRAP_PASSWORD)? })
    }

    pub fn transaction<T, E: From<StoreError>>(&self, f: impl FnOnce() -> std::result::Result<T, E>) -> std::result::Result<T, E> {
        if !self.conn.is_autocommit() {
            return f();
        }
        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| E::from(StoreError::from(e)))?;
        match f() {
            Ok(v) => {
                self.conn.execute_batch("COMMIT").map_err(|e| E::from(StoreError::from(e)))?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    pub fn try_clone(&self) -> Result<Store> {
        let conn = Connection::open(&self.path)?;
        tune(&conn)?;
        Ok(Store {
            conn,
            key: DataKey(self.key.0),
            index_key: DataKey(self.index_key.0),
            store_id: self.store_id.clone(),
            path: self.path.clone(),
        })
    }

    pub fn enroll_keychain(&self, kc: &dyn Keychain) -> Result<()> {
        let kek = Zeroizing::new(random::<32>());
        kc.set(&self.store_id, &*kek)?;
        self.save_wrap(WRAP_KEYCHAIN, &kek, "")
    }

    pub fn remove_keychain(&self, kc: &dyn Keychain) -> Result<()> {
        self.require_other_wrap(WRAP_KEYCHAIN)?;
        kc.delete(&self.store_id)?;
        self.conn.execute("DELETE FROM wraps WHERE name = ?1", params![WRAP_KEYCHAIN])?;
        Ok(())
    }

    pub fn set_password(&self, password: &str, kdf: KdfParams) -> Result<()> {
        let salt = random::<16>();
        let kek = derive(password.as_bytes(), &salt, kdf)?;
        let mut params_blob = salt.to_vec();
        params_blob.extend(kdf.encode().as_bytes());
        self.save_wrap(WRAP_PASSWORD, &kek, &hex(&params_blob))
    }

    pub fn remove_password(&self) -> Result<()> {
        self.require_other_wrap(WRAP_PASSWORD)?;
        self.conn.execute("DELETE FROM wraps WHERE name = ?1", params![WRAP_PASSWORD])?;
        Ok(())
    }

    pub fn has_password(&self) -> Result<bool> {
        self.has_wrap(WRAP_PASSWORD)
    }

    pub fn has_keychain(&self) -> Result<bool> {
        self.has_wrap(WRAP_KEYCHAIN)
    }

    pub fn rotate_keychain_key(&self, kc: &dyn Keychain) -> Result<()> {
        self.enroll_keychain(kc)
    }

    fn has_wrap(&self, name: &str) -> Result<bool> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM wraps WHERE name = ?1", params![name], |r| r.get(0))?;
        Ok(n > 0)
    }

    fn require_other_wrap(&self, name: &str) -> Result<()> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM wraps WHERE name != ?1", params![name], |r| r.get(0))?;
        if n == 0 { Err(StoreError::Locked) } else { Ok(()) }
    }

    fn save_wrap(&self, name: &str, kek: &[u8; 32], params_text: &str) -> Result<()> {
        let blob = seal(kek, &wrap_aad(name), &self.key.0);
        self.conn.execute(
            "INSERT INTO wraps (name, blob, params) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO UPDATE SET blob = excluded.blob, params = excluded.params",
            params![name, blob, params_text],
        )?;
        Ok(())
    }

    pub fn put(&self, kind: &str, id: &str, plain: &[u8]) -> Result<()> {
        let blob = seal(&self.key.0, &record_aad(kind, id), plain);
        self.conn
            .prepare_cached(
                "INSERT INTO records (kind, id, blob) VALUES (?1, ?2, ?3)
                 ON CONFLICT(kind, id) DO UPDATE SET blob = excluded.blob",
            )?
            .execute(params![kind, id, blob])?;
        Ok(())
    }

    pub fn get(&self, kind: &str, id: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let blob: Option<Vec<u8>> = self
            .conn
            .prepare_cached("SELECT blob FROM records WHERE kind = ?1 AND id = ?2")?
            .query_row(params![kind, id], |r| r.get(0))
            .optional()?;
        match blob {
            None => Ok(None),
            Some(b) => open(&self.key.0, &record_aad(kind, id), &b)
                .map(|v| Some(Zeroizing::new(v)))
                .ok_or_else(|| StoreError::Tampered(format!("{kind}/{id}"))),
        }
    }

    pub fn delete(&self, kind: &str, id: &str) -> Result<()> {
        self.conn
            .prepare_cached("DELETE FROM records WHERE kind = ?1 AND id = ?2")?
            .execute(params![kind, id])?;
        self.conn.prepare_cached("DELETE FROM idx WHERE id = ?1")?.execute(params![id])?;
        Ok(())
    }

    pub fn list(&self, kind: &str) -> Result<Vec<String>> {
        let mut st = self.conn.prepare_cached("SELECT id FROM records WHERE kind = ?1 ORDER BY id")?;
        let rows = st.query_map(params![kind], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn list_prefix(&self, kind: &str, prefix: &str) -> Result<Vec<String>> {
        let mut st = self
            .conn
            .prepare_cached("SELECT id FROM records WHERE kind = ?1 AND id >= ?2 AND id < ?3 ORDER BY id")?;
        let rows = st.query_map(params![kind, prefix, prefix_end(prefix)], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn scan_prefix(&self, kind: &str, prefix: &str) -> Result<Vec<(String, Zeroizing<Vec<u8>>)>> {
        self.scan_prefix_page(kind, prefix, None, usize::MAX)
    }

    pub fn scan_prefix_page(
        &self,
        kind: &str,
        prefix: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<(String, Zeroizing<Vec<u8>>)>> {
        let mut st = self.conn.prepare_cached(
            "SELECT id, blob FROM records WHERE kind = ?1 AND id >= ?2 AND id > ?3 AND id < ?4 ORDER BY id LIMIT ?5",
        )?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = st.query_map(params![kind, prefix, after.unwrap_or(""), prefix_end(prefix), limit], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, blob) = row?;
            let plain = open(&self.key.0, &record_aad(kind, &id), &blob)
                .ok_or_else(|| StoreError::Tampered(format!("{kind}/{id}")))?;
            out.push((id, Zeroizing::new(plain)));
        }
        Ok(out)
    }

    pub fn delete_prefix(&self, kind: &str, prefix: &str) -> Result<()> {
        let end = prefix_end(prefix);
        self.conn
            .prepare_cached("DELETE FROM idx WHERE id IN (SELECT id FROM records WHERE kind = ?1 AND id >= ?2 AND id < ?3)")?
            .execute(params![kind, prefix, end])?;
        self.conn
            .prepare_cached("DELETE FROM records WHERE kind = ?1 AND id >= ?2 AND id < ?3")?
            .execute(params![kind, prefix, end])?;
        Ok(())
    }

    fn term_tag(&self, term: &str) -> Vec<u8> {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&self.index_key.0).expect("any key length");
        mac.update(term.as_bytes());
        mac.finalize().into_bytes().to_vec()
    }

    pub fn index_text(&self, id: &str, text: &str) -> Result<()> {
        self.conn.prepare_cached("DELETE FROM idx WHERE id = ?1")?.execute(params![id])?;
        let mut insert = self.conn.prepare_cached("INSERT OR IGNORE INTO idx (tag, id) VALUES (?1, ?2)")?;
        for term in tokenize(text) {
            insert.execute(params![self.term_tag(&term), id])?;
        }
        Ok(())
    }

    pub fn search(&self, query: &str) -> Result<Vec<String>> {
        let terms = tokenize(query);
        let mut result: Option<Vec<String>> = None;
        for term in terms {
            let mut st = self.conn.prepare_cached("SELECT id FROM idx WHERE tag = ?1 ORDER BY id")?;
            let ids: Vec<String> = st
                .query_map(params![self.term_tag(&term)], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            result = Some(match result {
                None => ids,
                Some(prev) => {
                    let keep: std::collections::HashSet<&String> = ids.iter().collect();
                    prev.into_iter().filter(|i| keep.contains(i)).collect()
                }
            });
        }
        Ok(result.unwrap_or_default())
    }
}

fn tune(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;")?;
    Ok(())
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v BLOB NOT NULL);
         CREATE TABLE IF NOT EXISTS wraps (name TEXT PRIMARY KEY, blob BLOB NOT NULL, params TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS records (kind TEXT NOT NULL, id TEXT NOT NULL, blob BLOB NOT NULL, PRIMARY KEY (kind, id));
         CREATE TABLE IF NOT EXISTS idx (tag BLOB NOT NULL, id TEXT NOT NULL, PRIMARY KEY (tag, id));
         CREATE INDEX IF NOT EXISTS idx_by_id ON idx (id);",
    )?;
    Ok(())
}

fn load_wrap(conn: &Connection, name: &str) -> Result<(Vec<u8>, String)> {
    conn.query_row("SELECT blob, params FROM wraps WHERE name = ?1", params![name], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })
    .optional()?
    .ok_or(StoreError::NoSuchWrap)
}

fn unwrap_data_key(kek: &[u8; 32], name: &str, blob: &[u8]) -> Result<DataKey> {
    let plain = Zeroizing::new(open(kek, &wrap_aad(name), blob).ok_or(StoreError::WrongKey)?);
    let key: [u8; 32] = plain.as_slice().try_into().map_err(|_| StoreError::WrongKey)?;
    Ok(DataKey(key))
}

fn unwrap_with_keychain(conn: &Connection, kc: &dyn Keychain, store_id: &str) -> Result<DataKey> {
    let (blob, _) = load_wrap(conn, WRAP_KEYCHAIN)?;
    let kek = Zeroizing::new(kc.get(store_id)?.ok_or(StoreError::WrongKey)?);
    let kek: [u8; 32] = kek.as_slice().try_into().map_err(|_| StoreError::WrongKey)?;
    unwrap_data_key(&kek, WRAP_KEYCHAIN, &blob)
}

fn unwrap_with_password(conn: &Connection, password: &str) -> Result<DataKey> {
    let (blob, params_hex) = load_wrap(conn, WRAP_PASSWORD)?;
    let raw = unhex(&params_hex).ok_or(StoreError::WrongKey)?;
    if raw.len() < 16 {
        return Err(StoreError::WrongKey);
    }
    let (salt, kdf) = raw.split_at(16);
    let kdf = KdfParams::decode(&String::from_utf8_lossy(kdf)).ok_or(StoreError::WrongKey)?;
    let kek = derive(password.as_bytes(), salt, kdf)?;
    unwrap_data_key(&kek, WRAP_PASSWORD, &blob)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}
