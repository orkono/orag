//! The one edit ORAG makes to `config.toml` itself (step 82): after the
//! setup page downloaded and installed the generator the user chose, its id
//! becomes `generation_model`. Only that key changes; comments, order and
//! every other line stay as the user left them (toml_edit). The new file is
//! checked like a start reads it, then written atomically: a temporary file
//! in the same folder, fsync, rename over the old one (replace semantics on
//! Windows). On any failure the old file is untouched. A config.toml that is
//! a symbolic link (to a managed file) stays a link: its target is the file
//! replaced. The file's line endings (LF or CRLF) are kept.

use std::io::Write;
use std::path::Path;

use toml_edit::{DocumentMut, Item, value};

use crate::config::{Config, DEFAULT_CONFIG_TOML};
use crate::error::{OragError, Result};

const KEY: &str = "generation_model";

/// Sets `generation_model = "<id>"` in the config file at `path` (created
/// from the default template when missing). Returns whether the file was
/// written: `false` when the key already names `id`.
pub fn set_generation_model(path: &Path, id: &str) -> Result<bool> {
    let at = |err: OragError| at_path(err, path);
    let file = write_target(path).map_err(|err| at(err.into()))?;
    let old = match std::fs::read_to_string(&file) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(at(err.into())),
    };
    let Some(text) = edited(old.as_deref().unwrap_or(DEFAULT_CONFIG_TOML), id).map_err(at)? else {
        return Ok(false);
    };
    let home = path.parent().unwrap_or(Path::new("."));
    let config = Config::from_text(home, &text).map_err(at)?;
    if config.generation_model != id {
        return Err(at(OragError::Internal(format!(
            "the edited file names `{}`, not `{id}`",
            config.generation_model
        ))));
    }
    write_atomically(&file, &text).map_err(|err| at(err.into()))?;
    Ok(true)
}

/// The file to replace: `path` itself, or the file a symbolic link at
/// `path` points to (resolved fully, so a chain of links or a linked folder
/// on Windows ends at the real file). Replacing the link instead would cut
/// the managed file off. A link that points nowhere is an error.
fn write_target(path: &Path) -> std::io::Result<std::path::PathBuf> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::canonicalize(path).map_err(|err| {
            std::io::Error::new(
                err.kind(),
                format!("config.toml is a link that cannot be followed: {err}"),
            )
        }),
        _ => Ok(path.to_path_buf()),
    }
}

/// `out` (toml_edit writes LF) in the line ending style of `original`:
/// CRLF when most of its line breaks are CRLF.
fn with_line_endings(out: String, original: &str) -> String {
    let crlf = original.matches("\r\n").count();
    let lf = original.matches('\n').count() - crlf;
    if crlf == 0 || crlf < lf {
        return out;
    }
    out.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// `text` with the key set to `id`, or `None` when it already is.
fn edited(text: &str, id: &str) -> Result<Option<String>> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|err: toml_edit::TomlError| OragError::InvalidInput(err.to_string()))?;
    let current = doc.get(KEY);
    if current.and_then(Item::as_str) == Some(id) {
        return Ok(None);
    }
    let mut new = value(id);
    // A replaced value keeps its own spacing and trailing comment.
    if let (Some(old), Some(fresh)) = (current.and_then(Item::as_value), new.as_value_mut()) {
        *fresh.decor_mut() = old.decor().clone();
    }
    // Replaced in place, so the key keeps its own leading comments and lines.
    if let Some(item) = doc.get_mut(KEY) {
        *item = new;
        return Ok(Some(with_line_endings(doc.to_string(), text)));
    }
    if let Some(edited) = below_commented_default(text, id) {
        return Ok(Some(edited));
    }
    doc.insert(KEY, new);
    Ok(Some(with_line_endings(doc.to_string(), text)))
}

/// A missing key goes right under its commented-out default
/// (`# generation_model = "…"`, as the template has it) when that line is in
/// the top-level part of the file; the caller checks the result loads.
fn below_commented_default(text: &str, id: &str) -> Option<String> {
    let top_level = text
        .split_inclusive('\n')
        .take_while(|line| !line.trim_start().starts_with('['));
    let mut at = 0;
    for line in top_level {
        at += line.len();
        let commented = line.trim_start().strip_prefix('#').map(str::trim_start);
        if commented.is_some_and(|rest| rest.starts_with(&format!("{KEY} ="))) {
            // The file's own line ending (CRLF on a file edited on Windows).
            let eol = if line.ends_with("\r\n") { "\r\n" } else { "\n" };
            let lead = if line.ends_with('\n') { "" } else { eol };
            let quoted = value(id).as_value()?.to_string();
            let added = format!("{lead}{KEY} = {}{eol}", quoted.trim());
            return Some(format!("{}{added}{}", &text[..at], &text[at..]));
        }
    }
    None
}

