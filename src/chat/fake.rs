use super::backend::ChatBackend;
use super::model::*;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub const FAKE_START: i64 = 1_700_000_000;

struct World {
    me: Member,
    contacts: Vec<Member>,
    rooms: Vec<Room>,
    log: HashMap<RoomId, Vec<ChatMessage>>,
    subs: Vec<UnboundedSender<ChatEvent>>,
    caps: Capabilities,
    offline: bool,
    lag: Duration,
    fail_sends: u32,
    auto_reply: bool,
    clock: i64,
    next: u64,
    sends: u32,
}

impl World {
    fn emit(&mut self, event: ChatEvent) {
        self.subs.retain(|s| s.send(event.clone()).is_ok());
    }

    fn stamp(&mut self) -> i64 {
        self.clock += 30;
        self.clock
    }

    fn fresh_id(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    fn push(&mut self, mut m: ChatMessage) -> ChatMessage {
        m.delivery = Delivery::Sent;
        self.log.entry(m.room.clone()).or_default().push(m.clone());
        if let Some(r) = self.rooms.iter_mut().find(|r| r.id == m.room) {
            r.last_message = Some(m.clone());
            if m.sender != self.me.id {
                r.unread += 1;
            }
        }
        m
    }

    fn room_index(&self, id: &RoomId) -> ChatResult<usize> {
        self.rooms.iter().position(|r| &r.id == id).ok_or_else(|| ChatError::NotFound(id.0.clone()))
    }

    fn message_mut(&mut self, room: &RoomId, id: &MessageId) -> ChatResult<&mut ChatMessage> {
        self.log
            .get_mut(room)
            .and_then(|l| l.iter_mut().find(|m| &m.id == id))
            .ok_or_else(|| ChatError::NotFound(id.0.clone()))
    }

    fn guard(&self) -> ChatResult<()> {
        if self.offline { Err(ChatError::Offline) } else { Ok(()) }
    }
}

#[derive(Clone)]
pub struct FakeHandle {
    world: Arc<Mutex<World>>,
}

pub struct FakeChat {
    world: Arc<Mutex<World>>,
}

fn lock(world: &Arc<Mutex<World>>) -> MutexGuard<'_, World> {
    world.lock().unwrap_or_else(|p| p.into_inner())
}

impl FakeChat {
    pub fn empty() -> (FakeChat, FakeHandle) {
        let world = Arc::new(Mutex::new(World {
            me: Member::new("me", "You").with_role(Role::Owner),
            contacts: Vec::new(),
            rooms: Vec::new(),
            log: HashMap::new(),
            subs: Vec::new(),
            caps: Capabilities::everything(),
            offline: false,
            lag: Duration::ZERO,
            fail_sends: 0,
            auto_reply: false,
            clock: FAKE_START,
            next: 0,
            sends: 0,
        }));
        (FakeChat { world: world.clone() }, FakeHandle { world })
    }

    pub fn with_contacts() -> (FakeChat, FakeHandle) {
        let (chat, handle) = FakeChat::empty();
        lock(&handle.world).contacts = [
            ("ann", "Ann Byrne"),
            ("bo", "Bo Lind"),
            ("cy", "Cy Okafor"),
            ("di", "Di Marsh"),
            ("ed", "Ed Voss"),
        ]
        .iter()
        .map(|(id, name)| {
            let mut m = Member::new(id, name);
            m.presence = Some(Presence::Online);
            m
        })
        .collect();
        (chat, handle)
    }

    pub fn demo() -> (FakeChat, FakeHandle) {
        let (chat, handle) = FakeChat::with_contacts();
        handle.seed_demo();
        (chat, handle)
    }
}

impl FakeHandle {
    pub fn me(&self) -> Member {
        lock(&self.world).me.clone()
    }

    pub fn set_capabilities(&self, caps: Capabilities) {
        lock(&self.world).caps = caps;
    }

    pub fn set_offline(&self, offline: bool) {
        let mut w = lock(&self.world);
        w.offline = offline;
        w.emit(ChatEvent::Connection(!offline));
    }

    pub fn set_lag(&self, lag: Duration) {
        lock(&self.world).lag = lag;
    }

    pub fn fail_next_sends(&self, n: u32) {
        lock(&self.world).fail_sends = n;
    }

    pub fn set_auto_reply(&self, on: bool) {
        lock(&self.world).auto_reply = on;
    }

    pub fn sends_attempted(&self) -> u32 {
        lock(&self.world).sends
    }

    pub fn add_contact(&self, member: Member) {
        lock(&self.world).contacts.push(member);
    }

