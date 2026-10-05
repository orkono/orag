//! DOCX extraction (D-010): `word/document.xml` + `word/styles.xml` read with
//! `zip` and `roxmltree` (pure Rust). Produces headings (from paragraph styles),
//! paragraphs, list items and tables. Tracked deletions are skipped.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};

use roxmltree::Node;

use crate::domain::document::{Block, ParsedDocument};
use crate::error::{OragError, Result};

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
/// Word's "Strict Open XML Document" uses its own namespace and relationship types.
const W_STRICT_NS: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
const OFFICE_DOCUMENT_RELS: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument",
];
const STYLES_RELS: [&str; 2] = [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles",
    "http://purl.oclc.org/ooxml/officeDocument/relationships/styles",
];
/// Decompressed size cap per XML part (ZIP-bomb guard).
pub const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;
/// XML node cap per part, so a small part of tiny elements cannot grow the
/// parsed tree to gigabytes (far above any real document).
pub const MAX_XML_NODES: u32 = 2_000_000;
/// Element nesting cap: the XML parser and the walk below recurse once per
/// level, so deeper input would overflow the stack and abort the process.
/// Real documents stay well under 100 even with nested tables.
pub const MAX_XML_DEPTH: usize = 256;
/// Namespace declarations per part. The XML parser copies every in-scope
/// declaration into each element that declares one of its own, so thousands
/// of them make a small part quadratic in time and memory. Real parts declare
/// a few dozen, on the root.
pub const MAX_XML_NAMESPACES: usize = 1000;
/// Longest part name shown in an error; names come from the file itself.
const MAX_NAME_IN_ERROR: usize = 80;

#[derive(Debug, Clone, Copy)]
struct Limits {
    max_part_bytes: u64,
    max_nodes: u32,
    max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_part_bytes: MAX_PART_BYTES,
            max_nodes: MAX_XML_NODES,
            max_depth: MAX_XML_DEPTH,
        }
    }
}

pub fn parse_docx(bytes: &[u8]) -> Result<ParsedDocument> {
    parse_docx_with(bytes, Limits::default())
}

pub fn parse_docx_with_limit(bytes: &[u8], max_part_bytes: u64) -> Result<ParsedDocument> {
    parse_docx_with(
        bytes,
        Limits {
            max_part_bytes,
            ..Limits::default()
        },
    )
}

fn parse_docx_with(bytes: &[u8], limits: Limits) -> Result<ParsedDocument> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| {
        invalid("not a valid .docx file (password-protected and old .doc files are not supported)")
    })?;
    let main = main_part_name(&mut archive, limits);
    let document = read_part(&mut archive, &main, limits.max_part_bytes)?
        .ok_or_else(|| invalid(format!(".docx file has no {}", shown(&main))))?;
    let mut warnings = Vec::new();
    // Styles only name headings and lists: a broken styles part costs those,
    // never the document's text.
    let styles = match read_styles(&mut archive, &main, limits) {
        Ok(styles) => styles,
        Err(err) => {
            warnings.push(format!(
                "styles_unreadable: {err}; headings and lists set by styles were not detected"
            ));
            Styles::default()
        }
    };
    let xml = parse_xml(&document, &main, limits)?;
    let body = xml
        .root_element()
        .children()
        .find(|n| is_w(n, "body"))
        .ok_or_else(|| invalid("document.xml has no body"))?;
    let mut blocks = Vec::new();
    walk(body, &styles, &mut blocks);
    let title = blocks.iter().find_map(|b| match b {
        Block::Heading { level: 1, text } => Some(text.clone()),
        _ => None,
    });
    Ok(ParsedDocument {
        title,
        blocks,
        warnings,
    })
}

fn invalid(message: impl Into<String>) -> OragError {
    OragError::InvalidInput(message.into())
}

type Archive<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;

/// The main document part, from the package relationship (Word accepts any
/// part name there); `word/document.xml` when the relationship is missing or
/// names a part the file does not have.
fn main_part_name(archive: &mut Archive<'_>, limits: Limits) -> String {
    relationship_target(archive, "_rels/.rels", "", &OFFICE_DOCUMENT_RELS, limits)
        .filter(|name| archive.index_for_name(name).is_some())
        .unwrap_or_else(|| "word/document.xml".into())
}

/// A part name as shown in errors: quoted and escaped, and cut short, since
/// relationship targets come from the uploaded file.
fn shown(name: &str) -> String {
    let cut: String = name.chars().take(MAX_NAME_IN_ERROR).collect();
    let ellipsis = if cut.len() < name.len() { "…" } else { "" };
    format!("{:?}{ellipsis}", cut)
}

/// Target of the first relationship of `rel_type` in `rels_part`, resolved
/// against `base_dir`. Any problem reading it means "no relationship".
fn relationship_target(
    archive: &mut Archive<'_>,
    rels_part: &str,
    base_dir: &str,
    rel_types: &[&str],
    limits: Limits,
) -> Option<String> {
    let xml = read_part(archive, rels_part, limits.max_part_bytes).ok()??;
    let doc = parse_xml(&xml, rels_part, limits).ok()?;
    let target = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "Relationship")
        .find(|n| n.attribute("Type").is_some_and(|t| rel_types.contains(&t)))?
        .attribute("Target")?;
    Some(resolve_part(base_dir, target))
}

