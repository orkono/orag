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

/// A page language and when it was chosen (ms since 1970, the page's clock),
/// or no time for the default before any choice was seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub language: &'static str,
    pub at: Option<u64>,
}

impl Default for Choice {
    /// Turkish, with no choice made.
    fn default() -> Self {
        Choice {
            language: LANGUAGES[0],
            at: None,
        }
    }
}

impl Choice {
    /// The choice that stands when a page shows `seen` and the app knows
    /// `current`: a timed choice beats an untimed one, the later time wins (a
    /// tie goes to `seen`), and an untimed `seen`, a page's default, changes
    /// nothing. The pages apply the same rule (i18n.js `chooseLanguage`).
    pub fn prefer(current: Choice, seen: Choice) -> Choice {
        match (current.at, seen.at) {
            (_, None) => current,
            (None, Some(_)) => seen,
            (Some(known), Some(shown)) if shown >= known => seen,
            _ => current,
        }
    }
}

/// The language last chosen on a page; None until one was seen. A missing
/// or broken time (the step 77 round 1 file had none) reads as no time.
pub fn read_language(dir: &Path) -> Option<Choice> {
    let text = read(dir, LANGUAGE_FILE)?;
    let mut parts = text.split_whitespace();
    let language = parse_language(parts.next()?)?;
    let at = parts.next().and_then(|at| at.parse::<u64>().ok());
    Some(Choice { language, at })
}

pub fn write_language(dir: &Path, choice: Choice) -> io::Result<()> {
    let text = match choice.at {
        Some(at) => format!("{} {at}\n", choice.language),
        None => format!("{}\n", choice.language),
    };
    std::fs::write(dir.join(LANGUAGE_FILE), text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(language: &'static str, at: Option<u64>) -> Choice {
        Choice { language, at }
    }

    #[test]
    fn nothing_is_remembered_on_a_first_start() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_ui_port(dir.path()), None);
        assert_eq!(read_language(dir.path()), None);
        assert_eq!(Choice::default(), choice("tr", None));
    }

    #[test]
    fn the_page_port_and_the_language_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        write_ui_port(dir.path(), 42017).unwrap();
        write_language(dir.path(), choice("en", Some(1_700_000_000_000))).unwrap();
        assert_eq!(read_ui_port(dir.path()), Some(42017));
        assert_eq!(
            read_language(dir.path()),
            Some(choice("en", Some(1_700_000_000_000)))
        );
        write_language(dir.path(), choice("tr", None)).unwrap();
        assert_eq!(read_language(dir.path()), Some(choice("tr", None)));
    }

    #[test]
    fn damaged_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        for (port, language) in [("0", "de"), ("70000", "EN 5"), ("x", ""), ("-1", "en-US")] {
            std::fs::write(dir.path().join(UI_PORT_FILE), port).unwrap();
            std::fs::write(dir.path().join(LANGUAGE_FILE), language).unwrap();
            assert_eq!(read_ui_port(dir.path()), None, "{port}");
            assert_eq!(read_language(dir.path()), None, "{language}");
        }
        std::fs::write(dir.path().join(UI_PORT_FILE), " 42017\n").unwrap();
        // The step 77 round 1 format (no time) and a broken time still read.
        std::fs::write(dir.path().join(LANGUAGE_FILE), "en\n").unwrap();
        assert_eq!(read_ui_port(dir.path()), Some(42017));
        assert_eq!(read_language(dir.path()), Some(choice("en", None)));
        std::fs::write(dir.path().join(LANGUAGE_FILE), "en soon\n").unwrap();
        assert_eq!(read_language(dir.path()), Some(choice("en", None)));
    }

    #[test]
    fn languages_are_parsed_strictly() {
        assert_eq!(parse_language("tr"), Some("tr"));
        assert_eq!(parse_language("xx"), None);
    }

    #[test]
    fn a_later_choice_wins_and_a_default_never_overrides_one() {
        let prefer = |current, seen| Choice::prefer(current, seen);
        // A stale choice the page still shows does not undo a newer one.
        assert_eq!(
            prefer(choice("en", Some(200)), choice("tr", Some(100))),
            choice("en", Some(200))
        );
        assert_eq!(
            prefer(choice("tr", Some(100)), choice("en", Some(200))),
            choice("en", Some(200))
        );
        assert_eq!(
            prefer(choice("tr", Some(200)), choice("en", Some(200))),
            choice("en", Some(200)),
            "a tie goes to what the page shows"
        );
        // A page showing the default (no choice made) changes nothing.
        assert_eq!(
            prefer(choice("en", Some(200)), choice("tr", None)),
            choice("en", Some(200))
        );
        assert_eq!(
            prefer(choice("en", None), choice("tr", None)),
            choice("en", None)
        );
        // A timed choice beats one remembered without a time.
        assert_eq!(
            prefer(choice("en", None), choice("tr", Some(5))),
            choice("tr", Some(5))
        );
    }
}
