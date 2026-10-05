//! UTF-8 decoding and parsers producing `ParsedDocument`.

use std::borrow::Cow;

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::domain::document::{Block, ParsedDocument};
use crate::error::{OragError, Result};
use crate::ingest::format::SourceFormat;

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// Decodes UTF-8 (optional BOM). Rejects UTF-16 and binary content with actionable messages.
pub fn decode_utf8(bytes: &[u8]) -> Result<&str> {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(OragError::InvalidInput(
            "UTF-16 text is not supported; save the file as UTF-8".into(),
        ));
    }
    let body = bytes.strip_prefix(&UTF8_BOM).unwrap_or(bytes);
    let bom_len = bytes.len() - body.len();
    let text = std::str::from_utf8(body).map_err(|err| {
        OragError::InvalidInput(format!(
            "file is not valid UTF-8 (first invalid byte at offset {})",
            bom_len + err.valid_up_to()
        ))
    })?;
    if text.contains('\0') {
        return Err(OragError::InvalidInput(
            "file contains NUL bytes; it looks binary".into(),
        ));
    }
    Ok(text)
}

/// Parses a document; DOCX/PDF go through a child process when isolation is
/// enabled (`orag serve`), otherwise in-process (tests, eval).
pub fn parse_document(format: SourceFormat, bytes: &[u8]) -> Result<ParsedDocument> {
    parse_document_until(format, bytes, &|| false)
}

/// `parse_document` that stops an isolated parse with `Interrupted` once
/// `stop` turns true (the worker's shutdown signal).
pub fn parse_document_until(
    format: SourceFormat,
    bytes: &[u8],
    stop: &dyn Fn() -> bool,
) -> Result<ParsedDocument> {
    use crate::ingest::isolate::{PARSE_TIMEOUT, isolated_executable, parse_isolated_until};
    match (format.is_binary(), isolated_executable()) {
        (true, Some(executable)) => {
            parse_isolated_until(executable, format, bytes, PARSE_TIMEOUT, stop)
        }
        _ => parse_in_process(format, bytes),
    }
}

pub fn parse_in_process(format: SourceFormat, bytes: &[u8]) -> Result<ParsedDocument> {
    match format {
        // Both parsers handle CRLF and lone CR themselves: no extra copy here.
        SourceFormat::PlainText => Ok(parse_plain(decode_utf8(bytes)?)),
        SourceFormat::Markdown => Ok(parse_markdown(decode_utf8(bytes)?)),
        SourceFormat::Docx => crate::ingest::docx::parse_docx(bytes),
        SourceFormat::Pdf => crate::ingest::pdf::parse_pdf(bytes),
    }
}

/// Collapses every run of whitespace to one space and trims; used by both
/// parsers so block text does not depend on the source format.
fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Lines ended by CRLF, LF or a lone CR (old Mac files).
fn lines_any(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n')
        .flat_map(|line| line.strip_suffix('\r').unwrap_or(line).split('\r'))
}

/// Paragraphs separated by blank lines.
fn parse_plain(text: &str) -> ParsedDocument {
    ParsedDocument {
        title: None,
        blocks: paragraph_blocks(text),
        warnings: Vec::new(),
    }
}

/// Blank-line separated paragraphs; lines are trimmed and joined with spaces.
/// Shared by plain text and PDF page text. Any line ending counts (`lines_any`),
/// so old Mac files with lone CRs split like the rest.
pub(crate) fn paragraph_blocks(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut lines: Vec<&str> = Vec::new();
    for line in lines_any(text) {
        if line.trim().is_empty() {
            flush_paragraph(&mut lines, &mut blocks);
        } else {
            lines.push(line.trim());
        }
    }
    flush_paragraph(&mut lines, &mut blocks);
    blocks
}

fn flush_paragraph(lines: &mut Vec<&str>, blocks: &mut Vec<Block>) {
    if !lines.is_empty() {
        blocks.push(Block::Paragraph(collapse_whitespace(&lines.join(" "))));
        lines.clear();
    }
}