/// Writes `text` to a temporary file next to `path`, flushes it to disk and
/// renames it over `path`; the old file stays until the rename.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut file = tempfile::Builder::new()
        .prefix(".config.toml.")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    file.write_all(text.as_bytes())?;
    if let Ok(meta) = std::fs::metadata(path) {
        file.as_file().set_permissions(meta.permissions())?;
    }
    file.as_file().sync_all()?;
    file.persist(path).map_err(|err| err.error)?;
    sync_dir(dir);
    Ok(())
}

/// Makes the rename itself durable (best effort; not possible on Windows).
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(dir) = std::fs::File::open(dir) {
        let _ = dir.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

fn at_path(err: OragError, path: &Path) -> OragError {
    match err {
        OragError::Io(err) => OragError::Io(std::io::Error::new(
            err.kind(),
            format!("{}: {err}", path.display()),
        )),
        OragError::InvalidInput(message) => {
            OragError::InvalidInput(format!("{}: {message}", path.display()))
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CONFIG_FILE;

    const LARGE: &str = "qwen3.5-9b-q4_k_m";

    fn home() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE);
        (dir, path)
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn the_key_is_replaced_and_everything_else_kept() {
        let (_dir, path) = home();
        let before = "# My ORAG.\nbind = \"127.0.0.1:7700\" # my port\n\n\
                      generation_model = \"qwen3.5-4b-q4_k_m\"   # chosen by hand\n\
                      log_level = \"debug\"\n\n# Downloads.\n[models]\ndownload = true\n";
        std::fs::write(&path, before).unwrap();
        assert!(set_generation_model(&path, LARGE).unwrap());
        assert_eq!(
            read(&path),
            before.replace("qwen3.5-4b-q4_k_m", LARGE),
            "only the value changed"
        );
        let config = Config::load(path.parent().unwrap()).unwrap();
        assert_eq!(config.generation_model, LARGE);
        assert_eq!(config.log_level, "debug");
    }

    #[test]
    fn a_missing_key_is_added_before_the_first_table() {
        let (_dir, path) = home();
        let before = "# Mine.\nlog_level = \"warn\"\n\n[models]\ndownload = true\n";
        std::fs::write(&path, before).unwrap();
        assert!(set_generation_model(&path, LARGE).unwrap());
        let after = read(&path);
        assert!(
            after.starts_with("# Mine.\nlog_level = \"warn\"\n"),
            "{after}"
        );
        assert!(after.contains("[models]\ndownload = true\n"), "{after}");
        let at = after.find("generation_model").unwrap();
        assert!(at < after.find("[models]").unwrap(), "{after}");
        let config = Config::load(path.parent().unwrap()).unwrap();
        assert_eq!(
            (config.generation_model.as_str(), config.models.download),
            (LARGE, true)
        );
    }

    #[test]
    fn the_default_file_keeps_its_comments_and_a_missing_file_is_created() {
        let (_dir, path) = home();
        assert!(set_generation_model(&path, LARGE).unwrap());
        let created = read(&path);
        // The template with one line added under the commented default.
        let commented = "# generation_model = \"qwen3.5-4b-q4_k_m\"\n";
        assert_eq!(
            created,
            DEFAULT_CONFIG_TOML.replace(
                commented,
                &format!("{commented}generation_model = \"{LARGE}\"\n")
            )
        );
        assert_eq!(
            Config::load(path.parent().unwrap())
                .unwrap()
                .generation_model,
            LARGE
        );
    }

    #[test]
    fn a_windows_file_keeps_its_crlf_line_endings() {
        let (_dir, path) = home();
        let before = DEFAULT_CONFIG_TOML.replace('\n', "\r\n");
        std::fs::write(&path, &before).unwrap();
        assert!(set_generation_model(&path, LARGE).unwrap());
        let after = read(&path);
        assert!(
            after.contains(&format!(
                "# generation_model = \"qwen3.5-4b-q4_k_m\"\r\ngeneration_model = \"{LARGE}\"\r\n"
            )),
            "{after:?}"
        );
        assert_eq!(after.matches('\n').count(), after.matches("\r\n").count());
        // A commented line inside a table is not the top-level default.
        let (_dir, path) = home();
        std::fs::write(&path, "[models]\n# generation_model = \"x\"\n").unwrap();
        assert!(set_generation_model(&path, LARGE).unwrap());
        let config = Config::load(path.parent().unwrap()).unwrap();
        assert_eq!(config.generation_model, LARGE);
    }

    /// Every line break of `text` is CRLF.
    fn all_crlf(text: &str) -> bool {
        text.matches('\n').count() == text.matches("\r\n").count()
    }

    #[test]
    fn updating_a_key_keeps_crlf_line_endings() {
        let (_dir, path) = home();
        let before = "# mine\r\ngeneration_model = \"qwen3.5-4b-q4_k_m\" # hand\r\n\r\n[models]\r\ndownload = true\r\n";
        std::fs::write(&path, before).unwrap();
        assert!(set_generation_model(&path, LARGE).unwrap());
        assert_eq!(read(&path), before.replace("qwen3.5-4b-q4_k_m", LARGE));
    }

    #[test]
    fn inserting_without_a_commented_default_keeps_crlf_line_endings() {
        let (_dir, path) = home();
        std::fs::write(
            &path,
            "# mine\r\nlog_level = \"warn\"\r\n\r\n[models]\r\ndownload = true\r\n",
        )
        .unwrap();
        assert!(set_generation_model(&path, LARGE).unwrap());
        let after = read(&path);
        assert!(all_crlf(&after), "{after:?}");
        assert!(
            after.contains(&format!("generation_model = \"{LARGE}\"\r\n")),
            "{after:?}"
        );
        assert_eq!(
            Config::load(path.parent().unwrap())
                .unwrap()
                .generation_model,
            LARGE
        );
    }

    #[test]
    fn a_created_file_uses_the_template_line_endings() {
        let (_dir, path) = home();
        assert!(set_generation_model(&path, LARGE).unwrap());
        let created = read(&path);
        assert!(!created.contains('\r'), "LF like the template: {created:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_config_is_edited_through_the_link() {
        let (dir, path) = home();
        let managed = dir.path().join("managed").join("managed.toml");
        std::fs::create_dir(managed.parent().unwrap()).unwrap();
        std::fs::write(&managed, "# managed\nlog_level = \"warn\"\n").unwrap();
        std::os::unix::fs::symlink(&managed, &path).unwrap();
        assert!(set_generation_model(&path, LARGE).unwrap());
        assert!(
            std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the link stays a link"
        );
        assert_eq!(std::fs::read_link(&path).unwrap(), managed);
        let target = read(&managed);
        assert_eq!(
            target,
            format!("# managed\nlog_level = \"warn\"\ngeneration_model = \"{LARGE}\"\n")
        );
        // The temporary file was made next to the target, and is gone.
        let beside: Vec<_> = std::fs::read_dir(managed.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(beside, ["managed.toml"]);
        assert!(
            !set_generation_model(&path, LARGE).unwrap(),
            "unchanged now"
        );
    }

    #[test]
    fn nothing_is_written_when_the_key_already_names_the_model() {
        let (_dir, path) = home();
        let before = format!("generation_model = \"{LARGE}\" # kept\n");
        std::fs::write(&path, &before).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert!(!set_generation_model(&path, LARGE).unwrap());
        assert_eq!(read(&path), before);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
    }

    #[test]
    fn a_file_that_would_not_load_is_left_byte_identical() {
        let (_dir, path) = home();
        // Valid TOML, but not a valid ORAG config: the edit must not "fix" it.
        let before = b"max_document_mb = 99\n# keep\n".to_vec();
        std::fs::write(&path, &before).unwrap();
        let err = set_generation_model(&path, LARGE).unwrap_err();
        assert!(err.to_string().contains("max_document_mb"), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        // Not TOML at all.
        let broken = b"generation_model = \"unterminated\n".to_vec();
        std::fs::write(&path, &broken).unwrap();
        assert!(set_generation_model(&path, LARGE).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), broken);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_write_leaves_the_old_file_byte_identical_and_no_temp_file() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, path) = home();
        let before = b"# mine\ngeneration_model = \"qwen3.5-4b-q4_k_m\"\n".to_vec();
        std::fs::write(&path, &before).unwrap();
        // A folder the temporary file cannot be created in.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = set_generation_model(&path, LARGE);
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        if result.is_ok() {
            return; // running as root: permissions do not apply
        }
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains(&path.display().to_string()),
            "{err}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [CONFIG_FILE]);
    }

    #[cfg(unix)]
    #[test]
    fn the_written_file_keeps_its_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, path) = home();
        std::fs::write(&path, "log_level = \"info\"\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(set_generation_model(&path, LARGE).unwrap());
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }
}
