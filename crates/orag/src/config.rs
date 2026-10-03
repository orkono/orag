//! Runtime configuration (D-019): `$ORAG_HOME/config.toml`, read once at
//! startup. It is never watched; edit it and restart `orag serve`.

use std::ffi::OsString;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{OragError, Result};

pub const CONFIG_FILE: &str = "config.toml";
pub const DB_FILE: &str = "orag.db";
pub const DEFAULT_PORT: u16 = 7613;
pub const DEFAULT_BIND: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_PORT);
pub const DEFAULT_MAX_DOCUMENT_MB: u32 = 5;
pub const MAX_DOCUMENT_MB_LIMIT: u32 = 10;
pub const DEFAULT_EMBEDDING_MODEL: &str = "qwen3-embedding-0.6b-q8_0";
pub const DEFAULT_GENERATION_MODEL: &str = "qwen3.5-4b-q4_k_m";
pub const DEFAULT_LOG_LEVEL: &str = "info";

/// Written on first start. Every key is commented out, so an install keeps
/// following the built-in defaults of whatever version runs it (e.g. a later
/// default of 10 MB) until the user uncomments a line to pin a value.
pub const DEFAULT_CONFIG_TOML: &str = r#"# ORAG configuration.
# Read once when `orag serve` starts. After editing, restart orag.
# Lines starting with '#' are defaults; remove the '#' to change a value.

# Loopback address the HTTP API listens on (127.0.0.1 or ::1 only).
# bind = "127.0.0.1:7613"

# Maximum size of one uploaded document, in MB (1-10).
# max_document_mb = 5

# Installed model pack ids (see `orag models list`).
# embedding_model = "qwen3-embedding-0.6b-q8_0"
# generation_model = "qwen3.5-4b-q4_k_m"

# Log verbosity on stderr: error, warn, info, debug or trace.
# log_level = "info"
"#;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    bind: Option<String>,
    max_document_mb: Option<u32>,
    embedding_model: Option<String>,
    generation_model: Option<String>,
    log_level: Option<String>,
}

const LOG_LEVELS: [&str; 5] = ["error", "warn", "info", "debug", "trace"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Config {
    #[serde(skip)]
    pub home: PathBuf,
    pub bind: SocketAddr,
    pub max_document_mb: u32,
    pub embedding_model: String,
    pub generation_model: String,
    pub log_level: String,
}

impl Config {
    /// Reads `home/config.toml`, creating `home` (private to the user) and the
    /// file with defaults if they do not exist. Every error names the path
    /// that failed, so users know where to look.
    pub fn load(home: &Path) -> Result<Config> {
        create_home(home).map_err(|err| at_path(err.into(), home))?;
        let path = home.join(CONFIG_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => return Err(at_path(err.into(), &path)),
        };
        Self::from_text_or_create(home, &path, text)
    }

    /// Parses `text`, or creates the default file when there is none. If another
    /// process created it first (two starts on a fresh home), that file is read.
    fn from_text_or_create(home: &Path, path: &Path, text: Option<String>) -> Result<Config> {
        let parse = || -> Result<Config> {
            let text = match text {
                Some(text) => text,
                None => match write_default(path) {
                    Ok(()) => DEFAULT_CONFIG_TOML.to_string(),
                    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                        std::fs::read_to_string(path)?
                    }
                    Err(err) => return Err(err.into()),
                },
            };
            let file: FileConfig =
                toml::from_str(&text).map_err(|err| OragError::InvalidInput(err.to_string()))?;
            Self::from_file(home, file)
        };
        parse().map_err(|err| at_path(err, path))
    }

    fn from_file(home: &Path, file: FileConfig) -> Result<Config> {
        let max_document_mb = file.max_document_mb.unwrap_or(DEFAULT_MAX_DOCUMENT_MB);
        if !(1..=MAX_DOCUMENT_MB_LIMIT).contains(&max_document_mb) {
            return Err(OragError::InvalidInput(format!(
                "max_document_mb = {max_document_mb}; it must be between 1 and {MAX_DOCUMENT_MB_LIMIT}"
            )));
        }
        let bind = match file.bind.as_deref() {
            Some(text) => parse_loopback_bind(text)?,
            None => DEFAULT_BIND,
        };
        Ok(Config {
            home: home.to_path_buf(),
            bind,
            max_document_mb,
            embedding_model: model_id(
                file.embedding_model,
                DEFAULT_EMBEDDING_MODEL,
                "embedding_model",
            )?,
            generation_model: model_id(
                file.generation_model,
                DEFAULT_GENERATION_MODEL,
                "generation_model",
            )?,
            log_level: log_level(file.log_level)?,
        })
    }

    pub fn max_document_bytes(&self) -> usize {
        self.max_document_mb as usize * 1024 * 1024
    }

    pub fn config_path(&self) -> PathBuf {
        self.home.join(CONFIG_FILE)
    }

    pub fn db_path(&self) -> PathBuf {
        self.home.join(DB_FILE)
    }

    pub fn models_dir(&self) -> PathBuf {
        self.home.join("models")
    }

    /// The settings in effect, as reported by `GET /v1/version`.
    pub fn effective(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("Config has only string and number fields")
    }
}

