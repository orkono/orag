//! PDF text extraction with `pdf_oxide` (pure Rust, D-010). No OCR (D-011):
//! pages without a text layer are reported in an `ocr_required` warning.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use crate::domain::document::ParsedDocument;
use crate::error::{OragError, Result};
use crate::ingest::parse::paragraph_blocks;

/// Upper bound on pages processed from one file.
pub const MAX_PDF_PAGES: usize = 2000;
/// Upper bound on extracted text; far above any document within the upload
/// limit, so only a decompression trick reaches it.
pub const MAX_PDF_TEXT_BYTES: usize = 64 * 1024 * 1024;
/// Checked between pages; below the 120 s isolation deadline, so in-process
/// callers (eval, tests) stop too. One page can still take longer: under
/// `orag serve` the isolation deadline covers that.
pub const PDF_TIME_LIMIT: Duration = Duration::from_secs(100);

#[derive(Debug, Clone, Copy)]
struct Limits {
    max_text_bytes: usize,
    time_limit: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_text_bytes: MAX_PDF_TEXT_BYTES,
            time_limit: PDF_TIME_LIMIT,
        }
    }
}

pub fn parse_pdf(bytes: &[u8]) -> Result<ParsedDocument> {
    // A malformed file must fail this document, never the ingestion worker.
    catch_unwind(AssertUnwindSafe(|| extract(bytes, Limits::default()))).unwrap_or_else(|_| {
        Err(OragError::InvalidInput(
            "the PDF could not be parsed (malformed file)".into(),
        ))
    })
}

fn extract(bytes: &[u8], limits: Limits) -> Result<ParsedDocument> {
    let started = Instant::now();
    let doc = pdf_oxide::PdfDocument::from_bytes(bytes.to_vec()).map_err(|e| {
        OragError::InvalidInput(format!("cannot read PDF (encrypted or damaged?): {e}"))
    })?;
    let pages = doc
        .page_count()
        .map_err(|e| OragError::InvalidInput(format!("cannot read PDF page tree: {e}")))?;
    if pages == 0 {
        return Err(OragError::InvalidInput("the PDF has no pages".into()));
    }
    if pages > MAX_PDF_PAGES {
        return Err(OragError::InvalidInput(format!(
            "PDF has {pages} pages; the limit is {MAX_PDF_PAGES}"
        )));
    }
    let (mut blocks, mut empty, mut failed) = (Vec::new(), Vec::new(), Vec::new());
    let mut text_bytes = 0usize;
    for index in 0..pages {
        if started.elapsed() >= limits.time_limit {
            return Err(OragError::InvalidInput(format!(
                "reading the PDF took too long (stopped at page {} of {pages})",
                index + 1
            )));
        }
        match doc.extract_text(index) {
            Ok(text) => {
                text_bytes = text_bytes.saturating_add(text.len());
                if text_bytes > limits.max_text_bytes {
                    return Err(OragError::InvalidInput(format!(
                        "the PDF yields more text than {} MB",
                        limits.max_text_bytes / (1024 * 1024)
                    )));
                }
                let page_blocks = paragraph_blocks(&join_hyphenated(&text));
                if page_blocks.is_empty() {
                    empty.push(index + 1);
                }
                blocks.extend(page_blocks);
            }
            Err(_) => failed.push(index + 1),
        }
    }
    if failed.len() == pages {
        return Err(OragError::InvalidInput(
            "no page of the PDF could be read".into(),
        ));
    }
    let mut warnings = Vec::new();
    if !empty.is_empty() {
        warnings.push(format!(
            "ocr_required: page(s) {} have no extractable text (scanned?) and were not indexed",
            page_list(&empty)
        ));
    }
    if !failed.is_empty() {
        warnings.push(format!(
            "extraction_failed: page(s) {} could not be read",
            page_list(&failed)
        ));
    }
    Ok(ParsedDocument {
        title: None,
        blocks,
        warnings,
    })
}