fn parse_markdown(text: &str) -> ParsedDocument {
    // pulldown-cmark does not end fenced code at a lone CR (old Mac files).
    let text: Cow<'_, str> = if text.contains('\r') {
        Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(text)
    };
    let mut state = MarkdownState::default();
    for event in Parser::new_ext(&text, Options::ENABLE_TABLES) {
        state.handle(event);
    }
    state.finish()
}

#[derive(Default)]
struct MarkdownState {
    blocks: Vec<Block>,
    buf: String,
    /// Raw markup of the current HTML block; converted to text at its end.
    html: String,
    item_depth: usize,
    quote_depth: usize,
    /// The first H1 outside quotes and list items.
    title: Option<String>,
    row: Vec<String>,
    rows: Vec<String>,
}

impl MarkdownState {
    fn handle(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) | Event::Code(text) => self.buf.push_str(&text),
            Event::Html(html) => self.html.push_str(&html),
            // A break tag (`<br>`) separates words; formatting (`<b>`) does not.
            Event::InlineHtml(tag) if breaks_words(&tag) => self.buf.push(' '),
            Event::SoftBreak | Event::HardBreak => self.buf.push(' '),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            // A block that becomes its own `Block` first closes the item text
            // before it, so the item keeps its words.
            Tag::Heading { .. } | Tag::Table(_) => self.flush_item_text(),
            Tag::CodeBlock(_) => {
                self.flush_item_text();
                // Separators left by an enclosing or preceding quote are not code.
                let stray = self.take();
                self.push_nonempty(stray, Block::Paragraph);
            }
            Tag::Item => {
                self.flush_item_text();
                self.item_depth += 1;
            }
            // Other block starts separate words (a quote in a tight item);
            // inline tags (emphasis, links) do not, so `un*believ*able` stays one word.
            Tag::BlockQuote(_) => {
                self.quote_depth += 1;
                self.buf.push(' ');
            }
            Tag::Paragraph | Tag::List(_) | Tag::HtmlBlock => self.buf.push(' '),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(level) => {
                let text = self.take();
                let top_level = self.quote_depth == 0 && self.item_depth == 0;
                if level == HeadingLevel::H1 && top_level && self.title.is_none() {
                    self.title = Some(text.clone()).filter(|t| !t.is_empty());
                }
                self.push_nonempty(text, |text| Block::Heading {
                    level: heading_level(level),
                    text,
                });
            }
            TagEnd::Item => {
                self.flush_item_text();
                self.item_depth = self.item_depth.saturating_sub(1);
            }
            TagEnd::HtmlBlock => {
                let html = std::mem::take(&mut self.html);
                self.buf.push_str(&html_to_text(&html));
                self.end_paragraph();
            }
            TagEnd::Paragraph => self.end_paragraph(),
            TagEnd::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.buf.push(' ');
            }
            TagEnd::CodeBlock => {
                let raw = std::mem::take(&mut self.buf);
                let code = raw.trim_end().trim_start_matches('\n').to_string();
                self.push_nonempty(code, Block::Code);
            }
            TagEnd::TableCell => {
                // A literal `|` is escaped so the flattened row keeps its columns.
                let cell = self.take().replace('|', "\\|");
                self.row.push(cell);
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                let row = std::mem::take(&mut self.row);
                if row.iter().any(|cell| !cell.is_empty()) {
                    self.rows.push(row.join(" | "));
                }
            }
            TagEnd::Table => {
                let rows = std::mem::take(&mut self.rows);
                self.push_nonempty(rows.join("\n"), Block::Table);
            }
            _ => {}
        }
    }

    fn end_paragraph(&mut self) {
        if self.item_depth > 0 {
            self.buf.push(' ');
        } else {
            let text = self.take();
            self.push_nonempty(text, Block::Paragraph);
        }
    }

    fn take(&mut self) -> String {
        collapse_whitespace(&std::mem::take(&mut self.buf))
    }

    fn push_nonempty(&mut self, text: String, make: impl FnOnce(String) -> Block) {
        if !text.is_empty() {
            self.blocks.push(make(text));
        }
    }

    fn flush_item_text(&mut self) {
        if self.item_depth > 0 {
            let text = self.take();
            self.push_nonempty(text, Block::ListItem);
        }
    }

    fn finish(mut self) -> ParsedDocument {
        let rest = self.take();
        self.push_nonempty(rest, Block::Paragraph);
        ParsedDocument {
            title: self.title,
            blocks: self.blocks,
            warnings: Vec::new(),
        }
    }
}

