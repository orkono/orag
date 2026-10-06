//! Pure domain logic. Nothing in this module performs I/O.

pub mod chunker;
pub mod citations;
pub mod document;
pub mod lexical_query;
pub mod normalize;
pub mod rrf;
pub mod space;

pub type CollectionId = i64;
pub type DocumentId = i64;
pub type JobId = i64;
pub type ChunkId = i64;