/// A relationship target as a ZIP entry name: absolute targets start at the
/// package root, relative ones at `base_dir`; `.` and `..` are resolved.
fn resolve_part(base_dir: &str, target: &str) -> String {
    let joined = match target.strip_prefix('/') {
        Some(absolute) => absolute.to_string(),
        None if base_dir.is_empty() => target.to_string(),
        None => format!("{base_dir}/{target}"),
    };
    let mut parts: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn read_styles(archive: &mut Archive<'_>, main: &str, limits: Limits) -> Result<Styles> {
    let (dir, file) = main.rsplit_once('/').unwrap_or(("", main));
    let rels = if dir.is_empty() {
        format!("_rels/{file}.rels")
    } else {
        format!("{dir}/_rels/{file}.rels")
    };
    let name = relationship_target(archive, &rels, dir, &STYLES_RELS, limits)
        .unwrap_or_else(|| "word/styles.xml".into());
    match read_part(archive, &name, limits.max_part_bytes)? {
        Some(xml) => styles(&parse_xml(&xml, &name, limits)?),
        None => Ok(Styles::default()),
    }
}

fn read_part(archive: &mut Archive<'_>, name: &str, limit: u64) -> Result<Option<String>> {
    let file = match archive.by_name(name) {
        Ok(file) => file,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(err) => {
            return Err(invalid(format!(
                "cannot read {} from .docx: {err}",
                shown(name)
            )));
        }
    };
    let too_large = || invalid(format!("{} is too large when decompressed", shown(name)));
    if file.size() > limit {
        return Err(too_large());
    }
    // The header's size can lie: the read itself stops one byte past the
    // limit, and the size is checked before the bytes are decoded.
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| invalid(format!("cannot read {} from .docx: {e}", shown(name))))?;
    if bytes.len() as u64 > limit {
        return Err(too_large());
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| invalid(format!("{} is not valid UTF-8", shown(name))))
}

/// Parses one XML part within the depth, namespace and node limits.
fn parse_xml<'i>(xml: &'i str, part: &str, limits: Limits) -> Result<roxmltree::Document<'i>> {
    let shape = xml_shape(xml, limits.max_depth);
    if shape.too_deep {
        return Err(invalid(format!(
            "{} is nested more than {} levels deep",
            shown(part),
            limits.max_depth
        )));
    }
    if shape.namespace_declarations > MAX_XML_NAMESPACES {
        return Err(invalid(format!(
            "{} declares more than {MAX_XML_NAMESPACES} XML namespaces",
            shown(part)
        )));
    }
    let options = roxmltree::ParsingOptions {
        nodes_limit: limits.max_nodes,
        ..roxmltree::ParsingOptions::default()
    };
    roxmltree::Document::parse_with_options(xml, options).map_err(|e| match e {
        roxmltree::Error::NodesLimitReached => invalid(format!(
            "{} has too many XML nodes (more than {})",
            shown(part),
            limits.max_nodes
        )),
        e => invalid(format!("malformed {}: {e}", shown(part))),
    })
}

/// What the XML parser would face, measured by a linear scan that never
/// recurses.
struct XmlShape {
    /// Elements nest deeper than the limit (the scan stops there).
    too_deep: bool,
    /// `xmlns` / `xmlns:p` attributes in start tags.
    namespace_declarations: usize,
}

/// Start tags (quotes in attributes respected) open a level, end tags close
/// one; comments, CDATA, processing instructions and self-closing tags do
/// not count.
fn xml_shape(xml: &str, max_depth: usize) -> XmlShape {
    let b = xml.as_bytes();
    let find = |from: usize, pattern: &[u8]| {
        b.get(from..)
            .and_then(|rest| rest.windows(pattern.len()).position(|w| w == pattern))
            .map_or(b.len(), |at| from + at + pattern.len())
    };
    let mut shape = XmlShape {
        too_deep: false,
        namespace_declarations: 0,
    };
    let (mut depth, mut i) = (0usize, 0usize);
    while let Some(at) = b
        .get(i..)
        .and_then(|rest| rest.iter().position(|&c| c == b'<'))
    {
        i += at;
        let rest = &b[i..];
        if rest.starts_with(b"<!--") {
            i = find(i + 4, b"-->");
        } else if rest.starts_with(b"<![CDATA[") {
            i = find(i + 9, b"]]>");
        } else if rest.starts_with(b"<?") {
            i = find(i + 2, b"?>");
        } else if rest.starts_with(b"<!") {
            i = find(i + 2, b">");
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            i = find(i + 2, b">");
        } else {
            let mut j = i + 1;
            let mut quote = None;
            while j < b.len() {
                let c = b[j];
                match quote {
                    Some(q) if c == q => quote = None,
                    Some(_) => {}
                    None if c == b'"' || c == b'\'' => quote = Some(c),
                    None if c == b'>' => break,
                    // An attribute name starts after whitespace.
                    None if c.is_ascii_whitespace() && b[j + 1..].starts_with(b"xmlns") => {
                        shape.namespace_declarations += 1;
                    }
                    None => {}
                }
                j += 1;
            }
            if j < b.len() && b[j - 1] != b'/' {
                depth += 1;
                if depth > max_depth {
                    shape.too_deep = true;
                    return shape;
                }
            }
            i = j + 1;
        }
    }
    shape
}

