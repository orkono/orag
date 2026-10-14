//! Heading lines in plain text and PDF page text, which carry no markup.
//!
//! Only lines that read as titles count: short, not ending like a sentence, and
//! either written in capitals, framed as `-- name --`, or numbered as an
//! outline with a Roman numeral or a letter (`IV.`, `A.`). Levels nest the way
//! Turkish statutes and catalogues do: part > chapter > capitals > dashed >
//! Roman > letter. Decimal numbering (`1.`, `2.3`) is left as body text: in
//! manuals it numbers steps far more often than titles, and it gained nothing
//! in the step 52 measurement.
//!
//! `Outline` reads the lines of one text in order, because some decisions need
//! the line before: an outline number only starts a heading after a finished
//! sentence (else `M. Kemal` in wrapped prose would), `I.` after `H.` is a
//! letter, a chapter's name on the next line belongs to the chapter, and a
//! title in capitals wrapped over two lines is one title.

/// Longer lines are prose wrapped by the source, not titles.
const MAX_CHARS: usize = 80;
const MAX_WORDS: usize = 12;
/// Capitals with fewer letters are acronyms or codes (`KDV`, `PN 10`).
const MIN_LETTERS: usize = 4;
/// A chapter's name on the line after it is at most this long.
const MAX_NAME_WORDS: usize = 8;

/// Turkish ordinals come first (`BİRİNCİ KISIM`); a number follows (`BÖLÜM 2`).
const PART_WORDS: [&str; 2] = ["KISIM", "PART"];
const CHAPTER_WORDS: [&str; 3] = ["BÖLÜM", "CHAPTER", "SECTION"];
const ORDINAL_LAST: [&str; 2] = ["KISIM", "BÖLÜM"];
/// Labels of a warning box, set in capitals but not section titles.
const ADMONITIONS: [&str; 9] = [
    "UYARI",
    "DİKKAT",
    "ÖNEMLİ",
    "TEHLİKE",
    "WARNING",
    "CAUTION",
    "NOTE",
    "IMPORTANT",
    "DANGER",
];
use std::collections::{HashMap, HashSet};

use crate::domain::document::Block;

const SENTENCE_END: [char; 6] = ['.', ',', ';', ':', '!', '?'];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Capitals with a part or chapter word: its name may follow.
    Division,
    Capitals,
    Dashed,
    Roman,
    Letter,
}

