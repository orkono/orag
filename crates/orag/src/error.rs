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
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<std::io::Error> for OragError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

pub type Result<T> = std::result::Result<T, OragError>;
