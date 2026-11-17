# mailbench

A keyboard-first desktop mail client, part of the [bench suite](https://github.com/v0l/bench-suite). It is meant to replace Thunderbird for daily mail.

![Inbox](docs/screens/inbox.png)

- Every byte on disk is encrypted: messages, attachments, the search index, credentials and settings. The data key is unlocked by your OS keychain or a password. See [docs/storage.md](docs/storage.md).
- Messaging backends plug in behind one trait. IMAP (IDLE, CONDSTORE, STARTTLS and TLS) and SMTP (PLAIN, LOGIN and XOAUTH2) come first. See [docs/backends.md](docs/backends.md).
- Three panes: accounts and folders, a dense message list, and a reader. Remote images are blocked until you load them.

It is early: the app still shows sample mail while the store is being wired to the backends.

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