/// The visible text of an HTML block: tags become spaces; comments and
/// `<script>`/`<style>` bodies are dropped; entities are decoded (`&uuml;` is
/// `ü`). A `<` that does not start a complete tag (`a < b`, `a<b and more`) is
/// text.
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let skip = if tail.starts_with("<!--") {
            Some(tail.find("-->").map_or(tail.len(), |end| end + 3))
        } else if starts_tag(tail) {
            tag_end(tail).map(|end| match raw_text_element(tail) {
                Some(name) => closing_tag_end(tail, name).unwrap_or(tail.len()),
                None => end,
            })
        } else {
            None
        };
        match skip {
            Some(skip) => {
                out.push(' ');
                rest = &tail[skip..];
            }
            None => {
                out.push('<');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    html_escape::decode_html_entities(&out).into_owned()
}

/// `<` followed by a tag name, `/`, `!` or `?`.
fn starts_tag(tail: &str) -> bool {
    tail[1..]
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?'))
}

/// Byte offset just past the `>` that closes the tag at the start of `tail`,
/// ignoring a `>` inside a quoted attribute value. `None` if it never closes
/// or another `<` comes first.
fn tag_end(tail: &str) -> Option<usize> {
    let mut quote = None;
    for (i, ch) in tail.char_indices().skip(1) {
        match (quote, ch) {
            (None, '"' | '\'') => quote = Some(ch),
            (Some(open), ch) if ch == open => quote = None,
            (None, '>') => return Some(i + 1),
            // A new `<` first means this was text (`a<b and more <i>x</i>`).
            (None, '<') => return None,
            _ => {}
        }
    }
    None
}

/// The lowercased tag name at the start of `tail` (`<b>` -> `b`, `</br>` -> `br`).
fn tag_name(tail: &str) -> String {
    tail.trim_start_matches(['<', '/'])
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase()
}

/// `script` or `style` if `tail` opens one of them.
fn raw_text_element(tail: &str) -> Option<&'static str> {
    if tail[1..].starts_with('/') {
        return None;
    }
    let name = tag_name(tail);
    ["script", "style"].into_iter().find(|raw| *raw == name)
}

/// Byte offset just past `</name ...>` in `tail`, compared case-insensitively
/// without copying the rest of the block.
fn closing_tag_end(tail: &str, name: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(found) = tail[from..].find("</") {
        let start = from + found;
        if tag_name(&tail[start..]) == name {
            return tail[start..].find('>').map(|end| start + end + 1);
        }
        from = start + 2;
    }
    None
}

