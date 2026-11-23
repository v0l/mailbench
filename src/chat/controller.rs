use super::backend::ChatBackend;
use super::events::{ChatState, Outcome, Touched};
use super::model::*;
use super::store::{ChatStore, Queued};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

pub const PAGE: usize = 30;
pub const RETRY_CAP: u32 = 4;

enum Cmd {
    Refresh,
    History { room: RoomId, before: Option<MessageId>, limit: usize },
    Send { local: MessageId, room: RoomId, draft: ChatDraft },
    Edit { room: RoomId, id: MessageId, body: String },
    Delete { room: RoomId, id: MessageId },
    React { room: RoomId, id: MessageId, emoji: String, on: bool },
    Read { room: RoomId, up_to: MessageId },
    Create { name: String, members: Vec<MemberId> },
    Add { room: RoomId, members: Vec<MemberId> },
    Leave { room: RoomId },
    Topic { room: RoomId, topic: String },
}

enum Update {
    Ready { caps: Capabilities, me: Member },
    Rooms(Vec<Room>),
    Contacts(Vec<Member>),
    History { room: RoomId, older: Vec<ChatMessage>, was_before: bool },
    Sent { local: MessageId, room: RoomId, result: ChatResult<ChatMessage> },
    Event(ChatEvent),
    Created(Room),
    Error(String),
}

pub struct Notice {
    pub title: String,
    pub body: String,
    pub room: RoomId,
}

pub struct ChatController {
    pub state: ChatState,
    pub caps: Capabilities,
    pub contacts: Vec<Member>,
    pub error: Option<String>,
    pub backoff: Duration,
    pub notices: Vec<Notice>,
    pub loaded_all: HashMap<RoomId, bool>,
    pub loading: HashMap<RoomId, bool>,
    pub history_arrived: Option<(RoomId, usize)>,
    pub created: Option<RoomId>,
    pub ready: bool,
    cmd: UnboundedSender<Cmd>,
    updates: Receiver<Update>,
    store: Option<ChatStore>,
    queue: Vec<Queued>,
    due: HashMap<MessageId, Instant>,
    local_seq: u64,
    pub open: Option<RoomId>,
}

async fn run(mut backend: Box<dyn ChatBackend>, mut cmds: tokio::sync::mpsc::UnboundedReceiver<Cmd>, out: Sender<Update>) {
    let _ = out.send(Update::Ready { caps: backend.capabilities(), me: backend.me() });
    let mut events = backend.subscribe().await.ok();
    if let Ok(c) = backend.contacts().await {
        let _ = out.send(Update::Contacts(c));
    }
    match backend.rooms().await {
        Ok(r) => {
            let _ = out.send(Update::Rooms(r));
        }
        Err(e) => {
            let _ = out.send(Update::Error(e.to_string()));
        }
    }
    loop {
        let event = async {
            match events.as_mut() {
                Some(rx) => rx.recv().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            ev = event => match ev {
                Some(ev) => { let _ = out.send(Update::Event(ev)); }
                None => events = None,
            },
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { return };
                if events.is_none() {
                    events = backend.subscribe().await.ok();
                }
                handle(backend.as_mut(), cmd, &out).await;
            }
        }
    }
}

async fn handle(b: &mut dyn ChatBackend, cmd: Cmd, out: &Sender<Update>) {
    let fail = |e: ChatError| {
        let _ = out.send(Update::Error(e.to_string()));
    };
    match cmd {
        Cmd::Refresh => {
            if let Ok(c) = b.contacts().await {
                let _ = out.send(Update::Contacts(c));
            }
            match b.rooms().await {
                Ok(r) => {
                    let _ = out.send(Update::Rooms(r));
                }
                Err(e) => fail(e),
            }
        }
        Cmd::History { room, before, limit } => match b.history(&room, before.as_ref(), limit).await {
            Ok(older) => {
                let _ = out.send(Update::History { room, older, was_before: before.is_some() });
            }
            Err(e) => fail(e),
        },
        Cmd::Send { local, room, draft } => {
            let result = b.send(&room, &draft).await;
            let _ = out.send(Update::Sent { local, room, result });
        }
        Cmd::Edit { room, id, body } => {
            if let Err(e) = b.edit(&room, &id, &body).await {
                fail(e);
            }
        }
        Cmd::Delete { room, id } => {
            if let Err(e) = b.delete(&room, &id).await {
                fail(e);
            }
        }
        Cmd::React { room, id, emoji, on } => {
            if let Err(e) = b.react(&room, &id, &emoji, on).await {
                fail(e);
            }
        }
        Cmd::Read { room, up_to } => {
            if let Err(e) = b.mark_read(&room, &up_to).await {
                fail(e);
            }
        }
        Cmd::Create { name, members } => match b.create_room(&name, &members).await {
            Ok(room) => {
                let _ = out.send(Update::Created(room));
            }
            Err(e) => fail(e),
        },
        Cmd::Add { room, members } => {
            if let Err(e) = b.add_members(&room, &members).await {
                fail(e);
            }
            if let Ok(r) = b.rooms().await {
                let _ = out.send(Update::Rooms(r));
            }
        }
        Cmd::Leave { room } => {
            if let Err(e) = b.leave(&room).await {
                fail(e);
            }
        }
        Cmd::Topic { room, topic } => {
            if let Err(e) = b.set_topic(&room, &topic).await {
                fail(e);
            }
            if let Ok(r) = b.rooms().await {
                let _ = out.send(Update::Rooms(r));
            }
        }
    }
}

