# mailbench design plan

Status: three concepts rendered, one to choose. House style is egui_bench and is not redesigned. This plan covers the mail-specific decisions inside it. Renders are in `docs/concepts/`, produced by `cargo test --test concepts`.

## Brief

- **Subject:** a desktop mail client that replaces Thunderbird for daily use. Every byte is encrypted at rest, with pluggable backends (IMAP and SMTP first).
- **Audience:** a builder who lives in their inbox. Many accounts (personal, work, a mailing list identity), hundreds of messages a day, hands on the keyboard, mail that carries PDFs, quotes, build notifications and patch threads.
- **Primary job:** get to zero fast without losing anything. Read, decide (reply, archive, snooze, move, flag), move on. The second job is finding a message from three years ago in under a second, offline.
- **Voice:** an instrument panel, not a social app. Plain verbs, sentence case in prose, no exclamation marks, errors say what happened and what to press.

## Tokens within egui_bench

Nothing here adds a colour. Every colour is an egui_bench constant.

| meaning in mail | token | where it shows |
|---|---|---|
| Unread, flagged, the selected row, the primary key | `READOUT` amber | an unread rail and strong sender, a flag mark, selection outline, `send` key. Amber is what the user sets, and unread and flagged are the user's own to-do marks |
| New since you last looked, sync, IDLE | `TRACE` cyan | the `new` dot, the `idle` chip, "synced 09:41". Cyan is what the server told us, live |
| Remote content blocked, irreversible but not destructive | `WARN` | the blocked images banner, `trust new certificate` |
| Failures | `FAULT` | auth failed, send rejected, delete keys |
| Account healthy | `OK` | account lamp |
| Surfaces | `WELL` < `CHASSIS` < `PANEL`, `ETCH` rules, `BAND` hover | list wells, panes, cards |
| Read mail, labels, hints | `LEGEND` | read rows fade to grey so unread carries the weight |

No account colours and no avatar colours. An account is told apart by its name in Legend type and a lamp, never by a decorative hue.

### Type

- **Legend** (condensed, uppercase, tracked) stays for labels, tabs, keys and chips. **Figure** (mono) stays for times, counts, sizes, fingerprints and ports.
- **Reading face:** IBM Plex Sans Regular, embedded as the `body` family, with `body-strong` (SemiBold) and `body-italic`. Message bodies, subjects in lists and sender names use it. Body text is never the Legend face. Plex Sans is chosen over Plex Serif because mail is mostly short, technical and quoted, and it sits next to Figure without the two fighting. It is added with `Context::add_font` after `egui_bench::install`, so the house fonts are untouched.
- **Scale:** body 14.5 px, line height 1.55 (22.5 px). List rows 12.5 px. Message subject 17 px strong. Legend 11.5 px as in egui_bench.
- **Measure:** the reader column is capped at 560 px, about 70 characters at 14.5 px, left aligned. A wide window gets more margin, never longer lines.

### Density

Three steps, chosen by concept rather than a setting for now. Dense is 16 px rows (`Table`), comfortable is a two line 52 px row, focus is one message at a time. The final app can offer dense and comfortable for the list as a user setting, which would then be amber.

## Concept layouts

### A. Three pane

```
+--------------------------------------------------------------------------+
| MAILBENCH   [ search all accounts /        ]   IDLE 3/3     [COMPOSE]    |
+-----------+--------------------------+-----------------------------------+
| ACCOUNTS  | ALL | UNREAD | FLAGGED   | Rev C stencil quote and lead time |
| o kestrel |--------------------------| Tomas Rivera  to me  09:41   [..] |
| o home    | * Tomas Rivera  Rev C ...| [WARN blocked images]             |
| x lists   |   Wren Okafor   Reflow...| body, 560 px column               |
| FOLDERS   |   GitHub        PR #212  | ...                               |
| Inbox  14 |   ...            (Table) | [reply] [archive] [delete]        |
| Drafts  2 |                          |                                   |
+-----------+--------------------------+-----------------------------------+
| j k move  Enter open  e archive  r reply  / search                       |
+--------------------------------------------------------------------------+
```

Best at: scanning a lot of mail at once, many accounts visible, mouse and keyboard equally. Familiar to a Thunderbird user, so the lowest switching cost. Weakest at: long threads, since the reader shows one message.

### B. Conversations