    pub fn add_room(&self, id: &str, name: &str, kind: RoomKind, member_ids: &[&str]) -> RoomId {
        let mut w = lock(&self.world);
        let mut members = vec![w.me.clone()];
        for id in member_ids {
            if let Some(c) = w.contacts.iter().find(|c| c.id.as_str() == *id) {
                members.push(c.clone());
            }
        }
        let created_at = w.clock;
        let room = Room { id: RoomId::new(id), name: name.into(), kind, members, created_at, ..Default::default() };
        w.rooms.push(room.clone());
        w.log.entry(room.id.clone()).or_default();
        room.id
    }

    pub fn say(&self, room: &RoomId, from: &str, body: &str) -> MessageId {
        let mut w = lock(&self.world);
        let at = w.stamp();
        let id = w.fresh_id("m");
        let m = w.push(ChatMessage::new(&id, room, &MemberId::new(from), at, body));
        w.emit(ChatEvent::NewMessage(m.clone()));
        m.id
    }

    pub fn say_message(&self, mut m: ChatMessage) -> MessageId {
        let mut w = lock(&self.world);
        if m.id.0.is_empty() {
            m.id = MessageId::new(w.fresh_id("m"));
        }
        let m = w.push(m);
        w.emit(ChatEvent::NewMessage(m.clone()));
        m.id
    }

    pub fn backlog(&self, room: &RoomId, from: &str, count: usize) {
        let mut w = lock(&self.world);
        let mut batch = Vec::new();
        for i in 0..count {
            let id = w.fresh_id("old");
            batch.push(ChatMessage::new(&id, room, &MemberId::new(from), FAKE_START - 86_400 + i as i64 * 20, &format!("older message {i}")));
        }
        let log = w.log.entry(room.clone()).or_default();
        batch.append(log);
        *log = batch;
        let last = log.last().cloned();
        if let Some(r) = w.rooms.iter_mut().find(|r| &r.id == room) {
            if r.last_message.is_none() {
                r.last_message = last;
            }
        }
    }

    pub fn typing(&self, room: &RoomId, who: &str, on: bool) {
        lock(&self.world).emit(ChatEvent::Typing { room: room.clone(), member: MemberId::new(who), on });
    }

    pub fn react_as(&self, room: &RoomId, id: &MessageId, who: &str, emoji: &str, added: bool) {
        let mut w = lock(&self.world);
        let event = ChatEvent::Reaction { room: room.clone(), id: id.clone(), emoji: emoji.into(), member: MemberId::new(who), added };
        w.emit(event);
    }

    pub fn edit_as(&self, room: &RoomId, id: &MessageId, body: &str) {
        let mut w = lock(&self.world);
        if let Ok(m) = w.message_mut(room, id) {
            m.body = body.into();
            m.edited = true;
        }
        w.emit(ChatEvent::Edited { room: room.clone(), id: id.clone(), body: body.into() });
    }

    pub fn delete_as(&self, room: &RoomId, id: &MessageId) {
        let mut w = lock(&self.world);
        if let Ok(m) = w.message_mut(room, id) {
            m.deleted = true;
            m.body.clear();
        }
        w.emit(ChatEvent::Deleted { room: room.clone(), id: id.clone() });
    }

    pub fn log(&self, room: &RoomId) -> Vec<ChatMessage> {
        lock(&self.world).log.get(room).cloned().unwrap_or_default()
    }

    pub fn seed_demo(&self) {
        let crew = self.add_room("crew", "Field crew", RoomKind::Group, &["ann", "bo", "cy"]);
        self.add_room("lunch", "", RoomKind::Group, &["bo", "cy", "di", "ed"]);
        let ann = self.add_room("ann", "", RoomKind::Direct, &["ann"]);
        {
            let mut w = lock(&self.world);
            if let Some(r) = w.rooms.iter_mut().find(|r| r.id == crew) {
                r.topic = "Site visits and parts runs".into();
                r.pinned = true;
                r.members[1].role = Role::Admin;
            }
        }
        let first = self.say(&crew, "ann", "Van is loaded, leaving at nine.");
        let mut reply = ChatMessage::new("", &crew, &MemberId::new("bo"), 0, "I will meet you at the depot.");
        reply.reply_to = Some(first.clone());
        let second = self.say_message(reply);
        self.say(&crew, "bo", "Bring the long ladder.");
        self.say(&crew, "me", "On it, ladder is in the shed.");
        let mut with_file = ChatMessage::new("", &crew, &MemberId::new("cy"), 0, "Route for today attached.");
        with_file.attachments.push(ChatAttachment { name: "route.pdf".into(), size: 48_200, mime: "application/pdf".into() });
        self.say_message(with_file);
        self.react_as(&crew, &first, "bo", "+1", true);
        self.react_as(&crew, &first, "cy", "+1", true);
        self.react_as(&crew, &second, "me", "\u{2713}", true);
        let typo = self.say(&crew, "cy", "Weather looks fine tomorow.");
        self.edit_as(&crew, &typo, "Weather looks fine tomorrow.");
        let gone = self.say(&crew, "ann", "Wrong chat, sorry.");
        self.delete_as(&crew, &gone);
        self.say(&ann, "ann", "Can you cover Thursday?");
    }
}

