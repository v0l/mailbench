pub mod audit;
pub mod compose;
pub mod http;
pub mod mail;
pub mod prefs;
pub mod rpc;
pub mod sanitize;
pub mod token;
pub mod tools;

pub use http::{Server, ServerConfig, bind_loopback, ensure_loopback};
pub use mail::{AccountRef, MailTools};

pub const DEFAULT_PORT: u16 = 7480;
pub const MAX_BODY: usize = 1_000_000;
