# Backends

A backend talks to a server: it syncs folders and message summaries, fetches bodies, changes flags, moves and deletes, searches, sends and pushes change events. It never touches local storage. Synced data goes to a `SyncSink` (the encrypted store implements it), kept apart from the UI's `MailStore`.

Code lives in `src/backend/`.

| File | Contents |
| --- | --- |
| `mod.rs` | `Backend` trait, `Account` (a mail backend plus an outbound backend), `Watch` |
| `types.rs` | `Folder`, `SyncToken`, `Flags`, `MessageSummary`, `FolderDelta`, `OutgoingMessage`, `Event`, `Auth`, `Security` |
| `imap.rs` | `ImapBackend` |
| `smtp.rs` | `SmtpBackend` |
| `sync.rs` | `SyncSink` and `sync_account` |
| `store_sink.rs` | `StoreSink`, a `SyncSink` over the encrypted `Store` (summaries, flags, tokens, optional bodies and search index) |
| `jmap.rs` | `JmapBackend`, JMAP mail (RFC 8620/8621) |
| `nostr_dm.rs` | `NostrBackend`, NIP-17 private DMs |
| `memory.rs` | `MemorySink`, an in memory sink for tests |

## The trait

Every method but `kind` has a default that returns `BackendError::Unsupported`, so a backend implements only what it can do. `capabilities` tells callers what to expect.

Sync is token based. `sync_folder(folder, since)` returns a `FolderDelta`: the new `SyncToken`, new message summaries, changed flags for known messages, and the full list of live UIDs so the sink can drop expunged messages. `reset: true` means the old token was unusable (for IMAP, UIDVALIDITY changed) and the sink must discard the folder first. `sync_account` runs this for every selectable folder against a `SyncSink`.

`watch(folder)` returns a `Watch` with a channel of `Event`s. Dropping it stops the watcher.

## IMAP

`ImapConfig` takes host, port, `Security` (`Tls`, `StartTls`, `Plain`) and `Auth` (`Password` or `XOAuth2`). Certificates are verified unless `accept_invalid_certs` is set.

- Sync token is UIDVALIDITY, UIDNEXT and HIGHESTMODSEQ. New mail is fetched from the saved UIDNEXT. Flag changes use `CHANGEDSINCE` when the server advertises CONDSTORE or QRESYNC, and a full flag fetch otherwise.
- Move uses `UID MOVE` when advertised, else copy, flag `\Deleted`, expunge (`UID EXPUNGE` with UIDPLUS).
- `search` passes the IMAP SEARCH string straight through and returns UIDs.
- `watch` opens a second connection and uses IDLE, re-issuing it every `idle_timeout` (29 minutes by default). Without IDLE it returns `Unsupported`.

## SMTP

`SmtpConfig` mirrors the IMAP one. Use `Security::StartTls` with port 587 and `Security::Tls` with port 465. Password auth offers PLAIN then LOGIN, XOAuth2 uses XOAUTH2 with the access token. Refreshing OAuth2 tokens is the caller's job: build a new config with the new token.

## Nostr (NIP-17)

`NostrBackend` is a mail backend over Nostr private DMs. Add it from the add account screen with an nsec (hex also works). The secret is stored with the other credentials in the encrypted store. Paste a `bunker://` URL instead of an nsec to use a NIP-46 remote signer: press connect bunker, approve in your signer app, and no nsec is stored. Mailbench keeps the bunker URL and a generated client app key in the encrypted store, and the bunker signs the gift wraps and decrypts the incoming ones.

- Relays: the account's kind 10050 list, else its NIP-65 read relays, else the configured relays, else `DEFAULT_RELAYS`. `publish_inbox` publishes a kind 10050 when none exists.
- Folders: Inbox (received), Sent (own messages, from the self copy gift wrap) and Archive (local only, empty).
- Sync: kind 1059 events tagged with the account's key, going back 2 days from the newest wrap seen (wrap times are randomised), capped at `BACKFILL_CAP` events. Wraps are opened with NIP-44, the seal and rumor authors must match, and messages dedupe by rumor id.
- Mapping: kind 14 becomes a plain text mail, kind 15 a mail with the decrypted file attached. `From` and `To` are `<npub>@nostr` with kind 0 names, refreshed daily. The thread key (participants plus normalised subject) goes in `References`, and `e` tags become `In-Reply-To`.
- Send: one kind 14 gift wrap per recipient on that recipient's 10050 relays, plus a self copy. Recipients can be an npub, hex key or NIP-05 name.
- Capabilities: `server_folders`, `server_search` and `server_flags` are false, so the UI hides folder create and rename, the server search fallback and the flag action. Unread state and moves to Archive are kept in the local store only.
- Profiles: kind 0 name, picture and NIP-05 are cached for a day. The picture shows only after the user allows remote images for that message. A NIP-05 badge needs `/.well-known/nostr.json` to return the sender's key.
- When the account has no kind 10050 list, the add account screen asks whether to publish one before saving.

Tests: `tests/backend_nostr.rs` uses rust-nostr's in-process `MockRelay`, never a public relay.

## JMAP

