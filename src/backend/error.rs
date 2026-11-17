use std::fmt;

#[derive(Debug)]
pub enum BackendError {
    Unsupported(&'static str),
    Connection(String),
    Auth(String),
    Protocol(String),
    Store(String),
}

pub type Result<T> = std::result::Result<T, BackendError>;

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendError::Unsupported(op) => write!(f, "not supported by this backend: {op}"),
            BackendError::Connection(e) => write!(f, "connection failed: {e}"),
            BackendError::Auth(e) => write!(f, "authentication failed: {e}"),
            BackendError::Protocol(e) => write!(f, "protocol error: {e}"),
            BackendError::Store(e) => write!(f, "store error: {e}"),
        }
    }
}

impl std::error::Error for BackendError {}

impl From<imap::Error> for BackendError {
    fn from(e: imap::Error) -> Self {
        match e {
            imap::Error::Io(_) | imap::Error::ConnectionLost => {
                BackendError::Connection(e.to_string())
            }
            imap::Error::No(_) => BackendError::Protocol(e.to_string()),
            other => BackendError::Protocol(other.to_string()),
        }
    }
}

impl From<std::io::Error> for BackendError {
    fn from(e: std::io::Error) -> Self {
        BackendError::Connection(e.to_string())
    }
}