/// Joins words split at a line end: "gün-\ndür" → "gündür", a soft hyphen
/// (U+00AD) before the break as well, CRLF included. Only a hyphen after a
/// letter and before a lowercase letter counts, so codes such as "ORX-\nE104"
/// stay apart (an all-caps word split this way does too: telling it from a
/// code is guesswork). After a one-letter prefix the hyphen is real
/// ("e-\nfatura" → "e-fatura"): only the line break is removed.
fn join_hyphenated(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let after_letter = i > 0 && chars[i - 1].is_alphabetic();
        let newline = match (chars.get(i + 1), chars.get(i + 2)) {
            (Some('\r'), Some('\n')) => 2,
            (Some('\n'), _) => 1,
            _ => 0,
        };
        let continues = chars
            .get(i + 1 + newline)
            .is_some_and(|next| next.is_lowercase());
        if (c == '-' || c == '\u{00AD}') && after_letter && newline > 0 && continues {
            let one_letter_prefix = i < 2 || !chars[i - 2].is_alphabetic();
            if c == '-' && one_letter_prefix {
                out.push('-');
            }
            i += 1 + newline;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// `1-3, 5` style list of 1-based page numbers.
fn page_list(pages: &[usize]) -> String {
    let mut parts = Vec::new();
    let mut i = 0;
    while i < pages.len() {
        let start = pages[i];
        let mut end = start;
        while i + 1 < pages.len() && pages[i + 1] == end + 1 {
            i += 1;
            end = pages[i];
        }
        parts.push(if start == end {
            start.to_string()
        } else {
            format!("{start}-{end}")
        });
        i += 1;
    }
    parts.join(", ")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::domain::document::Block;

    /// Minimal PDF: `Some(text)` pages draw ASCII text in Helvetica, `None` pages are blank.
    pub(crate) fn test_pdf(pages: &[Option<&str>]) -> Vec<u8> {
        let mut objects: Vec<String> = vec![
            String::new(),
            String::new(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        ];
        let mut kids = Vec::new();
        for page in pages {
            let contents = page.map(|text| {
                let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
                objects.push(format!(
                    "<< /Length {} >>\nstream\n{stream}\nendstream",
                    stream.len()
                ));
                objects.len()
            });
            let contents_ref = contents
                .map(|n| format!(" /Contents {n} 0 R"))
                .unwrap_or_default();
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >>{contents_ref} >>"
            ));
            kids.push(format!("{} 0 R", objects.len()));
        }
        objects[0] = "<< /Type /Catalog /Pages 2 0 R >>".into();
        objects[1] = format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            pages.len()
        );
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in offsets {
            out.extend(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    #[test]
    fn turkish_fixture_text_is_extracted_intact() {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample-tr.pdf"
        ))
        .unwrap();
        let doc = parse_pdf(&bytes).unwrap();
        let text: String = doc.blocks.iter().map(|b| format!("{b:?}")).collect();
        for expected in ["İade süresi 14 gündür", "Iğdır", "şık", "çay", "öğün"] {
            assert!(text.contains(expected), "missing {expected:?} in {text}");
        }
        assert!(doc.warnings.is_empty(), "{:?}", doc.warnings);
    }

    #[test]
    fn blank_pages_are_reported_as_ocr_required() {
        let doc = parse_pdf(&test_pdf(&[
            Some("Hello world"),
            None,
            None,
            Some("Second page"),
        ]))
        .unwrap();
        assert_eq!(
            doc.blocks,
            vec![
                Block::Paragraph("Hello world".into()),
                Block::Paragraph("Second page".into())
            ]
        );
        assert_eq!(doc.warnings.len(), 1);
        assert!(
            doc.warnings[0].starts_with("ocr_required: page(s) 2-3 "),
            "{:?}",
            doc.warnings
        );
    }

    #[test]
    fn fully_scanned_pdf_has_no_blocks_and_a_warning() {
        let doc = parse_pdf(&test_pdf(&[None, None])).unwrap();
        assert!(doc.blocks.is_empty());
        assert!(doc.warnings[0].contains("1-2"));
    }

    #[test]
    fn garbage_is_rejected_not_panicking() {
        // `extract` directly: the catch_unwind in `parse_pdf` would hide a panic.
        assert!(extract(b"%PDF-1.7\nthis is not a pdf", Limits::default()).is_err());
        assert!(extract(b"", Limits::default()).is_err());
    }

    #[test]
    fn a_pdf_without_pages_is_an_error() {
        let err = parse_pdf(&test_pdf(&[])).unwrap_err();
        assert!(err.to_string().contains("no pages"), "{err}");
    }

    #[test]
    fn extraction_is_bounded_in_text_and_time() {
        let pdf = test_pdf(&[Some("Hello world"), Some("Second page")]);
        let small = Limits {
            max_text_bytes: 12,
            ..Limits::default()
        };
        let err = extract(&pdf, small).unwrap_err();
        assert!(err.to_string().contains("more text than"), "{err}");
        let no_time = Limits {
            time_limit: std::time::Duration::ZERO,
            ..Limits::default()
        };
        let err = extract(&pdf, no_time).unwrap_err();
        assert!(err.to_string().contains("too long"), "{err}");
    }

    #[test]
    fn words_hyphenated_at_line_ends_are_joined() {
        assert_eq!(
            join_hyphenated("İade süresi 14 gün-\ndür."),
            "İade süresi 14 gündür."
        );
        // Not a hyphenation: a dash before a capital, a digit or a blank line.
        assert_eq!(join_hyphenated("Kod ORX-\nE104"), "Kod ORX-\nE104");
        assert_eq!(join_hyphenated("bitti -\n\nyeni"), "bitti -\n\nyeni");
        // A one-letter prefix is a real hyphen: only the line break goes.
        assert_eq!(join_hyphenated("bir e-\nfatura ile"), "bir e-fatura ile");
        // A soft hyphen and CRLF line ends.
        assert_eq!(join_hyphenated("gün\u{00AD}\ndür"), "gündür");
        assert_eq!(join_hyphenated("gün-\r\ndür"), "gündür");
    }

    #[test]
    fn page_lists_are_compact() {
        assert_eq!(page_list(&[1, 2, 3, 5, 7, 8]), "1-3, 5, 7-8");
    }
}
