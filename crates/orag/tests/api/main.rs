//! HTTP API integration tests: real router, temp SQLite, fake models.
//! One test binary (`cargo test --test api`), split by area.

mod binary;
mod cors;
mod documents;
mod download;
mod models;
mod query;
mod support;
mod system;
mod ui;
