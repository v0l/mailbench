# Chat scaffold

IM style chats (direct and group rooms) behind one trait, with no real backend chosen yet. Only an in-memory fake exists. The feature is hidden behind the setting `Developer: chat scaffold` (off by default). With it on, the sidebar shows a Chats section and `t` opens the Chats view.

## Model (`src/chat/model.rs`)

- `Room`: id, name, kind (`Direct` or `Group`), members, topic, created_at, unread, last_message, muted, pinned. A room without a name is titled with the first three other member names and `+N`.
- `Member`: id, display name, avatar, role (`Owner`, `Admin`, `Member`), optional presence.
- `ChatMessage`: id, room, sender, sent_at, body, reply_to, attachments, reactions, edited, deleted, `Delivery` (`Sending`, `Sent`, `Delivered`, `Failed(reason)`). Ids starting with `local-` are outbox messages not yet accepted by the backend.
- `ChatEvent`: NewMessage, Edited, Deleted, Reaction, Typing, Presence, RoomChanged, RoomLeft, MemberChanged, Read, Connection.
- `Capabilities`: edit, delete, reactions, threads, attachments, typing, presence, read receipts, max message length, paged history. The UI hides what a backend cannot do.

## Pieces

- `backend.rs`: the async `ChatBackend` trait.
- `events.rs`: `ChatState` and a pure reducer (`apply`) for events, unread counts and ordering (pinned first, then newest).
- `controller.rs`: runs a backend on the shared runtime, owns the outbox (retry with backoff, `Failed` after 4 attempts or a non retryable error, Retry and Delete actions), paging, notifications and read markers.
- `store.rs`: the local cache (rooms, messages, read markers, outbox) as encrypted records in the app store under new record kinds, so old databases open unchanged.
- `fake.rs`: the in-memory backend. Its `FakeHandle` scripts contacts, messages, reactions, edits, typing, lag, failures and going offline.
- `ui.rs`: the Chats view. Room list, room view, composer, members panel, new chat flow, search.

## UI notes

- Enter sends and Shift+Enter adds a line. This differs from the mail composer on purpose: chat messages are short and sent often.
- At phone width the room list and the room are separate screens with a back button, and tap targets are 44 px.
- A room is marked read when it is open, the bottom is visible and a short dwell has passed. A desktop notification fires for a new message in a room that is not muted and not open.
- Search is a substring match over cached message text and sender names. A result opens the room and scrolls to the message.

## How to add a backend

1. Add `src/chat/<name>.rs` with a struct implementing `ChatBackend`. Nothing else in the app changes.
2. Declare `Capabilities` honestly. Methods for a missing capability keep the default `ChatError::Unsupported`.
3. Map failures: `ChatError::Offline` for transport problems (the outbox retries these), `Rejected` for refusals (the message ends as `Failed`).
4. Emit `ChatEvent`s from `subscribe`. Sent messages must come back from `send` with the backend id.
5. Run the shared contract: in `tests/chat_contract.rs` call `contract(&mut backend)` for your backend. It needs at least two contacts. It covers rooms, send and reply, paged history, read, edit, react, add member, topic, delete, leave and the capability gates.
6. Wire an account kind in `src/accounts.rs` when the backend is chosen. The scaffold does not add one yet.

## Candidates (notes only, none chosen)

- Nostr NIP-17: private DMs map to `Direct`, a member set maps to a `Group` keyed by its `p` tags.
- Nostr NIP-29: relay based groups with admins map to rooms with roles.
- Matrix: rooms, membership, reactions, edits and read receipts map almost one to one.
- XMPP: MUC rooms with MAM for paged history.
- IRC: channels with no history, edits or reactions, so most capabilities are false.
