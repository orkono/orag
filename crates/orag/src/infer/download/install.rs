//! Verified `.part` files become an installed model, the way `orag models
//! import` installs one: a fresh staging folder renamed into place, so a
//! model folder is either complete or absent.

use std::path::{Path, PathBuf};

use crate::infer::download::catalog::CatalogEntry;
use crate::infer::download::fetch::{disk_error, host};
use crate::infer::download::status::{DownloadError, ErrorCode};
use crate::infer::models::{MANIFEST_FILE, create_staging_dir, remove_stale_staging, sha256_file};

/// The `.part` of `name` in a model's download folder.
pub fn part_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.part"))
}

/// Hashes the finished `.part`; one that does not match is removed, so the
/// next start downloads it again instead of failing the same way.
pub fn verify(part: &Path, name: &str, url: &str, expected: &str) -> Result<(), DownloadError> {
    let actual = sha256_file(part).map_err(|err| {
        DownloadError::new(
            ErrorCode::Disk,
            format!("cannot read {}: {err}", part.display()),
        )
    })?;
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

/// Moves the verified files of `entry` from `dir` into `models_dir/<id>/`
/// with the manifest the pack script writes. On failure the files go back to
/// `dir`, so nothing is downloaded twice.
pub fn install(models_dir: &Path, entry: &CatalogEntry, dir: &Path) -> Result<(), DownloadError> {
    let manifest = entry
        .manifest()
        .map_err(|err| DownloadError::internal(format!("catalog entry {}: {err}", entry.id)))?;
    let dest = models_dir.join(&entry.id);
    if dest.exists() {
        return Err(DownloadError::internal(format!(
            "{} already exists but is not a working install of `{}`; delete that folder and start again",
            dest.display(),
            entry.id
        )));
    }
    remove_stale_staging(models_dir);
    let staging = create_staging_dir(models_dir, &entry.id).map_err(|err| {
        DownloadError::new(ErrorCode::Disk, format!("{}: {err}", models_dir.display()))
    })?;
    let text = toml::to_string(&manifest)
        .map_err(|err| DownloadError::internal(format!("manifest of {}: {err}", entry.id)))?;
    let moved = move_files(entry, dir, &staging)
        .and_then(|()| write_file(&staging.join(MANIFEST_FILE), &text))
        .and_then(|()| rename(&staging, &dest));
    if moved.is_err() {
        put_back(entry, &staging, dir);
        let _ = std::fs::remove_dir_all(&staging);
        return moved;
    }
    // Best effort: an empty leftover folder only costs a directory entry.
    let _ = std::fs::remove_dir_all(dir);
    if let Some(parent) = dir.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    Ok(())
}

fn move_files(entry: &CatalogEntry, dir: &Path, staging: &Path) -> Result<(), DownloadError> {
    for file in entry.files() {
        rename(&part_path(dir, &file.name), &staging.join(&file.name))?;
    }
    Ok(())
}

fn put_back(entry: &CatalogEntry, staging: &Path, dir: &Path) {
    for file in entry.files() {
        let _ = std::fs::rename(staging.join(&file.name), part_path(dir, &file.name));
    }
}

fn rename(from: &Path, to: &Path) -> Result<(), DownloadError> {
    std::fs::rename(from, to).map_err(|err| {
        DownloadError::new(
            ErrorCode::Disk,
            format!("cannot move {} to {}: {err}", from.display(), to.display()),
        )
    })
}

fn write_file(path: &Path, text: &str) -> Result<(), DownloadError> {
    std::fs::write(path, text).map_err(|err| disk_error(path, &err))
}

/// Refuses to start when the platform says `folder` has less free space than
/// `needed`; a platform that cannot tell lets the download start.
pub fn check_free_space(folder: &Path, needed: u64) -> Result<(), DownloadError> {
    let Some(free) = free_bytes(folder) else {
        return Ok(());
    };
    if free >= needed {
        return Ok(());
    }
    Err(DownloadError::new(
        ErrorCode::Disk,
        format!(
            "{} has {} free, but the models need {} ({needed} bytes); free some space and start again",
            folder.display(),
            gigabytes(free),
            gigabytes(needed)
        ),
    ))
}

fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// Bytes the current user may still write on the file system of `folder`.
#[cfg(unix)]
pub fn free_bytes(folder: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(folder.as_os_str().as_bytes()).ok()?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is NUL-terminated and `stats` is writable; statvfs only
    // fills `stats` and reports failure through its return value.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: statvfs returned 0, so it initialised `stats`.
    let stats = unsafe { stats.assume_init() };
    #[allow(clippy::useless_conversion)] // the field types differ between platforms
    let (available, fragment) = (u64::from(stats.f_bavail), u64::from(stats.f_frsize));
    available.checked_mul(fragment)
}

/// Bytes the current user may still write on the volume of `folder`.
#[cfg(windows)]
pub fn free_bytes(folder: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = folder.as_os_str().encode_wide().chain([0]).collect();
    let mut available = 0u64;
    // SAFETY: `wide` is NUL-terminated; the null pointers are the optional
    // outputs this call does not need.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}

#[cfg(not(any(unix, windows)))]
pub fn free_bytes(_folder: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_space_is_known_here_and_a_shortfall_names_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let free = free_bytes(dir.path()).expect("this platform reports free space");
        assert!(free > 0);
        check_free_space(dir.path(), 1).unwrap();
        let err = check_free_space(dir.path(), u64::MAX).unwrap_err();
        assert_eq!(err.code, ErrorCode::Disk);
        assert!(
            err.message.contains(&dir.path().display().to_string()),
            "{}",
            err.message
        );
        assert!(
            err.message.contains(&u64::MAX.to_string()),
            "{}",
            err.message
        );
    }
}
