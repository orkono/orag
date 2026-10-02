//! Pure domain logic. Nothing in this module performs I/O.

pub mod document;
pub mod normalize;

pub type CollectionId = i64;
pub type DocumentId = i64;
pub type JobId = i64;
pub type ChunkId = i64;