fn is_w(node: &Node<'_, '_>, name: &str) -> bool {
    node.is_element()
        && node.tag_name().name() == name
        && matches!(node.tag_name().namespace(), Some(W_NS | W_STRICT_NS))
}

/// A `w:` attribute in either the transitional or the strict namespace.
fn w_attr<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attribute((W_NS, name))
        .or_else(|| node.attribute((W_STRICT_NS, name)))
}

fn w_child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|n| is_w(n, name))
}

fn w_val(node: Node<'_, '_>) -> Option<String> {
    w_attr(node, "val").map(str::to_string)
}

/// What paragraph styles mean for structure.
#[derive(Debug, Default)]
struct Styles {
    /// Style id → heading level.
    headings: HashMap<String, u8>,
    /// Style ids whose paragraphs are numbered or bulleted (e.g. "List Bullet").
    lists: HashSet<String>,
}

/// Numbering set directly: `Some(true)` for a numbering id, `Some(false)` for
/// id 0 (Word's "numbering off"), `None` when there is no id (a bare level
/// override decides nothing; the style does).
fn numbering(props: Option<Node<'_, '_>>) -> Option<bool> {
    let num_pr = props.and_then(|pr| w_child(pr, "numPr"))?;
    let num_id = w_child(num_pr, "numId").and_then(w_val)?;
    Some(num_id != "0")
}

/// Longest `w:basedOn` chain followed; longer chains and cycles stop there.
const MAX_STYLE_INHERITANCE: usize = 16;

/// The first setting found along a style's `w:basedOn` chain (cycle-safe:
/// the chain is followed at most `MAX_STYLE_INHERITANCE` steps).
fn inherited<T>(
    entries: &HashMap<String, StyleEntry>,
    id: &str,
    own: impl Fn(&StyleEntry) -> Option<T>,
) -> Option<T> {
    let mut current = id;
    for _ in 0..MAX_STYLE_INHERITANCE {
        let entry = entries.get(current)?;
        if let Some(value) = own(entry) {
            return Some(value);
        }
        current = entry.based_on.as_deref()?;
    }
    None
}

/// One style's own settings, before inheritance.
#[derive(Default)]
struct StyleEntry {
    heading: Option<u8>,
    list: Option<bool>,
    based_on: Option<String>,
}

/// Paragraph style ids → heading levels, from style names ("heading 1".."heading 6",
/// "Title") or outline levels, and the list styles. Style *names* stay English
/// in localized Word (a Turkish "Başlık 1" has id `Balk1` but name `heading 1`).
/// A style without its own setting inherits it through `w:basedOn`, so a
/// template's "Bölüm Başlığı" based on Heading 1 is a heading too.
fn styles(doc: &roxmltree::Document<'_>) -> Result<Styles> {
    let mut entries: HashMap<String, StyleEntry> = HashMap::new();
    for style in doc.descendants().filter(|n| is_w(n, "style")) {
        let Some(id) = w_attr(style, "styleId") else {
            continue;
        };
        let name = w_child(style, "name")
            .and_then(w_val)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let props = w_child(style, "pPr");
        let outline = props
            .and_then(|p| w_child(p, "outlineLvl"))
            .and_then(w_val)
            .and_then(|v| v.parse::<u8>().ok());
        let heading = if name == "title" {
            Some(1)
        } else if let Some(n) = name
            .strip_prefix("heading ")
            .and_then(|n| n.trim().parse::<u8>().ok())
        {
            Some(n)
        } else {
            outline.and_then(outline_to_level)
        };
        entries.insert(
            id.to_string(),
            StyleEntry {
                heading: heading.filter(|l| (1..=6).contains(l)),
                list: numbering(props),
                based_on: w_child(style, "basedOn").and_then(w_val),
            },
        );
    }
    let mut found = Styles::default();
    for id in entries.keys() {
        if let Some(level) = inherited(&entries, id, |entry| entry.heading) {
            found.headings.insert(id.clone(), level);
        }
        if inherited(&entries, id, |entry| entry.list) == Some(true) {
            found.lists.insert(id.clone());
        }
    }
    Ok(found)
}

/// `w:outlineLvl` 0-5 → heading level 1-6; anything else (incl. 9 = body text) is not a heading.
fn outline_to_level(outline: u8) -> Option<u8> {
    (outline <= 5).then(|| outline + 1)
}

const MC_NS: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

