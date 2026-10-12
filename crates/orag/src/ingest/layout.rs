//! Heuristics for PDF pages whose extracted text is probably garbled.
//!
//! Advisory only: a flagged page is still indexed; the warning tells the user
//! that answers drawn from it can be wrong.

use std::fmt;

use unicode_segmentation::UnicodeSegmentation;

/// A page needs this many non-space characters before it is judged.
const MIN_CHARS: usize = 200;
/// Share (per mille) of unmapped characters that marks a font without a
/// usable ToUnicode map.
const UNMAPPED_PER_MILLE: usize = 10;
/// Share (percent) of tokens that sit in runs of one-letter tokens.
const SPLIT_LETTERS_PERCENT: usize = 40;
/// One-letter tokens count only in runs this long: "T e s l i m a t" is a
/// split word, while "1. A 2. C" (an answer key) or "a x + b" (a formula)
/// has its letters apart.
const MIN_LETTER_RUN: usize = 4;

/// Why a page's text looks garbled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LayoutIssue {
    /// U+FFFD or control characters: glyphs the font could not map.
    UnmappedCharacters,
    /// Words split into single letters by glyph positioning.
    SplitLetters,
}

impl fmt::Display for LayoutIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LayoutIssue::UnmappedCharacters => "unmapped characters",
            LayoutIssue::SplitLetters => "words split into single letters",
        })
    }
}

/// U+FFFD or a control character other than whitespace. Private-use code
/// points are not counted: icon fonts use them for bullets legitimately.
pub fn is_unmapped(c: char) -> bool {
    c == '\u{FFFD}' || (c.is_control() && !c.is_whitespace())
}

/// Why a page's text looks garbled, or `None` if it looks fine or is too
/// short to judge. Short lines alone are no sign: tables, poems and code
/// lists have them legitimately. Text with its spaces lost ("Teslimatsüresi")
/// is not detected: long words are normal in Turkish.
pub fn page_looks_broken(text: &str) -> Option<LayoutIssue> {
    let (mut chars, mut unmapped) = (0, 0);
    for c in text.chars().filter(|c| !c.is_whitespace()) {
        chars += 1;
        unmapped += usize::from(is_unmapped(c));
    }
    if chars < MIN_CHARS {
        return None;
    }
    if unmapped * 1000 >= chars * UNMAPPED_PER_MILLE {
        return Some(LayoutIssue::UnmappedCharacters);
    }
    let (mut tokens, mut in_runs, mut run) = (0, 0, 0);
    for token in text.split_whitespace() {
        tokens += 1;
        if is_one_letter(token) {
            run += 1;
        } else {
            in_runs += if run >= MIN_LETTER_RUN { run } else { 0 };
            run = 0;
        }
    }
    in_runs += if run >= MIN_LETTER_RUN { run } else { 0 };
    (in_runs * 100 >= tokens * SPLIT_LETTERS_PERCENT).then_some(LayoutIssue::SplitLetters)
}

/// One user-perceived letter, so a decomposed "i̇" (i + U+0307) counts too.
fn is_one_letter(token: &str) -> bool {
    let mut graphemes = token.graphemes(true);
    graphemes
        .next()
        .is_some_and(|g| g.chars().next().is_some_and(char::is_alphabetic))
        && graphemes.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbled_pages_are_flagged() {
        let fffd = format!(
            "{}{}",
            "Teslimat süresi üç gündür. ".repeat(10),
            "\u{FFFD}".repeat(5)
        );
        assert_eq!(
            page_looks_broken(&fffd),
            Some(LayoutIssue::UnmappedCharacters)
        );
        let control = format!(
            "{}{}",
            "Teslimat süresi üç gündür. ".repeat(10),
            "\u{1}".repeat(5)
        );
        assert_eq!(
            page_looks_broken(&control),
            Some(LayoutIssue::UnmappedCharacters)
        );
        let split = "T e s l i m a t s ü r e s i ü ç g ü n d ü r . ".repeat(12);
        assert_eq!(page_looks_broken(&split), Some(LayoutIssue::SplitLetters));
        let decomposed = "i\u{307} ş l e m s ü r e s i . ".repeat(25);
        assert_eq!(
            page_looks_broken(&decomposed),
            Some(LayoutIssue::SplitLetters)
        );
    }

    #[test]
    fn short_line_tables_are_not_suspicious() {
        let table: String = (100..160).map(|n| format!("ORX-{n}\n{n} TL\n")).collect();
        assert_eq!(page_looks_broken(&table), None);
        let poem = "Gün\nbatar\nsu\nakar\n".repeat(40);
        assert_eq!(page_looks_broken(&poem), None);
        assert_eq!(page_looks_broken("a b c"), None); // too short to judge
    }

    #[test]
    fn ordinary_prose_with_some_one_letter_words_is_fine() {
        let prose = "Bu bir örnek metindir ve o gün a ile b arasında karar verildi. ".repeat(6);
        assert_eq!(page_looks_broken(&prose), None);
    }

    #[test]
    fn answer_keys_formulas_and_icon_bullets_are_fine() {
        let key: String = (1..=100)
            .map(|n| format!("{n}. {} ", ["A", "B", "C", "D"][n % 4]))
            .collect();
        assert_eq!(page_looks_broken(&key), None);
        let math = "f ( x ) = a x 2 + b x + c ve g ( y ) = m y + n olur. ".repeat(8);
        assert_eq!(page_looks_broken(&math), None);
        let bullets = "\u{F0B7} Teslimat süresi üç gündür.\n".repeat(12);
        assert_eq!(page_looks_broken(&bullets), None);
    }
}