impl ChatController {
    pub fn start(backend: Box<dyn ChatBackend>, store: Option<ChatStore>) -> ChatController {
        let me = backend.me();
        let (cmd, cmds) = unbounded_channel();
        let (tx, updates) = channel();
        crate::runtime::spawn(run(backend, cmds, tx));
        let mut c = ChatController {
            state: ChatState::new(me.id.clone()),
            caps: Capabilities::default(),
            contacts: Vec::new(),
            error: None,
            backoff: Duration::from_secs(2),
            notices: Vec::new(),
            loaded_all: HashMap::new(),
            loading: HashMap::new(),
            history_arrived: None,
            created: None,
            ready: false,
            cmd,
            updates,
            store,
            queue: Vec::new(),
            due: HashMap::new(),
            local_seq: 0,
            open: None,
        };
        c.restore();
        c
    }

    fn restore(&mut self) {
        let Some(store) = &self.store else { return };
        self.state.rooms = store.load_rooms();
        for r in self.state.rooms.clone() {
            let list = store.load_messages(&r.id);
            for m in list {
                self.state.insert_message(m);
            }
        }
        for marker in store.load_reads() {
            self.state.reads.insert(marker.room, marker.up_to);
        }
        self.queue = store.load_queued();
        for q in self.queue.clone() {
            self.local_seq = self.local_seq.max(q.local.0.trim_start_matches(LOCAL_PREFIX).parse().unwrap_or(0));
            let mut m = ChatMessage::new(q.local.as_str(), &q.room, &self.state.me.clone(), q.queued_at, &q.draft.body);
            m.reply_to = q.draft.reply_to.clone();
            m.attachments = q.draft.attachments.clone();
            m.delivery = match &q.failed {
                Some(reason) => Delivery::Failed(reason.clone()),
                None => Delivery::Sending,
            };
            self.state.insert_message(m);
            self.due.insert(q.local.clone(), Instant::now());
        }
    }

    pub fn me(&self) -> &MemberId {
        &self.state.me
    }

    pub fn online(&self) -> bool {
        self.state.online
    }

