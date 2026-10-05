//! Source format detection (extension first, then content type) and a
//! content-signature check that runs before anything is stored.

use std::path::Path;

use crate::error::{OragError, Result};

const DOCX_MIME: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const SUPPORTED: &str = "supported: .txt, .md, .docx, .pdf";

/// Document types that are never accepted, whatever the content type says (D-010).
pub const REJECTED_EXTENSIONS: &[&str] = &[
    "doc", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "html", "htm", "csv", "json",
    "xml", "zip", "epub", "pages", "key", "numbers",
];

/// Lowercased extension; a dotfile such as `.md` counts as one.
fn extension(filename: Option<&str>) -> Option<String> {
    // Surrounding whitespace is trimmed, as the stored filename is.
    let base = Path::new(filename?.trim()).file_name()?.to_str()?;
    let ext = match Path::new(base).extension() {
        Some(ext) => ext.to_str()?,
        None => base.strip_prefix('.')?,
    };
    Some(ext.to_ascii_lowercase())
}

fn rejected(ext: &str) -> OragError {
    OragError::UnsupportedFormat(format!("`.{ext}` files are not supported ({SUPPORTED})"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    PlainText,
    Markdown,
    Docx,
    Pdf,
}

impl SourceFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceFormat::PlainText => "text",
            SourceFormat::Markdown => "markdown",
            SourceFormat::Docx => "docx",
            SourceFormat::Pdf => "pdf",
        }
    }

    pub fn from_name(name: &str) -> Result<SourceFormat> {
        match name {
            "text" => Ok(SourceFormat::PlainText),
            "markdown" => Ok(SourceFormat::Markdown),
            "docx" => Ok(SourceFormat::Docx),
            "pdf" => Ok(SourceFormat::Pdf),
            other => Err(OragError::UnsupportedFormat(format!(
                "format `{other}` ({SUPPORTED})"
            ))),
        }
    }

    /// Binary formats must arrive as multipart file uploads.
    pub fn is_binary(self) -> bool {
        matches!(self, SourceFormat::Docx | SourceFormat::Pdf)
    }

    /// Format of a text sent as a JSON string. Filename rules come first
    /// (D-010): a known unsupported type is rejected whatever `declared` says,
    /// and a `declared` format that contradicts a supported extension is an
    /// error. Without a supported extension (no name, `Toplantı 12.10.2026`,
    /// `notes.v2`) `declared` applies, else plain text. DOCX and PDF must be
    /// uploaded as files, never as JSON text.
    pub fn detect_text(
        filename: Option<&str>,
        declared: Option<SourceFormat>,
    ) -> Result<SourceFormat> {
        if let Some(ext) = extension(filename).filter(|e| REJECTED_EXTENSIONS.contains(&e.as_str()))
        {
            return Err(rejected(&ext));
        }
        let named = Self::detect(filename, None).ok();
        // Binary first, so `rapor.pdf` + `format: text` is told to use multipart.
        let is_binary = |f: &SourceFormat| f.is_binary();
        if let Some(binary) = declared.filter(is_binary).or(named.filter(is_binary)) {
            return Err(OragError::InvalidInput(format!(
                "{} files must be uploaded as multipart/form-data (field `file`)",
                binary.as_str()
            )));
        }
        let format = match (declared, named) {
            (Some(declared), Some(named)) if declared != named => {
                return Err(OragError::InvalidInput(format!(
                    "format `{}` contradicts the filename extension",
                    declared.as_str()
                )));
            }
            (Some(declared), _) => declared,
            (None, named) => named.unwrap_or(SourceFormat::PlainText),
        };
        Ok(format)
    }

    /// Format of an uploaded file: a supported extension decides; a known
    /// unsupported type is rejected; otherwise the content type decides
    /// (dotted names such as `notes.v2` sent as `text/plain` are text).
    /// `application/octet-stream` and text types are generic, but another
    /// content type that contradicts a supported extension is rejected (a PDF
    /// named `x.md`).
    pub fn detect(filename: Option<&str>, content_type: Option<&str>) -> Result<SourceFormat> {
        let ext = extension(filename);
        let by_ext = match ext.as_deref() {
            Some("txt") => Some(SourceFormat::PlainText),
            Some("md" | "markdown") => Some(SourceFormat::Markdown),
            Some("docx") => Some(SourceFormat::Docx),
            Some("pdf") => Some(SourceFormat::Pdf),
            _ => None,
        };
        let mime = content_type.map(|ct| {
            ct.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        });
        let by_mime = match mime.as_deref() {
            Some("text/plain") => Some(SourceFormat::PlainText),
            Some(
                "text/markdown"
                | "text/x-markdown"
                | "text/x-web-markdown"
                | "application/x-markdown",
            ) => Some(SourceFormat::Markdown),
            Some(DOCX_MIME) => Some(SourceFormat::Docx),
            Some("application/pdf") => Some(SourceFormat::Pdf),
            _ => None,
        };
        // Fits any supported extension: no type, octet-stream, or any text type.
        let generic = match mime.as_deref() {
            None | Some("" | "application/octet-stream") => true,
            Some(mime) => mime.starts_with("text/"),
        };
        match (ext.as_deref(), by_ext, by_mime) {
            (Some(ext), _, _) if REJECTED_EXTENSIONS.contains(&ext) => Err(rejected(ext)),
            (_, Some(format), mime_format) if generic || mime_format == Some(format) => Ok(format),
            (_, None, Some(format)) => Ok(format),
            _ => Err(OragError::UnsupportedFormat(format!(
                "extension `{}`, content type `{}` ({SUPPORTED})",
                ext.as_deref().unwrap_or("none"),
                mime.as_deref().unwrap_or("none")
            ))),
        }
    }

    /// Rejects files whose bytes do not match the claimed binary format.
    pub fn check_signature(self, bytes: &[u8]) -> Result<()> {
        match self {
            SourceFormat::Pdf => {
                let head = &bytes[..bytes.len().min(1024)];
                if head.windows(5).any(|w| w == b"%PDF-") {
                    Ok(())
                } else {
                    Err(OragError::InvalidInput("the file is not a PDF".into()))
                }
            }
            SourceFormat::Docx => {
                if bytes.starts_with(b"PK\x03\x04") && declares_word_document(bytes) {
                    Ok(())
                } else {
                    Err(OragError::InvalidInput(
                        "the file is not a .docx (old .doc and password-protected files are not supported)".into(),
                    ))
                }
            }
            SourceFormat::PlainText | SourceFormat::Markdown => Ok(()),
        }
    }
}

