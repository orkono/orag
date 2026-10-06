//! Lexical normalization shared by indexing and querying (D-006).
//!
//! The same function must be applied to indexed text and to queries; changing
//! it requires bumping `NORMALIZER_VERSION`, which changes the embedding-space
//! fingerprint and forces a reindex. Its output also depends on Unicode tables
//! (`unicode-normalization`, pinned with `=`, and std's case and category
//! tables, fixed by `rust-toolchain.toml`): bumping either is a normalizer
//! change too.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

pub const NORMALIZER_VERSION: u32 = 2;
pub const MAX_QUERY_TERMS: usize = 32;
/// Questions are short; only this much of a query is normalized. It also
/// bounds the MATCH expression.
const MAX_QUERY_CHARS: usize = 4096;

/// NFKC (done as NFKD here, recomposed at the end), then:
/// - drop invisible format characters (Default_Ignorable_Code_Point: soft
///   hyphen, zero-width space, joiners, BOM, variation selectors); `unicode61`
///   would split a word at them, so `bağ­lantı` (a PDF soft hyphen) could never
///   be found as `bağlantı`;
/// - fold `İ`/`I`/`ı` to `i` and drop a U+0307 dot among the marks after an `i`
///   (left by decomposed `i̇`); other letters keep their dot;
/// - drop a U+0302 circumflex among the marks after `a`, `i` or `u`, so the
///   Turkish `resmî`, `kâğıt`, `Ûmit` match the usual `resmi`, `kağıt`,
///   `Umit`; other letters (`ê`, `ô`) keep it;
/// - private-use characters (PDF glyph codes) become spaces, so they never glue
///   onto a word;
/// - lowercase, with `ß`/`ẞ` folded to `ss` so `STRASSE` finds `straße`;
/// - recompose (NFC), so a decomposed accent left by the folding (`ı` + U+0301)
///   ends up as the same character as its precomposed form (`í`).
pub fn normalize_for_lexical(input: &str) -> String {
    let mut after_i = false;
    let mut after_aiu = false;
    // Decomposed (NFKD) while folding, so `İ` is `I` + U+0307 and an accent on
    // `i` is a separate mark; the final NFC gives the same result as NFKC.
    input
        .nfkd()
        .filter(|&ch| !is_default_ignorable(ch))
        .filter(move |&ch| {
            let keep = !(ch == '\u{0307}' && after_i) && !(ch == '\u{0302}' && after_aiu);
            // Marks (e.g. an acute typed before the dot) keep the state, so the
            // dot or circumflex is dropped anywhere in the marks after the letter.
            if !is_combining_mark(ch) {
                after_i = matches!(ch, 'i' | 'ı' | 'I' | 'İ');
                after_aiu = after_i || matches!(ch, 'a' | 'A' | 'u' | 'U');
            }
            keep
        })
        .map(|ch| match ch {
            'İ' | 'I' | 'ı' => 'i',
            ch if is_private_use(ch) => ' ',
            ch => ch,
        })
        .flat_map(char::to_lowercase)
        .flat_map(|ch| {
            let (first, second) = if ch == 'ß' {
                ('s', Some('s'))
            } else {
                (ch, None)
            };
            std::iter::once(first).chain(second)
        })
        .nfc()
        .collect()
}

fn is_private_use(ch: char) -> bool {
    matches!(ch, '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{FFFFD}' | '\u{100000}'..='\u{10FFFD}')
}

