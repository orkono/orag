//! Document ingestion: format detection, parsing, and the job worker.

pub mod docx;
pub mod format;
mod headings;
pub mod isolate;
pub mod layout;
pub mod parse;
pub mod pdf;

pub mod worker;
