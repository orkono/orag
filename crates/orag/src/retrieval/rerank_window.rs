//! The passage a reranker reads: the sentences of a chunk around the query's
//! terms, at most `max_chars` long (v0.4 experiment, step 65). A cross-encoder
//! over a full 384-512-token chunk is too slow for ORAG's targets (step 64);
//! a window cuts the tokens per pair.

use crate::domain::normalize::lexical_terms;

/// Characters of a query term that must open a passage term to count as a
/// match, so `dinlenmeyi` meets `dinlenmek`.
const KEY_CHARS: usize = 5;
/// Shorter query terms are too common to place a window.
const MIN_KEY_CHARS: usize = 3;

/// `text` when it fits in `max_chars`; otherwise the best-matching sentence
/// grown with its neighbours (after first, then before) while they fit. With
/// no matching sentence the window opens the chunk; a single sentence longer
/// than `max_chars` is cut at a character boundary.
pub fn passage_window(query: &str, text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let keys = query_keys(query);
    let sentences = sentences(text);
    let best = best_sentence(&sentences, &keys);
    let len = |range: (usize, usize)| sentences[range.0..range.1].concat().chars().count();
    let mut range = (best, best + 1);
    loop {
        let grown_after = (range.0, range.1 + 1);
        let grown_before = (range.0.wrapping_sub(1), range.1);
        if range.1 < sentences.len() && len(grown_after) <= max_chars {
            range = grown_after;
        } else if range.0 > 0 && len(grown_before) <= max_chars {
            range = grown_before;
        } else {
            break;
        }
    }
    sentences[range.0..range.1]
        .concat()
        .chars()
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_string()
}

fn query_keys(query: &str) -> Vec<String> {
    let mut keys: Vec<String> = lexical_terms(query)
        .into_iter()
        .filter(|term| term.chars().count() >= MIN_KEY_CHARS)
        .map(|term| term.chars().take(KEY_CHARS).collect())
        .collect();
    keys.dedup();
    keys
}

/// Sentences with their delimiters and following whitespace, so they concat
/// back to `text`. A sentence ends after `.`, `!`, `?`, `;` or a newline.
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

/// The sentence matching the most distinct keys; the first on a tie.
fn best_sentence(sentences: &[&str], keys: &[String]) -> usize {
    let score = |sentence: &str| {
        let terms = lexical_terms(sentence);
        keys.iter()
            .filter(|key| terms.iter().any(|term| term.starts_with(key.as_str())))
            .count()
    };
    let mut best = (0, 0);
    for (i, sentence) in sentences.iter().enumerate() {
        let s = score(sentence);
        if s > best.1 {
            best = (i, s);
        }
    }
    best.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "Madde 1. Türkiye Devleti bir Cumhuriyettir. \
        Madde 2. Herkes çalışma hakkına sahiptir. \
        Madde 3. Dinlenmek, çalışanların hakkıdır. \
        Madde 4. Ücretli hafta ve bayram tatili kanunla düzenlenir.";

    #[test]
    fn a_short_chunk_is_read_whole() {
        assert_eq!(passage_window("dinlenme", TEXT, 1_000), TEXT);
    }

    #[test]
    fn the_window_centres_on_the_matching_sentence() {
        let window = passage_window("Çalışanların dinlenmeyi hakkı var mı?", TEXT, 60);
        assert!(
            window.contains("Dinlenmek, çalışanların hakkıdır."),
            "{window}"
        );
        assert!(window.chars().count() <= 60, "{window}");
        assert!(!window.contains("Cumhuriyettir"), "{window}");
    }

    #[test]
    fn neighbours_fill_the_room_left() {
        let window = passage_window("Dinlenmek hakkı", TEXT, 100);
        assert!(window.contains("Dinlenmek"), "{window}");
        assert!(window.contains("Madde 4."), "{window}");
        assert!(window.chars().count() <= 100);
    }

    #[test]
    fn without_a_match_the_window_opens_the_chunk() {
        let window = passage_window("What is the pump pressure?", TEXT, 50);
        assert!(window.starts_with("Madde 1."), "{window}");
        assert!(window.chars().count() <= 50);
    }

    #[test]
    fn a_long_sentence_is_cut_on_a_character_boundary() {
        let text = "ğüşıöç".repeat(40);
        let window = passage_window("ğüşıö", &text, 25);
        assert_eq!(window.chars().count(), 25);
    }

    #[test]
    fn sentences_concat_back_to_the_text() {
        let text = "Bir. İki!  Üç?\nDört; beş";
        assert_eq!(sentences(text).concat(), text);
        assert_eq!(sentences(text).len(), 5);
    }
}
