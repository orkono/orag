//! The SHA-256 check of a finished `.part`, read without following a link
//! and stopped between chunks by a cancel.

use std::fs::OpenOptions;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

use crate::infer::download::fetch::host;
use crate::infer::download::guard;
use crate::infer::download::status::{DownloadError, ErrorCode};

const CHUNK_BYTES: usize = 1 << 20;

/// Hashes the finished `.part`; one that does not match is removed, so the
/// next start downloads it again instead of failing the same way.
pub fn verify(
    part: &Path,
    name: &str,
    url: &str,
    expected: &str,
    cancel: &AtomicBool,
) -> Result<(), DownloadError> {
    let actual = hash_part(part, cancel)?;
    if actual == expected {
        return Ok(());
    }
    let _ = std::fs::remove_file(part);
    Err(DownloadError::new(
        ErrorCode::Checksum,
        format!(
            "{name} from {} does not match its pinned SHA-256 (expected {expected}, got {actual}); \
             the file was removed, start again",
            host(url)
        ),
    ))
}

/// The hex SHA-256 of `part`; a cancel stops it before the next chunk.
fn hash_part(part: &Path, cancel: &AtomicBool) -> Result<String, DownloadError> {
    let mut file = guard::open_file(part, OpenOptions::new().read(true))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK_BYTES];
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(DownloadError::cancelled());
        }
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => {
                return Err(DownloadError::new(
                    ErrorCode::Disk,
                    format!("cannot read {}: {err}", part.display()),
                ));
            }
        };
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancel_stops_the_hash_and_keeps_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("model.gguf.part");
        std::fs::write(&part, b"weights").unwrap();
        let expected = hex::encode(Sha256::digest(b"weights"));
        verify(
            &part,
            "model.gguf",
            "http://h/x",
            &expected,
            &AtomicBool::new(false),
        )
        .unwrap();
        let err = verify(
            &part,
            "model.gguf",
            "http://h/x",
            &expected,
            &AtomicBool::new(true),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::Cancelled, "{err}");
        assert_eq!(std::fs::read(&part).unwrap(), b"weights");
    }
}
