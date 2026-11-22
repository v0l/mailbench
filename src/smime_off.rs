use crate::crypt::{CryptError, KeyInfo, SmimeVerdict};
use crate::store::Store;

pub fn import_pkcs12(_: &Store, _: &[u8], _: &str) -> Result<KeyInfo, CryptError> {
    Err(CryptError::Unsupported)
}

pub fn smime_decrypt(_: &Store, _: &[u8]) -> Result<Vec<u8>, CryptError> {
    Err(CryptError::Unsupported)
}

pub fn smime_verify_default(_: &[u8]) -> Result<SmimeVerdict, CryptError> {
    Err(CryptError::Unsupported)
}
