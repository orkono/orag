//! The passage a reranker reads: the sentences of a chunk around the query's
//! terms, at most `max_chars` long (v0.4 experiment, step 65). A cross-encoder
//! over a full 384-512-token chunk is too slow for ORAG's targets (step 64);
//! a window cuts the tokens per pair.

use unicode_normalization::char::is_combining_mark;

use crate::domain::normalize::{lexical_terms, normalize_for_lexical};

/// Characters of a query term that must open a passage term to count as a
/// match, so `dinlenmeyi` meets `dinlenmek`.
const KEY_CHARS: usize = 5;
/// Shorter query terms are too common to place a window.
const MIN_KEY_CHARS: usize = 3;

/// The window of `text` to rerank: `text` itself when it fits in
/// `max_chars`; otherwise the piece matching the most query keys grown with
/// its neighbours (after first, then before) while they fit. `None` when no
/// piece matches a key (a cross-language pair, say): the caller reads the
/// whole chunk rather than a window placed by position.
pub fn passage_window(query: &str, text: &str, max_chars: usize) -> Option<String> {
    if text.chars().count() <= max_chars {
        return Some(text.to_string());
    }
    let keys = query_keys(query);
    let pieces = pieces(text, max_chars);
    let best = best_piece(&pieces, &keys)?;
    let len = |range: (usize, usize)| pieces[range.0..range.1].concat().chars().count();
    let mut range = (best, best + 1);
    loop {
        let after = (range.0, range.1 + 1);
        if range.1 < pieces.len() && len(after) <= max_chars {
            range = after;
            continue;
        }
        let before = (range.0.wrapping_sub(1), range.1);
        if range.0 > 0 && len(before) <= max_chars {
            range = before;
            continue;
        }
        break;
    }
    Some(
        pieces[range.0..range.1]
            .concat()
            .chars()
            .take(max_chars)
            .collect::<String>()
            .trim()
            .to_string(),
    )
}

