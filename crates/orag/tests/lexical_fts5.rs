//! The normalizer against a real FTS5 table with the index's tokenizer: what
//! `normalize_for_lexical` writes, `fts_query` must find (D-006).

use orag::domain::lexical_query::LexicalQuery;
use orag::domain::normalize::{fts_query, normalize_for_lexical};
use rusqlite::Connection;

fn matches(document: &str, query: &str) -> bool {
    matches_with(document, &fts_query(query).expect("query has terms"))
}

fn matches_with(document: &str, expression: &str) -> bool {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE VIRTUAL TABLE t USING fts5(x, tokenize = 'unicode61 remove_diacritics 0');",
    )
    .unwrap();
    db.execute(
        "INSERT INTO t(rowid, x) VALUES (1, ?1)",
        [normalize_for_lexical(document)],
    )
    .unwrap();
    db.query_row(
        "SELECT count(*) FROM t WHERE t MATCH ?1",
        [expression],
        |row| row.get::<_, i64>(0),
    )
    .unwrap()
        == 1
}

#[test]
fn indexed_text_is_found_by_its_query_form() {
    for (document, query) in [
        ("İSTANBUL'DA AĞ BAĞLANTISI", "istanbul ağ bağlantısı"),
        ("ıstanbul", "İstanbul"),
        ("kı\u{301}rmızı", "kírmizi"),
        ("\u{e000}abc rapor", "abc"),
        ("नमस्ते दुनिया", "नमस्ते"),
        ("x\u{301}y", "x\u{301}y"),
        ("ﬁle №5", "file no5"),
        ("情報検索システム", "情報検索システム"),
        ("bağ\u{00AD}lantı kuruldu", "bağlantı"),
        ("Straße", "STRASSE"),
        ("resmî dili", "resmi"),
        ("Millî marşı", "MİLLÎ"),
        ("norm_text değeri", r#"norm_text:foo" NEAR(bar*) -baz OR"#),
    ] {
        assert!(
            matches(document, query),
            "{document:?} not found by {query:?}"
        );
    }
}

#[test]
fn different_words_do_not_match() {
    assert!(
        !matches("şeker", "seker"),
        "ş is kept, so accentless typing is a different term"
    );
    assert!(!matches("istanbul", "ankara"));
    assert!(!matches("fête", "fete"), "only a, i, u lose the circumflex");
}

#[test]
fn every_query_is_valid_fts5_syntax() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE VIRTUAL TABLE t USING fts5(x, tokenize = 'unicode61 remove_diacritics 0');",
    )
    .unwrap();
    for query in [
        r#"" OR "#,
        "a\"b",
        "NEAR(x y, 3)",
        "col:*",
        "^start",
        "a AND NOT b",
        "❤️ teşekkürler",
        "(((",
        "x\u{301}",
    ] {
        if let Some(expression) = fts_query(query) {
            db.query_row(
                "SELECT count(*) FROM t WHERE t MATCH ?1",
                [&expression],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or_else(|err| panic!("{query:?} -> {expression:?}: {err}"));
        }
    }
}

#[test]
fn runs_without_spaces_are_one_token() {
    // Known limit of `unicode61` (D-006): CJK and Thai text without spaces is a
    // single token, so only the whole run matches, not a part of it.
    assert!(!matches("情報検索システム", "情報検索"));
}

#[test]
fn prefix_terms_find_suffixed_words_and_stay_valid_syntax() {
    let prefix = LexicalQuery::parse("prefix:3").unwrap();
    let query = |q: &str| prefix.fts_query(q).expect("query has terms");
    assert!(matches_with(
        "resmî dili Türkçedir",
        &query("resmi dil nedir")
    ));
    assert!(matches_with("Meclisinindir", &query("meclis")));
    assert!(
        !matches_with("dil", &query("diller")),
        "a prefix only extends a term"
    );
    assert!(
        !matches("resmî dili", "dil"),
        "exact terms still need the whole word"
    );
    for hostile in [r#"x" OR y*"#, "NEAR(a b) *", "\"dil\"*"] {
        matches_with("x", &query(hostile));
    }
}
