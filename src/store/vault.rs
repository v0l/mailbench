use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use super::{Keychain, Result, StoreError};

pub trait Sealer {
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>>;
    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>>;
}

pub struct FileKeychain<S: Sealer> {
    dir: PathBuf,
    sealer: S,
}

impl<S: Sealer> FileKeychain<S> {
    pub fn new(dir: PathBuf, sealer: S) -> Self {
        FileKeychain { dir, sealer }
    }

    fn path(&self, account: &str) -> PathBuf {
        let name: String = account.bytes().map(|b| format!("{b:02x}")).collect();
        self.dir.join(format!("{name}.sealed"))
    }

    fn io(e: std::io::Error) -> StoreError {
        StoreError::Keychain(e.to_string())
    }
}

impl<S: Sealer> Keychain for FileKeychain<S> {
    fn get(&self, account: &str) -> Result<Option<Vec<u8>>> {
        match fs::read(self.path(account)) {
            Ok(sealed) => self.sealer.open(&sealed).map(Some),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Self::io(e)),
        }
    }

    fn set(&self, account: &str, secret: &[u8]) -> Result<()> {
        let sealed = self.sealer.seal(secret)?;
        fs::create_dir_all(&self.dir).map_err(Self::io)?;
        let path = self.path(account);
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, sealed).map_err(Self::io)?;
        fs::rename(&tmp, &path).map_err(Self::io)
    }

    fn delete(&self, account: &str) -> Result<()> {
        match fs::remove_file(self.path(account)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Self::io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Xor;

    impl Sealer for Xor {
        fn seal(&self, plain: &[u8]) -> Result<Vec<u8>> {
            Ok(plain.iter().map(|b| b ^ 0x5a).collect())
        }
        fn open(&self, sealed: &[u8]) -> Result<Vec<u8>> {
            self.seal(sealed)
        }
    }

    #[test]
    fn round_trips_and_never_stores_plain_text() {
        let dir = tempfile::tempdir().unwrap();
        let kc = FileKeychain::new(dir.path().join("kc"), Xor);
        assert_eq!(kc.get("a/b").unwrap(), None);
        kc.set("a/b", b"hunter2-secret").unwrap();
        assert_eq!(kc.get("a/b").unwrap().as_deref(), Some(&b"hunter2-secret"[..]));
        let raw = fs::read(kc.path("a/b")).unwrap();
        assert!(!raw.windows(7).any(|w| w == b"hunter2"));
        kc.delete("a/b").unwrap();
        kc.delete("a/b").unwrap();
        assert_eq!(kc.get("a/b").unwrap(), None);
    }
}