fn query_keys(query: &str) -> Vec<Key> {
    let mut keys: Vec<Key> = Vec::new();
    for term in lexical_terms(query) {
        if term.chars().count() >= MIN_KEY_CHARS {
            let key = if term.chars().any(char::is_numeric) {
                Key::Whole(term)
            } else {
                Key::Prefix(term.chars().take(KEY_CHARS).collect())
            };
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

/// How a query term finds a passage word: a word opening with its first
/// `KEY_CHARS` characters, or, for a term with a digit (a code or a number),
/// only the same word, so `104` does not meet `1040`.
#[derive(Debug, PartialEq, Eq)]
enum Key {
    Prefix(String),
    Whole(String),
}

impl Key {
    fn matches(&self, word: &str) -> bool {
        match self {
            Key::Prefix(key) => word.starts_with(key.as_str()),
            Key::Whole(term) => word == term,
        }
    }
}

/// Sentences of `text` with their delimiters and following whitespace, so
/// they concat back to `text`; a sentence ends after `.`, `!`, `?`, `;` or a
/// newline. A sentence longer than the window is split at whitespace (or,
/// without any, at characters) into parts of at most half the window, so a
/// long table row or PDF line can still be windowed around its match.
fn pieces(text: &str, max_chars: usize) -> Vec<&str> {
    let half = (max_chars / 2).max(1);
    sentences(text)
        .into_iter()
        .flat_map(|sentence| {
            if sentence.chars().count() > max_chars {
                split_long(sentence, half)
            } else {
                vec![sentence]
            }
        })
        .collect()
}

fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?' | ';' | '\n') {
            let mut end = i + c.len_utf8();
            while let Some(&(j, next)) = chars.peek() {
                if !next.is_whitespace() {
                    break;
                }
                end = j + next.len_utf8();
                chars.next();
            }
            out.push(&text[start..end]);
            start = end;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// `sentence` in parts of at most `limit` characters, cut after the last
/// whitespace that fits, else after the last other non-word character (a
/// table's `|`), else at `limit` characters, so a word is split only when a
/// part holds nothing but that word.
fn split_long(sentence: &str, limit: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = sentence;
    while rest.chars().count() > limit {
        let hard = rest
            .char_indices()
            .nth(limit)
            .map_or(rest.len(), |(i, _)| i);
        let after_last = |pred: fn(char) -> bool| {
            rest[..hard]
                .char_indices()
                .rfind(|&(_, c)| pred(c))
                .map(|(i, c)| i + c.len_utf8())
        };
        let cut = after_last(char::is_whitespace)
            .or_else(|| after_last(|c| !(c.is_alphanumeric() || is_combining_mark(c))))
            .unwrap_or(hard);
        out.push(&rest[..cut]);
        rest = &rest[cut..];
    }
    if !rest.is_empty() {
        out.push(rest);
    }
    out
}

/// Every word of `piece`, normalized like query terms but without the
/// query's term cap.
fn piece_terms(piece: &str) -> Vec<String> {
    normalize_for_lexical(piece)
        .split(|c: char| !(c.is_alphanumeric() || is_combining_mark(c)))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// The piece matching the most distinct keys, the first on a tie; `None`
/// when none matches.
fn best_piece(pieces: &[&str], keys: &[Key]) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None;
    for (i, piece) in pieces.iter().enumerate() {
        let terms = piece_terms(piece);
        let score = keys
            .iter()
            .filter(|key| terms.iter().any(|term| key.matches(term)))
            .count();
        if score > 0 && best.is_none_or(|(_, top)| score > top) {
            best = Some((i, score));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "Madde 1. Türkiye Devleti bir Cumhuriyettir. \
        Madde 2. Herkes çalışma hakkına sahiptir. \
        Madde 3. Dinlenmek, çalışanların hakkıdır. \
        Madde 4. Ücretli hafta ve bayram tatili kanunla düzenlenir.";

    fn window(query: &str, text: &str, max: usize) -> String {
        passage_window(query, text, max).expect("a key matches")
    }

    #[test]
    fn a_short_chunk_is_read_whole() {
        assert_eq!(passage_window("pump", TEXT, 1_000).as_deref(), Some(TEXT));
    }

    #[test]
    fn the_window_centres_on_the_matching_sentence() {
        let w = window("Çalışanların dinlenmeyi hakkı var mı?", TEXT, 60);
        assert!(w.contains("Dinlenmek, çalışanların hakkıdır."), "{w}");
        assert!(w.chars().count() <= 60, "{w}");
        assert!(!w.contains("Cumhuriyettir"), "{w}");
    }

    #[test]
    fn neighbours_fill_the_room_left() {
        let w = window("Dinlenmek hakkı", TEXT, 100);
        assert!(w.contains("Dinlenmek") && w.contains("Madde 4."), "{w}");
        assert!(w.chars().count() <= 100);
    }

    #[test]
    fn without_a_match_the_chunk_is_read_whole() {
        assert_eq!(passage_window("What is the pump pressure?", TEXT, 50), None);
    }

    #[test]
    fn a_match_late_in_a_long_line_stays_in_the_window() {
        // A table row or PDF line with no sentence break and the evidence at its end.
        let cells: String = (1..=60)
            .map(|i| format!("| DS-{i:04} | {i},00 TL "))
            .collect();
        let text = format!("{cells}| P-450 basınç regülatörü | 3.500,00 TL |");
        let w = window("P-450 regülatörü fiyatı", &text, 120);
        assert!(w.contains("regülatörü"), "{w}");
        assert!(w.chars().count() <= 120, "{w}");
    }

    #[test]
    fn terms_beyond_the_query_cap_still_match() {
        // More than MAX_QUERY_TERMS distinct words before the matching one.
        let words: String = (0..80).map(|i| format!("kelime{i} ")).collect();
        let text = format!("{words}son dinlenmek hakkı. Başka bir cümle burada duruyor.");
        let w = window("dinlenmeyi", &text, 200);
        assert!(w.contains("dinlenmek"), "{w}");
    }

    #[test]
    fn a_numeric_key_matches_only_its_whole_term() {
        let text = format!(
            "DS-1040 kartuşu {}. DS-104 kartuşu 2 yıl garantilidir. {}",
            "dolgu ".repeat(30),
            "son ".repeat(30)
        );
        let w = window("DS-104 kartuşu", &text, 60);
        assert!(w.contains("DS-104 kartuşu 2 yıl"), "{w}");
    }

    #[test]
    fn an_unspaced_line_splits_between_words() {
        // No whitespace at all: cells joined by `|`, the key straddling the
        // character cut at 30.
        let text = format!("{}|regülatörü|3500TL|{}", "x".repeat(25), "|yy".repeat(40));
        let w = window("regülatörü fiyatı", &text, 60);
        assert!(w.contains("regülatörü"), "{w}");
    }

    #[test]
    fn a_long_sentence_is_cut_on_a_character_boundary() {
        let text = "ğüşıöç".repeat(40);
        let w = window("ğüşıö", &text, 25);
        assert!((12..=25).contains(&w.chars().count()), "{w}");
    }

    #[test]
    fn pieces_concat_back_to_the_text() {
        let text = "Bir. İki!  Üç?\nDört; beş";
        assert_eq!(pieces(text, 100).concat(), text);
        assert_eq!(pieces(text, 100).len(), 5);
        let long = "a b c d e f g h i j";
        assert_eq!(pieces(long, 8).concat(), long);
        assert!(pieces(long, 8).iter().all(|p| p.chars().count() <= 4));
        assert_eq!(pieces("kısa cümle. ikinci", 100).len(), 2);
    }
}
