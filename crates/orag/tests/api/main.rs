//! HTTP API integration tests: real router, temp SQLite, fake models.
//! One test binary (`cargo test --test api`), split by area.

mod binary;
mod documents;
mod query;
mod support;
mod system;
mod ui;
