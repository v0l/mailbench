use super::model::*;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Touched {
    Room(RoomId),
    Message(RoomId, MessageId),
    RoomRemoved(RoomId),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub touched: Vec<Touched>,
    pub notify: Option<(RoomId, MessageId)>,
}

#[derive(Clone, Debug, Default)]
pub struct ChatState {
    pub me: MemberId,
    pub rooms: Vec<Room>,
    pub messages: HashMap<RoomId, Vec<ChatMessage>>,
    pub typing: HashMap<RoomId, Vec<MemberId>>,
    pub presence: HashMap<MemberId, Presence>,
    pub reads: HashMap<RoomId, MessageId>,
    pub receipts: HashMap<(RoomId, MemberId), MessageId>,
    pub online: bool,
}

impl ChatState {
    pub fn new(me: MemberId) -> ChatState {
        ChatState { me, online: true, ..Default::default() }
    }

    pub fn room(&self, id: &RoomId) -> Option<&Room> {
        self.rooms.iter().find(|r| &r.id == id)
    }

    fn room_mut(&mut self, id: &RoomId) -> Option<&mut Room> {
        self.rooms.iter_mut().find(|r| &r.id == id)
    }

    pub fn ordered_rooms(&self) -> Vec<&Room> {
        let mut out: Vec<&Room> = self.rooms.iter().collect();
        out.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.activity().cmp(&a.activity())).then(a.id.cmp(&b.id)));
        out
    }

    pub fn total_unread(&self) -> u32 {
        self.rooms.iter().filter(|r| !r.muted).map(|r| r.unread).sum()
    }

    pub fn thread(&self, room: &RoomId) -> &[ChatMessage] {
        self.messages.get(room).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn message(&self, room: &RoomId, id: &MessageId) -> Option<&ChatMessage> {
        self.thread(room).iter().find(|m| &m.id == id)
    }

    pub fn typing_in(&self, room: &RoomId) -> Vec<MemberId> {
        self.typing.get(room).cloned().unwrap_or_default()
    }

    pub fn name_of(&self, room: &RoomId, who: &MemberId) -> String {
        if who == &self.me {
            return "you".into();
        }
        self.room(room)
            .and_then(|r| r.member(who))
            .map(|m| m.display_name.clone())
            .unwrap_or_else(|| who.0.clone())
    }

    pub fn insert_message(&mut self, m: ChatMessage) -> bool {
        let list = self.messages.entry(m.room.clone()).or_default();
        if list.iter().any(|x| x.id == m.id) {
            return false;
        }
        let at = list.partition_point(|x| x.sent_at <= m.sent_at);
        list.insert(at, m);
        true
    }

    pub fn prepend_older(&mut self, older: Vec<ChatMessage>) -> usize {
        older.into_iter().filter(|m| self.insert_message(m.clone())).count()
    }

    pub fn refresh_last(&mut self, room: &RoomId) {
        let last = self.messages.get(room).and_then(|l| l.iter().rev().next()).cloned();
        if let Some(r) = self.room_mut(room) {
            r.last_message = last;
        }
    }

    pub fn recount_unread(&mut self, room: &RoomId) {
        let me = self.me.clone();
        let marker = self.reads.get(room).cloned();
        let list = self.messages.get(room).cloned().unwrap_or_default();
        let after: &[ChatMessage] = match marker.and_then(|id| list.iter().position(|m| m.id == id)) {
            Some(i) => &list[i + 1..],
            None => &list,
        };
        let count = after.iter().filter(|m| m.sender != me && !m.deleted).count() as u32;
        if let Some(r) = self.room_mut(room) {
            r.unread = count;
        }
    }

    pub fn mark_read(&mut self, room: &RoomId, up_to: &MessageId) {
        self.reads.insert(room.clone(), up_to.clone());
        let me = self.me.clone();
        let list = self.messages.get(room).cloned().unwrap_or_default();
        let after: u32 = match list.iter().position(|m| &m.id == up_to) {
            Some(i) => list[i + 1..].iter().filter(|m| m.sender != me && !m.deleted).count() as u32,
            None => 0,
        };
        if let Some(r) = self.room_mut(room) {
            r.unread = after;
        }
    }

    pub fn apply(&mut self, event: ChatEvent, open: Option<&RoomId>) -> Outcome {
        let mut out = Outcome::default();
        match event {
            ChatEvent::NewMessage(m) => {
                let room = m.room.clone();
                if self.room(&room).is_none() {
                    return out;
                }
                let from_me = m.sender == self.me;
                let (id, preview) = (m.id.clone(), m.clone());
                if !self.insert_message(m) {
                    return out;
                }
                self.refresh_last(&room);
                let muted = self.room(&room).is_some_and(|r| r.muted);
                if !from_me {
                    if let Some(r) = self.room_mut(&room) {
                        r.unread += 1;
                    }
                    if !muted && open != Some(&room) {
                        out.notify = Some((room.clone(), preview.id.clone()));
                    }
                    self.typing.entry(room.clone()).or_default().retain(|t| t != &preview.sender);
                }
                out.touched.push(Touched::Message(room.clone(), id));
                out.touched.push(Touched::Room(room));
            }
            ChatEvent::Edited { room, id, body } => {
                if let Some(m) = self.messages.get_mut(&room).and_then(|l| l.iter_mut().find(|m| m.id == id)) {
                    m.body = body;
                    m.edited = true;
                    out.touched.push(Touched::Message(room.clone(), id));
                    self.refresh_last(&room);
                    out.touched.push(Touched::Room(room));
                }
            }
            ChatEvent::Deleted { room, id } => {
                if let Some(m) = self.messages.get_mut(&room).and_then(|l| l.iter_mut().find(|m| m.id == id)) {
                    m.deleted = true;
                    m.body.clear();
                    m.attachments.clear();
                    out.touched.push(Touched::Message(room.clone(), id));
                    self.refresh_last(&room);
                    self.recount_unread(&room);
                    out.touched.push(Touched::Room(room));
                }
            }
            ChatEvent::Reaction { room, id, emoji, member, added } => {
                if let Some(m) = self.messages.get_mut(&room).and_then(|l| l.iter_mut().find(|m| m.id == id)) {
                    match m.reactions.iter().position(|r| r.emoji == emoji) {
                        Some(i) => {
                            let r = &mut m.reactions[i];
                            r.members.retain(|x| x != &member);
                            if added {
                                r.members.push(member);
                            }
                            if r.members.is_empty() {
                                m.reactions.remove(i);
                            }
                        }
                        None if added => m.reactions.push(Reaction { emoji, members: vec![member] }),
                        None => {}
                    }
                    out.touched.push(Touched::Message(room, id));
                }
            }
            ChatEvent::Typing { room, member, on } => {
                let list = self.typing.entry(room).or_default();
                list.retain(|t| t != &member);
                if on && member != self.me {
                    list.push(member);
                }
            }
            ChatEvent::Presence { member, presence } => {
                self.presence.insert(member.clone(), presence);
                for r in &mut self.rooms {
                    if let Some(m) = r.members.iter_mut().find(|m| m.id == member) {
                        m.presence = Some(presence);
                    }
                }
            }
            ChatEvent::RoomChanged(room) => {
                let id = room.id.clone();
                match self.room_mut(&id) {
                    Some(existing) => {
                        let (unread, last) = (existing.unread, existing.last_message.clone());
                        *existing = room;
                        if existing.last_message.is_none() {
                            existing.last_message = last;
                        }
                        existing.unread = unread.max(existing.unread);
                    }
                    None => self.rooms.push(room),
                }
                out.touched.push(Touched::Room(id));
            }
            ChatEvent::RoomLeft(id) => {
                self.rooms.retain(|r| r.id != id);
                self.messages.remove(&id);
                self.reads.remove(&id);
                self.typing.remove(&id);
                out.touched.push(Touched::RoomRemoved(id));
            }
            ChatEvent::MemberChanged { room, member, present } => {
                if let Some(r) = self.room_mut(&room) {
                    r.members.retain(|m| m.id != member.id);
                    if present {
                        r.members.push(member);
                    }
                    out.touched.push(Touched::Room(room));
                }
            }
            ChatEvent::Read { room, member, up_to } => {
                if member == self.me {
                    self.mark_read(&room, &up_to);
                    out.touched.push(Touched::Room(room));
                } else {
                    self.receipts.insert((room, member), up_to);
                }
            }
            ChatEvent::Connection(on) => self.online = on,
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> MemberId {
        MemberId::new("me")
    }

    fn state() -> (ChatState, RoomId) {
        let id = RoomId::new("r1");
        let mut s = ChatState::new(me());
        s.rooms.push(Room {
            id: id.clone(),
            name: "Crew".into(),
            members: vec![Member::new("me", "Me"), Member::new("ann", "Ann Byrne"), Member::new("bo", "Bo Lind")],
            created_at: 1,
            ..Default::default()
        });
        (s, id)
    }

    fn msg(id: &str, room: &RoomId, from: &str, at: i64, body: &str) -> ChatMessage {
        ChatMessage::new(id, room, &MemberId::new(from), at, body)
    }

    #[test]
    fn new_message_counts_unread_and_dedupes() {
        let (mut s, r) = state();
        let ev = ChatEvent::NewMessage(msg("m1", &r, "ann", 10, "hi"));
        let out = s.apply(ev.clone(), None);
        assert_eq!(s.room(&r).unwrap().unread, 1);
        assert_eq!(out.notify, Some((r.clone(), MessageId::new("m1"))));
        let again = s.apply(ev, None);
        assert_eq!(s.room(&r).unwrap().unread, 1);
        assert!(again.touched.is_empty());
    }

    #[test]
    fn own_messages_do_not_count_and_open_rooms_do_not_notify() {
        let (mut s, r) = state();
        s.apply(ChatEvent::NewMessage(msg("m1", &r, "me", 10, "mine")), None);
        assert_eq!(s.room(&r).unwrap().unread, 0);
        let out = s.apply(ChatEvent::NewMessage(msg("m2", &r, "ann", 11, "yo")), Some(&r));
        assert_eq!(out.notify, None);
        assert_eq!(s.room(&r).unwrap().unread, 1);
    }

    #[test]
    fn muted_rooms_count_but_stay_quiet() {
        let (mut s, r) = state();
        s.rooms[0].muted = true;
        let out = s.apply(ChatEvent::NewMessage(msg("m1", &r, "ann", 10, "hi")), None);
        assert_eq!(out.notify, None);
        assert_eq!(s.room(&r).unwrap().unread, 1);
        assert_eq!(s.total_unread(), 0);
    }

    #[test]
    fn mark_read_counts_what_is_after_the_marker() {
        let (mut s, r) = state();
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            s.apply(ChatEvent::NewMessage(msg(id, &r, "ann", 10 + i as i64, "x")), None);
        }
        s.mark_read(&r, &MessageId::new("b"));
        assert_eq!(s.room(&r).unwrap().unread, 1);
        s.mark_read(&r, &MessageId::new("c"));
        assert_eq!(s.room(&r).unwrap().unread, 0);
    }

    #[test]
    fn edit_delete_and_reaction_events_reflect() {
        let (mut s, r) = state();
        s.apply(ChatEvent::NewMessage(msg("a", &r, "ann", 10, "first")), None);
        let a = MessageId::new("a");
        s.apply(ChatEvent::Edited { room: r.clone(), id: a.clone(), body: "second".into() }, None);
        assert!(s.message(&r, &a).unwrap().edited);
        assert_eq!(s.message(&r, &a).unwrap().body, "second");
        let react = |added| ChatEvent::Reaction { room: r.clone(), id: a.clone(), emoji: "+1".into(), member: MemberId::new("bo"), added };
        s.apply(react(true), None);
        assert_eq!(s.message(&r, &a).unwrap().reactions.len(), 1);
        s.apply(react(false), None);
        assert!(s.message(&r, &a).unwrap().reactions.is_empty());
        s.apply(ChatEvent::Deleted { room: r.clone(), id: a.clone() }, None);
        let m = s.message(&r, &a).unwrap();
        assert!(m.deleted && m.body.is_empty());
        assert_eq!(s.room(&r).unwrap().unread, 0);
    }

    #[test]
    fn rooms_order_pinned_first_then_newest() {
        let (mut s, r) = state();
        for (id, at, pinned) in [("old", 5, false), ("new", 50, false), ("pin", 1, true)] {
            s.rooms.push(Room { id: RoomId::new(id), name: id.into(), created_at: at, pinned, ..Default::default() });
        }
        let order: Vec<&str> = s.ordered_rooms().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(order, vec!["pin", "new", "old", r.as_str()]);
    }

    #[test]
    fn typing_member_changes_and_leave() {
        let (mut s, r) = state();
        s.apply(ChatEvent::Typing { room: r.clone(), member: MemberId::new("ann"), on: true }, None);
        assert_eq!(s.typing_in(&r), vec![MemberId::new("ann")]);
        s.apply(ChatEvent::NewMessage(msg("a", &r, "ann", 10, "x")), None);
        assert!(s.typing_in(&r).is_empty());
        s.apply(ChatEvent::MemberChanged { room: r.clone(), member: Member::new("cy", "Cy"), present: true }, None);
        assert_eq!(s.room(&r).unwrap().members.len(), 4);
        s.apply(ChatEvent::MemberChanged { room: r.clone(), member: Member::new("cy", "Cy"), present: false }, None);
        assert_eq!(s.room(&r).unwrap().members.len(), 3);
        let out = s.apply(ChatEvent::RoomLeft(r.clone()), None);
        assert!(s.room(&r).is_none());
        assert_eq!(out.touched, vec![Touched::RoomRemoved(r)]);
    }

    #[test]
    fn unnamed_group_title_lists_three_names_and_a_count() {
        let mut room = Room { members: vec![Member::new("me", "Me")], ..Default::default() };
        for (i, n) in ["Ann", "Bo", "Cy", "Di", "Ed"].iter().enumerate() {
            room.members.push(Member::new(&format!("u{i}"), n));
        }
        assert_eq!(room.title(&me()), "Ann, Bo, Cy, +2");
    }
}