/// Subtrees whose text must not be read as part of the enclosing paragraph:
/// text boxes (indexed as their own paragraphs), the duplicate `mc:Fallback`
/// rendering of alternate content, and the source side of tracked moves.
fn is_skipped_subtree(node: &Node<'_, '_>) -> bool {
    is_w(node, "txbxContent")
        || is_w(node, "moveFrom")
        || is_w(node, "delText")
        || (node.is_element()
            && node.tag_name().name() == "Fallback"
            && node.tag_name().namespace() == Some(MC_NS))
}

/// Text boxes anchored inside a paragraph, excluding `mc:Fallback` copies.
fn text_boxes<'a, 'i>(p: Node<'a, 'i>) -> Vec<Node<'a, 'i>> {
    p.descendants()
        .filter(|n| is_w(n, "txbxContent"))
        .filter(|n| {
            !n.ancestors()
                .skip(1)
                .take_while(|a| *a != p)
                .any(|a| is_skipped_subtree(&a))
        })
        .collect()
}

fn walk(node: Node<'_, '_>, styles: &Styles, blocks: &mut Vec<Block>) {
    for child in node.children().filter(Node::is_element) {
        if is_w(&child, "p") {
            paragraph(child, styles, blocks);
        } else if is_w(&child, "tbl") {
            table(child, blocks);
        } else if is_w(&child, "sdt") || is_w(&child, "sdtContent") || is_w(&child, "customXml") {
            walk(child, styles, blocks);
        }
    }
}

fn paragraph(p: Node<'_, '_>, styles: &Styles, blocks: &mut Vec<Block>) {
    let text = paragraph_text(p);
    if !text.is_empty() {
        blocks.push(classify(p, styles, text));
    }
    for text_box in text_boxes(p) {
        walk(text_box, styles, blocks);
    }
}

fn classify(p: Node<'_, '_>, styles: &Styles, text: String) -> Block {
    let props = w_child(p, "pPr");
    let style = props.and_then(|pr| w_child(pr, "pStyle")).and_then(w_val);
    // Direct paragraph properties override the style: an explicit outline
    // level 9 (body text) cancels a heading style, numId 0 a list style.
    let explicit_outline = props
        .and_then(|pr| w_child(pr, "outlineLvl"))
        .and_then(w_val)
        .and_then(|v| v.parse::<u8>().ok());
    let level = match explicit_outline {
        Some(outline) => outline_to_level(outline),
        None => style
            .as_ref()
            .and_then(|id| styles.headings.get(id).copied()),
    };
    let is_list = numbering(props)
        .unwrap_or_else(|| style.as_ref().is_some_and(|id| styles.lists.contains(id)));
    match level {
        Some(level) => Block::Heading { level, text },
        None if is_list => Block::ListItem(text),
        None => Block::Paragraph(text),
    }
}

