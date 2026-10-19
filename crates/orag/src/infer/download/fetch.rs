//! One file over HTTP into `<name>.part`, resumed with a Range request.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::infer::download::catalog::RemoteFile;
use crate::infer::download::guard;
use crate::infer::download::status::{DownloadError, ErrorCode};

const CHUNK_BYTES: usize = 64 * 1024;
/// A Range the server answers with the wrong bytes or 416 restarts once.
const ATTEMPTS: usize = 2;

/// What `fetch` needs to bring one file to disk.
pub struct Fetch<'a> {
    pub agent: &'a ureq::Agent,
    /// Where the bytes come from (the catalog URL or its test rewrite).
    pub url: &'a str,
    pub file: &'a RemoteFile,
    pub part: &'a Path,
    pub cancel: &'a AtomicBool,
    /// Called with the bytes of the file on disk so far.
    pub progress: &'a dyn Fn(u64),
}

/// The host of `url`, for messages.
pub fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?']).next().unwrap_or(rest)
}

/// Brings `part` to exactly `file.size` bytes, resuming what is there. The
/// bytes are not checked here: the caller hashes the finished `.part`.
pub fn fetch(job: &Fetch) -> Result<(), DownloadError> {
    let mut have = part_len(job)?;
    for _ in 0..ATTEMPTS {
        (job.progress)(have);
        if have == job.file.size {
            return Ok(());
        }
        let response = request(job, have)?;
        let status = response.status().as_u16();
        let resumed = match status {
            206 if content_range_start(&response) == Some(have) => true,
            200 => false,
            // A range from elsewhere or one the server refuses: start over.
            206 | 416 => {
                have = restart(job)?;
                continue;
            }
            _ => return Err(http_error(job, status)),
        };
        let mut output = open_part(job, resumed)?;
        have = if resumed { have } else { 0 };
        copy_body(job, response.into_body().into_reader(), &mut output, have)?;
        return Ok(());
    }
    Err(DownloadError::new(
        ErrorCode::Http,
        format!(
            "{} did not send {} from where the earlier download stopped",
            host(job.url),
            job.file.name
        ),
    ))
}

/// The length of the `.part` left by an earlier run; one longer than the file
/// cannot be a prefix of it and is removed. A link there is refused.
fn part_len(job: &Fetch) -> Result<u64, DownloadError> {
    let len = guard::existing(job.part)?.map_or(0, |meta| meta.len());
    if len > job.file.size {
        return restart(job);
    }
    Ok(len)
}

fn restart(job: &Fetch) -> Result<u64, DownloadError> {
    match std::fs::remove_file(job.part) {
        Ok(()) => Ok(0),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(disk_error(job.part, &err)),
    }
}

fn request(job: &Fetch, have: u64) -> Result<ureq::http::Response<ureq::Body>, DownloadError> {
    let mut request = job.agent.get(job.url);
    if have > 0 {
        request = request.header("Range", format!("bytes={have}-"));
    }
    request.call().map_err(|err| {
        // A cancel ends the call with an I/O error from the watched socket.
        if job.cancel.load(Ordering::SeqCst) {
            return DownloadError::cancelled();
        }
        network_error(job, &describe(&err))
    })
}

fn describe(err: &ureq::Error) -> String {
    match err {
        ureq::Error::Timeout(_) => "the server stopped answering (timed out)".to_string(),
        other => other.to_string(),
    }
}

/// `bytes 100-199/200` → 100.
fn content_range_start(response: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    let value = response.headers().get("content-range")?.to_str().ok()?;
    let range = value.strip_prefix("bytes ")?;
    range.split('-').next()?.trim().parse().ok()
}

fn open_part(job: &Fetch, append: bool) -> Result<File, DownloadError> {
    let mut options = OpenOptions::new();
    options.create(true);
    if append {
        options.append(true);
    } else {
        options.write(true).truncate(true);
    }
    guard::open_file(job.part, &mut options)
}

/// Appends the body to `output` chunk by chunk, checking for cancellation
/// between chunks; a read waiting on the network wakes for a cancel too (see
/// `http::CANCEL_POLL`). A cut connection keeps the `.part` for the next start.
fn copy_body(
    job: &Fetch,
    mut body: impl Read,
    output: &mut File,
    mut have: u64,
) -> Result<(), DownloadError> {
    let mut buffer = vec![0u8; CHUNK_BYTES];
    loop {
        if job.cancel.load(Ordering::SeqCst) {
            return Err(DownloadError::cancelled());
        }
        let read = match body.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(_) if job.cancel.load(Ordering::SeqCst) => {
                return Err(DownloadError::cancelled());
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) if is_timeout(&err) => {
                return Err(network_error(
                    job,
                    "no data arrived for too long (timed out)",
                ));
            }
            Err(err) => return Err(network_error(job, &err.to_string())),
        };
        have += read as u64;
        if have > job.file.size {
            let _ = std::fs::remove_file(job.part);
            return Err(too_long(job));
        }
        output
            .write_all(&buffer[..read])
            .map_err(|err| disk_error(job.part, &err))?;
        (job.progress)(have);
    }
    output
        .sync_all()
        .map_err(|err| disk_error(job.part, &err))?;
    if have < job.file.size {
        return Err(network_error(
            job,
            &format!(
                "the connection closed after {have} of {} bytes",
                job.file.size
            ),
        ));
    }
    Ok(())
}

/// A read that hit the idle limit: a socket timeout, or ureq's own timeout
/// wrapped in the I/O error its body reader returns (the TLS path).
fn is_timeout(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::TimedOut
        || err
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<ureq::Error>())
            .is_some_and(|inner| matches!(inner, ureq::Error::Timeout(_)))
}

fn too_long(job: &Fetch) -> DownloadError {
    DownloadError::new(
        ErrorCode::Checksum,
        format!(
            "{} from {} is larger than its pinned {} bytes; the partial file was removed, start again",
            job.file.name,
            host(job.url),
            job.file.size
        ),
    )
}

fn http_error(job: &Fetch, status: u16) -> DownloadError {
    DownloadError::new(
        ErrorCode::Http,
        format!(
            "{} answered HTTP {status} for {}",
            host(job.url),
            job.file.name
        ),
    )
}

fn network_error(job: &Fetch, cause: &str) -> DownloadError {
    DownloadError::new(
        ErrorCode::Network,
        format!(
            "downloading {} from {} failed: {cause}; check the connection and start again to resume",
            job.file.name,
            host(job.url)
        ),
    )
}

pub fn disk_error(path: &Path, err: &std::io::Error) -> DownloadError {
    DownloadError::new(
        ErrorCode::Disk,
        format!("cannot write {}: {err}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_is_the_authority_of_the_url() {
        assert_eq!(host("https://huggingface.co/a/b"), "huggingface.co");
        assert_eq!(host("http://127.0.0.1:9/x?y"), "127.0.0.1:9");
        assert_eq!(host("https://www.apache.org"), "www.apache.org");
    }
}
