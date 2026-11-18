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

impl From<async_imap::error::Error> for BackendError {
    fn from(e: async_imap::error::Error) -> Self {
        use async_imap::error::Error;
        match e {
            Error::Io(_) | Error::ConnectionLost => BackendError::Connection(e.to_string()),
            other => BackendError::Protocol(other.to_string()),
        }
    }
}

impl From<std::io::Error> for BackendError {
    fn from(e: std::io::Error) -> Self {
        BackendError::Connection(e.to_string())
    }
}

impl From<native_tls::Error> for BackendError {
    fn from(e: native_tls::Error) -> Self {
        BackendError::Connection(format!("tls: {e}"))
    }
}

impl From<tokio::time::error::Elapsed> for BackendError {
    fn from(_: tokio::time::error::Elapsed) -> Self {
        BackendError::Connection("timed out".into())
    }
}

impl From<crate::store::StoreError> for BackendError {
    fn from(e: crate::store::StoreError) -> Self {
        BackendError::Store(e.to_string())
    }
}