/// Inline tags that separate the words around them: line and block breaks.
/// Formatting tags (`<b>`, `<em>`, `<span>`) do not, so `Ankara'<em>nın</em>`
/// stays one word.
fn breaks_words(tag: &str) -> bool {
    matches!(
        tag_name(tag).as_str(),
        "br" | "hr"
            | "img"
            | "p"
            | "div"
            | "li"
            | "td"
            | "th"
            | "tr"
            | "table"
            | "ul"
            | "ol"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "blockquote"
            | "pre"
    )
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::document::Block;

    #[test]
    fn parses_markdown_structure() {
        let md = "# Guide\n\nIntro text\nwraps here.\n\n## Install\n\n- step one\n- step two\n\n```sh\nmake\nmake install\n```\n\n| Key | Value |\n|---|---|\n| a | 1 |\n";
        let doc = parse_document(SourceFormat::Markdown, md.as_bytes()).unwrap();
        assert_eq!(doc.title.as_deref(), Some("Guide"));
        assert_eq!(
            doc.blocks,
            vec![
                Block::Heading {
                    level: 1,
                    text: "Guide".into()
                },
                Block::Paragraph("Intro text wraps here.".into()),
                Block::Heading {
                    level: 2,
                    text: "Install".into()
                },
                Block::ListItem("step one".into()),
                Block::ListItem("step two".into()),
                Block::Code("make\nmake install".into()),
                Block::Table("Key | Value\na | 1".into()),
            ]
        );
    }

    #[test]
    fn nested_and_loose_list_items_become_separate_items() {
        let md = "- parent\n\n  continued\n  - child\n";
        let doc = parse_document(SourceFormat::Markdown, md.as_bytes()).unwrap();
        assert_eq!(
            doc.blocks,
            vec![
                Block::ListItem("parent continued".into()),
                Block::ListItem("child".into())
            ]
        );
    }

    #[test]
    fn plain_text_paragraphs_split_on_blank_lines_and_crlf() {
        let text = "first line\r\nsame paragraph\r\n\r\n  \r\nsecond";
        let doc = parse_document(SourceFormat::PlainText, text.as_bytes()).unwrap();
        assert_eq!(
            doc.blocks,
            vec![
                Block::Paragraph("first line same paragraph".into()),
                Block::Paragraph("second".into())
            ]
        );
        assert_eq!(doc.title, None);
    }

    #[test]
    fn strips_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("Merhaba".as_bytes());
        assert_eq!(decode_utf8(&bytes).unwrap(), "Merhaba");
    }

    #[test]
    fn rejects_utf16_with_actionable_message() {
        let err = decode_utf8(&[0xFF, 0xFE, b'a', 0]).unwrap_err().to_string();
        assert!(err.contains("save the file as UTF-8"), "{err}");
    }

    #[test]
    fn rejects_invalid_utf8_with_offset() {
        let err = decode_utf8(&[b'o', b'k', 0xC3, 0x28])
            .unwrap_err()
            .to_string();
        assert!(err.contains("offset 2"), "{err}");
    }

    #[test]
    fn rejects_binary_nul_bytes() {
        assert!(decode_utf8(b"abc\0def").is_err());
    }

    fn md(text: &str) -> Vec<Block> {
        parse_document(SourceFormat::Markdown, text.as_bytes())
            .unwrap()
            .blocks
    }

    #[test]
    fn a_table_inside_a_list_item_keeps_the_item_text() {
        assert_eq!(
            md("- intro\n  | a | b |\n  |---|---|\n  | 1 | 2 |\n"),
            vec![
                Block::ListItem("intro".into()),
                Block::Table("a | b\n1 | 2".into())
            ]
        );
    }

    #[test]
    fn html_text_is_kept_and_tags_separate_words() {
        assert_eq!(
            md("<div>\nImportant policy text\n</div>\n\nline one<br>line two\n"),
            vec![
                Block::Paragraph("Important policy text".into()),
                Block::Paragraph("line one line two".into()),
            ]
        );
    }

    #[test]
    fn nested_blocks_in_a_tight_item_do_not_glue_words() {
        assert_eq!(
            md("- item\n  > quote\n- next\n"),
            vec![
                Block::ListItem("item quote".into()),
                Block::ListItem("next".into())
            ]
        );
    }

    #[test]
    fn invalid_utf8_offset_counts_the_bom() {
        let err = decode_utf8(&[0xEF, 0xBB, 0xBF, b'o', b'k', 0xC3, 0x28])
            .unwrap_err()
            .to_string();
        assert!(err.contains("offset 5"), "{err}");
    }

    #[test]
    fn lone_carriage_returns_are_line_breaks() {
        let doc = parse_document(SourceFormat::PlainText, b"a\rb\r\rc").unwrap();
        assert_eq!(
            doc.blocks,
            vec![Block::Paragraph("a b".into()), Block::Paragraph("c".into())]
        );
    }

    #[test]
    fn whitespace_is_collapsed_the_same_in_both_formats() {
        let text = "a\t\tb   c";
        let plain = parse_document(SourceFormat::PlainText, text.as_bytes())
            .unwrap()
            .blocks;
        assert_eq!(plain, md(text));
        assert_eq!(plain, vec![Block::Paragraph("a b c".into())]);
    }

    #[test]
    fn empty_table_rows_and_leading_code_blank_lines_are_dropped() {
        assert_eq!(md("| a |\n|---|\n|   |\n"), vec![Block::Table("a".into())]);
        assert_eq!(
            md("```\n\n\n  lead\n```\n"),
            vec![Block::Code("  lead".into())]
        );
    }

    #[test]
    fn inline_formatting_does_not_split_words() {
        assert_eq!(
            md("un*believ*able Ankara'**nın** [li]nk"),
            vec![Block::Paragraph("unbelievable Ankara'nın [li]nk".into())]
        );
    }

    #[test]
    fn html_markup_comments_and_scripts_are_not_text() {
        assert_eq!(
            md(
                "<div\n class=\"secret-attr\">\nhi\n</div>\n\n<!--\nhidden comment\n-->\n\n<script>\nvar x = 1;\n</script>\n\nend\n"
            ),
            vec![
                Block::Paragraph("hi".into()),
                Block::Paragraph("end".into())
            ]
        );
    }

    #[test]
    fn a_less_than_sign_in_html_text_is_kept() {
        assert_eq!(
            md("<p>if a < b and c > d then</p>\n"),
            vec![Block::Paragraph("if a < b and c > d then".into())]
        );
    }

    #[test]
    fn code_after_or_inside_a_quote_keeps_its_indentation() {
        assert_eq!(
            md("> quote\n\n```\nx\n```\n").last(),
            Some(&Block::Code("x".into()))
        );
        assert_eq!(
            md("> ```\n> code\n> ```\n"),
            vec![Block::Code("code".into())]
        );
    }

    #[test]
    fn markdown_with_lone_carriage_returns_keeps_code_blocks() {
        assert_eq!(md("```\ra\rb\r```\r"), vec![Block::Code("a\nb".into())]);
    }

    #[test]
    fn an_escaped_pipe_stays_inside_its_cell() {
        assert_eq!(
            md("| a \\| b | c |\n|---|---|\n"),
            vec![Block::Table("a \\| b | c".into())]
        );
    }

    #[test]
    fn inline_html_formatting_does_not_split_words_but_breaks_do() {
        assert_eq!(
            md("un<b>believ</b>able Ankara'<em>nın</em> one<br>two"),
            vec![Block::Paragraph("unbelievable Ankara'nın one two".into())]
        );
    }

    #[test]
    fn html_entities_are_decoded() {
        assert_eq!(
            md("<div>\nG&uuml;venlik &amp; Q&#38;A &#x15F;\n</div>\n"),
            vec![Block::Paragraph("Güvenlik & Q&A ş".into())]
        );
    }

    #[test]
    fn an_unterminated_or_quoted_angle_bracket_does_not_eat_text() {
        assert_eq!(
            md("<div>\nprice a<b and more text here\n</div>\n"),
            vec![Block::Paragraph("price a<b and more text here".into())]
        );
        assert_eq!(
            md("<div>\n<a title=\"x>leak\">link</a>\n</div>\n"),
            vec![Block::Paragraph("link".into())]
        );
    }

    #[test]
    fn the_title_is_a_top_level_h1() {
        let doc = parse_document(SourceFormat::Markdown, b"> # Quoted title\n\n# Real\n").unwrap();
        assert_eq!(doc.title.as_deref(), Some("Real"));
    }
}
