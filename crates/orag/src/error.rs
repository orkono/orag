//! Library error type. Variants map to HTTP status codes in `server::errors`.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum OragError {
    #[error("{kind} {id} not found")]
    NotFound { kind: &'static str, id: i64 },
    #[error("conflict: {0}")]
    Conflict(String),
    #[error(
        "collection {collection_id} was indexed with a different embedding model; reindex required"
    )]
    ReindexRequired { collection_id: i64 },
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("model error: {0}")]
    Model(String),
    #[error(
        "database schema version {found} is newer than this build supports ({supported}); upgrade orag"
    )]
    SchemaTooNew { found: u32, supported: u32 },
    /// Display carries the cause ("I/O error: <cause>") so `%err` in logs is
    /// diagnosable. There is no `source`, so `{err:#}` prints it once.
    #[error("I/O error: {0}")]
    Io(std::io::Error),
    /// Like `Io`: the cause is in Display and there is no `source`.
    #[error("storage error: {0}")]
    Storage(rusqlite::Error),
    /// A storage error while opening or migrating the database at `path`.
    #[error("database {path}: {cause}")]
    Database {
        path: String,
        cause: rusqlite::Error,
    },
    #[error("serialization error: {0}")]
    Serialization(serde_json::Error),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<std::io::Error> for OragError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<rusqlite::Error> for OragError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Storage(err)
    }
}

impl From<serde_json::Error> for OragError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization(err)
    }
}

pub type Result<T> = std::result::Result<T, OragError>;
