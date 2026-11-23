use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(text: impl Into<String>) -> Self {
                $name(text.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(RoomId);
id_type!(MemberId);
id_type!(MessageId);

pub const LOCAL_PREFIX: &str = "local-";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoomKind {
    Direct,
    #[default]
    Group,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Owner,
    Admin,
    #[default]
    Member,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Presence {
    Online,
    Away,
    Offline,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub id: MemberId,
    pub display_name: String,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub role: Role,
    #[serde(default)]
    pub presence: Option<Presence>,
}

impl Member {
    pub fn new(id: &str, name: &str) -> Member {
        Member { id: MemberId::new(id), display_name: name.to_string(), ..Default::default() }
    }

    pub fn with_role(mut self, role: Role) -> Member {
        self.role = role;
        self
    }

    pub fn initials(&self) -> String {
        initials(&self.display_name)
    }
}

pub fn initials(name: &str) -> String {
    let mut words = name.split_whitespace().filter_map(|w| w.chars().find(|c| c.is_alphanumeric()));
    let first = words.next();
    let last = words.last();
    match (first, last) {
        (Some(a), Some(b)) => format!("{}{}", a.to_uppercase(), b.to_uppercase()),
        (Some(a), None) => a.to_uppercase().to_string(),
        _ => "?".into(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Delivery {
    Sending,
    Sent,
    Delivered,
    Failed(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatAttachment {
    pub name: String,
    pub size: u64,
    #[serde(default)]
    pub mime: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    pub members: Vec<MemberId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: MessageId,
    pub room: RoomId,
    pub sender: MemberId,
    pub sent_at: i64,
    pub body: String,
    #[serde(default)]
    pub reply_to: Option<MessageId>,
    #[serde(default)]
    pub attachments: Vec<ChatAttachment>,
    #[serde(default)]
    pub reactions: Vec<Reaction>,
    #[serde(default)]
    pub edited: bool,
    #[serde(default)]
    pub deleted: bool,
    pub delivery: Delivery,
}

impl ChatMessage {
    pub fn new(id: &str, room: &RoomId, sender: &MemberId, sent_at: i64, body: &str) -> ChatMessage {
        ChatMessage {
            id: MessageId::new(id),
            room: room.clone(),
            sender: sender.clone(),
            sent_at,
            body: body.to_string(),
            reply_to: None,
            attachments: Vec::new(),
            reactions: Vec::new(),
            edited: false,
            deleted: false,
            delivery: Delivery::Sent,
        }
    }

    pub fn is_local(&self) -> bool {
        self.id.0.starts_with(LOCAL_PREFIX)
    }

    pub fn reacted(&self, emoji: &str, who: &MemberId) -> bool {
        self.reactions.iter().any(|r| r.emoji == emoji && r.members.contains(who))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatDraft {
    pub body: String,
    #[serde(default)]
    pub reply_to: Option<MessageId>,
    #[serde(default)]
    pub attachments: Vec<ChatAttachment>,
}

impl ChatDraft {
    pub fn text(body: &str) -> ChatDraft {
        ChatDraft { body: body.to_string(), ..Default::default() }
    }

    pub fn is_empty(&self) -> bool {
        self.body.trim().is_empty() && self.attachments.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Room {
    pub id: RoomId,
    pub name: String,
    pub kind: RoomKind,
    pub members: Vec<Member>,
    #[serde(default)]
    pub topic: String,
    pub created_at: i64,
    #[serde(default)]
    pub unread: u32,
    #[serde(default)]
    pub last_message: Option<ChatMessage>,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub pinned: bool,
}

impl Room {
    pub fn activity(&self) -> i64 {
        self.last_message.as_ref().map_or(self.created_at, |m| m.sent_at)
    }

    pub fn member(&self, id: &MemberId) -> Option<&Member> {
        self.members.iter().find(|m| &m.id == id)
    }

    pub fn title(&self, me: &MemberId) -> String {
        if !self.name.trim().is_empty() {
            return self.name.clone();
        }
        let names: Vec<&str> = self.members.iter().filter(|m| &m.id != me).map(|m| m.display_name.as_str()).collect();
        match names.len() {
            0 => "Empty room".into(),
            1..=3 => names.join(", "),
            n => format!("{}, +{}", names[..3].join(", "), n - 3),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadMarker {
    pub room: RoomId,
    pub up_to: MessageId,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub edit: bool,
    pub delete: bool,
    pub reactions: bool,
    pub threads: bool,
    pub attachments: bool,
    pub typing: bool,
    pub presence: bool,
    pub read_receipts: bool,
    pub max_message_len: Option<usize>,
    pub paged_history: bool,
}

impl Capabilities {
    pub fn everything() -> Capabilities {
        Capabilities {
            edit: true,
            delete: true,
            reactions: true,
            threads: true,
            attachments: true,
            typing: true,
            presence: true,
            read_receipts: true,
            max_message_len: Some(4000),
            paged_history: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatEvent {
    NewMessage(ChatMessage),
    Edited { room: RoomId, id: MessageId, body: String },
    Deleted { room: RoomId, id: MessageId },
    Reaction { room: RoomId, id: MessageId, emoji: String, member: MemberId, added: bool },
    Typing { room: RoomId, member: MemberId, on: bool },
    Presence { member: MemberId, presence: Presence },
    RoomChanged(Room),
    RoomLeft(RoomId),
    MemberChanged { room: RoomId, member: Member, present: bool },
    Read { room: RoomId, member: MemberId, up_to: MessageId },
    Connection(bool),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ChatError {
    #[error("offline")]
    Offline,
    #[error("{0}")]
    Rejected(String),
    #[error("{0} is not supported by this backend")]
    Unsupported(&'static str),
    #[error("not found: {0}")]
    NotFound(String),
}

impl ChatError {
    pub fn retryable(&self) -> bool {
        matches!(self, ChatError::Offline)
    }
}

pub type ChatResult<T> = Result<T, ChatError>;