/// Unicode's Default_Ignorable_Code_Point property (DerivedCoreProperties,
/// Unicode 17.0, the version `unicode_tables_are_pinned` checks).
pub(crate) fn is_default_ignorable(ch: char) -> bool {
    matches!(
        ch,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// Letters, numbers and combining marks. Every other character separates
/// terms. Marks stay inside a term: FTS5 tokenizes the inside of a quoted term
/// with the index's own tokenizer, so a term like `नमस्ते` matches however
/// `unicode61` splits it, and no character list has to mirror SQLite's.
/// Enclosed letter emoji (🅰, 🅐; U+1F100-1F1FF) are symbols to `unicode61`,
/// though std counts some as alphabetic, so they separate terms too.
fn is_term_char(ch: char) -> bool {
    !matches!(ch, '\u{1F100}'..='\u{1F1FF}') && (ch.is_alphanumeric() || is_combining_mark(ch))
}

/// Normalized, de-duplicated query terms in first-seen order: at most
/// `MAX_QUERY_TERMS`, from the first `MAX_QUERY_CHARS` of the input. A term
/// needs a letter or number; a run of marks alone is not a word. When the
/// length cap cuts inside a term, that partial term is dropped; the boundary is
/// decided on normalized text, since some symbols become letters under NFKC
/// (`№` -> `no`).
pub fn lexical_terms(input: &str) -> Vec<String> {
    let (head, rest) = match input.char_indices().nth(MAX_QUERY_CHARS) {
        Some((cut, _)) => input.split_at(cut),
        None => (input, ""),
    };
    let normalized = normalize_for_lexical(head);
    let cut_inside_a_term = normalized.chars().next_back().is_some_and(is_term_char)
        && normalize_for_lexical(
            &rest
                .chars()
                .filter(|&c| !is_default_ignorable(c))
                .take(8)
                .collect::<String>(),
        )
        .chars()
        .next()
        .is_some_and(is_term_char);
    let mut segments: Vec<&str> = normalized.split(|c: char| !is_term_char(c)).collect();
    if cut_inside_a_term {
        segments.pop();
    }
    let mut terms: Vec<String> = Vec::new();
    for term in segments
        .into_iter()
        .filter(|term| term.chars().any(char::is_alphanumeric))
    {
        if !terms.iter().any(|existing| existing == term) {
            terms.push(term.to_string());
        }
        if terms.len() == MAX_QUERY_TERMS {
            break;
        }
    }
    terms
}

/// FTS5 MATCH expression with every term quoted and OR-ed. Terms contain only
/// letters, numbers and marks (never `"`), so no user input can reach FTS5
/// syntax.
pub fn fts_query(input: &str) -> Option<String> {
    let terms = lexical_terms(input);
    if terms.is_empty() {
        return None;
    }
    let quoted: Vec<String> = terms.iter().map(|term| format!("\"{term}\"")).collect();
    Some(quoted.join(" OR "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_all_turkish_i_variants_to_plain_i() {
        assert_eq!(normalize_for_lexical("İSTANBUL"), "istanbul");
        assert_eq!(normalize_for_lexical("Istanbul"), "istanbul");
        assert_eq!(normalize_for_lexical("ıstanbul"), "istanbul");
        assert_eq!(normalize_for_lexical("IT department"), "it department");
    }

    #[test]
    fn preserves_turkish_diacritics_other_than_i() {
        assert_eq!(normalize_for_lexical("ŞEKER Çay ĞÖÜ"), "şeker çay ğöü");
    }

    #[test]
    fn folds_the_circumflex_on_a_i_u() {
        assert_eq!(
            normalize_for_lexical("Resmî Millî kâğıt ÛMİT"),
            "resmi milli kağit umit"
        );
        assert_eq!(normalize_for_lexical("I\u{0302}"), "i");
        assert_eq!(lexical_terms("maddî"), lexical_terms("maddi"));
        // Other letters keep it.
        assert_eq!(normalize_for_lexical("fête côte"), "fête côte");
    }

    #[test]
    fn applies_nfkc_compatibility_folding() {
        assert_eq!(normalize_for_lexical("ﬁle №5"), "file no5");
    }

    #[test]
    fn drops_combining_dot_above_left_by_decomposed_input() {
        assert_eq!(normalize_for_lexical("i\u{0307}stanbul"), "istanbul");
    }

    #[test]
    fn builds_quoted_or_query_with_unique_terms() {
        assert_eq!(
            fts_query("Ağ bağlantısı nedir? Ağ").as_deref(),
            Some("\"ağ\" OR \"bağlantisi\" OR \"nedir\"")
        );
    }

    #[test]
    fn fts_query_neutralizes_syntax() {
        assert_eq!(
            fts_query(r#"norm_text:foo" NEAR(bar*) -baz OR"#).as_deref(),
            Some("\"norm\" OR \"text\" OR \"foo\" OR \"near\" OR \"bar\" OR \"baz\" OR \"or\"")
        );
    }

    #[test]
    fn query_without_terms_yields_none() {
        assert_eq!(fts_query("  ?!  -- "), None);
    }

    #[test]
    fn query_terms_are_capped() {
        let long: String = (0..100).map(|i| format!("w{i} ")).collect();
        assert_eq!(lexical_terms(&long).len(), MAX_QUERY_TERMS);
    }

    #[test]
    fn output_is_nfc_whatever_the_dotless_i_input() {
        let composed = normalize_for_lexical("í");
        assert_eq!(composed, "\u{ed}");
        assert_eq!(normalize_for_lexical("ı\u{0301}"), composed);
        assert_eq!(normalize_for_lexical("i\u{0307}\u{0301}"), composed);
        assert_eq!(normalize_for_lexical("I\u{0301}"), composed);
    }

    #[test]
    fn private_use_characters_separate_words() {
        assert_eq!(normalize_for_lexical("\u{e000}abc"), " abc");
        assert_eq!(lexical_terms("\u{e000}abc"), vec!["abc"]);
    }

    #[test]
    fn combining_marks_stay_inside_terms_for_fts5_to_split() {
        // FTS5 tokenizes the inside of a quoted term with the same tokenizer as
        // the index, so marks are kept and the phrase matches the indexed text.
        assert_eq!(lexical_terms("नमस्ते"), vec!["नमस्ते"]);
        assert_eq!(fts_query("x\u{0301}y").as_deref(), Some("\"x\u{301}y\""));
    }

    #[test]
    fn huge_queries_are_bounded() {
        let giant = "a".repeat(1_000_000);
        assert!(
            lexical_terms(&giant).is_empty(),
            "a term longer than the cap is dropped"
        );
        let many = "kelime ".repeat(200_000);
        assert_eq!(lexical_terms(&many), vec!["kelime"]);
    }

    #[test]
    fn terms_without_a_letter_or_number_are_dropped() {
        assert_eq!(fts_query("❤️"), None);
        assert_eq!(
            fts_query("Teşekkürler ❤️").as_deref(),
            Some("\"teşekkürler\"")
        );
    }

    #[test]
    fn a_cut_never_leaves_part_of_a_word() {
        // `№` becomes the letters `no` under NFKC, so the boundary is decided
        // on normalized text.
        let query = format!("{}№xyz", "a ".repeat(2047));
        assert!(
            !lexical_terms(&query).iter().any(|term| term != "a"),
            "{:?}",
            lexical_terms(&query)
        );
    }

    #[test]
    fn invisible_format_characters_are_dropped_not_split_on() {
        assert_eq!(normalize_for_lexical("bağ\u{00AD}lantı"), "bağlanti");
        assert_eq!(normalize_for_lexical("a\u{200B}b\u{2060}c\u{FEFF}"), "abc");
        assert_eq!(fts_query("❤\u{FE0F}"), None);
    }

    #[test]
    fn sharp_s_folds_to_ss() {
        assert_eq!(
            normalize_for_lexical("Straße STRASSE ẞ"),
            "strasse strasse ss"
        );
    }

    #[test]
    fn dot_above_is_dropped_only_after_i() {
        assert_eq!(normalize_for_lexical("i\u{0307}"), "i");
        assert_eq!(
            normalize_for_lexical("x\u{0307}"),
            "\u{1E8B}",
            "x + dot above is ẋ (NFC)"
        );
    }

    #[test]
    fn enclosed_letter_emoji_are_not_words() {
        assert_eq!(fts_query("🅰\u{FE0F} 🅐"), None);
    }

    #[test]
    fn a_cut_keeps_the_whole_words_before_it() {
        let commas = format!("tek{}", ",a".repeat(3000));
        let terms = lexical_terms(&commas);
        assert_eq!(terms, vec!["tek", "a"]);
        let accents = format!("{}foo bar", "ä".repeat(4094));
        assert_eq!(
            lexical_terms(&accents),
            Vec::<String>::new(),
            "the run cut at the limit is dropped"
        );
    }

    #[test]
    fn unicode_tables_are_pinned() {
        // The normalizer's output depends on these tables (std case and
        // category data, unicode-normalization, the ignorable list above). A
        // toolchain or crate bump that changes them changes indexed text:
        // bump NORMALIZER_VERSION, re-check `is_default_ignorable`, then
        // update this test.
        assert_eq!(char::UNICODE_VERSION, (17, 0, 0), "bump NORMALIZER_VERSION");
        assert_eq!(
            unicode_normalization::UNICODE_VERSION,
            (17, 0, 0),
            "bump NORMALIZER_VERSION"
        );
    }

    #[test]
    fn dot_above_is_dropped_after_other_marks_on_i() {
        assert_eq!(
            normalize_for_lexical("i\u{0301}\u{0307}x"),
            normalize_for_lexical("íx")
        );
    }

    #[test]
    fn invisible_characters_after_the_cut_do_not_hide_it() {
        let query = format!("{}ab{}cd", "x ".repeat(2047), "\u{200B}".repeat(9));
        assert!(!lexical_terms(&query).contains(&"ab".to_string()));
    }
}
