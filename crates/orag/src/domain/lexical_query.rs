//! How a question becomes an FTS5 MATCH expression (D-006). Query side only:
//! changing it needs no reindex and no `LEXICAL_VERSION` bump.

use crate::domain::normalize::lexical_terms;
use unicode_normalization::char::is_combining_mark;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LexicalQuery {
    /// Terms of at least this many characters that contain a letter also
    /// match words they begin (`"dil"*` finds `dili`), so a Turkish suffix does
    /// not hide a match. Numbers stay whole: `104` must not find `1040`.
    pub prefix_min_chars: Option<usize>,
    /// Terms longer than this many characters with a letter and no digit also
    /// add their first characters as a prefix term (`dinlenmeyi` adds
    /// `"dinle"*`, which finds `dinlenmek`): a crude Turkish stem (D-006).
    pub stem_chars: Option<usize>,
}

/// Stems shorter than this match too much to help.
const MIN_STEM_CHARS: usize = 3;

impl LexicalQuery {
    /// Whole terms only: the behaviour before 0.2.0-alpha.4.
    pub const EXACT: LexicalQuery = LexicalQuery {
        prefix_min_chars: None,
        stem_chars: None,
    };

    /// `exact` or `prefix:<n>`, optionally followed by `+stem:<m>` (m >= 3).
    pub fn parse(text: &str) -> Option<LexicalQuery> {
        let (base, stem) = match text.split_once('+') {
            Some((base, stem)) => (base, Some(stem)),
            None => (text, None),
        };
        let prefix_min_chars = match base {
            "exact" => None,
            other => Some(positive(other.strip_prefix("prefix:")?)?),
        };
        let stem_chars = match stem {
            None => None,
            Some(stem) => {
                Some(positive(stem.strip_prefix("stem:")?).filter(|&n| n >= MIN_STEM_CHARS)?)
            }
        };
        Some(LexicalQuery {
            prefix_min_chars,
            stem_chars,
        })
    }

    pub fn name(self) -> String {
        let base = match self.prefix_min_chars {
            Some(n) => format!("prefix:{n}"),
            None => "exact".into(),
        };
        match self.stem_chars {
            Some(n) => format!("{base}+stem:{n}"),
            None => base,
        }
    }

    /// The MATCH expression: every term quoted (so no user input reaches
    /// FTS5 syntax) and OR-ed, then the stems. `None` when the question has
    /// no terms.
    pub fn fts_query(self, input: &str) -> Option<String> {
        let terms = lexical_terms(input);
        if terms.is_empty() {
            return None;
        }
        let quoted: Vec<String> = terms.iter().map(|term| self.quote(term)).collect();
        let stems = self.stems(&terms, &quoted);
        Some(
            quoted
                .into_iter()
                .chain(stems)
                .collect::<Vec<_>>()
                .join(" OR "),
        )
    }

    fn quote(self, term: &str) -> String {
        match self.prefix_min_chars {
            Some(min) if term.chars().count() >= min && term.chars().any(char::is_alphabetic) => {
                format!("\"{term}\"*")
            }
            _ => format!("\"{term}\""),
        }
    }

    /// One prefix term per stemmable term, skipping any already present.
    fn stems(self, terms: &[String], quoted: &[String]) -> Vec<String> {
        let Some(n) = self.stem_chars else {
            return Vec::new();
        };
        terms
            .iter()
            .filter(|term| {
                term.chars().count() > n
                    && term.chars().any(char::is_alphabetic)
                    && !term.chars().any(char::is_numeric)
            })
            .map(|term| format!("\"{}\"*", stem_of(term, n)))
            .fold(Vec::new(), |mut stems, stem| {
                if !quoted.contains(&stem) && !stems.contains(&stem) {
                    stems.push(stem);
                }
                stems
            })
    }
}

fn positive(text: &str) -> Option<usize> {
    text.parse().ok().filter(|&n| n > 0)
}

/// The first `n` characters of `term`, plus any combining marks that follow
/// them, so a mark is never cut from its letter.
fn stem_of(term: &str, n: usize) -> String {
    let mut chars = term.chars();
    let head: String = chars.by_ref().take(n).collect();
    let marks: String = chars.take_while(|&c| is_combining_mark(c)).collect();
    head + &marks
}

#[cfg(test)]
mod tests {
    use super::*;

    const Q: &str = "Anayasaya göre resmi dil nedir?";

    #[test]
    fn exact_quotes_whole_terms() {
        assert_eq!(
            LexicalQuery::EXACT.fts_query(Q).as_deref(),
            Some("\"anayasaya\" OR \"göre\" OR \"resmi\" OR \"dil\" OR \"nedir\"")
        );
        assert_eq!(LexicalQuery::EXACT.fts_query(" ?! "), None);
    }