#[async_trait]
impl ChatBackend for FakeChat {
    fn kind(&self) -> &'static str {
        "fake"
    }

    fn capabilities(&self) -> Capabilities {
        lock(&self.world).caps.clone()
    }

    fn me(&self) -> Member {
        lock(&self.world).me.clone()
    }

    async fn contacts(&mut self) -> ChatResult<Vec<Member>> {
        let w = lock(&self.world);
        w.guard()?;
        Ok(w.contacts.clone())
    }

    async fn rooms(&mut self) -> ChatResult<Vec<Room>> {
        let w = lock(&self.world);
        w.guard()?;
        Ok(w.rooms.clone())
    }

    async fn history(&mut self, room: &RoomId, before: Option<&MessageId>, limit: usize) -> ChatResult<Vec<ChatMessage>> {
        let lag = {
            let w = lock(&self.world);
            w.guard()?;
            w.room_index(room)?;
            w.lag
        };
        if !lag.is_zero() {
            tokio::time::sleep(lag).await;
        }
        let w = lock(&self.world);
        let log = w.log.get(room).cloned().unwrap_or_default();
        let end = match before {
            Some(id) => log.iter().position(|m| &m.id == id).ok_or_else(|| ChatError::NotFound(id.0.clone()))?,
            None => log.len(),
        };
        let start = end.saturating_sub(limit);
        Ok(log[start..end].to_vec())
    }

    async fn send(&mut self, room: &RoomId, draft: &ChatDraft) -> ChatResult<ChatMessage> {
        let lag = {
            let mut w = lock(&self.world);
            w.sends += 1;
            w.guard()?;
            w.room_index(room)?;
            if w.fail_sends > 0 {
                w.fail_sends -= 1;
                return Err(ChatError::Rejected("the server refused the message".into()));
            }
            if let Some(max) = w.caps.max_message_len {
                if draft.body.chars().count() > max {
                    return Err(ChatError::Rejected(format!("messages are limited to {max} characters")));
                }
            }
            w.lag
        };
        if !lag.is_zero() {
            tokio::time::sleep(lag).await;
        }
        let mut w = lock(&self.world);
        w.guard()?;
        let at = w.stamp();
        let id = w.fresh_id("m");
        let mut m = ChatMessage::new(&id, room, &w.me.id.clone(), at, &draft.body);
        m.reply_to = draft.reply_to.clone();
        m.attachments = draft.attachments.clone();
        let m = w.push(m);
        if w.auto_reply {
            let other = w.rooms.iter().find(|r| &r.id == room).and_then(|r| r.members.iter().find(|x| x.id != w.me.id)).cloned();
            if let Some(other) = other {
                w.emit(ChatEvent::Typing { room: room.clone(), member: other.id.clone(), on: true });
                let at = w.stamp();
                let rid = w.fresh_id("m");
                let first = draft.body.split_whitespace().next().unwrap_or("that");
                let text = format!("Got it, {first} noted.");
                let reply = w.push(ChatMessage::new(&rid, room, &other.id, at, &text));
                w.emit(ChatEvent::NewMessage(reply));
            }
        }
        Ok(m)
    }

    async fn edit(&mut self, room: &RoomId, id: &MessageId, body: &str) -> ChatResult<()> {
        let mut w = lock(&self.world);
        w.guard()?;
        if !w.caps.edit {
            return Err(ChatError::Unsupported("edit"));
        }
        let me = w.me.id.clone();
        let m = w.message_mut(room, id)?;
        if m.sender != me {
            return Err(ChatError::Rejected("only your own messages can be edited".into()));
        }
        m.body = body.into();
        m.edited = true;
        w.emit(ChatEvent::Edited { room: room.clone(), id: id.clone(), body: body.into() });
        Ok(())
    }

    async fn delete(&mut self, room: &RoomId, id: &MessageId) -> ChatResult<()> {
        let mut w = lock(&self.world);
        w.guard()?;
        if !w.caps.delete {
            return Err(ChatError::Unsupported("delete"));
        }
        let m = w.message_mut(room, id)?;
        m.deleted = true;
        m.body.clear();
        w.emit(ChatEvent::Deleted { room: room.clone(), id: id.clone() });
        Ok(())
    }

    async fn react(&mut self, room: &RoomId, id: &MessageId, emoji: &str, on: bool) -> ChatResult<()> {
        let mut w = lock(&self.world);
        w.guard()?;
        if !w.caps.reactions {
            return Err(ChatError::Unsupported("reactions"));
        }
        let me = w.me.id.clone();
        let m = w.message_mut(room, id)?;
        match m.reactions.iter().position(|r| r.emoji == emoji) {
            Some(i) => {
                m.reactions[i].members.retain(|x| x != &me);
                if on {
                    m.reactions[i].members.push(me.clone());
                }
                if m.reactions[i].members.is_empty() {
                    m.reactions.remove(i);
                }
            }
            None if on => m.reactions.push(Reaction { emoji: emoji.into(), members: vec![me.clone()] }),
            None => {}
        }
        w.emit(ChatEvent::Reaction { room: room.clone(), id: id.clone(), emoji: emoji.into(), member: me, added: on });
        Ok(())
    }

    async fn mark_read(&mut self, room: &RoomId, up_to: &MessageId) -> ChatResult<()> {
        let mut w = lock(&self.world);
        w.guard()?;
        let at = w.room_index(room)?;
        let seen = w.log.get(room).and_then(|l| l.iter().position(|m| &m.id == up_to));
        let me = w.me.id.clone();
        let unread = match (seen, w.log.get(room)) {
            (Some(i), Some(l)) => l[i + 1..].iter().filter(|m| m.sender != me && !m.deleted).count() as u32,
            _ => 0,
        };
        w.rooms[at].unread = unread;
        Ok(())
    }

    async fn create_room(&mut self, name: &str, members: &[MemberId]) -> ChatResult<Room> {
        let mut w = lock(&self.world);
        w.guard()?;
        if members.is_empty() {
            return Err(ChatError::Rejected("a room needs at least one other person".into()));
        }
        let mut list = vec![w.me.clone()];
        for id in members {
            let c = w.contacts.iter().find(|c| &c.id == id).cloned().ok_or_else(|| ChatError::NotFound(id.0.clone()))?;
            list.push(c);
        }
        let kind = if members.len() == 1 && name.trim().is_empty() { RoomKind::Direct } else { RoomKind::Group };
        let id = RoomId::new(w.fresh_id("room"));
        let created_at = w.stamp();
        let room = Room { id: id.clone(), name: name.trim().into(), kind, members: list, created_at, ..Default::default() };
        w.rooms.push(room.clone());
        w.log.insert(id, Vec::new());
        Ok(room)
    }

    async fn add_members(&mut self, room: &RoomId, members: &[MemberId]) -> ChatResult<()> {
        let mut w = lock(&self.world);
        w.guard()?;
        let at = w.room_index(room)?;
        for id in members {
            let c = w.contacts.iter().find(|c| &c.id == id).cloned().ok_or_else(|| ChatError::NotFound(id.0.clone()))?;
            if w.rooms[at].member(id).is_none() {
                w.rooms[at].members.push(c.clone());
                w.emit(ChatEvent::MemberChanged { room: room.clone(), member: c, present: true });
            }
        }
        Ok(())
    }

    async fn leave(&mut self, room: &RoomId) -> ChatResult<()> {
        let mut w = lock(&self.world);
        w.guard()?;
        let at = w.room_index(room)?;
        w.rooms.remove(at);
        w.log.remove(room);
        w.emit(ChatEvent::RoomLeft(room.clone()));
        Ok(())
    }

    async fn set_topic(&mut self, room: &RoomId, topic: &str) -> ChatResult<()> {
        let mut w = lock(&self.world);
        w.guard()?;
        let at = w.room_index(room)?;
        w.rooms[at].topic = topic.into();
        let changed = w.rooms[at].clone();
        w.emit(ChatEvent::RoomChanged(changed));
        Ok(())
    }

    async fn subscribe(&mut self) -> ChatResult<UnboundedReceiver<ChatEvent>> {
        let (tx, rx) = unbounded_channel();
        let mut w = lock(&self.world);
        w.guard()?;
        w.subs.push(tx);
        Ok(rx)
    }
}
