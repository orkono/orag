//! Heuristics for PDF pages whose extracted text is probably garbled.
//!
//! Advisory only: a flagged page is still indexed; the warning tells the user
//! that answers drawn from it can be wrong.

/// A page needs this many non-space characters before it is judged.
const MIN_CHARS: usize = 200;
/// U+FFFD share (per mille) that marks missing ToUnicode maps.
const REPLACEMENT_PER_MILLE: usize = 10;
/// Share (percent) of one-letter tokens that marks letters split apart by
/// positioning.
const SINGLE_LETTER_PERCENT: usize = 40;

/// Why a page's text looks broken, or `None` if it looks fine or is too
/// short to judge. Short lines alone are no sign: tables, poems and code
/// lists have them legitimately.
pub fn page_looks_broken(text: &str) -> Option<&'static str> {
    let chars = text.chars().filter(|c| !c.is_whitespace()).count();
    if chars < MIN_CHARS {
        return None;
    }
    let replacements = text.chars().filter(|&c| c == '\u{FFFD}').count();
    if replacements * 1000 >= chars * REPLACEMENT_PER_MILLE {
        return Some("replacement characters");
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let single_letters = tokens
        .iter()
        .filter(|token| {
            let mut letters = token.chars();
            letters.next().is_some_and(char::is_alphabetic) && letters.next().is_none()
        })
        .count();
    if single_letters * 100 >= tokens.len() * SINGLE_LETTER_PERCENT {
        return Some("mostly single letters");
    }
    None
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
        assert_eq!(page_looks_broken(&fffd), Some("replacement characters"));
        let split = "T e s l i m a t s ü r e s i ü ç g ü n d ü r . ".repeat(12);
        assert_eq!(page_looks_broken(&split), Some("mostly single letters"));
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
}