    #[test]
    fn numbers_are_never_prefixes() {
        let q = LexicalQuery::parse("prefix:3").unwrap();
        assert_eq!(
            q.fts_query("Madde 104 ve 2016 yılı, 5237 sayılı TCK'nun 53A")
                .as_deref(),
            Some(
                "\"madde\"* OR \"104\" OR \"ve\" OR \"2016\" OR \"yili\"* OR \"5237\" OR \"sayili\"* OR \"tck\"* OR \"nun\"* OR \"53a\"*"
            )
        );
    }

    #[test]
    fn prefixes_apply_from_the_minimum_length() {
        let q = LexicalQuery::parse("prefix:4").unwrap();
        assert_eq!(
            q.fts_query(Q).as_deref(),
            Some("\"anayasaya\"* OR \"göre\"* OR \"resmi\"* OR \"dil\" OR \"nedir\"*")
        );
        let q = LexicalQuery::parse("prefix:3").unwrap();
        assert_eq!(
            q.fts_query("dil ve ne").as_deref(),
            Some("\"dil\"* OR \"ve\" OR \"ne\"")
        );
    }

    #[test]
    fn modes_parse_and_print_by_name() {
        for name in ["exact", "prefix:3", "prefix:5"] {
            assert_eq!(LexicalQuery::parse(name).unwrap().name(), name);
        }
        for bad in ["", "prefix:", "prefix:0", "prefix:x", "stop", "fuzzy"] {
            assert_eq!(LexicalQuery::parse(bad), None, "{bad}");
        }
    }

    fn stems(mode: &str, input: &str) -> String {
        LexicalQuery::parse(mode).unwrap().fts_query(input).unwrap()
    }

    #[test]
    fn stems_add_the_first_characters_of_long_words() {
        assert_eq!(
            stems("prefix:3+stem:5", "Dinlenmeyi tanıyor mu?"),
            "\"dinlenmeyi\"* OR \"taniyor\"* OR \"mu\" OR \"dinle\"* OR \"taniy\"*"
        );
    }

    #[test]
    fn stems_are_cut_by_characters() {
        assert_eq!(
            stems("prefix:3+stem:5", "çalışmalarını"),
            "\"çalişmalarini\"* OR \"çaliş\"*"
        );
    }

    #[test]
    fn words_with_digits_are_never_stemmed() {
        assert_eq!(
            stems("prefix:3+stem:5", "DS-1600 53abcdef borusu 2016/347"),
            "\"ds\" OR \"1600\" OR \"53abcdef\"* OR \"borusu\"* OR \"2016\" OR \"347\" OR \"borus\"*"
        );
    }

    #[test]
    fn a_stem_already_in_the_query_is_not_repeated() {
        assert_eq!(
            stems("prefix:3+stem:5", "dinle dinlenmeyi"),
            "\"dinle\"* OR \"dinlenmeyi\"*"
        );
        assert_eq!(
            stems("exact+stem:5", "dinle dinlenmeyi"),
            "\"dinle\" OR \"dinlenmeyi\" OR \"dinle\"*"
        );
    }

    #[test]
    fn a_combining_mark_stays_with_its_letter() {
        assert_eq!(
            stems("prefix:3+stem:5", "abcdx\u{301}yz"),
            "\"abcdx\u{301}yz\"* OR \"abcdx\u{301}\"*"
        );
    }

    #[test]
    fn long_questions_get_at_most_one_stem_per_term() {
        let max = crate::domain::normalize::MAX_QUERY_TERMS;
        let word = |i: usize| {
            let (a, b) = (
                (b'a' + (i / 26) as u8) as char,
                (b'a' + (i % 26) as u8) as char,
            );
            format!("{a}{b}xyzkelime")
        };
        let distinct: Vec<String> = (0..40).map(word).collect();
        let expression = stems("prefix:3+stem:5", &distinct.join(" "));
        assert_eq!(expression.split(" OR ").count(), 2 * max, "{expression}");
        let shared: Vec<String> = (0..40).map(|i| format!("uzunkelime{}", word(i))).collect();
        let expression = stems("prefix:3+stem:5", &shared.join(" "));
        assert_eq!(expression.split(" OR ").count(), max + 1, "{expression}");
    }

    #[test]
    fn stem_modes_parse_and_print_by_name() {
        for name in ["prefix:3+stem:5", "exact+stem:4", "prefix:4+stem:7"] {
            assert_eq!(LexicalQuery::parse(name).unwrap().name(), name);
        }
        for bad in [
            "stem:5",
            "prefix:3+stem:",
            "prefix:3+stem:2",
            "prefix:3+stem:0",
            "prefix:3+stem:x",
            "prefix:3+stem:5+stem:6",
            "prefix:3+",
        ] {
            assert_eq!(LexicalQuery::parse(bad), None, "{bad}");
        }
    }
}
