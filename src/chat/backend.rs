use super::model::*;
use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedReceiver;

#[async_trait]
pub trait ChatBackend: Send {
    fn kind(&self) -> &'static str;

    fn capabilities(&self) -> Capabilities;

    fn me(&self) -> Member;

    async fn contacts(&mut self) -> ChatResult<Vec<Member>>;

    async fn rooms(&mut self) -> ChatResult<Vec<Room>>;

    async fn history(&mut self, room: &RoomId, before: Option<&MessageId>, limit: usize) -> ChatResult<Vec<ChatMessage>>;

    async fn send(&mut self, room: &RoomId, draft: &ChatDraft) -> ChatResult<ChatMessage>;

    async fn edit(&mut self, room: &RoomId, id: &MessageId, body: &str) -> ChatResult<()> {
        let _ = (room, id, body);
        Err(ChatError::Unsupported("edit"))
    }

    async fn delete(&mut self, room: &RoomId, id: &MessageId) -> ChatResult<()> {
        let _ = (room, id);
        Err(ChatError::Unsupported("delete"))
    }

    async fn react(&mut self, room: &RoomId, id: &MessageId, emoji: &str, on: bool) -> ChatResult<()> {
        let _ = (room, id, emoji, on);
        Err(ChatError::Unsupported("reactions"))
    }

    async fn mark_read(&mut self, room: &RoomId, up_to: &MessageId) -> ChatResult<()>;

    async fn create_room(&mut self, name: &str, members: &[MemberId]) -> ChatResult<Room>;

    async fn add_members(&mut self, room: &RoomId, members: &[MemberId]) -> ChatResult<()>;

    async fn leave(&mut self, room: &RoomId) -> ChatResult<()>;

    async fn set_topic(&mut self, room: &RoomId, topic: &str) -> ChatResult<()>;

    async fn subscribe(&mut self) -> ChatResult<UnboundedReceiver<ChatEvent>>;
}
