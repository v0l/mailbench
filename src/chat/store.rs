use super::model::*;
use crate::store::Store;

const ROOMS: &str = "chat-room";
const MESSAGES: &str = "chat-message";
const READS: &str = "chat-read";
const OUTBOX: &str = "chat-outbox";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Queued {
    pub local: MessageId,
    pub room: RoomId,
    pub draft: ChatDraft,
    pub queued_at: i64,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub failed: Option<String>,
}

pub struct ChatStore {
    store: Store,
    account: String,
}

fn bytes<T: serde::Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

impl ChatStore {
    pub fn new(store: Store, account: &str) -> ChatStore {
        ChatStore { store, account: account.to_string() }
    }

    fn key(&self, rest: &str) -> String {
        format!("{}/{rest}", self.account)
    }

    fn message_key(&self, m: &ChatMessage) -> String {
        self.key(&format!("{}/{}", m.room, m.id))
    }

    fn scan<T: serde::de::DeserializeOwned>(&self, kind: &str, prefix: &str) -> Vec<T> {
        self.store
            .scan_prefix(kind, prefix)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(_, raw)| serde_json::from_slice(&raw).ok())
            .collect()
    }

    pub fn save_rooms(&self, rooms: &[Room]) {
        let wanted: Vec<String> = rooms.iter().map(|r| self.key(r.id.as_str())).collect();
        for id in self.store.list_prefix(ROOMS, &self.key("")).unwrap_or_default() {
            if !wanted.contains(&id) {
                let _ = self.store.delete(ROOMS, &id);
            }
        }
        for r in rooms {
            let _ = self.store.put(ROOMS, &self.key(r.id.as_str()), &bytes(r));
        }
    }

    pub fn load_rooms(&self) -> Vec<Room> {
        self.scan(ROOMS, &self.key(""))
    }

    pub fn save_messages(&self, messages: &[ChatMessage]) {
        for m in messages.iter().filter(|m| !m.is_local()) {
            let _ = self.store.put(MESSAGES, &self.message_key(m), &bytes(m));
        }
    }

    pub fn forget_message(&self, m: &ChatMessage) {
        let _ = self.store.delete(MESSAGES, &self.message_key(m));
    }

    pub fn load_messages(&self, room: &RoomId) -> Vec<ChatMessage> {
        let mut all: Vec<ChatMessage> = self.scan(MESSAGES, &self.key(&format!("{room}/")));
        all.sort_by(|a, b| a.sent_at.cmp(&b.sent_at).then(a.id.cmp(&b.id)));
        all
    }

    pub fn drop_room(&self, room: &RoomId) {
        let _ = self.store.delete(ROOMS, &self.key(room.as_str()));
        let _ = self.store.delete_prefix(MESSAGES, &self.key(&format!("{room}/")));
        let _ = self.store.delete(READS, &self.key(room.as_str()));
    }

    pub fn save_read(&self, marker: &ReadMarker) {
        let _ = self.store.put(READS, &self.key(marker.room.as_str()), &bytes(marker));
    }

    pub fn load_reads(&self) -> Vec<ReadMarker> {
        self.scan(READS, &self.key(""))
    }

    pub fn save_queued(&self, q: &Queued) {
        let _ = self.store.put(OUTBOX, &self.key(q.local.as_str()), &bytes(q));
    }

    pub fn drop_queued(&self, local: &MessageId) {
        let _ = self.store.delete(OUTBOX, &self.key(local.as_str()));
    }

    pub fn load_queued(&self) -> Vec<Queued> {
        let mut all: Vec<Queued> = self.scan(OUTBOX, &self.key(""));
        all.sort_by(|a, b| a.queued_at.cmp(&b.queued_at).then(a.local.cmp(&b.local)));
        all
    }

    pub fn search(&self, text: &str) -> Vec<ChatMessage> {
        let needle = text.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let rooms = self.load_rooms();
        let mut hits: Vec<ChatMessage> = self
            .scan::<ChatMessage>(MESSAGES, &self.key(""))
            .into_iter()
            .filter(|m| !m.deleted)
            .filter(|m| {
                let sender = rooms
                    .iter()
                    .find(|r| r.id == m.room)
                    .and_then(|r| r.member(&m.sender))
                    .map(|s| s.display_name.to_lowercase())
                    .unwrap_or_default();
                m.body.to_lowercase().contains(&needle) || sender.contains(&needle)
            })
            .collect();
        hits.sort_by(|a, b| b.sent_at.cmp(&a.sent_at));
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::KdfParams;

    fn open(dir: &std::path::Path) -> ChatStore {
        let store = Store::create(&dir.join("chat.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
        ChatStore::new(store, "acct")
    }

    #[test]
    fn rooms_messages_reads_and_queue_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(dir.path());
        let room = Room { id: RoomId::new("r"), name: "Crew".into(), members: vec![Member::new("ann", "Ann")], ..Default::default() };
        s.save_rooms(&[room.clone()]);
        assert_eq!(s.load_rooms(), vec![room.clone()]);
        let m1 = ChatMessage::new("a", &room.id, &MemberId::new("ann"), 20, "second Lunch");
        let m2 = ChatMessage::new("b", &room.id, &MemberId::new("ann"), 10, "first");
        s.save_messages(&[m1.clone(), m2.clone()]);
        assert_eq!(s.load_messages(&room.id), vec![m2.clone(), m1.clone()]);
        assert_eq!(s.search("lunch"), vec![m1.clone()]);
        assert_eq!(s.search("ann").len(), 2);
        s.save_read(&ReadMarker { room: room.id.clone(), up_to: m2.id.clone() });
        assert_eq!(s.load_reads()[0].up_to, m2.id);
        let q = Queued { local: MessageId::new("local-1"), room: room.id.clone(), draft: ChatDraft::text("hi"), queued_at: 5, attempts: 0, failed: None };
        s.save_queued(&q);
        assert_eq!(s.load_queued(), vec![q.clone()]);
        s.drop_queued(&q.local);
        assert!(s.load_queued().is_empty());
        s.drop_room(&room.id);
        assert!(s.load_rooms().is_empty() && s.load_messages(&room.id).is_empty());
    }

    #[test]
    fn an_old_database_without_chat_records_keeps_working() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        {
            let store = Store::create(&path, None, Some(("pw", KdfParams::fast()))).unwrap();
            store.put("account", "a1", b"{}").unwrap();
        }
        let store = Store::open(&path, &[crate::store::Unlock::Password("pw")]).unwrap();
        let chat = ChatStore::new(store.try_clone().unwrap(), "a1");
        assert!(chat.load_rooms().is_empty());
        chat.save_rooms(&[Room { id: RoomId::new("r"), ..Default::default() }]);
        assert_eq!(store.get("account", "a1").unwrap().unwrap().as_slice(), b"{}");
        assert_eq!(chat.load_rooms().len(), 1);
    }

    #[test]
    fn accounts_do_not_see_each_others_chats() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::create(&dir.path().join("c.db"), None, Some(("pw", KdfParams::fast()))).unwrap();
        let a = ChatStore::new(store.try_clone().unwrap(), "a");
        let b = ChatStore::new(store, "b");
        a.save_rooms(&[Room { id: RoomId::new("r"), ..Default::default() }]);
        assert!(b.load_rooms().is_empty());
    }
}
