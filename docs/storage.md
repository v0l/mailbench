# Storage

One SQLite file (`rusqlite`, bundled) under the app data dir. Library layer in `src/store/`.

## Why SQLite

Transactional, single file, well understood, available on every target including Android. Only values are encrypted, so SQL stays usable for keys that are not secret (record kind, record id, wrap names). redb or sled would give the same property with less tooling and no easy way to inspect or repair a store.

## Key hierarchy

- A random 256-bit data key encrypts every record with XChaCha20-Poly1305. Each record gets a fresh 192-bit random nonce stored in front of the ciphertext.
- Associated data is `record\0<kind>\0<id>`, so a record copied to another id or kind fails authentication.
- The data key is wrapped (encrypted) by one or two key encryption keys, stored in the `wraps` table:
  - `keychain`: a random 256-bit key held in the OS keychain (`keyring`: Secret Service or kwallet, Keychain, Credential Manager, Keystore). Unlocks silently.
  - `password`: Argon2id of the user password. Salt and parameters (memory, iterations, lanes) are stored beside the wrap. `KdfParams::calibrate` tunes iterations to about 0.5 s at 64 MiB.
- Both may be present. `Store::open` takes an ordered list of unlock methods and tries each, so a missing or broken keychain falls back to the password.
- Changing the password or rotating the keychain key only rewrites the wrap. Bulk records are untouched.
- The last remaining unlock method cannot be removed.
- The data key is held in a zeroizing struct. Decrypted records are returned in `Zeroizing<Vec<u8>>`. Nothing is written to temp files.

## Record kinds

`message` (raw RFC 5322 plus parsed headers), `attachment`, `settings`, `credential`, `draft`, and any other kind the layers above choose. Callers serialise into bytes. Large attachments are single records for now.

## Search index

The index is blind: a term is stored as `HMAC-SHA256(index_key, term)` with `index_key` derived from the data key, next to the message id. Plaintext terms never touch disk, and tags are useless without the data key.

Tradeoffs:

- Only exact term matches. No prefix or substring search from the index alone.
- Leaks, to someone with the file but not the key: how many distinct terms each message has, and which messages share a term (equal tags). Term frequency across the mailbox is visible. It does not reveal the terms themselves, and dictionary attacks need the key.
- Ranking and phrase search need positions, which are not stored. A later slice can add an encrypted position blob per (term, message) as a record.

## Not covered

SQLite free pages and the journal may hold older ciphertext of deleted records, never plaintext. Record kinds and ids are plaintext metadata, so ids must not contain secrets (use random or hashed ids).