impl Kind {
    fn level(self, division: u8) -> u8 {
        match self {
            Kind::Division => division,
            Kind::Capitals => 3,
            Kind::Dashed => 4,
            Kind::Roman => 5,
            Kind::Letter => 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Line {
    Blank,
    Body,
    Heading {
        level: u8,
        text: String,
    },
    /// More text of the heading just emitted (a wrapped title, a chapter name).
    ExtendHeading(String),
    /// The heading just emitted was the start of a block of lines in capitals
    /// (labels, a paragraph in capitals): it and this line are body text.
    RetractHeading,
}

/// Reads one text's lines in order and says what each one is.
#[derive(Debug)]
pub(crate) struct Outline {
    /// The previous non-blank line ended a sentence or was a heading.
    after_complete_line: bool,
    last_letter: Option<char>,
    /// The kind of the last heading while no body has followed it.
    open: Option<Kind>,
    /// Lines in capitals joined into the open title so far.
    capitals_run: usize,
}

impl Default for Outline {
    fn default() -> Self {
        Self {
            after_complete_line: true,
            last_letter: None,
            open: None,
            capitals_run: 0,
        }
    }
}

impl Outline {
    pub(crate) fn line(&mut self, line: &str) -> Line {
        let line = line.trim();
        if line.is_empty() {
            // A block in capitals ends at a blank line.
            if self.capitals_run > MAX_TITLE_LINES {
                self.capitals_run = 0;
            }
            return Line::Blank;
        }
        if self.capitals_run > MAX_TITLE_LINES {
            // Inside a block in capitals: everything up to a blank line is body.
            if capitals(line).is_some() {
                return Line::Body;
            }
            self.capitals_run = 0;
        }
        if let Some((kind, division, text)) = self.classify(line) {
            if let Some(extra) = self.extension(kind, &text) {
                return extra;
            }
            self.after_complete_line = true;
            self.open = Some(kind);
            self.capitals_run = usize::from(kind == Kind::Capitals);
            match kind {
                Kind::Letter => self.last_letter = line.chars().next(),
                Kind::Roman => self.last_letter = None,
                _ => {}
            }
            return Line::Heading {
                level: kind.level(division),
                text,
            };
        }
        if self.open == Some(Kind::Division) && is_name(line) {
            self.open = None;
            return Line::ExtendHeading(format!(" - {line}"));
        }
        self.open = None;
        self.capitals_run = 0;
        if letters(line) > 0 {
            self.after_complete_line = line.ends_with(SENTENCE_END) || line.ends_with(')');
        }
        Line::Body
    }

    /// What to do with the open heading when `text` continues it: append a
    /// chapter's name in capitals or the second line of a wrapped title, or
    /// take the title back when a third line in capitals shows a block.
    fn extension(&mut self, kind: Kind, text: &str) -> Option<Line> {
        if kind != Kind::Capitals {
            return None;
        }
        match self.open? {
            Kind::Division => {
                self.open = None;
                Some(Line::ExtendHeading(format!(" - {text}")))
            }
            Kind::Capitals if self.capitals_run < MAX_TITLE_LINES => {
                self.capitals_run += 1;
                Some(Line::ExtendHeading(format!(" {text}")))
            }
            Kind::Capitals => {
                self.open = None;
                self.capitals_run = MAX_TITLE_LINES + 1;
                self.after_complete_line = false;
                Some(Line::RetractHeading)
            }
            _ => None,
        }
    }

    fn classify(&self, line: &str) -> Option<(Kind, u8, String)> {
        let words = line.split_whitespace().count();
        if words > MAX_WORDS
            || line.chars().count() > MAX_CHARS
            || has_columns(line)
            || line.ends_with(SENTENCE_END)
            || broken_word(line)
        {
            return None;
        }
        if let Some(inner) = dashed(line) {
            return Some((Kind::Dashed, 0, clean(inner)));
        }
        if let Some(division) = capitals(line) {
            let kind = if division < 3 {
                Kind::Division
            } else {
                Kind::Capitals
            };
            return Some((kind, division, clean(line)));
        }
        if !self.after_complete_line {
            return None;
        }
        let kind = enumerated(line, self.last_letter)?;
        Some((kind, 0, clean(line)))
    }
}

/// A title in capitals spans at most this many lines.
const MAX_TITLE_LINES: usize = 2;
/// Text under a title in capitals has at least this many letters.
const MIN_BODY_LETTERS: usize = 20;
/// A paragraph that starts with this many words in capitals continues a
/// paragraph in capitals rather than following a title.
const CAPITAL_LEAD_WORDS: usize = 3;

/// The line ends in a word broken by a hyphen (`MER-`), not in a code
/// (`DS-1600`) or a dash standing alone.
fn broken_word(line: &str) -> bool {
    line.strip_suffix('-')
        .is_some_and(|rest| rest.ends_with(char::is_alphabetic))
}

/// `blocks` with titles in capitals turned back into body text when what
/// follows is not real text: a short label, or a paragraph that carries on in
/// capitals (the title was the first line of it). Part and chapter headings,
/// and a title followed by another heading, are kept.
pub(crate) fn demote_unsupported_titles(blocks: Vec<Block>) -> Vec<Block> {
    let unsupported: Vec<bool> = blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            matches!(block, Block::Heading { level: 3, .. })
                && match blocks.get(index + 1) {
                    Some(Block::Paragraph(next)) => !supports_title(next),
                    _ => false,
                }
        })
        .collect();
    blocks
        .into_iter()
        .zip(unsupported)
        .map(|(block, demote)| match block {
            Block::Heading { text, .. } if demote => Block::Paragraph(text),
            other => other,
        })
        .collect()
}

fn supports_title(text: &str) -> bool {
    let capital_lead = text
        .split_whitespace()
        .take(CAPITAL_LEAD_WORDS)
        .filter(|word| letters(word) > 0 && !word.chars().any(char::is_lowercase))
        .count();
    letters(text) >= MIN_BODY_LETTERS && capital_lead < CAPITAL_LEAD_WORDS
}

/// A heading text repeated on this many pages, and on at least half of the
/// pages with text, is a running header or footer, not a section title.
const RUNNING_HEADER_PAGES: usize = 3;

/// `blocks` with running headers turned back into body text, so a header
/// printed on every page neither replaces the real section heading nor cuts
/// a chunk at every page. `pages` is parallel to `blocks`.
pub(crate) fn demote_running_headers(blocks: Vec<Block>, pages: &[Option<u32>]) -> Vec<Block> {
    let mut pages_of: HashMap<&str, HashSet<Option<u32>>> = HashMap::new();
    for (block, page) in blocks.iter().zip(pages) {
        if let Block::Heading { text, .. } = block {
            pages_of.entry(text.as_str()).or_default().insert(*page);
        }
    }
    let text_pages = pages.iter().collect::<HashSet<_>>().len();
    let running: HashSet<String> = pages_of
        .into_iter()
        .filter(|(_, on)| on.len() >= RUNNING_HEADER_PAGES && on.len() * 2 >= text_pages)
        .map(|(text, _)| text.to_string())
        .collect();
    if running.is_empty() {
        return blocks;
    }
    blocks
        .into_iter()
        .map(|block| match block {
            Block::Heading { text, .. } if running.contains(&text) => Block::Paragraph(text),
            other => other,
        })
        .collect()
}

/// A tab, a ` | ` cell separator, or two gaps of three spaces: a table row.
fn has_columns(line: &str) -> bool {
    line.contains('\t') || line.contains(" | ") || line.matches("   ").count() >= 2
}

/// The name inside `-- name --`, the section marker of plain-text lists.
fn dashed(line: &str) -> Option<&str> {
    let inner = line.strip_prefix("--")?.strip_suffix("--")?.trim();
    (letters(inner) >= MIN_LETTERS).then_some(inner)
}

/// The level of a line without a single lower-case letter: 1 or 2 for a part
/// or chapter, else 3.
fn capitals(line: &str) -> Option<u8> {
    if letters(line) < MIN_LETTERS || line.chars().any(char::is_lowercase) {
        return None;
    }
    let words: Vec<&str> = line
        .split_whitespace()
        .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|word| !word.is_empty())
        .collect();
    if let [word] = words.as_slice()
        && ADMONITIONS.contains(word)
    {
        return None;
    }
    let division = |keywords: &[&str]| {
        let numbered_first = matches!(words.as_slice(), [first, next, ..]
            if keywords.contains(first) && is_number(next));
        let ordinal_last = words.len() >= 2
            && words
                .last()
                .is_some_and(|last| keywords.contains(last) && ORDINAL_LAST.contains(last));
        numbered_first || ordinal_last
    };
    Some(if division(&PART_WORDS) {
        1
    } else if division(&CHAPTER_WORDS) {
        2
    } else {
        3
    })
}