/// Longest `[Content_Types].xml` read for the signature check.
const MAX_CONTENT_TYPES_BYTES: u64 = 1024 * 1024;
/// ZIP entries a .docx may have. Real documents have dozens to a few hundred;
/// the cap keeps the directory read (one allocation per entry) small.
pub const MAX_DOCX_ENTRIES: usize = 10_000;
/// Main-part content types of a Word document (also macro-enabled).
const WORD_MAIN_TYPES: [&str; 2] = [
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
    "application/vnd.ms-word.document.macroEnabled.main+xml",
];

/// A ZIP is a Word document only if its `[Content_Types].xml` declares a Word
/// *main part* (an `Override`): a renamed .xlsx, .pptx, .odt or .epub is
/// refused before it is stored, also one that merely embeds a .docx. The
/// entry count is read from the end-of-directory record first, so a ZIP of
/// millions of tiny entries is refused without reading its directory.
fn declares_word_document(bytes: &[u8]) -> bool {
    use std::io::Read;
    if zip_entry_count(bytes).is_none_or(|n| n > MAX_DOCX_ENTRIES) {
        return false;
    }
    let Ok(mut archive) = zip::ZipArchive::new(std::io::Cursor::new(bytes)) else {
        return false;
    };
    let Ok(types) = archive.by_name("[Content_Types].xml") else {
        return false;
    };
    let mut raw = Vec::new();
    if types
        .take(MAX_CONTENT_TYPES_BYTES)
        .read_to_end(&mut raw)
        .is_err()
    {
        return false;
    }
    let text = decode_xml_text(&raw);
    let Ok(doc) = roxmltree::Document::parse_with_options(
        &text,
        roxmltree::ParsingOptions {
            nodes_limit: 100_000,
            ..roxmltree::ParsingOptions::default()
        },
    ) else {
        return false;
    };
    doc.descendants()
        .filter(|n| n.tag_name().name() == "Override")
        .filter_map(|n| n.attribute("ContentType"))
        .any(|ct| WORD_MAIN_TYPES.contains(&ct))
}

/// XML text in UTF-8 or UTF-16 (with a byte-order mark); invalid sequences
/// become U+FFFD instead of failing a legal document.
fn decode_xml_text(raw: &[u8]) -> String {
    let utf16 = |bytes: &[u8], to_u16: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| to_u16(pair))
            .collect();
        String::from_utf16_lossy(&units)
    };
    match raw {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(raw).into_owned(),
    }
}

