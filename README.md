# mailbench

A keyboard-first desktop mail client, part of the [bench suite](https://github.com/v0l/bench-suite). It is meant to replace Thunderbird for daily mail.

![Inbox](docs/screens/inbox.png)

- Every byte on disk is encrypted: messages, attachments, the search index, credentials and settings. The data key is unlocked by your OS keychain or a password. See [docs/storage.md](docs/storage.md).
- Messaging backends plug in behind one trait. IMAP (IDLE, CONDSTORE, STARTTLS and TLS) and SMTP (PLAIN, LOGIN and XOAUTH2) come first, and Nostr NIP-17 private DMs sit beside them. See [docs/backends.md](docs/backends.md).
- An IM style chat scaffold (rooms, groups, outbox, fake backend, a contract test for future backends) sits behind a developer setting. See [docs/chat.md](docs/chat.md).
- Three panes: accounts and folders, a dense message list, and a reader. Remote images are blocked until you load them.
- Paste or drop an image into the composer to send it inline. See [docs/inline-images.md](docs/inline-images.md).

## Running it

```
cargo run --release
./target/release/mailbench
./target/release/mailbench --demo
```

The first start asks how to protect your mail (system keychain, a password, or both), then asks for your first account. Type the address and press detect: settings come from the provider autoconfig, the Mozilla ISP database, DNS SRV records or a Google/Microsoft guess. Test the connection, then save and sync. `--demo` shows sample mail and touches nothing.

Sign in with Google or Microsoft needs an OAuth client id at build or run time (`MAILBENCH_GOOGLE_CLIENT_ID`, `MAILBENCH_MICROSOFT_CLIENT_ID`, optional `..._CLIENT_SECRET`). Without one, use an app password.

Data lives in one encrypted file, `~/.local/share/mailbench/mail.db` on Linux (the platform data dir elsewhere, or `$MAILBENCH_DATA/mail.db`). To reset everything, quit the app and delete that file. The keychain entry (service `mailbench`) is then unused and can be removed with your keychain tool.

## Command line

```
mailbench list --unread --json
mailbench read <id> --text
mailbench search "from:acme has:attachment" --json
mailbench send --to a@b.c --subject Hi --body-file note.txt
mailbench archive|flag|mark-read|unread <id>...
```

The CLI opens the same encrypted store as the app (WAL, so both can run at once). It unlocks with the keychain, then `MAILBENCH_PASSWORD`, then a prompt. The agent skill is in `skills/mailbench/SKILL.md`.

## Agents over MCP

mailbench can run a local MCP server (Streamable HTTP, 127.0.0.1 only, bearer token) so an agent can search, read and draft mail. It is off by default, you choose which accounts agents may see, and agents only create drafts unless you allow sending per account. Turn it on in Settings, agents, or run `mailbench mcp serve`. See [docs/mcp.md](docs/mcp.md).

## Keys

| key | does |
| --- | --- |
| `j` `k` | move through the list |
| `Enter` | open |
| `e` | archive |
| `r` / `a` / `f` | reply, reply all, forward |
| `s` | snooze |
| `x` | flag |
| `c` | compose |
| `/` | search |
| `g a`, `g i` | go to archive, go to inbox |
| `Esc` | back |

## Build

```
cargo run --release
```

The backend tests start Greenmail and Dovecot in Docker:

```
cargo test
```

## Design

The design plan and the three concepts it was chosen from are in [docs/design.md](docs/design.md) and [docs/concepts/](docs/concepts/).

## Licence

GPL-3.0-only. The IBM Plex Sans font in `assets/` is under the SIL Open Font License.

## Releasing

A release is a pushed `v*` tag. `.github/workflows/release.yml` checks the tag against the Cargo version and `CHANGELOG.md`, then builds:

- Linux x86_64 and arm64: `.tar.gz` and `.deb`
- macOS arm64: `.tar.gz` and an ad-hoc signed `.dmg`
- Windows x86_64: `.zip` and `.msi`

Each archive holds the `mailbench` binary (GUI and CLI are the same binary), `README.md`, `LICENSE` and `skills/`. A `workflow_dispatch` run with a `tag` input builds everything and publishes nothing. Settings has a check for updates button that reads the GitHub releases. Android is not built yet.