fn is_number(word: &str) -> bool {
    word.starts_with(|c: char| c.is_ascii_digit()) || is_roman(word)
}

fn is_roman(word: &str) -> bool {
    !word.is_empty() && word.len() <= 6 && word.chars().all(|c| "IVXLC".contains(c))
}

/// A Roman numeral or a letter followed by a capitalised title. A lone `I`,
/// `V` or `X` right after the letter before it (`H.`, `U.`/`Ü.`, `W.`)
/// continues a lettered list.
fn enumerated(line: &str, last_letter: Option<char>) -> Option<Kind> {
    let (marker, title) = line.split_once(char::is_whitespace)?;
    if !title.trim_start().starts_with(char::is_uppercase) || letters(title) < 2 {
        return None;
    }
    let body = marker.strip_suffix('.')?;
    let mut chars = body.chars();
    let single = match (chars.next(), chars.next()) {
        (Some(letter), None) if letter.is_uppercase() => Some(letter),
        _ => None,
    };
    let continues_letters = single.zip(last_letter).is_some_and(|(letter, last)| {
        matches!((last, letter), ('H', 'I') | ('U' | 'Ü', 'V') | ('W', 'X'))
    });
    if is_roman(body) && !continues_letters && (body.len() > 1 || "IVX".contains(body)) {
        Some(Kind::Roman)
    } else if single.is_some() {
        Some(Kind::Letter)
    } else {
        None
    }
}

/// A short capitalised line with no sentence mark: the name of a chapter.
fn is_name(line: &str) -> bool {
    line.split_whitespace().count() <= MAX_NAME_WORDS
        && line.starts_with(char::is_uppercase)
        && letters(line) >= 3
        && !line.ends_with(SENTENCE_END)
        && !has_columns(line)
}