/// Entry count from the ZIP end-of-central-directory record (the last 22 to
/// 22 + 65535 bytes); `None` if there is no such record.
fn zip_entry_count(bytes: &[u8]) -> Option<usize> {
    const EOCD: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];
    const EOCD_LEN: usize = 22;
    let earliest = bytes
        .len()
        .checked_sub(EOCD_LEN)?
        .saturating_sub(u16::MAX as usize);
    let at = (earliest..=bytes.len() - EOCD_LEN)
        .rev()
        .find(|&i| bytes[i..i + 4] == EOCD)?;
    // "Total number of entries in the central directory" (0xFFFF means ZIP64:
    // more than any .docx needs, so it counts as too many).
    Some(u16::from_le_bytes([bytes[at + 10], bytes[at + 11]]) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_decides_before_content_type() {
        assert_eq!(
            SourceFormat::detect(Some("a.md"), None).unwrap(),
            SourceFormat::Markdown
        );
        assert_eq!(
            SourceFormat::detect(Some("A.MARKDOWN"), None).unwrap(),
            SourceFormat::Markdown
        );
        assert_eq!(
            SourceFormat::detect(Some("x.md"), Some("text/plain")).unwrap(),
            SourceFormat::Markdown
        );
        assert_eq!(
            SourceFormat::detect(Some("notes.txt"), None).unwrap(),
            SourceFormat::PlainText
        );
        assert_eq!(
            SourceFormat::detect(Some("Rapor.DOCX"), Some("application/octet-stream")).unwrap(),
            SourceFormat::Docx
        );
        assert_eq!(
            SourceFormat::detect(Some("scan.pdf"), Some("text/plain")).unwrap(),
            SourceFormat::Pdf
        );
    }

    #[test]
    fn content_type_is_used_without_extension() {
        assert_eq!(
            SourceFormat::detect(None, Some("text/markdown; charset=utf-8")).unwrap(),
            SourceFormat::Markdown
        );
        assert_eq!(
            SourceFormat::detect(None, Some("text/plain")).unwrap(),
            SourceFormat::PlainText
        );
        assert_eq!(
            SourceFormat::detect(None, Some("application/pdf")).unwrap(),
            SourceFormat::Pdf
        );
        assert_eq!(
            SourceFormat::detect(
                None,
                Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
            )
            .unwrap(),
            SourceFormat::Docx
        );
    }

    #[test]
    fn everything_else_is_unsupported() {
        for (name, mime) in [
            (Some("a.doc"), None),
            (Some("a.xlsx"), None),
            (Some("a.html"), Some("text/html")),
            (None, None),
        ] {
            let err = SourceFormat::detect(name, mime).unwrap_err();
            assert!(matches!(err, OragError::UnsupportedFormat(_)), "{name:?}");
            assert!(err.to_string().contains(".txt, .md, .docx, .pdf"));
        }
    }

    #[test]
    fn content_type_decides_for_unknown_but_harmless_extensions() {
        assert_eq!(
            SourceFormat::detect(Some("notes.v2"), Some("text/plain")).unwrap(),
            SourceFormat::PlainText
        );
        assert_eq!(
            SourceFormat::detect(Some("Toplantı 12.10.2026"), Some("text/plain")).unwrap(),
            SourceFormat::PlainText
        );
        assert!(SourceFormat::detect(Some("notes.v2"), None).is_err());
        assert!(
            SourceFormat::detect(Some("page.html"), Some("text/plain")).is_err(),
            "rejected types stay rejected"
        );
    }

    #[test]
    fn a_contradicting_content_type_is_rejected() {
        assert!(SourceFormat::detect(Some("x.md"), Some("application/pdf")).is_err());
        assert!(SourceFormat::detect(Some("a.docx"), Some("image/png")).is_err());
        assert_eq!(
            SourceFormat::detect(Some(".md"), None).unwrap(),
            SourceFormat::Markdown
        );
        assert_eq!(
            SourceFormat::detect(None, Some("text/x-markdown")).unwrap(),
            SourceFormat::Markdown
        );
        assert!(SourceFormat::detect(Some("a.md"), Some("text/x-web-markdown")).is_ok());
        assert!(
            SourceFormat::detect(Some("a.pdf"), Some("text/plain")).is_ok(),
            "the PDF signature check decides"
        );
        assert!(SourceFormat::detect(Some("a.html"), Some("text/plain")).is_err());
    }

    #[test]
    fn json_text_detection() {
        use SourceFormat::*;
        let ok = |name, declared| SourceFormat::detect_text(name, declared).unwrap();
        assert_eq!(ok(None, None), PlainText);
        assert_eq!(ok(Some("a.md"), None), Markdown);
        assert_eq!(ok(Some("Toplantı 12.10.2026"), None), PlainText);
        assert_eq!(ok(Some("notes.v2"), Some(Markdown)), Markdown);
        assert_eq!(ok(Some("a.md"), Some(Markdown)), Markdown);
        let err = |name, declared| SourceFormat::detect_text(name, declared).unwrap_err();
        // Filename rules win over an explicit format.
        assert!(matches!(
            err(Some("rapor.xlsx"), None),
            OragError::UnsupportedFormat(_)
        ));
        assert!(matches!(
            err(Some("rapor.xlsx"), Some(PlainText)),
            OragError::UnsupportedFormat(_)
        ));
        assert!(matches!(
            err(Some("a.md"), Some(PlainText)),
            OragError::InvalidInput(_)
        ));
        let md_as_pdf = err(Some("a.md"), Some(Pdf));
        assert!(md_as_pdf.to_string().contains("multipart"), "{md_as_pdf}");
        let pdf_as_text = err(Some("a.pdf"), Some(PlainText));
        assert!(
            pdf_as_text.to_string().contains("multipart"),
            "{pdf_as_text}"
        );
        // Binary formats are files only.
        assert!(matches!(
            err(Some("a.pdf"), None),
            OragError::InvalidInput(_)
        ));
        assert!(matches!(err(None, Some(Docx)), OragError::InvalidInput(_)));
    }

    #[test]
    fn pdf_without_signature_is_rejected() {
        assert!(SourceFormat::Pdf.check_signature(b"%PDF-1.7\n...").is_ok());
        assert!(SourceFormat::Pdf.check_signature(b"\n\n%PDF-1.4").is_ok());
        let err = SourceFormat::Pdf
            .check_signature(b"<html>")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a PDF"), "{err}");
    }

    #[test]
    fn docx_must_be_a_zip_container() {
        // A ZIP header alone is not enough (see `a_renamed_spreadsheet_is_not_a_docx`).
        assert!(
            SourceFormat::Docx
                .check_signature(b"PK\x03\x04rest")
                .is_err()
        );
        assert!(
            SourceFormat::Docx
                .check_signature(b"\xD0\xCF\x11\xE0")
                .is_err(),
            "legacy .doc / encrypted"
        );
    }

    #[test]
    fn names_round_trip() {
        for format in [
            SourceFormat::PlainText,
            SourceFormat::Markdown,
            SourceFormat::Docx,
            SourceFormat::Pdf,
        ] {
            assert_eq!(SourceFormat::from_name(format.as_str()).unwrap(), format);
        }
    }

    /// A ZIP with the given `[Content_Types].xml` (or none) and one part.
    fn zip_with_types(types: Option<&str>) -> Vec<u8> {
        use std::io::Write;
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            let options = zip::write::SimpleFileOptions::default();
            if let Some(types) = types {
                zip.start_file("[Content_Types].xml", options).unwrap();
                zip.write_all(types.as_bytes()).unwrap();
            }
            zip.start_file("xl/workbook.xml", options).unwrap();
            zip.write_all(b"<workbook/>").unwrap();
            zip.finish().unwrap();
        }
        out.into_inner()
    }

    #[test]
    fn a_renamed_spreadsheet_is_not_a_docx() {
        let xlsx = zip_with_types(Some(
            r#"<Types><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#,
        ));
        assert!(SourceFormat::Docx.check_signature(&xlsx).is_err());
        assert!(
            SourceFormat::Docx
                .check_signature(&zip_with_types(None))
                .is_err()
        );
        let fixture = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample.docx"
        ))
        .unwrap();
        assert!(SourceFormat::Docx.check_signature(&fixture).is_ok());
    }

    /// A ZIP with the given raw `[Content_Types].xml` bytes and `extra` empty entries.
    fn zip_raw_types(types: &[u8], extra: usize) -> Vec<u8> {
        use std::io::Write;
        let mut out = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            zip.start_file("[Content_Types].xml", options).unwrap();
            zip.write_all(types).unwrap();
            for i in 0..extra {
                zip.start_file(format!("x/{i}"), options).unwrap();
            }
            zip.finish().unwrap();
        }
        out.into_inner()
    }

    const WORD_MAIN: &str = r#"<Types><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;

    #[test]
    fn only_a_word_main_part_makes_a_docx() {
        // A spreadsheet that embeds a .docx declares the type, not a main part.
        let embedding = r#"<Types><Default Extension="docx" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/></Types>"#;
        assert!(
            SourceFormat::Docx
                .check_signature(&zip_raw_types(embedding.as_bytes(), 0))
                .is_err()
        );
        assert!(
            SourceFormat::Docx
                .check_signature(&zip_raw_types(WORD_MAIN.as_bytes(), 0))
                .is_ok()
        );
        // UTF-16 with a byte-order mark is legal XML.
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain(WORD_MAIN.encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert!(
            SourceFormat::Docx
                .check_signature(&zip_raw_types(&utf16, 0))
                .is_ok()
        );
    }

    #[test]
    fn a_zip_with_too_many_entries_is_refused_before_its_directory_is_read() {
        let many = zip_raw_types(WORD_MAIN.as_bytes(), MAX_DOCX_ENTRIES);
        assert!(SourceFormat::Docx.check_signature(&many).is_err());
    }
}