    pub fn pending(&self) -> usize {
        self.queue.iter().filter(|q| q.failed.is_none()).count()
    }

    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        while let Ok(update) = self.updates.try_recv() {
            changed = true;
            self.absorb(update);
        }
        changed |= self.retry_due();
        changed
    }

    fn persist_touched(&self, out: &Outcome) {
        let Some(store) = &self.store else { return };
        for t in &out.touched {
            match t {
                Touched::Message(room, id) => {
                    if let Some(m) = self.state.message(room, id) {
                        store.save_messages(std::slice::from_ref(m));
                    }
                }
                Touched::Room(id) => {
                    if let Some(r) = self.state.room(id) {
                        store.save_rooms(&self.state.rooms);
                        let _ = r;
                    }
                }
                Touched::RoomRemoved(id) => store.drop_room(id),
            }
        }
    }

    fn absorb(&mut self, update: Update) {
        match update {
            Update::Ready { caps, me } => {
                self.caps = caps;
                self.state.me = me.id;
                self.ready = true;
            }
            Update::Rooms(rooms) => {
                let local: HashMap<RoomId, (u32, Option<ChatMessage>)> =
                    self.state.rooms.iter().map(|r| (r.id.clone(), (r.unread, r.last_message.clone()))).collect();
                self.state.rooms = rooms;
                for r in &mut self.state.rooms {
                    let Some((unread, last)) = local.get(&r.id) else { continue };
                    let newer = matches!((last, &r.last_message), (Some(mine), Some(theirs)) if mine.sent_at > theirs.sent_at);
                    if newer {
                        r.last_message = last.clone();
                    }
                    if self.state.reads.contains_key(&r.id) {
                        r.unread = r.unread.min(*unread);
                    }
                }
                if let Some(store) = &self.store {
                    store.save_rooms(&self.state.rooms);
                }
                let empty: Vec<RoomId> = self.state.rooms.iter().filter(|r| self.state.thread(&r.id).is_empty() && r.last_message.is_some()).map(|r| r.id.clone()).collect();
                for room in empty {
                    self.load_history(&room);
                }
            }
            Update::Contacts(c) => self.contacts = c,
            Update::History { room, older, was_before } => {
                let got = older.len();
                if got < PAGE {
                    self.loaded_all.insert(room.clone(), true);
                }
                self.loading.insert(room.clone(), false);
                if let Some(store) = &self.store {
                    store.save_messages(&older);
                }
                let added = self.state.prepend_older(older);
                if added > 0 && was_before {
                    self.history_arrived = Some((room.clone(), added));
                }
            }
            Update::Sent { local, room, result } => self.sent(local, room, result),
            Update::Event(ev) => {
                let was_online = self.state.online;
                let open = self.open.clone();
                let out = self.state.apply(ev, open.as_ref());
                self.persist_touched(&out);
                if let Some((room, id)) = out.notify {
                    if let Some(m) = self.state.message(&room, &id) {
                        let title = self.state.room(&room).map(|r| r.title(&self.state.me)).unwrap_or_default();
                        let body = format!("{}: {}", self.state.name_of(&room, &m.sender), m.body);
                        self.notices.push(Notice { title, body, room });
                    }
                }
                if !was_online && self.state.online {
                    for d in self.due.values_mut() {
                        *d = Instant::now();
                    }
                    let _ = self.cmd.send(Cmd::Refresh);
                }
            }
            Update::Created(room) => {
                self.created = Some(room.id.clone());
                if !self.state.rooms.iter().any(|r| r.id == room.id) {
                    self.state.rooms.push(room);
                }
                if let Some(store) = &self.store {
                    store.save_rooms(&self.state.rooms);
                }
            }
            Update::Error(e) => self.error = Some(e),
        }
    }

    fn sent(&mut self, local: MessageId, room: RoomId, result: ChatResult<ChatMessage>) {
        let Some(at) = self.queue.iter().position(|q| q.local == local) else { return };
        match result {
            Ok(real) => {
                let q = self.queue.remove(at);
                if let Some(store) = &self.store {
                    store.drop_queued(&q.local);
                    store.save_messages(std::slice::from_ref(&real));
                }
                self.due.remove(&local);
                if let Some(list) = self.state.messages.get_mut(&room) {
                    list.retain(|m| m.id != local);
                }
                self.state.insert_message(real);
                self.state.refresh_last(&room);
                if let Some(store) = &self.store {
                    store.save_rooms(&self.state.rooms);
                }
            }
            Err(e) => {
                let attempts = {
                    let q = &mut self.queue[at];
                    q.attempts += 1;
                    q.attempts
                };
                let give_up = !e.retryable() || attempts >= RETRY_CAP;
                if give_up {
                    self.queue[at].failed = Some(e.to_string());
                    self.due.remove(&local);
                    self.set_delivery(&room, &local, Delivery::Failed(e.to_string()));
                } else {
                    let wait = self.backoff * 2u32.saturating_pow(attempts - 1);
                    self.due.insert(local.clone(), Instant::now() + wait);
                    self.set_delivery(&room, &local, Delivery::Sending);
                }
                if let Some(store) = &self.store {
                    store.save_queued(&self.queue[at]);
                }
            }
        }
    }

    fn set_delivery(&mut self, room: &RoomId, id: &MessageId, d: Delivery) {
        if let Some(m) = self.state.messages.get_mut(room).and_then(|l| l.iter_mut().find(|m| &m.id == id)) {
            m.delivery = d;
        }
        self.state.refresh_last(room);
    }

    fn retry_due(&mut self) -> bool {
        if !self.state.online {
            return false;
        }
        let now = Instant::now();
        let ready: Vec<MessageId> = self.due.iter().filter(|(_, at)| **at <= now).map(|(id, _)| id.clone()).collect();
        let mut sent = false;
        for local in ready {
            self.due.remove(&local);
            if let Some(q) = self.queue.iter().find(|q| q.local == local && q.failed.is_none()) {
                let _ = self.cmd.send(Cmd::Send { local: local.clone(), room: q.room.clone(), draft: q.draft.clone() });
                self.due.insert(local, now + Duration::from_secs(3600));
                sent = true;
            }
        }
        sent
    }

    pub fn next_retry_in(&self) -> Option<Duration> {
        let now = Instant::now();
        self.due.values().map(|at| at.saturating_duration_since(now).min(Duration::from_millis(500))).min()
    }

    pub fn refresh(&self) {
        let _ = self.cmd.send(Cmd::Refresh);
    }

    pub fn load_history(&mut self, room: &RoomId) {
        if self.loading.get(room).copied().unwrap_or(false) || self.loaded_all.get(room).copied().unwrap_or(false) {
            return;
        }
        self.loading.insert(room.clone(), true);
        let before = self.state.thread(room).iter().find(|m| !m.is_local()).map(|m| m.id.clone());
        let _ = self.cmd.send(Cmd::History { room: room.clone(), before, limit: PAGE });
    }

    pub fn send(&mut self, room: &RoomId, draft: ChatDraft) -> Option<MessageId> {
        if draft.is_empty() {
            return None;
        }
        self.local_seq += 1;
        let local = MessageId::new(format!("{LOCAL_PREFIX}{}", self.local_seq));
        let at = self.state.thread(room).last().map_or(0, |m| m.sent_at).max(now_secs());
        let mut m = ChatMessage::new(local.as_str(), room, &self.state.me.clone(), at, &draft.body);
        m.reply_to = draft.reply_to.clone();
        m.attachments = draft.attachments.clone();
        m.delivery = Delivery::Sending;
        self.state.insert_message(m);
        self.state.refresh_last(room);
        let q = Queued { local: local.clone(), room: room.clone(), draft, queued_at: at, attempts: 0, failed: None };
        if let Some(store) = &self.store {
            store.save_queued(&q);
        }
        self.queue.push(q);
        self.due.insert(local.clone(), Instant::now());
        Some(local)
    }

    pub fn retry(&mut self, local: &MessageId) {
        let Some(q) = self.queue.iter_mut().find(|q| &q.local == local) else { return };
        q.failed = None;
        q.attempts = 0;
        let room = q.room.clone();
        if let Some(store) = &self.store {
            store.save_queued(q);
        }
        self.set_delivery(&room, local, Delivery::Sending);
        self.due.insert(local.clone(), Instant::now());
    }

    pub fn discard(&mut self, local: &MessageId) {
        let Some(at) = self.queue.iter().position(|q| &q.local == local) else { return };
        let q = self.queue.remove(at);
        self.due.remove(local);
        if let Some(store) = &self.store {
            store.drop_queued(local);
        }
        if let Some(list) = self.state.messages.get_mut(&q.room) {
            list.retain(|m| &m.id != local);
        }
        self.state.refresh_last(&q.room);
    }

    pub fn edit(&self, room: &RoomId, id: &MessageId, body: &str) {
        let _ = self.cmd.send(Cmd::Edit { room: room.clone(), id: id.clone(), body: body.into() });
    }

    pub fn delete(&self, room: &RoomId, id: &MessageId) {
        let _ = self.cmd.send(Cmd::Delete { room: room.clone(), id: id.clone() });
    }

    pub fn react(&self, room: &RoomId, id: &MessageId, emoji: &str) {
        let on = !self.state.message(room, id).is_some_and(|m| m.reacted(emoji, &self.state.me));
        let _ = self.cmd.send(Cmd::React { room: room.clone(), id: id.clone(), emoji: emoji.into(), on });
    }

    pub fn mark_read(&mut self, room: &RoomId) {
        let Some(last) = self.state.thread(room).iter().rev().find(|m| !m.is_local()).map(|m| m.id.clone()) else { return };
        if self.state.reads.get(room) == Some(&last) && self.state.room(room).is_some_and(|r| r.unread == 0) {
            return;
        }
        self.state.mark_read(room, &last);
        if let Some(store) = &self.store {
            store.save_read(&ReadMarker { room: room.clone(), up_to: last.clone() });
            store.save_rooms(&self.state.rooms);
        }
        let _ = self.cmd.send(Cmd::Read { room: room.clone(), up_to: last });
    }

    pub fn create(&self, name: &str, members: Vec<MemberId>) {
        let _ = self.cmd.send(Cmd::Create { name: name.into(), members });
    }

    pub fn add_members(&self, room: &RoomId, members: Vec<MemberId>) {
        let _ = self.cmd.send(Cmd::Add { room: room.clone(), members });
    }

    pub fn leave(&self, room: &RoomId) {
        let _ = self.cmd.send(Cmd::Leave { room: room.clone() });
    }

    pub fn set_topic(&self, room: &RoomId, topic: &str) {
        let _ = self.cmd.send(Cmd::Topic { room: room.clone(), topic: topic.into() });
    }

    pub fn search(&self, text: &str) -> Vec<ChatMessage> {
        let needle = text.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for r in &self.state.rooms {
            for m in self.state.thread(&r.id) {
                let sender = self.state.name_of(&r.id, &m.sender).to_lowercase();
                if !m.deleted && (m.body.to_lowercase().contains(&needle) || sender.contains(&needle)) {
                    hits.push(m.clone());
                }
            }
        }
        hits.sort_by(|a, b| b.sent_at.cmp(&a.sent_at));
        hits
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
