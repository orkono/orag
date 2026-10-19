//! Keeps every download write inside the models folder. The paths of the
//! download subtree (`.download`, `.download/<id>`, the `.part` files) are
//! built from the canonical models folder and plain names only, and each is
//! checked without following links: a symbolic link or Windows reparse point
//! (junction) there is refused, so no append, truncate, rename or delete can
//! reach a file outside the models folder through it.
//!
//! The models folder itself may be a link (a user who keeps models on another
//! disk); it is resolved once, before anything is written.

use std::ffi::OsStr;
use std::fs::{File, Metadata, OpenOptions};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use crate::infer::download::fetch::disk_error;
use crate::infer::download::status::{DownloadError, ErrorCode};

/// The canonical models folder, created when missing.
pub fn anchor(models_dir: &Path) -> Result<PathBuf, DownloadError> {
    std::fs::create_dir_all(models_dir).map_err(|err| disk_error(models_dir, &err))?;
    std::fs::canonicalize(models_dir).map_err(|err| disk_error(models_dir, &err))
}

/// `parent/name`, where `name` must be one plain path component.
pub fn child(parent: &Path, name: &str) -> Result<PathBuf, DownloadError> {
    let mut parts = Path::new(name).components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(part)), None) if part == OsStr::new(name) => Ok(parent.join(part)),
        _ => Err(DownloadError::internal(format!(
            "`{name}` is not a plain file name"
        ))),
    }
}

/// Creates the folder `path` unless it is there, and refuses anything at
/// `path` that is not a real folder.
pub fn ensure_dir(path: &Path) -> Result<(), DownloadError> {
    match std::fs::create_dir(path) {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {}
        Err(err) => return Err(disk_error(path, &err)),
    }
    match existing(path)? {
        Some(meta) if meta.is_dir() => Ok(()),
        _ => Err(DownloadError::new(
            ErrorCode::Disk,
            format!(
                "{} is not a folder; move it out of the way and start again",
                path.display()
            ),
        )),
    }
}

/// What is at `path`, without following a link; `None` when nothing is.
/// A link is refused.
pub fn existing(path: &Path) -> Result<Option<Metadata>, DownloadError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if is_link(&meta) => Err(link_refused(path)),
        Ok(meta) => Ok(Some(meta)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(disk_error(path, &err)),
    }
}

/// Opens the file `path` with `options`, never through a link: one there
/// is refused before opening, and the open itself does not follow one where
/// the platform allows (unix `O_NOFOLLOW`, Windows `FILE_FLAG_OPEN_REPARSE_POINT`).
pub fn open_file(path: &Path, options: &mut OpenOptions) -> Result<File, DownloadError> {
    existing(path)?;
    no_follow(options);
    options.open(path).map_err(|err| {
        if opened_a_link(&err) {
            link_refused(path)
        } else {
            disk_error(path, &err)
        }
    })
}

fn link_refused(path: &Path) -> DownloadError {
    DownloadError::new(
        ErrorCode::Disk,
        format!(
            "{} is a symbolic link or junction; downloads are written only to real folders \
             and files inside the models folder, so it was left alone. Remove it and start again",
            path.display()
        ),
    )
}

fn is_link(meta: &Metadata) -> bool {
    meta.file_type().is_symlink() || is_reparse_point(meta)
}

#[cfg(windows)]
fn is_reparse_point(meta: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_meta: &Metadata) -> bool {
    false
}

#[cfg(unix)]
fn no_follow(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.custom_flags(libc::O_NOFOLLOW);
}

#[cfg(windows)]
fn no_follow(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
}

#[cfg(not(any(unix, windows)))]
fn no_follow(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn opened_a_link(err: &std::io::Error) -> bool {
    err.raw_os_error() == Some(libc::ELOOP)
}

#[cfg(not(unix))]
fn opened_a_link(_err: &std::io::Error) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_child_is_one_plain_name() {
        let parent = Path::new("models");
        assert_eq!(child(parent, "emb").unwrap(), parent.join("emb"));
        assert_eq!(
            child(parent, "model.gguf.part").unwrap(),
            parent.join("model.gguf.part")
        );
        for bad in ["", ".", "..", "a/b", "../x", "/abs", "emb/"] {
            assert_eq!(
                child(parent, bad).unwrap_err().code,
                ErrorCode::Internal,
                "{bad}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn opening_a_linked_file_is_refused_and_leaves_the_target_alone() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, b"keep").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let err = open_file(&link, OpenOptions::new().write(true).truncate(true)).unwrap_err();
        assert_eq!(err.code, ErrorCode::Disk);
        assert!(err.message.contains("symbolic link"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        let err = ensure_dir(&link).unwrap_err();
        assert!(err.message.contains("symbolic link"), "{err}");
    }
}
