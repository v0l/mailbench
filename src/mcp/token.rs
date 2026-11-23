use crate::store::Store;
use rand::RngCore;

pub const KIND: &str = "mcp";
const TOKEN_ID: &str = "token";

fn generate() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn existing(store: &Store) -> Option<String> {
    store.get(KIND, TOKEN_ID).ok().flatten().and_then(|raw| String::from_utf8(raw.to_vec()).ok()).filter(|t| !t.is_empty())
}

pub fn load_or_create(store: &Store) -> Result<(String, bool), String> {
    if let Some(token) = existing(store) {
        return Ok((token, false));
    }
    rotate(store).map(|t| (t, true))
}

pub fn rotate(store: &Store) -> Result<String, String> {
    let token = generate();
    store.put(KIND, TOKEN_ID, token.as_bytes()).map_err(|e| e.to_string())?;
    Ok(token)
}