/// Whitespace collapsed, and a footnote number glued to the last word
/// (`ANAYASASI1`, `kesinhesap26`) dropped.
fn clean(text: &str) -> String {
    let mut words: Vec<&str> = text.split_whitespace().collect();
    if let Some(last) = words.last_mut() {
        let stem = last.trim_end_matches(|c: char| c.is_ascii_digit());
        if stem.len() < last.len()
            && letters(stem) >= MIN_LETTERS
            && stem.ends_with(char::is_alphabetic)
        {
            *last = stem;
        }
    }
    words.join(" ")
}

fn letters(text: &str) -> usize {
    text.chars().filter(|c| c.is_alphabetic()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `line` is at the start of a text.
    fn first(line: &str) -> Line {
        Outline::default().line(line)
    }

    fn heading(level: u8, text: &str) -> Line {
        Line::Heading {
            level,
            text: text.into(),
        }
    }

    fn read(lines: &[&str]) -> Vec<Line> {
        let mut outline = Outline::default();
        lines.iter().map(|line| outline.line(line)).collect()
    }

    #[test]
    fn upper_case_lines_are_headings_ranked_by_a_part_or_chapter_word() {
        assert_eq!(first("BİRİNCİ KISIM  "), heading(1, "BİRİNCİ KISIM"));
        assert_eq!(first("PART 1 - GENERAL"), heading(1, "PART 1 - GENERAL"));
        assert_eq!(first("İKİNCİ BÖLÜM"), heading(2, "İKİNCİ BÖLÜM"));
        assert_eq!(
            first("BÖLÜM 2 - BAĞLANTI PARÇALARI"),
            heading(2, "BÖLÜM 2 - BAĞLANTI PARÇALARI")
        );
        assert_eq!(
            first("CHAPTER III INSTALLATION"),
            heading(2, "CHAPTER III INSTALLATION")
        );
        assert_eq!(first("HAKKIMIZDA"), heading(3, "HAKKIMIZDA"));
        assert_eq!(
            first("DS-1640 YASSI DAMLA SULAMA ŞERİDİ 16 MM, 8 MİL"),
            heading(3, "DS-1640 YASSI DAMLA SULAMA ŞERİDİ 16 MM, 8 MİL")
        );
    }

    #[test]
    fn a_part_word_without_a_number_or_ordinal_is_an_ordinary_title() {
        // Table headers and drawing labels must not reset the whole path.
        assert_eq!(first("PART NO"), heading(3, "PART NO"));
        assert_eq!(first("SECTION A-A"), heading(3, "SECTION A-A"));
        assert_eq!(first("SPARE PART"), heading(3, "SPARE PART"));
    }

    #[test]
    fn warning_labels_are_not_titles() {
        assert_eq!(first("UYARI"), Line::Body);
        assert_eq!(first("DİKKAT"), Line::Body);
        assert_eq!(first("WARNING"), Line::Body);
    }

    #[test]
    fn dashed_section_lines_keep_only_their_name() {
        assert_eq!(
            first("-- PE 100 ANA HAT BORULARI --"),
            heading(4, "PE 100 ANA HAT BORULARI")
        );
        assert_eq!(
            first("-- Bağlantı parçaları --"),
            heading(4, "Bağlantı parçaları")
        );
    }

    #[test]
    fn footnote_numbers_glued_to_a_title_are_dropped() {
        assert_eq!(
            first("TÜRKİYE CUMHURİYETİ ANAYASASI1"),
            heading(3, "TÜRKİYE CUMHURİYETİ ANAYASASI")
        );
        assert_eq!(
            first("A. Bütçe ve kesinhesap26"),
            heading(6, "A. Bütçe ve kesinhesap")
        );
        assert_eq!(
            first("DS-3120 ELEK FİLTRE PN10"),
            heading(3, "DS-3120 ELEK FİLTRE PN10")
        );
    }

    #[test]
    fn spaced_out_titles_are_still_titles() {
        assert_eq!(first("BİRİNCİ   KISIM"), heading(1, "BİRİNCİ KISIM"));
    }

    #[test]
    fn enumerated_titles_nest_roman_then_letter() {
        assert_eq!(
            first("IV. Değiştirilemeyecek hükümler "),
            heading(5, "IV. Değiştirilemeyecek hükümler")
        );
        assert_eq!(first("I. Devletin şekli"), heading(5, "I. Devletin şekli"));
        assert_eq!(
            first("A. Özel hayatın gizliliği"),
            heading(6, "A. Özel hayatın gizliliği")
        );
        assert_eq!(
            first("Ç. Seyahat hürriyeti"),
            heading(6, "Ç. Seyahat hürriyeti")
        );
    }

    #[test]
    fn i_after_h_continues_the_letters() {
        let lines = read(&[
            "H. Askerî yargı",
            "Metin.",
            "I. Bilim ve sanat hürriyeti",
            "II. Kişinin hakları",
        ]);
        assert_eq!(lines[2], heading(6, "I. Bilim ve sanat hürriyeti"));
        assert_eq!(lines[3], heading(5, "II. Kişinin hakları"));
    }

    #[test]
    fn an_outline_number_inside_wrapped_prose_is_not_a_title() {
        let lines = read(&[
            "Bu Anayasa, Türkiye Cumhuriyetinin kurucusu, ölümsüz önder",
            "M. Kemal Atatürk'ün belirlediği milliyetçilik anlayışı",
            "doğrultusunda hazırlanmıştır.",
            "A. Yılmaz vekili tarafından",
        ]);
        assert_eq!(lines[1], Line::Body);
        // After a finished sentence the same shape is a title.
        assert_eq!(lines[3], heading(6, "A. Yılmaz vekili tarafından"));
    }

    #[test]
    fn a_chapter_takes_the_name_on_the_next_line() {
        let lines = read(&[
            "BİRİNCİ KISIM",
            "Genel Esaslar",
            "I. Devletin şekli",
            "İKİNCİ BÖLÜM",
            "",
            "TEMEL HAKLAR",
        ]);
        assert_eq!(lines[0], heading(1, "BİRİNCİ KISIM"));
        assert_eq!(lines[1], Line::ExtendHeading(" - Genel Esaslar".into()));
        assert_eq!(lines[2], heading(5, "I. Devletin şekli"));
        assert_eq!(lines[5], Line::ExtendHeading(" - TEMEL HAKLAR".into()));
    }

    #[test]
    fn a_title_in_capitals_wrapped_over_two_lines_is_one_title() {
        let lines = read(&[
            "DS-1620 YASSI DAMLA SULAMA",
            "",
            "ŞERİDİ 16 MM, 6 MİL",
            "Gövde.",
        ]);
        assert_eq!(lines[2], Line::ExtendHeading(" ŞERİDİ 16 MM, 6 MİL".into()));
        // Body in between ends the title.
        let lines = read(&["HAKKIMIZDA", "Gövde metni.", "SEÇİM REHBERİ"]);
        assert_eq!(lines[2], heading(3, "SEÇİM REHBERİ"));
    }

    #[test]
    fn a_line_ending_in_a_broken_word_is_not_a_title() {
        assert_eq!(
            first("ALL EXPRESS AND IMPLIED WARRANTIES, INCLUDING THE WARRANTY OF MER-"),
            Line::Body
        );
        // A hyphen inside a code is not a broken word.
        assert_eq!(
            first("DS-1640 YASSI ŞERİT"),
            heading(3, "DS-1640 YASSI ŞERİT")
        );
    }

    #[test]
    fn three_lines_in_capitals_are_a_label_block_not_a_title() {
        let lines = read(&[
            "DATE OF PURCHASE",
            "DEALER NAME",
            "DEALER ADDRESS",
            "Body text.",
        ]);
        assert_eq!(lines[0], heading(3, "DATE OF PURCHASE"));
        assert_eq!(lines[1], Line::ExtendHeading(" DEALER NAME".into()));
        assert_eq!(lines[2], Line::RetractHeading);
        // Two lines still make one wrapped title.
        let lines = read(&[
            "DS-1620 YASSI DAMLA SULAMA",
            "ŞERİDİ 16 MM, 6 MİL",
            "Gövde.",
        ]);
        assert_eq!(lines[1], Line::ExtendHeading(" ŞERİDİ 16 MM, 6 MİL".into()));
        assert_eq!(lines[2], Line::Body);
    }

    #[test]
    fn a_block_in_capitals_ends_at_a_blank_line() {
        let lines = read(&[
            "DATE OF PURCHASE",
            "DEALER NAME",
            "DEALER ADDRESS",
            "",
            "IMPORTANT SAFETY INSTRUCTIONS",
        ]);
        assert_eq!(lines[4], heading(3, "IMPORTANT SAFETY INSTRUCTIONS"));
    }

    #[test]
    fn a_title_in_capitals_needs_real_text_after_it() {
        let paragraph = |text: &str| Block::Paragraph(text.into());
        let title = |text: &str| Block::Heading {
            level: 3,
            text: text.into(),
        };
        let blocks = vec![
            // A diagram label followed by another label.
            title("TIME SET"),
            paragraph("OFF"),
            // The first line of a paragraph in capitals.
            title("TO PREVENT ELECTRIC SHOCK MATCH"),
            paragraph("WIDE BLADE OF PLUG TO WIDE SLOT, FULLY INSERT."),
            // A real section.
            title("IMPORTANT SAFETY INSTRUCTIONS"),
            paragraph("Read the operating instructions carefully before using the unit."),
            // A part heading is never demoted, nor a title before a subsection.
            Block::Heading {
                level: 1,
                text: "BİRİNCİ KISIM".into(),
            },
            title("HAKKIMIZDA"),
            Block::Heading {
                level: 5,
                text: "I. Kuruluş".into(),
            },
            paragraph("Kısa."),
            // A title at the very end has nothing to show it wrong.
            title("SON BÖLÜM BAŞLIĞI"),
        ];
        let out = demote_unsupported_titles(blocks);
        assert_eq!(out[0], paragraph("TIME SET"));
        assert_eq!(out[2], paragraph("TO PREVENT ELECTRIC SHOCK MATCH"));
        assert_eq!(out[4], title("IMPORTANT SAFETY INSTRUCTIONS"));
        assert!(matches!(out[6], Block::Heading { level: 1, .. }));
        assert_eq!(out[7], title("HAKKIMIZDA"));
        assert_eq!(out[10], title("SON BÖLÜM BAŞLIĞI"));
    }

    #[test]
    fn body_lines_are_not_headings() {
        for line in [
            "MADDE 1- Türkiye Devleti bir Cumhuriyettir.",
            "Türk Vatanı ve Milletinin ebedi varlığını ve Yüce Türk Devletinin",
            // Mixed case, even when it starts in capitals.
            "TÜRK MİLLETİ TARAFINDAN, demokrasiye âşık Türk evlatlarının vatan",
            // Column layout: a table header or row, not a heading.
            "KOD        ÜRÜN ADI                     BİRİM     FİYAT (TL)",
            "DS-1600    Yuvarlak damla sulama borusu 16 mm    m     4,20",
            "---------  --------------------------------  -------",
            "DS-1840 | 40 | PN 10 | 27,60",
            // Decimal numbering numbers steps in manuals; it stays body text.
            "1. Milletin temsili",
            "2.3 Kapsam ve tanımlar",
            // A sentence, even a short numbered one.
            "1. Pompayı çalıştırmadan önce vanayı açın.",
            "A. Bu madde, kanunla sınırlanabilir;",
            // A footnote, a page number, an acronym.
            "1 Bu Anayasa; Kurucu Meclis tarafından kabul edilmiştir",
            "2 ",
            "KDV",
            "10/6/26, 9:06 AM about:blank",
            // A garbled title that ends in a sentence mark.
            "DENİZ SULAMA SİSTEMLERİ A. DAMLA SULAMA ÜRÜN KATALOĞU Ş.",
            // Lower-case after the enumerator: a wrapped sentence.
            "2. maddesindeki Cumhuriyetin nitelikleri",
        ] {
            assert_eq!(first(line), Line::Body, "{line:?}");
        }
        assert_eq!(first("   "), Line::Blank);
    }

    #[test]
    fn a_heading_on_most_pages_is_a_running_header() {
        let header = || Block::Heading {
            level: 3,
            text: "YARGITAY 1. CEZA DAİRESİ".into(),
        };
        let mut blocks = Vec::new();
        let mut pages = Vec::new();
        for page in 1..=4 {
            blocks.push(header());
            blocks.push(Block::Paragraph(format!("Sayfa {page} metni.")));
            pages.extend([Some(page), Some(page)]);
        }
        blocks.push(Block::Heading {
            level: 3,
            text: "HÜKÜM".into(),
        });
        pages.push(Some(4));
        let out = demote_running_headers(blocks, &pages);
        assert_eq!(out[0], Block::Paragraph("YARGITAY 1. CEZA DAİRESİ".into()));
        assert!(matches!(out[8], Block::Heading { .. }));
    }

    #[test]
    fn long_lines_are_not_headings() {
        assert_eq!(first(&"ÇOK UZUN BİR SATIR ".repeat(6)), Line::Body);
        assert_eq!(first(&format!("IV. {}", "Kelime ".repeat(14))), Line::Body);
    }
}