/// Visible text of a paragraph: `w:t` runs (insertions and move targets
/// included), tabs and breaks as whitespace, collapsed to single spaces.
/// Skipped subtrees (see `is_skipped_subtree`) contribute nothing.
fn paragraph_text(p: Node<'_, '_>) -> String {
    let mut raw = String::new();
    collect_text(p, &mut raw);
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect_text(node: Node<'_, '_>, out: &mut String) {
    for child in node.children().filter(Node::is_element) {
        if is_skipped_subtree(&child) {
            continue;
        }
        if is_w(&child, "t") {
            out.push_str(child.text().unwrap_or(""));
        } else if is_w(&child, "noBreakHyphen") {
            out.push('-');
        } else if is_w(&child, "softHyphen") {
            // Invisible unless a line breaks there: the word stays whole.
        } else if is_w(&child, "tab") || is_w(&child, "br") || is_w(&child, "cr") {
            out.push(' ');
        } else {
            collect_text(child, out);
        }
    }
}

fn table(tbl: Node<'_, '_>, blocks: &mut Vec<Block>) {
    let rows: Vec<String> = structural_children(tbl, "tr")
        .into_iter()
        .map(|tr| {
            structural_children(tr, "tc")
                .into_iter()
                .map(cell_text)
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .filter(|row| !row.trim_matches(|c| c == ' ' || c == '|').is_empty())
        .collect();
    if !rows.is_empty() {
        blocks.push(Block::Table(rows.join("\n")));
    }
}

/// Children named `name`, looking through content-control and custom-XML
/// wrappers (`w:sdt`, `w:sdtContent`, `w:customXml`) but not into other elements.
fn structural_children<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Vec<Node<'a, 'i>> {
    let mut found = Vec::new();
    for child in node.children().filter(Node::is_element) {
        if is_w(&child, name) {
            found.push(child);
        } else if is_w(&child, "sdt") || is_w(&child, "sdtContent") || is_w(&child, "customXml") {
            found.extend(structural_children(child, name));
        }
    }
    found
}

/// Cell text: its paragraphs (and nested tables) in order, read with the same
/// rules as body text, so text boxes and fallback copies are not duplicated.
/// One line per row: a nested table's line breaks become spaces, and a
/// literal `|` is escaped as `\|`, as the Markdown parser does.
fn cell_text(tc: Node<'_, '_>) -> String {
    let mut blocks = Vec::new();
    walk(tc, &Styles::default(), &mut blocks);
    blocks
        .iter()
        .map(Block::text)
        .collect::<Vec<_>>()
        .join(" ")
        .replace('\n', " ")
        .replace('|', "\\|")
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    use super::*;

    const NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

    /// Minimal .docx with the given body XML and optional styles XML.
    pub(crate) fn docx(body: &str, styles: Option<&str>) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("word/document.xml", options).unwrap();
            write!(zip, r#"<?xml version="1.0" encoding="UTF-8"?><w:document {NS}><w:body>{body}</w:body></w:document>"#).unwrap();
            if let Some(styles) = styles {
                zip.start_file("word/styles.xml", options).unwrap();
                write!(
                    zip,
                    r#"<?xml version="1.0" encoding="UTF-8"?><w:styles {NS}>{styles}</w:styles>"#
                )
                .unwrap();
            }
            zip.finish().unwrap();
        }
        out.into_inner()
    }

    fn p(style: Option<&str>, text: &str) -> String {
        let props = style
            .map(|s| format!(r#"<w:pPr><w:pStyle w:val="{s}"/></w:pPr>"#))
            .unwrap_or_default();
        format!("<w:p>{props}<w:r><w:t>{text}</w:t></w:r></w:p>")
    }

    #[test]
    fn pandoc_fixture_has_headings_paragraphs_lists_and_tables() {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample.docx"
        ))
        .unwrap();
        let doc = parse_docx(&bytes).unwrap();
        assert_eq!(doc.title.as_deref(), Some("Kargo Politikası"));
        assert!(
            doc.blocks.contains(&Block::Heading {
                level: 2,
                text: "İade Koşulları".into()
            }),
            "{:?}",
            doc.blocks
        );
        assert!(doc.blocks.contains(&Block::Paragraph(
            "Ürünler 14 gün içinde iade edilebilir.".into()
        )));
        assert!(
            doc.blocks
                .contains(&Block::ListItem("Orijinal ambalaj".into()))
        );
        assert!(
            doc.blocks
                .iter()
                .any(|b| matches!(b, Block::Table(t) if t.contains("500 TL altı | 49,90 TL"))),
            "{:?}",
            doc.blocks
        );
    }

    #[test]
    fn localized_heading_style_ids_are_recognized_by_name() {
        let styles = r#"<w:style w:type="paragraph" w:styleId="Balk1"><w:name w:val="heading 1"/></w:style>"#;
        let doc = parse_docx(&docx(&p(Some("Balk1"), "Giriş"), Some(styles))).unwrap();
        assert_eq!(
            doc.blocks,
            vec![Block::Heading {
                level: 1,
                text: "Giriş".into()
            }]
        );
        assert_eq!(doc.title.as_deref(), Some("Giriş"));
    }

    #[test]
    fn entities_tabs_and_breaks_become_text() {
        let body = r#"<w:p><w:r><w:t>A &amp; B</w:t><w:tab/><w:t>&#304;stanbul</w:t><w:br/><w:t>son</w:t></w:r></w:p>"#;
        let doc = parse_docx(&docx(body, None)).unwrap();
        assert_eq!(
            doc.blocks,
            vec![Block::Paragraph("A & B İstanbul son".into())]
        );
    }

    #[test]
    fn tracked_deletions_are_skipped_and_insertions_kept() {
        let body = r#"<w:p><w:del><w:r><w:delText>eski</w:delText></w:r></w:del><w:ins><w:r><w:t>yeni</w:t></w:r></w:ins></w:p>"#;
        let doc = parse_docx(&docx(body, None)).unwrap();
        assert_eq!(doc.blocks, vec![Block::Paragraph("yeni".into())]);
    }

    #[test]
    fn text_boxes_are_separate_paragraphs_and_fallback_copies_are_ignored() {
        let body = r#"<w:p xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><w:r><w:t>Gövde</w:t></w:r><w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><w:txbxContent><w:p><w:r><w:t>İade 14 gün</w:t></w:r></w:p></w:txbxContent></w:drawing></mc:Choice><mc:Fallback><w:pict><w:txbxContent><w:p><w:r><w:t>İade 14 gün</w:t></w:r></w:p></w:txbxContent></w:pict></mc:Fallback></mc:AlternateContent></w:r></w:p>"#;
        let doc = parse_docx(&docx(body, None)).unwrap();
        assert_eq!(
            doc.blocks,
            vec![
                Block::Paragraph("Gövde".into()),
                Block::Paragraph("İade 14 gün".into())
            ]
        );
    }

    #[test]
    fn tracked_moves_keep_only_the_destination() {
        let body = r#"<w:p><w:moveFrom><w:r><w:t>taşınan</w:t></w:r></w:moveFrom></w:p><w:p><w:moveTo><w:r><w:t>taşınan</w:t></w:r></w:moveTo></w:p>"#;
        let doc = parse_docx(&docx(body, None)).unwrap();
        assert_eq!(doc.blocks, vec![Block::Paragraph("taşınan".into())]);
    }

    #[test]
    fn out_of_range_outline_levels_do_not_overflow() {
        let body = r#"<w:p><w:pPr><w:outlineLvl w:val="255"/></w:pPr><w:r><w:t>x</w:t></w:r></w:p><w:p><w:pPr><w:outlineLvl w:val="1"/></w:pPr><w:r><w:t>y</w:t></w:r></w:p>"#;
        let doc = parse_docx(&docx(body, None)).unwrap();
        assert_eq!(
            doc.blocks,
            vec![
                Block::Paragraph("x".into()),
                Block::Heading {
                    level: 2,
                    text: "y".into()
                }
            ]
        );
    }

    #[test]
    fn table_rows_and_cells_inside_content_controls_are_kept() {
        let cell = |t: &str| format!("<w:tc>{}</w:tc>", p(None, t));
        let body = format!(
            "<w:tbl><w:tr>{}{}</w:tr><w:sdt><w:sdtContent><w:tr>{}<w:sdt><w:sdtContent>{}</w:sdtContent></w:sdt></w:tr></w:sdtContent></w:sdt></w:tbl>",
            cell("Ad"),
            cell("Değer"),
            cell("süre"),
            cell("14 gün")
        );
        let doc = parse_docx(&docx(&body, None)).unwrap();
        assert_eq!(
            doc.blocks,
            vec![Block::Table("Ad | Değer\nsüre | 14 gün".into())]
        );
    }

    #[test]
    fn text_box_in_a_table_cell_is_read_once() {
        let body = r#"<w:tbl xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><w:tr><w:tc><w:p><w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><w:txbxContent><w:p><w:r><w:t>kutu</w:t></w:r></w:p></w:txbxContent></w:drawing></mc:Choice><mc:Fallback><w:pict><w:txbxContent><w:p><w:r><w:t>kutu</w:t></w:r></w:p></w:txbxContent></w:pict></mc:Fallback></mc:AlternateContent></w:r></w:p></w:tc></w:tr></w:tbl>"#;
        let doc = parse_docx(&docx(body, None)).unwrap();
        assert_eq!(doc.blocks, vec![Block::Table("kutu".into())]);
    }

    #[test]
    fn content_controls_are_traversed() {
        let body = format!(
            "<w:sdt><w:sdtContent>{}</w:sdtContent></w:sdt>",
            p(None, "içerik")
        );
        let doc = parse_docx(&docx(&body, None)).unwrap();
        assert_eq!(doc.blocks, vec![Block::Paragraph("içerik".into())]);
    }

    #[test]
    fn docx_that_is_not_a_zip_is_rejected() {
        let err = parse_docx(b"<html>not a docx</html>")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a valid .docx"), "{err}");
    }

    #[test]
    fn docx_bomb_is_rejected() {
        let filler = "a".repeat(2 * 1024 * 1024);
        let bytes = docx(&p(None, &filler), None);
        let err = parse_docx_with_limit(&bytes, 1024 * 1024)
            .unwrap_err()
            .to_string();
        assert!(err.contains("too large when decompressed"), "{err}");
    }

    /// A .docx built from raw `(part name, content)` pairs.
    fn zip_parts(parts: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut out);
            for (name, content) in parts {
                zip.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(content.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        out.into_inner()
    }

    fn document_xml(body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:document {NS}><w:body>{body}</w:body></w:document>"#
        )
    }

    #[test]
    fn deeply_nested_xml_is_rejected_without_overflowing_the_stack() {
        let depth = 100_000;
        let body = format!("{}{}", "<w:sdt>".repeat(depth), "</w:sdt>".repeat(depth));
        let err = parse_docx(&docx(&body, None)).unwrap_err();
        assert!(err.to_string().contains("nested"), "{err}");
        // A deep styles.xml must not crash either (styles are optional).
        let styles = format!("{}{}", "<w:x>".repeat(depth), "</w:x>".repeat(depth));
        let doc = parse_docx(&docx(&p(None, "metin"), Some(&styles))).unwrap();
        assert_eq!(doc.blocks, vec![Block::Paragraph("metin".into())]);
    }

    #[test]
    fn xml_with_too_many_nodes_is_rejected() {
        let body = p(None, "x").repeat(200);
        let limits = Limits {
            max_nodes: 100,
            ..Limits::default()
        };
        let err = parse_docx_with(&docx(&body, None), limits).unwrap_err();
        assert!(err.to_string().contains("too many"), "{err}");
        assert!(parse_docx(&docx(&body, None)).is_ok());
    }

    /// A .docx whose document.xml declares an uncompressed size of 10 bytes
    /// (local and central headers), whatever its real size.
    fn with_forged_size(body: &str) -> Vec<u8> {
        let xml = document_xml(body);
        let mut bytes = zip_parts(&[("word/document.xml", &xml)]);
        let (real, fake) = ((xml.len() as u32).to_le_bytes(), 10u32.to_le_bytes());
        let mut patched = 0;
        for i in 0..bytes.len().saturating_sub(4) {
            if bytes[i..i + 4] == real {
                bytes[i..i + 4].copy_from_slice(&fake);
                patched += 1;
            }
        }
        assert!(patched >= 2, "size fields not found");
        bytes
    }

    #[test]
    fn a_size_header_that_lies_cannot_bypass_the_part_limit() {
        let bytes = with_forged_size(&p(None, &"a".repeat(5000)));
        assert!(parse_docx_with_limit(&bytes, 1000).is_err());
    }

    #[test]
    fn a_limit_inside_a_multibyte_character_still_reports_the_size() {
        // Past the header check (it says 10 bytes), the streaming read stops
        // inside a two-byte "ş": the error is the size, not the encoding.
        let bytes = with_forged_size(&p(None, &"ş".repeat(2000)));
        let limit = (document_xml("").len() + 1001) as u64;
        let err = parse_docx_with_limit(&bytes, limit).unwrap_err();
        assert!(err.to_string().contains("too large"), "{err}");
    }

    #[test]
    fn unreadable_styles_are_a_warning_not_a_failure() {
        let doc = parse_docx(&docx(&p(Some("Heading1"), "Başlık"), Some("<w:style"))).unwrap();
        assert_eq!(doc.blocks, vec![Block::Paragraph("Başlık".into())]);
        assert!(
            doc.warnings
                .iter()
                .any(|w| w.starts_with("styles_unreadable")),
            "{:?}",
            doc.warnings
        );
    }

    #[test]
    fn the_main_part_is_found_through_its_relationship() {
        let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document2.xml"/></Relationships>"#;
        let bytes = zip_parts(&[
            ("_rels/.rels", rels),
            (
                "word/document2.xml",
                &document_xml(&p(None, "ikinci ana bölüm")),
            ),
        ]);
        let doc = parse_docx(&bytes).unwrap();
        assert_eq!(
            doc.blocks,
            vec![Block::Paragraph("ikinci ana bölüm".into())]
        );
    }

    #[test]
    fn table_cells_keep_one_row_per_line_and_escape_pipes() {
        let inner = "<w:tbl><w:tr><w:tc><w:p><w:r><w:t>i1</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>i2</w:t></w:r></w:p></w:tc></w:tr></w:tbl>";
        let body = format!(
            "<w:tbl><w:tr><w:tc>{}</w:tc><w:tc>{inner}</w:tc></w:tr></w:tbl>",
            p(None, "A|B")
        );
        let doc = parse_docx(&docx(&body, None)).unwrap();
        assert_eq!(doc.blocks, vec![Block::Table("A\\|B | i1 i2".into())]);
    }

    #[test]
    fn list_and_heading_properties_follow_word_rules() {
        let styles = r#"<w:style w:type="paragraph" w:styleId="ListBullet"><w:name w:val="List Bullet"/><w:pPr><w:numPr><w:numId w:val="3"/></w:numPr></w:pPr></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/></w:style>"#;
        let body = format!(
            "{}{}{}",
            p(Some("ListBullet"), "madde"),
            r#"<w:p><w:pPr><w:numPr><w:numId w:val="0"/></w:numPr></w:pPr><w:r><w:t>numarasız</w:t></w:r></w:p>"#,
            r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/><w:outlineLvl w:val="9"/></w:pPr><w:r><w:t>gövde</w:t></w:r></w:p>"#,
        );
        let doc = parse_docx(&docx(&body, Some(styles))).unwrap();
        assert_eq!(
            doc.blocks,
            vec![
                Block::ListItem("madde".into()),
                Block::Paragraph("numarasız".into()),
                Block::Paragraph("gövde".into()),
            ]
        );
        assert_eq!(doc.title, None);
    }

    #[test]
    fn mass_namespace_declarations_are_rejected_before_parsing() {
        let decls: String = (0..5000).map(|i| format!(r#" xmlns:p{i}="u""#)).collect();
        let children = r#"<a xmlns:z="v"/>"#.repeat(20);
        let xml = format!(
            r#"<?xml version="1.0"?><w:document {NS}{decls}><w:body>{children}</w:body></w:document>"#
        );
        let bytes = zip_parts(&[("word/document.xml", &xml)]);
        let started = std::time::Instant::now();
        let err = parse_docx(&bytes).unwrap_err();
        assert!(err.to_string().contains("namespace"), "{err}");
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn strict_ooxml_documents_are_read() {
        const STRICT: &str = r#"xmlns:w="http://purl.oclc.org/ooxml/wordprocessingml/main""#;
        let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
        let doc_rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://purl.oclc.org/ooxml/officeDocument/relationships/styles" Target="styles.xml"/></Relationships>"#;
        let styles = format!(
            r#"<?xml version="1.0"?><w:styles {STRICT}><w:style w:type="paragraph" w:styleId="H"><w:name w:val="heading 1"/></w:style></w:styles>"#
        );
        let document = format!(
            r#"<?xml version="1.0"?><w:document {STRICT}><w:body><w:p><w:pPr><w:pStyle w:val="H"/></w:pPr><w:r><w:t>Katı Başlık</w:t></w:r></w:p></w:body></w:document>"#
        );
        let bytes = zip_parts(&[
            ("_rels/.rels", rels),
            ("word/_rels/document.xml.rels", doc_rels),
            ("word/document.xml", &document),
            ("word/styles.xml", &styles),
        ]);
        let doc = parse_docx(&bytes).unwrap();
        assert_eq!(doc.title.as_deref(), Some("Katı Başlık"));
    }

    #[test]
    fn styles_are_found_through_the_document_relationships() {
        let doc_rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="../custom/stil.xml"/></Relationships>"#;
        let styles = format!(
            r#"<?xml version="1.0"?><w:styles {NS}><w:style w:type="paragraph" w:styleId="H"><w:name w:val="heading 2"/></w:style></w:styles>"#
        );
        let bytes = zip_parts(&[
            ("word/_rels/document.xml.rels", doc_rels),
            ("word/document.xml", &document_xml(&p(Some("H"), "Bölüm"))),
            ("custom/stil.xml", &styles),
        ]);
        let doc = parse_docx(&bytes).unwrap();
        assert_eq!(
            doc.blocks,
            vec![Block::Heading {
                level: 2,
                text: "Bölüm".into()
            }]
        );
    }

    #[test]
    fn relationship_targets_resolve_inside_the_package() {
        assert_eq!(resolve_part("word", "styles.xml"), "word/styles.xml");
        assert_eq!(resolve_part("word", "/word/styles.xml"), "word/styles.xml");
        assert_eq!(resolve_part("word", "../custom/s.xml"), "custom/s.xml");
        assert_eq!(resolve_part("word", "../../../etc/passwd"), "etc/passwd");
        assert_eq!(resolve_part("", "word/document.xml"), "word/document.xml");
    }

    #[test]
    fn a_bad_main_part_relationship_falls_back_and_errors_stay_short() {
        let target = format!("missing/{}\nline", "x".repeat(10_000));
        let rels = format!(
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="{target}"/></Relationships>"#
        );
        // The target does not exist, but word/document.xml does: it is used.
        let bytes = zip_parts(&[
            ("_rels/.rels", &rels),
            ("word/document.xml", &document_xml(&p(None, "yedek"))),
        ]);
        assert_eq!(
            parse_docx(&bytes).unwrap().blocks,
            vec![Block::Paragraph("yedek".into())]
        );
        // Neither exists: the error names the part briefly, without the raw line break.
        let err = parse_docx(&zip_parts(&[("_rels/.rels", &rels)]))
            .unwrap_err()
            .to_string();
        assert!(err.len() < 300 && !err.contains('\n'), "{err}");
    }

    #[test]
    fn hyphen_elements_keep_their_meaning() {
        let body = r#"<w:p><w:r><w:t>e</w:t><w:noBreakHyphen/><w:t>fatura ay</w:t><w:softHyphen/><w:t>rıntı</w:t></w:r></w:p>"#;
        let doc = parse_docx(&docx(body, None)).unwrap();
        assert_eq!(
            doc.blocks,
            vec![Block::Paragraph("e-fatura ayrıntı".into())]
        );
    }

    #[test]
    fn numbering_without_a_num_id_and_style_inheritance() {
        let styles = r#"<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/></w:style><w:style w:type="paragraph" w:styleId="BolumBasligi"><w:name w:val="Bölüm Başlığı"/><w:basedOn w:val="Heading1"/></w:style><w:style w:type="paragraph" w:styleId="Liste"><w:name w:val="Liste"/><w:pPr><w:numPr><w:numId w:val="4"/></w:numPr></w:pPr></w:style><w:style w:type="paragraph" w:styleId="AltListe"><w:name w:val="Alt Liste"/><w:basedOn w:val="Liste"/></w:style><w:style w:type="paragraph" w:styleId="A"><w:basedOn w:val="B"/></w:style><w:style w:type="paragraph" w:styleId="B"><w:basedOn w:val="A"/></w:style>"#;
        let body = format!(
            "{}{}{}{}",
            p(Some("BolumBasligi"), "Şirket Başlığı"),
            p(Some("AltListe"), "alt madde"),
            r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/></w:numPr></w:pPr><w:r><w:t>düz metin</w:t></w:r></w:p>"#,
            p(Some("A"), "döngü"),
        );
        let doc = parse_docx(&docx(&body, Some(styles))).unwrap();
        assert_eq!(
            doc.blocks,
            vec![
                Block::Heading {
                    level: 1,
                    text: "Şirket Başlığı".into()
                },
                Block::ListItem("alt madde".into()),
                Block::Paragraph("düz metin".into()),
                Block::Paragraph("döngü".into()),
            ]
        );
    }
}