```
+--------------------------------------------------------------------------+
| MAILBENCH   ALL | KESTREL | HOME | LISTS      [ search ]  IDLE  [COMPOSE]|
+-------------------------+------------------------------------------------+
| Tomas, me, Wren     3   | Rev C stencil quote and lead time              |
| Rev C stencil quote...  | +-- Tomas Rivera 09:41  quote attached ------+ |
|-------------------------| | collapsed earlier turns, one line each      | |
| GitHub          12  PR  | +-- me 09:52 --------------------------------+ |
| ...                     | +-- Tomas Rivera 10:07 (new) ----------------+ |
|                         | | body                                        | |
|                         | +---------------------------------------------+ |
|                         | [ reply here ........................ ] [SEND]   |
+-------------------------+------------------------------------------------+
```

Best at: patch threads, quotes and negotiations where context matters. Reply happens beside the thing you are replying to, so there is no modal composer. Weakest at: raw triage speed.

### C. Triage queue

```
+--------------------------------------------------------------------------+
| MAILBENCH   [ type a command or search          Ctrl+K ]   IDLE   14 left|
+--------------------------------------------------------------------------+
|                 QUEUE  14 of 31 ====----                                 |
|                 +-- Tomas Rivera ------------------------------ 09:41 --+|
|                 | Rev C stencil quote and lead time                      ||
|                 | three lines of the message                             ||
|                 | [E archive] [R reply] [S snooze] [V move] [# delete]   ||
|                 +--------------------------------------------------------+|
|                 UP NEXT  rows, one line each                             |
+--------------------------------------------------------------------------+
```

Best at: getting to zero. One decision per screen, every action one key, snooze offers times in place. Weakest at: browsing and mouse use, and many-account overview, which is why the search and command bar is always present.

## Review against the generic defaults

The skill lists traits that appear in generated designs. Checked against this plan:

- **Warm cream and terracotta, or near black with one acid accent:** the palette is fixed by egui_bench, and it is a near-black chassis with amber and cyan. That is the house default, not ours, and it is correct here. What we choose is meaning: amber for the user's marks, cyan for server state, so the colour tells the user something every time.
- **Cards, one radius, soft shadows, gradient washes:** egui_bench has 2 px corners and no shadows or gradients. The first draft of B put every message in a card of equal weight. Revised: earlier turns are a collapsed one line row on a rule, only the newest turn is a railed block. Cards are reserved for state (error, blocked content).
- **ALL-CAPS eyebrow labels over everything:** Legend is uppercase by the house rule. The first draft titled every pane ("MESSAGES", "READER"). Revised: panes carry no title where the content is self-evident, and the only labels kept are filters, column heads and keys.
- **Middle-dot meta strings and arrows on buttons:** the first draft joined meta as `Tomas · to me · 09:41`. Revised: sender, recipient and time sit in aligned columns, with no dot joins and no arrows on keys. No em or en dashes anywhere, enforced by a test.
- **A monospace face for small data labels:** Figure is house style for numbers only. Sender names and subjects are in the reading face, so the list reads as mail, not as a log.
- **Numbered markers:** none. The queue count in C is a real count.
- **Plain body on a tinted wash for emphasis:** warnings use a rail block with a tinted fill at 0.12, which is egui_bench's own `rail_block`.

Spend of boldness: in each concept one thing is unusual. A has the unread rail with the read rows faded to grey. B has the collapsed turns that make a long thread one screen. C has the single focus card with its key row.

## Keyboard flow (all concepts)

`j` and `k` move, `Enter` opens, `e` archive, `r` reply, `a` reply all, `f` forward, `s` snooze, `v` move, `#` delete, `x` flag, `u` toggle unread, `/` search, `Ctrl+K` command bar, `g` then `i` inbox, `Ctrl+Enter` send, `Esc` back. Hints are always on the bottom bar and change with the screen.

## Copy

Sentence case, plain verbs, the action keeps its name through the flow: `archive` key, "Archived. Undo" notice. Empty state is an invitation. Errors state what happened, what is still safe, and the next key. Examples are in the renders.

## Choosing

Recommendation: **A as the shell with B's conversation view as its reader** for threads, and C's focus mode as a triage mode that any list can enter. A for first release because it replaces Thunderbird without relearning; C is the feature that makes it better than Thunderbird.
