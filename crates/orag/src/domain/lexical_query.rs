//! How a question becomes an FTS5 MATCH expression (D-006). Query side only:
//! changing it needs no reindex and no `LEXICAL_VERSION` bump.

use crate::domain::normalize::lexical_terms;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LexicalQuery {
    /// Terms of at least this many characters that contain a letter also
    /// match words they begin (`"dil"*` finds `dili`), so a Turkish suffix does
    /// not hide a match. Numbers stay whole: `104` must not find `1040`.
    pub prefix_min_chars: Option<usize>,
}

impl LexicalQuery {
    /// Whole terms only: the behaviour before 0.2.0-alpha.4.
    pub const EXACT: LexicalQuery = LexicalQuery {
        prefix_min_chars: None,
    };

    /// `exact` or `prefix:<n>`.
    pub fn parse(text: &str) -> Option<LexicalQuery> {
        let prefix_min_chars = match text {
            "exact" => None,
            other => Some(
                other
                    .strip_prefix("prefix:")?
                    .parse()
                    .ok()
                    .filter(|&n| n > 0)?,
            ),
        };
        Some(LexicalQuery { prefix_min_chars })
    }

    pub fn name(self) -> String {
        match self.prefix_min_chars {
            Some(n) => format!("prefix:{n}"),
            None => "exact".into(),
        }
    }

    /// The MATCH expression: every term quoted (so no user input reaches
    /// FTS5 syntax) and OR-ed. `None` when the question has no terms.
    pub fn fts_query(self, input: &str) -> Option<String> {
        let terms = lexical_terms(input);
        if terms.is_empty() {
            return None;
        }
        let quoted: Vec<String> = terms
            .iter()
            .map(|term| match self.prefix_min_chars {
                Some(min)
                    if term.chars().count() >= min && term.chars().any(char::is_alphabetic) =>
                {
                    format!("\"{term}\"*")
                }
                _ => format!("\"{term}\""),
            })
            .collect();
        Some(quoted.join(" OR "))
    }
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
}