`JmapConfig::new(url, auth)` takes a server URL (a bare host is resolved through `/.well-known/jmap`, redirects are followed with the credentials kept) or a full session URL. `Auth::Password` sends HTTP Basic, `Auth::XOAuth2` sends the token as a Bearer. Fastmail and Stalwart work the same way.

- Session: the primary mail account, `apiUrl`, `downloadUrl`, `uploadUrl` and `eventSourceUrl` come from the session resource. A 401 is `BackendError::Auth`.
- Folders: `Mailbox/get`, with the mailbox tree flattened to `Parent/Child` paths and JMAP roles mapped to `FolderRole`. Create, rename and delete use `Mailbox/set`.
- UIDs: JMAP ids are strings, so each email id is mapped to a stable `u32` (FNV hash, probed on collision) for the life of the backend. The sync token is `uid_validity` 1 and `uid_next` above the largest UID; real sync state is the JMAP `Email` state kept in the backend.
- Sync: the first sync pages `Email/query` plus a back-referenced `Email/get` for mailbox ids and keywords. Later syncs call `Email/changes` (paged with `hasMoreChanges`) and fall back to a full load on `cannotCalculateChanges`. Only created or updated emails are fetched with `Email/get` headers, so an unchanged mailbox costs one request. Flag changes and expunges ride in the delta.
- Bodies: `fetch` downloads the RFC 5322 blob through `downloadUrl`.
- Flags: `$seen`, `$answered`, `$flagged`, `$draft` and other keywords map to `Flags` and write back through `Email/set` patches. Move rewrites `mailboxIds`, delete destroys the email.
- Search: `Email/query` with a `text` filter inside the mailbox. The query is plain text, not IMAP SEARCH syntax, and `ALL` or an empty string lists the folder.
- Send: the rendered message is uploaded, imported into Drafts and submitted with `EmailSubmission/set` in one request. On success the server clears `$draft` and moves it to Sent, so `sent_copy_by_server` is true. A sending identity is created if the account has none.
- Append: `Email/import` with the given keywords.
- Watch: the `eventSourceUrl` stream filtered to `Email`, one `FolderChanged` per new state. A dropped stream sends `Disconnected`.
- Capabilities: `idle` (push available), `move_command`, `send` (submission capability present), `server_folders`, `server_search` and `server_flags` are true. `condstore`, `qresync`, `uidplus` and `gmail_ext` are false.

Tests: `tests/backend_jmap.rs` runs against Stalwart 0.13 in Docker (`tests/fixtures/stalwart/config.toml`, HTTP on 8090, admin `admin:adminsecret`, users created per test in the `example.org` domain; Stalwart rejects dotless and `.example` domains for identities). It starts `stalwartlabs/stalwart:v0.13` itself when port 8090 is closed and skips without docker.

## Capability flags

The UI reads `server_folders` (folder create and rename), `server_search` (server search fallback), `server_flags` (flag action) and `send`. A backend that leaves them false hides the matching action.

| Flag | IMAP | SMTP | JMAP | Nostr |
| --- | --- | --- | --- | --- |
| `server_folders` | yes | no | yes | no |
| `server_search` | yes | no | yes | no |
| `server_flags` | yes | no | yes | no |
| `send` | no (via SMTP in `Account`) | yes | yes | yes |
| `idle` (push) | if advertised | no | yes (EventSource) | no |

## Adding a backend

1. Add `src/backend/<name>.rs` with a config struct and a struct implementing `Backend`.
2. Implement only the methods the protocol supports. Map failures to `BackendError`: `Connection` for transport problems (the IMAP backend reconnects after these), `Auth`, `Protocol`.
3. Fill `Capabilities` honestly.
4. Produce a `SyncToken` that is stable across runs. If the protocol has no UIDs, map its own ids onto `u32` or extend `types.rs` and discuss it first.
5. Export it from `mod.rs`.
6. Add tests next to `tests/backend_greenmail.rs`, against a local server.

## Tests

`tests/backend_greenmail.rs` runs against Greenmail in Docker, never a real account. It starts `greenmail/standalone:2.1.3` itself if port 8080 is closed, and creates a fresh user per test through the Greenmail API.

```sh
docker run -d --rm --name mailbench-greenmail \
  -p 127.0.0.1:3025:3025 -p 127.0.0.1:3143:3143 -p 127.0.0.1:3465:3465 \
  -p 127.0.0.1:3993:3993 -p 127.0.0.1:8080:8080 \
  -e GREENMAIL_OPTS="-Dgreenmail.setup.test.all -Dgreenmail.hostname=0.0.0.0" \
  greenmail/standalone:2.1.3
cargo test --test backend_greenmail
```

`tests/backend_dovecot.rs` runs Dovecot 2.3 (`tests/fixtures/dovecot/dovecot.conf`, self signed cert, static passdb) on ports 3144 and 3994 for STARTTLS, CONDSTORE `CHANGEDSINCE` flag sync and UIDVALIDITY resets. `tests/backend_store.rs` syncs into an encrypted store and checks the files hold no plaintext. `tests/backend_xoauth2.rs` checks the XOAUTH2 string against mock servers. Docker tests skip when docker is missing.
