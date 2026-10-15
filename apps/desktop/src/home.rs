//! The app's private ORAG home: `<app data dir>/orag`, never a CLI home.

use std::io;
use std::path::{Path, PathBuf};

/// Written once; later starts keep the user's edits.
pub const DEFAULT_CONFIG: &str = "\
# ORAG desktop: the app starts the service on free loopback ports and shows
# its page. Other settings: see docs/configuration.md in the ORAG repository.
bind = \"127.0.0.1:0\"
ui_bind = \"127.0.0.1:0\"
";

/// Creates `<app_data>/orag` and its `config.toml` if missing; returns the home.
pub fn ensure_home(app_data: &Path) -> io::Result<PathBuf> {
    let home = app_data.join("orag");
    std::fs::create_dir_all(&home)?;
    let config = home.join("config.toml");
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&config)
    {
        Ok(mut file) => io::Write::write_all(&mut file, DEFAULT_CONFIG.as_bytes())?,
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(err),
    }
    Ok(home)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_start_writes_loopback_ephemeral_ports() {
        let dir = tempfile::tempdir().unwrap();
        let home = ensure_home(dir.path()).unwrap();
        assert_eq!(home, dir.path().join("orag"));
        let text = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(text.contains("bind = \"127.0.0.1:0\""));
        assert!(text.contains("ui_bind = \"127.0.0.1:0\""));
    }

    #[test]
    fn later_starts_keep_the_users_config() {
        let dir = tempfile::tempdir().unwrap();
        let home = ensure_home(dir.path()).unwrap();
        std::fs::write(home.join("config.toml"), "log_level = \"debug\"\n").unwrap();
        ensure_home(dir.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(home.join("config.toml")).unwrap(),
            "log_level = \"debug\"\n"
        );
    }
}
