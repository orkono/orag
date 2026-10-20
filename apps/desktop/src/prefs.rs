//! What the app remembers between launches, in its data folder: the service
//! page's port (so the page keeps its origin, and with it the browser storage
//! the page keeps: selected collection, help seen, language) and the language
//! last chosen on either of its pages.

use std::io;
use std::path::Path;

pub const UI_PORT_FILE: &str = "ui-port";
pub const LANGUAGE_FILE: &str = "language";
/// The pages' languages; the pages open in Turkish unless English was chosen.
pub const LANGUAGES: [&str; 2] = ["tr", "en"];

/// `text` as one of [`LANGUAGES`], or None.
pub fn parse_language(text: &str) -> Option<&'static str> {
    LANGUAGES.into_iter().find(|&lang| lang == text)
}

fn read(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name))
        .ok()
        .map(|text| text.trim().to_string())
}

/// The service page's last port; None on a first start or a damaged file.
pub fn read_ui_port(dir: &Path) -> Option<u16> {
    read(dir, UI_PORT_FILE)?
        .parse::<u16>()
        .ok()
        .filter(|&port| port != 0)
}

pub fn write_ui_port(dir: &Path, port: u16) -> io::Result<()> {
    std::fs::write(dir.join(UI_PORT_FILE), format!("{port}\n"))
}

/// The language last chosen on a page; None until one was seen.
pub fn read_language(dir: &Path) -> Option<&'static str> {
    parse_language(&read(dir, LANGUAGE_FILE)?)
}

pub fn write_language(dir: &Path, language: &str) -> io::Result<()> {
    let language = parse_language(language).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unknown language {language:?}"),
        )
    })?;
    std::fs::write(dir.join(LANGUAGE_FILE), format!("{language}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_remembered_on_a_first_start() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_ui_port(dir.path()), None);
        assert_eq!(read_language(dir.path()), None);
    }

    #[test]
    fn the_page_port_and_the_language_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        write_ui_port(dir.path(), 42017).unwrap();
        write_language(dir.path(), "en").unwrap();
        assert_eq!(read_ui_port(dir.path()), Some(42017));
        assert_eq!(read_language(dir.path()), Some("en"));
        write_language(dir.path(), "tr").unwrap();
        assert_eq!(read_language(dir.path()), Some("tr"));
    }

    #[test]
    fn damaged_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        for (port, language) in [("0", "de"), ("70000", "EN"), ("x", ""), ("-1", "en-US")] {
            std::fs::write(dir.path().join(UI_PORT_FILE), port).unwrap();
            std::fs::write(dir.path().join(LANGUAGE_FILE), language).unwrap();
            assert_eq!(read_ui_port(dir.path()), None, "{port}");
            assert_eq!(read_language(dir.path()), None, "{language}");
        }
        std::fs::write(dir.path().join(UI_PORT_FILE), " 42017\n").unwrap();
        std::fs::write(dir.path().join(LANGUAGE_FILE), "en\n").unwrap();
        assert_eq!(read_ui_port(dir.path()), Some(42017));
        assert_eq!(read_language(dir.path()), Some("en"));
    }

    #[test]
    fn only_known_languages_are_written() {
        let dir = tempfile::tempdir().unwrap();
        assert!(write_language(dir.path(), "de").is_err());
        assert_eq!(read_language(dir.path()), None);
        assert_eq!(parse_language("tr"), Some("tr"));
        assert_eq!(parse_language("xx"), None);
    }
}
