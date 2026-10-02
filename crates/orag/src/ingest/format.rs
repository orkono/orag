//! Source format detection from file name and content type.

use std::path::Path;

use crate::error::{OragError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    PlainText,
    Markdown,
}

impl SourceFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceFormat::PlainText => "text",
            SourceFormat::Markdown => "markdown",
        }
    }

    /// Parses the stored/JSON name produced by `as_str`.
    pub fn from_name(name: &str) -> Result<SourceFormat> {
        match name {
            "text" => Ok(SourceFormat::PlainText),
            "markdown" => Ok(SourceFormat::Markdown),
            other => Err(OragError::UnsupportedFormat(format!(
                "format `{other}` (supported: text, markdown)"
            ))),
        }
    }

    /// Format of an uploaded file. A supported extension decides; a known
    /// non-text type is rejected; any other extension (or none, `notes.v2`)
    /// falls back to the content type. A text or Markdown content type fits a
    /// supported extension (browsers and `curl` send `text/plain` or
    /// `application/octet-stream` for anything), but a non-text one that
    /// contradicts it is rejected, so a PDF named `x.md` fails as unsupported
    /// rather than as invalid UTF-8.
    pub fn detect(filename: Option<&str>, content_type: Option<&str>) -> Result<SourceFormat> {
        let mime = content_type
            .map(|ct| {
                ct.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
            })
            .filter(|mime| !mime.is_empty());
        let ext = filename.and_then(extension);
        let fail = || unsupported(mime.as_deref(), ext.as_deref());
        if ext
            .as_deref()
            .is_some_and(|e| REJECTED_EXTENSIONS.contains(&e))
        {
            return Err(fail());
        }
        let by_ext = match ext.as_deref() {
            Some("md" | "markdown") => Some(SourceFormat::Markdown),
            Some("txt") => Some(SourceFormat::PlainText),
            _ => None,
        };
        match (by_ext, mime_kind(mime.as_deref())) {
            (Some(_), MimeKind::NotText) => Err(fail()),
            (Some(format), _) => Ok(format),
            (None, MimeKind::Is(format)) => Ok(format),
            (None, _) => Err(fail()),
        }
    }
}

/// Never parsed as text: other document, markup and archive types, and DOCX
/// and PDF until their parsers exist (Task 23).
pub const REJECTED_EXTENSIONS: &[&str] = &[
    "pdf", "docx", "doc", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "html", "htm",
    "csv", "json", "xml", "zip", "epub", "pages", "key", "numbers",
];

/// What a content type says about the format.
enum MimeKind {
    Is(SourceFormat),
    /// Some other `text/*` type: fits a supported extension, decides nothing.
    OtherText,
    /// `application/octet-stream` or none: says nothing about the format.
    Opaque,
    NotText,
}

fn mime_kind(mime: Option<&str>) -> MimeKind {
    match mime {
        None | Some("application/octet-stream") => MimeKind::Opaque,
        Some(
            "text/markdown" | "text/x-markdown" | "text/x-web-markdown" | "application/x-markdown",
        ) => MimeKind::Is(SourceFormat::Markdown),
        Some("text/plain") => MimeKind::Is(SourceFormat::PlainText),
        Some(other) if other.starts_with("text/") => MimeKind::OtherText,
        Some(_) => MimeKind::NotText,
    }
}

/// Lowercased extension; a dotfile such as `.md` counts as one.
fn extension(name: &str) -> Option<String> {
    let base = Path::new(name).file_name()?.to_str()?;
    let ext = match Path::new(base).extension() {
        Some(ext) => ext.to_str()?,
        None => base.strip_prefix('.')?,
    };
    Some(ext.to_ascii_lowercase())
}

fn unsupported(mime: Option<&str>, ext: Option<&str>) -> OragError {
    OragError::UnsupportedFormat(format!(
        "content type `{}`, extension `{}` (supported: .txt, .md)",
        mime.unwrap_or("none"),
        ext.unwrap_or("none")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_content_type_or_extension() {
        assert_eq!(
            SourceFormat::detect(Some("a.md"), None).unwrap(),
            SourceFormat::Markdown
        );
        assert_eq!(
            SourceFormat::detect(Some("A.MARKDOWN"), None).unwrap(),
            SourceFormat::Markdown
        );
        assert_eq!(
            SourceFormat::detect(None, Some("text/markdown; charset=utf-8")).unwrap(),
            SourceFormat::Markdown
        );
        assert_eq!(
            SourceFormat::detect(Some("notes.txt"), None).unwrap(),
            SourceFormat::PlainText
        );
        assert_eq!(
            SourceFormat::detect(None, Some("text/plain")).unwrap(),
            SourceFormat::PlainText
        );
    }

    #[test]
    fn markdown_extension_wins_over_generic_text_plain() {
        assert_eq!(
            SourceFormat::detect(Some("x.md"), Some("text/plain")).unwrap(),
            SourceFormat::Markdown
        );
    }

    #[test]
    fn unknown_formats_are_rejected() {
        assert!(SourceFormat::detect(Some("a.xlsx"), None).is_err());
        assert!(SourceFormat::detect(Some("a.html"), Some("text/html")).is_err());
        assert!(SourceFormat::detect(None, None).is_err());
    }

    #[test]
    fn a_content_type_that_is_not_text_is_rejected() {
        assert!(SourceFormat::detect(Some("x.md"), Some("application/pdf")).is_err());
        assert!(SourceFormat::detect(Some("x.txt"), Some("image/png")).is_err());
        assert!(SourceFormat::detect(None, Some("application/octet-stream")).is_err());
        assert_eq!(
            SourceFormat::detect(Some("x.md"), Some("application/octet-stream")).unwrap(),
            SourceFormat::Markdown,
            "browsers send octet-stream for unknown files"
        );
    }

    #[test]
    fn the_extension_decides_between_supported_types() {
        assert_eq!(
            SourceFormat::detect(Some("notes.txt"), Some("text/markdown")).unwrap(),
            SourceFormat::PlainText
        );
        assert_eq!(
            SourceFormat::detect(None, Some("text/x-markdown")).unwrap(),
            SourceFormat::Markdown
        );
        assert_eq!(
            SourceFormat::detect(Some(".md"), None).unwrap(),
            SourceFormat::Markdown
        );
    }

    #[test]
    fn content_type_decides_for_unknown_extensions() {
        assert_eq!(
            SourceFormat::detect(Some("notes.v2"), Some("text/plain")).unwrap(),
            SourceFormat::PlainText
        );
        assert!(SourceFormat::detect(Some("notes.v2"), None).is_err());
    }

    #[test]
    fn text_plain_does_not_make_a_known_binary_or_markup_file_text() {
        for name in ["a.pdf", "a.html", "a.docx", "a.xlsx"] {
            assert!(
                SourceFormat::detect(Some(name), Some("text/plain")).is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn any_text_or_markdown_content_type_fits_a_supported_extension() {
        for mime in ["text/x-web-markdown", "application/x-markdown", "text/csv"] {
            assert!(
                SourceFormat::detect(Some("a.md"), Some(mime)).is_ok(),
                "{mime}"
            );
        }
        assert_eq!(
            SourceFormat::detect(None, Some("application/x-markdown")).unwrap(),
            SourceFormat::Markdown
        );
    }
}