/// Prefixes I/O and validation errors with the path they concern.
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

/// Creates the home directory. It will hold every indexed document, so a new
/// home is private to the user (0700) on Unix. Missing parents are created
/// with normal permissions, and an existing home is left as it is.
fn create_home(home: &Path) -> std::io::Result<()> {
    if let Some(parent) = home.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    match builder.create(home) {
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists && home.is_dir() => Ok(()),
        result => result,
    }
}

/// Creates the default file. `create_new` fails on any existing path (a
/// dangling symlink included, so a link's target is never created), which
/// also makes two concurrent first starts safe: the loser reads the file.
/// The template is comments only, so even a partial write parses to the
/// defaults; a failed write is still removed so the next start rewrites it.
fn write_default(path: &Path) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let written = file
        .write_all(DEFAULT_CONFIG_TOML.as_bytes())
        .and_then(|()| file.sync_all());
    if written.is_err() {
        let _ = std::fs::remove_file(path); // best effort; `written` is reported
    }
    written
}

/// A model pack id names a directory under `models/`, so it must be a plain
/// name: lowercase ASCII letters, digits, `.`, `_`, `-`, starting with a
/// letter or digit (not `.` or `-`, so it is never hidden or read as a command
/// option). Lowercase only, so it resolves the same on case-insensitive
/// (macOS) and case-sensitive (Linux) filesystems.
fn model_id(value: Option<String>, default: &str, key: &str) -> Result<String> {
    let raw = value.unwrap_or_else(|| default.to_string());
    let id = raw.trim();
    let plain = id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    let first_ok = id
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if id.len() > 128 || !first_ok || !plain {
        return Err(OragError::InvalidInput(format!(
            "{key} = {raw:?} is not a model pack id (lowercase letters, digits, '.', '_', '-'; see `orag models list`)"
        )));
    }
    Ok(id.to_string())
}

fn log_level(value: Option<String>) -> Result<String> {
    let raw = value.unwrap_or_else(|| DEFAULT_LOG_LEVEL.to_string());
    let level = raw.trim().to_ascii_lowercase();
    if !LOG_LEVELS.contains(&level.as_str()) {
        return Err(OragError::InvalidInput(format!(
            "log_level = {raw:?}; it must be one of {}",
            LOG_LEVELS.join(", ")
        )));
    }
    Ok(level)
}

/// `ORAG_HOME`, else `~/.orag`. `env` is `std::env::var_os`-shaped, so a
/// non-UTF-8 `ORAG_HOME` is used as given rather than silently ignored.
pub fn resolve_home(env: &dyn Fn(&str) -> Option<OsString>) -> Result<PathBuf> {
    home_from(env, std::env::home_dir())
}

/// The home directory must be absolute: a relative one (or an unexpanded `~`)
/// would depend on the working directory orag was started from. A set but
/// empty `ORAG_HOME` is an error too (e.g. `ORAG_HOME=$UNSET_VAR`), not a
/// silent fallback to `~/.orag`.
fn home_from(
    env: &dyn Fn(&str) -> Option<OsString>,
    user_home: Option<PathBuf>,
) -> Result<PathBuf> {
    let (home, source) = match env("ORAG_HOME") {
        Some(home) if home.is_empty() => {
            return Err(OragError::InvalidInput(
                "ORAG_HOME is set but empty; unset it or give an absolute path".into(),
            ));
        }
        Some(home) => (PathBuf::from(home), "ORAG_HOME"),
        None => {
            let user_home = user_home.ok_or_else(|| {
                OragError::InvalidInput("cannot determine home directory; set ORAG_HOME".into())
            })?;
            (user_home.join(".orag"), "the home directory ($HOME)")
        }
    };
    if !home.is_absolute() {
        return Err(OragError::InvalidInput(format!(
            "{source} gives {:?}, which is not an absolute path; set ORAG_HOME to one",
            home.display().to_string()
        )));
    }
    Ok(home)
}

/// Parses `IP:PORT` and requires 127.0.0.1 or ::1 (D-013). Host names are not
/// resolved. Port 0 is allowed: the OS picks a free port and the server
/// reports the one in use (the desktop app starts orag this way).
pub fn parse_loopback_bind(text: &str) -> Result<SocketAddr> {
    let addr: SocketAddr = text
        .parse()
        .map_err(|_| OragError::InvalidInput(format!("bind address `{text}` is not IP:PORT")))?;
    if addr.ip() != IpAddr::V4(Ipv4Addr::LOCALHOST) && addr.ip() != IpAddr::V6(Ipv6Addr::LOCALHOST)
    {
        return Err(OragError::InvalidInput(format!(
            "bind address {addr} is not 127.0.0.1 or ::1; orag only serves loopback"
        )));
    }
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home_with(config: Option<&str>) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        if let Some(text) = config {
            std::fs::write(home.path().join(CONFIG_FILE), text).unwrap();
        }
        home
    }

    #[test]
    fn missing_file_is_created_with_documented_defaults() {
        let home = home_with(None);
        let cfg = Config::load(home.path()).unwrap();
        assert_eq!(cfg.bind, "127.0.0.1:7613".parse().unwrap());
        assert_eq!(cfg.max_document_mb, 5);
        assert_eq!(cfg.max_document_bytes(), 5 * 1024 * 1024);
        assert_eq!(cfg.embedding_model, DEFAULT_EMBEDDING_MODEL);
        assert_eq!(cfg.generation_model, DEFAULT_GENERATION_MODEL);
        let written = std::fs::read_to_string(home.path().join(CONFIG_FILE)).unwrap();
        assert!(written.contains("# max_document_mb = 5"), "{written}");
        assert!(
            written.contains("restart"),
            "the file must say changes need a restart"
        );
        assert_eq!(
            Config::load(home.path()).unwrap(),
            cfg,
            "the written file loads back identically"
        );
    }

    #[test]
    fn commented_defaults_document_the_real_defaults() {
        let uncommented: String = DEFAULT_CONFIG_TOML
            .lines()
            .filter_map(|line| line.strip_prefix("# "))
            .filter(|line| line.contains(" = "))
            .map(|line| format!("{line}\n"))
            .collect();
        let pinned = home_with(Some(&uncommented));
        let fresh = home_with(Some(""));
        let mut from_file = Config::load(pinned.path()).unwrap();
        from_file.home = fresh.path().to_path_buf();
        assert_eq!(
            from_file,
            Config::load(fresh.path()).unwrap(),
            "commented values must equal the code defaults"
        );
    }

    #[test]
    fn file_values_are_used() {
        let home = home_with(Some(
            "bind = \"127.0.0.1:9000\"\nmax_document_mb = 10\nembedding_model = \"e\"\ngeneration_model = \"g\"\n",
        ));
        let cfg = Config::load(home.path()).unwrap();
        assert_eq!((cfg.bind.port(), cfg.max_document_mb), (9000, 10));
        assert_eq!(
            (cfg.embedding_model.as_str(), cfg.generation_model.as_str()),
            ("e", "g")
        );
    }

    #[test]
    fn partial_file_falls_back_to_defaults_per_key() {
        let home = home_with(Some("generation_model = \"g\"\n"));
        let cfg = Config::load(home.path()).unwrap();
        assert_eq!(cfg.generation_model, "g");
        assert_eq!(cfg.max_document_mb, DEFAULT_MAX_DOCUMENT_MB);
    }

    #[test]
    fn document_limit_must_be_between_1_and_10_mb() {
        for bad in ["0", "11", "100"] {
            let home = home_with(Some(&format!("max_document_mb = {bad}\n")));
            let err = Config::load(home.path()).unwrap_err().to_string();
            assert!(err.contains("between 1 and 10"), "{bad}: {err}");
        }
    }

    #[test]
    fn unknown_keys_and_empty_model_names_are_rejected() {
        let typo = home_with(Some("max_document_size = 5\n"));
        assert!(matches!(
            Config::load(typo.path()),
            Err(OragError::InvalidInput(_))
        ));
        let empty = home_with(Some("generation_model = \"  \"\n"));
        assert!(matches!(
            Config::load(empty.path()),
            Err(OragError::InvalidInput(_))
        ));
        let level = home_with(Some("log_level = \"loud\"\n"));
        assert!(matches!(
            Config::load(level.path()),
            Err(OragError::InvalidInput(_))
        ));
    }

    #[test]
    fn every_invalid_value_names_the_config_file() {
        for text in [
            "bind = \"0.0.0.0:7613\"\n",
            "log_level = \"verbose\"\n",
            "embedding_model = \"\"\n",
            "max_document_mb = 0\n",
        ] {
            let home = home_with(Some(text));
            let err = Config::load(home.path()).unwrap_err().to_string();
            assert!(err.contains("config.toml"), "{text}: {err}");
        }
    }

    #[test]
    fn non_loopback_bind_is_rejected() {
        assert!(parse_loopback_bind("0.0.0.0:7613").is_err());
        assert!(parse_loopback_bind("192.168.1.5:7613").is_err());
        assert!(parse_loopback_bind("[::1]:7613").is_ok());
        assert!(parse_loopback_bind("127.0.0.1:0").is_ok());
        assert!(parse_loopback_bind("localhost:7613").is_err()); // names are not resolved
    }

    #[test]
    fn home_resolution_prefers_orag_home() {
        let env = |key: &str| (key == "ORAG_HOME").then(|| OsString::from("/tmp/x"));
        let user = Some(PathBuf::from("/home/u"));
        assert_eq!(
            home_from(&env, user.clone()).unwrap(),
            PathBuf::from("/tmp/x")
        );
        let none = |_: &str| None;
        assert_eq!(
            home_from(&none, user).unwrap(),
            PathBuf::from("/home/u/.orag")
        );
    }

    #[test]
    fn effective_config_lists_every_setting() {
        let home = home_with(None);
        let json = Config::load(home.path()).unwrap().effective();
        for key in [
            "bind",
            "max_document_mb",
            "embedding_model",
            "generation_model",
            "log_level",
        ] {
            assert!(json.get(key).is_some(), "{key} missing from {json}");
        }
    }

    #[test]
    fn io_errors_name_the_config_file() {
        let home = home_with(None);
        std::fs::create_dir(home.path().join(CONFIG_FILE)).unwrap();
        let err = Config::load(home.path()).unwrap_err().to_string();
        assert!(err.contains("config.toml"), "{err}");

        let latin1 = home_with(None);
        std::fs::write(latin1.path().join(CONFIG_FILE), b"# \xfe\n").unwrap();
        let err = Config::load(latin1.path()).unwrap_err().to_string();
        assert!(err.contains("config.toml"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_config_symlink_is_not_followed() {
        let home = home_with(None);
        let target = home.path().join("elsewhere.toml");
        std::os::unix::fs::symlink(&target, home.path().join(CONFIG_FILE)).unwrap();
        let err = Config::load(home.path()).unwrap_err().to_string();
        assert!(err.contains("config.toml"), "{err}");
        assert!(!target.exists(), "the symlink target must not be created");
    }

    #[test]
    fn model_ids_are_plain_pack_names() {
        for bad in [
            "../../etc",
            "/abs/path",
            "Qwen3-Embedding",
            "--help",
            "-rf",
            "a/b",
            ".hidden",
            "bad\\u0007id",
            "with space",
        ] {
            let home = home_with(Some(&format!("embedding_model = \"{bad}\"\n")));
            let err = Config::load(home.path()).unwrap_err().to_string();
            assert!(err.contains("embedding_model"), "{bad}: {err}");
        }
        let ok = home_with(Some("generation_model = \"qwen3.5-4b-q4_k_m\"\n"));
        assert!(Config::load(ok.path()).is_ok());
    }

    #[test]
    fn only_127_0_0_1_and_ipv6_loopback_are_accepted() {
        assert!(parse_loopback_bind("127.8.9.10:7613").is_err());
        assert!(parse_loopback_bind("127.0.0.1:7613").is_ok());
    }

    #[test]
    fn orag_home_must_be_absolute() {
        for bad in ["data", "~/orag", " /x", ""] {
            let env = |key: &str| (key == "ORAG_HOME").then(|| OsString::from(bad));
            let err = resolve_home(&env).unwrap_err().to_string();
            assert!(err.contains("ORAG_HOME"), "{bad}: {err}");
        }
    }

    #[test]
    fn log_level_is_trimmed_and_errors_show_the_value() {
        let home = home_with(Some("log_level = \" Debug \"\n"));
        assert_eq!(Config::load(home.path()).unwrap().log_level, "debug");
        let bad = home_with(Some("log_level = \"LOUD \"\n"));
        let err = Config::load(bad.path()).unwrap_err().to_string();
        assert!(err.contains("\"LOUD \""), "the value as written: {err}");
        let big = home_with(Some("max_document_mb = 42\n"));
        assert!(
            Config::load(big.path())
                .unwrap_err()
                .to_string()
                .contains("42")
        );
    }

    #[test]
    fn a_config_created_concurrently_is_read_not_rejected() {
        // Another process created the file between our read and our create.
        let home = home_with(Some("max_document_mb = 7\n"));
        let path = home.path().join(CONFIG_FILE);
        let cfg = Config::from_text_or_create(home.path(), &path, None).unwrap();
        assert_eq!(cfg.max_document_mb, 7);
    }

    #[test]
    fn the_default_file_is_never_left_partial() {
        let home = home_with(None);
        Config::load(home.path()).unwrap();
        let names: Vec<_> = std::fs::read_dir(home.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(
            names,
            vec![CONFIG_FILE.to_string()],
            "no temporary file is left behind"
        );
        assert_eq!(
            std::fs::read_to_string(home.path().join(CONFIG_FILE)).unwrap(),
            DEFAULT_CONFIG_TOML
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_new_home_is_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let parent = tempfile::tempdir().unwrap();
        let home = parent.path().join("nested/.orag");
        Config::load(&home).unwrap();
        let mode = std::fs::metadata(&home).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "{mode:o}");
    }

    #[test]
    fn a_home_that_cannot_be_created_is_named() {
        let parent = tempfile::tempdir().unwrap();
        let blocker = parent.path().join("file");
        std::fs::write(&blocker, "").unwrap();
        let home = blocker.join("orag");
        let err = Config::load(&home).unwrap_err().to_string();
        assert!(err.contains(&home.display().to_string()), "{err}");
        assert!(
            !err.contains(CONFIG_FILE),
            "the directory, not the file, failed: {err}"
        );
    }

    #[test]
    fn a_relative_home_directory_is_rejected() {
        let none = |_: &str| None;
        assert!(home_from(&none, Some(PathBuf::from("relative/dir"))).is_err());
        assert!(
            home_from(&none, Some(PathBuf::from("/home/u")))
                .unwrap()
                .ends_with(".orag")
        );
    }

    #[test]
    fn every_setting_is_documented_in_the_default_file() {
        let home = home_with(None);
        let json = Config::load(home.path()).unwrap().effective();
        for key in json.as_object().unwrap().keys() {
            assert!(
                DEFAULT_CONFIG_TOML.contains(&format!("\n# {key} = ")),
                "{key} is not in DEFAULT_CONFIG_TOML"
            );
        }
        assert!(DEFAULT_CONFIG_TOML.contains(&format!("# log_level = \"{DEFAULT_LOG_LEVEL}\"")));
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_orag_home_is_used_not_ignored() {
        use std::os::unix::ffi::OsStringExt;
        let raw = OsString::from_vec(b"/tmp/\xfe-orag".to_vec());
        let env = |key: &str| (key == "ORAG_HOME").then(|| raw.clone());
        assert_eq!(resolve_home(&env).unwrap(), PathBuf::from(raw.clone()));
    }

    #[test]
    fn the_default_file_states_the_real_limits() {
        assert!(DEFAULT_CONFIG_TOML.contains(&format!("(1-{MAX_DOCUMENT_MB_LIMIT})")));
        assert!(DEFAULT_CONFIG_TOML.contains(&format!("{DEFAULT_BIND}")));
        let listed = LOG_LEVELS[..LOG_LEVELS.len() - 1].join(", ");
        let levels = format!("{listed} or {}", LOG_LEVELS[LOG_LEVELS.len() - 1]);
        assert!(DEFAULT_CONFIG_TOML.contains(&levels), "{levels}");
    }

    #[test]
    fn model_id_errors_show_the_value_as_written() {
        let home = home_with(Some("embedding_model = \" Qwen3 \"\n"));
        let err = Config::load(home.path()).unwrap_err().to_string();
        assert!(err.contains("\" Qwen3 \""), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn missing_parents_keep_normal_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let parent = tempfile::tempdir().unwrap();
        let home = parent.path().join("shared/.orag");
        Config::load(&home).unwrap();
        let mode = std::fs::metadata(parent.path().join("shared"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_ne!(mode, 0o700, "only the home itself is private");
    }
}
