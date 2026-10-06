//! Tests for the chunker (`super`).

use super::*;
use crate::domain::document::{Block, ParsedDocument};

fn words(text: &str) -> usize {
    text.split_whitespace().count()
}

fn doc(blocks: Vec<Block>) -> ParsedDocument {
    ParsedDocument {
        title: None,
        blocks,
        warnings: Vec::new(),
    }
}

fn h(level: u8, text: &str) -> Block {
    Block::Heading {
        level,
        text: text.into(),
    }
}

fn p(text: &str) -> Block {
    Block::Paragraph(text.into())
}

fn chunk(
    d: &ParsedDocument,
    cfg: &ChunkerConfig,
    count: &dyn Fn(&str) -> usize,
) -> Vec<ChunkDraft> {
    chunk_document(d, cfg, count).unwrap()
}

#[test]
fn headings_start_new_chunks_and_form_breadcrumbs() {
    let d = doc(vec![h(1, "Guide"), p("a b c"), h(2, "Install"), p("d e")]);
    let chunks = chunk(&d, &ChunkerConfig::default(), &words);
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].heading_path, vec!["Guide"]);
    assert_eq!(chunks[0].text, "a b c");
    assert_eq!(chunks[1].heading_path, vec!["Guide", "Install"]);
    assert_eq!(chunks[1].embedding_text(), "Guide > Install\n\nd e");
}

#[test]
fn sibling_heading_replaces_previous_sibling() {
    let d = doc(vec![h(1, "A"), h(2, "B"), p("x"), h(2, "C"), p("y")]);
    let chunks = chunk(&d, &ChunkerConfig::default(), &words);
    assert_eq!(chunks[1].heading_path, vec!["A", "C"]);
}

#[test]
fn small_blocks_merge_up_to_target() {
    let cfg = ChunkerConfig {
        target_tokens: 5,
        max_tokens: 8,
        overlap_tokens: 2,
    };
    let d = doc(vec![p("a b c"), p("d e"), p("f g h")]);
    let texts: Vec<String> = chunk(&d, &cfg, &words)
        .into_iter()
        .map(|c| c.text)
        .collect();
    assert_eq!(texts, vec!["a b c\n\nd e", "f g h"]);
}

#[test]
fn oversized_block_splits_with_overlap_within_max() {
    let cfg = ChunkerConfig {
        target_tokens: 4,
        max_tokens: 5,
        overlap_tokens: 2,
    };
    let text: Vec<String> = (1..=12).map(|i| format!("w{i}")).collect();
    let d = doc(vec![p(&text.join(" "))]);
    let texts: Vec<String> = chunk(&d, &cfg, &words)
        .into_iter()
        .map(|c| c.text)
        .collect();
    assert_eq!(
        texts,
        vec![
            "w1 w2 w3 w4 w5",
            "w4 w5 w6 w7 w8",
            "w7 w8 w9 w10 w11",
            "w10 w11 w12"
        ]
    );
}

#[test]
fn unit_that_cannot_fit_is_an_explicit_error() {
    let cfg = ChunkerConfig {
        target_tokens: 2,
        max_tokens: 3,
        overlap_tokens: 1,
    };
    let long_word = |t: &str| {
        t.split_whitespace()
            .map(|w| if w.contains('h') { 10 } else { 1 })
            .sum::<usize>()
    };
    let d = doc(vec![p("a huge b")]);
    assert!(matches!(
        chunk_document(&d, &cfg, &long_word),
        Err(crate::error::OragError::Model(_))
    ));
}

#[test]
fn budget_includes_the_heading_breadcrumb() {
    let cfg = ChunkerConfig {
        target_tokens: 6,
        max_tokens: 6,
        overlap_tokens: 0,
    };
    let d = doc(vec![h(1, "Long Section Title"), p("a b c d e f")]);
    for chunk in chunk(&d, &cfg, &words) {
        assert!(words(&chunk.embedding_text()) <= 6, "{chunk:?}");
        assert_eq!(chunk.token_count, words(&chunk.embedding_text()));
    }
}

#[test]
fn very_long_words_are_split_without_altering_the_text() {
    // ~1 token per 10 characters, like a BPE tokenizer on a base64 blob.
    let chars_tokens = |t: &str| {
        t.split_whitespace()
            .map(|w| w.chars().count().div_ceil(10))
            .sum::<usize>()
    };
    let cfg = ChunkerConfig {
        target_tokens: 8,
        max_tokens: 8,
        overlap_tokens: 0,
    };
    let blob = "x".repeat(150);
    let chunks = chunk(&doc(vec![p(&blob)]), &cfg, &chars_tokens);
    let lengths: Vec<usize> = chunks.iter().map(|c| c.text.len()).collect();
    assert_eq!(lengths, vec![80, 70]);
    assert_eq!(
        chunks.iter().map(|c| c.text.as_str()).collect::<String>(),
        blob,
        "no invented spaces"
    );
    assert!(chunks.iter().all(|c| c.token_count <= 8));
}

#[test]
fn split_windows_are_exact_slices_of_the_source() {
    let cfg = ChunkerConfig {
        target_tokens: 3,
        max_tokens: 3,
        overlap_tokens: 0,
    };
    let text = "alpha  beta\tgamma delta";
    let chunks = chunk(&doc(vec![p(text)]), &cfg, &words);
    assert_eq!(chunks[0].text, "alpha  beta\tgamma");
    assert_eq!(chunks[1].text, "delta");
}

#[test]
fn oversized_breadcrumb_is_trimmed_by_tokens() {
    let cfg = ChunkerConfig {
        target_tokens: 8,
        max_tokens: 8,
        overlap_tokens: 0,
    };
    let d = doc(vec![h(1, "one two three four five six"), p("body")]);
    let chunks = chunk(&d, &cfg, &words);
    assert_eq!(chunks[0].breadcrumb, "one two");
    assert_eq!(chunks[0].heading_path, vec!["one two three four five six"]);
}

#[test]
fn ordinals_are_sequential_and_empty_doc_yields_nothing() {
    assert!(chunk(&doc(vec![]), &ChunkerConfig::default(), &words).is_empty());
    let d = doc(vec![
        h(1, "A"),
        p("x"),
        h(1, "B"),
        p("y"),
        h(1, "C"),
        p("z"),
    ]);
    let ordinals: Vec<u32> = chunk(&d, &ChunkerConfig::default(), &words)
        .iter()
        .map(|c| c.ordinal)
        .collect();
    assert_eq!(ordinals, vec![0, 1, 2]);
}

#[test]
fn an_inconsistent_config_is_rejected() {
    let d = doc(vec![p("a")]);
    for cfg in [
        ChunkerConfig {
            target_tokens: 600,
            max_tokens: 512,
            overlap_tokens: 0,
        },
        ChunkerConfig {
            target_tokens: 8,
            max_tokens: 8,
            overlap_tokens: 4,
        },
        ChunkerConfig {
            target_tokens: 0,
            max_tokens: 8,
            overlap_tokens: 0,
        },
    ] {
        assert!(chunk_document(&d, &cfg, &words).is_err(), "{cfg:?}");
    }
}

#[test]
fn windows_end_and_start_at_word_boundaries() {
    // ~1 token per 6 characters: long Turkish words cost several tokens and
    // are cut into 16-character units, but a window never ends inside one.
    let bpe = |t: &str| {
        t.split_whitespace()
            .map(|w| w.chars().count().div_ceil(6))
            .sum::<usize>()
    };
    let cfg = ChunkerConfig {
        target_tokens: 10,
        max_tokens: 10,
        overlap_tokens: 3,
    };
    let text = "değerlendirilmesinin ".repeat(12);
    let chunks = chunk(&doc(vec![p(text.trim_end())]), &cfg, &bpe);
    assert!(chunks.len() > 1);
    for c in &chunks {
        for w in c.text.split_whitespace() {
            assert_eq!(w, "değerlendirilmesinin", "{:?}", c.text);
        }
    }
}

#[test]
fn multibyte_runs_that_cost_more_than_their_length_still_split() {
    // Each emoji costs 4 tokens: a 16-character unit (64 tokens) cannot fit
    // a 10-token budget, so units are cut smaller instead of failing.
    let bytes = |t: &str| {
        t.split_whitespace()
            .map(|w| w.chars().count() * 4)
            .sum::<usize>()
    };
    let cfg = ChunkerConfig {
        target_tokens: 10,
        max_tokens: 10,
        overlap_tokens: 0,
    };
    let blob = "😀".repeat(20);
    let chunks = chunk(&doc(vec![p(&blob)]), &cfg, &bytes);
    assert_eq!(
        chunks.iter().map(|c| c.text.as_str()).collect::<String>(),
        blob
    );
    assert!(chunks.iter().all(|c| c.token_count <= 10));
}

#[test]
fn the_tail_of_a_split_block_merges_with_what_follows() {
    let cfg = ChunkerConfig {
        target_tokens: 5,
        max_tokens: 5,
        overlap_tokens: 0,
    };
    let d = doc(vec![p("w1 w2 w3 w4 w5 w6"), p("x")]);
    let texts: Vec<String> = chunk(&d, &cfg, &words)
        .into_iter()
        .map(|c| c.text)
        .collect();
    assert_eq!(texts, vec!["w1 w2 w3 w4 w5", "w6\n\nx"]);
}

#[test]
fn a_long_breadcrumb_keeps_the_deepest_whole_headings() {
    let cfg = ChunkerConfig {
        target_tokens: 12,
        max_tokens: 12,
        overlap_tokens: 0,
    };
    let d = doc(vec![
        h(1, "Alpha Beta"),
        h(2, "Gamma Delta"),
        h(3, "Install"),
        p("body"),
    ]);
    let chunks = chunk(&d, &cfg, &words);
    assert_eq!(chunks[0].breadcrumb, "Install");
    let cfg = ChunkerConfig {
        target_tokens: 16,
        max_tokens: 16,
        overlap_tokens: 0,
    };
    assert_eq!(
        chunk(&d, &cfg, &words)[0].breadcrumb,
        "Gamma Delta > Install"
    );
}

fn quarter_chars(t: &str) -> usize {
    t.split_whitespace()
        .map(|w| w.chars().count().div_ceil(4))
        .sum()
}

#[test]
fn short_words_before_a_long_blob_are_not_repeated() {
    let cfg = ChunkerConfig {
        target_tokens: 20,
        max_tokens: 20,
        overlap_tokens: 6,
    };
    let text = format!("a b c d e f g h i j {}", "X".repeat(400));
    let chunks = chunk(&doc(vec![p(&text)]), &cfg, &quarter_chars);
    let with_j = chunks
        .iter()
        .filter(|c| c.text.split_whitespace().any(|w| w == "j"))
        .count();
    assert!(
        with_j <= 2,
        "{:?}",
        chunks.iter().map(|c| &c.text).collect::<Vec<_>>()
    );
}

#[test]
fn a_window_is_not_cut_back_to_a_sliver() {
    let cfg = ChunkerConfig {
        target_tokens: 20,
        max_tokens: 20,
        overlap_tokens: 0,
    };
    let text = format!("a {}", "X".repeat(400));
    let first = &chunk(&doc(vec![p(&text)]), &cfg, &quarter_chars)[0];
    assert!(first.token_count >= 10, "{first:?}");
}

#[test]
fn blank_blocks_add_nothing() {
    let d = doc(vec![p("x"), p(""), p("   "), Block::Code("\n  \n".into())]);
    assert_eq!(chunk(&d, &ChunkerConfig::default(), &words)[0].text, "x");
    let only_blank = doc(vec![h(1, "T"), p("   ")]);
    let texts: Vec<String> = chunk(&only_blank, &ChunkerConfig::default(), &words)
        .into_iter()
        .map(|c| c.text)
        .collect();
    assert_eq!(
        texts,
        vec!["T"],
        "a heading with only blank text is a bodiless heading"
    );
}

fn chars(t: &str) -> usize {
    t.chars().count()
}

#[test]
fn a_huge_overlap_is_rejected_without_overflow() {
    let cfg = ChunkerConfig {
        target_tokens: 5,
        max_tokens: 5,
        overlap_tokens: usize::MAX / 2 + 1,
    };
    assert!(chunk_document(&doc(vec![p("a")]), &cfg, &words).is_err());
}

#[test]
fn overlap_is_measured_on_the_repeated_text_itself() {
    let cfg = ChunkerConfig {
        target_tokens: 20,
        max_tokens: 20,
        overlap_tokens: 9,
    };
    let chunks = chunk(&doc(vec![p("ab cd ef gh ij kl mn op qr")]), &cfg, &chars);
    for pair in chunks.windows(2) {
        let shared: Vec<&str> = pair[1]
            .text
            .split(' ')
            .filter(|w| pair[0].text.split(' ').any(|v| v == *w))
            .collect();
        assert!(
            chars(&shared.join(" ")) <= 9,
            "{:?} -> {:?}",
            pair[0].text,
            pair[1].text
        );
    }
}

#[test]
fn overlap_never_takes_more_than_half_a_window() {
    let cfg = ChunkerConfig {
        target_tokens: 3,
        max_tokens: 3,
        overlap_tokens: 1,
    };
    let d = doc(vec![h(1, "H"), p("a b c d e f g")]);
    let chunks = chunk(&d, &cfg, &words);
    assert!(
        chunks.len() <= 4,
        "{:?}",
        chunks.iter().map(|c| &c.text).collect::<Vec<_>>()
    );
}

#[test]
fn a_tiny_budget_drops_the_breadcrumb_instead_of_failing() {
    let cfg = ChunkerConfig {
        target_tokens: 1,
        max_tokens: 1,
        overlap_tokens: 0,
    };
    let chunks = chunk(&doc(vec![h(1, "H"), p("a b c")]), &cfg, &words);
    assert_eq!(chunks.len(), 3);
    assert!(
        chunks
            .iter()
            .all(|c| c.breadcrumb.is_empty() && c.token_count == 1)
    );
}

#[test]
fn cuts_never_separate_a_combining_mark_or_joiner() {
    let cfg = ChunkerConfig {
        target_tokens: 4,
        max_tokens: 4,
        overlap_tokens: 0,
    };
    let quarter = |t: &str| t.chars().count().div_ceil(4);
    let word = "I\u{307}".repeat(20) + &"👩\u{200D}💻".repeat(6);
    for c in chunk(&doc(vec![p(&word)]), &cfg, &quarter) {
        let first = c.text.chars().next().unwrap();
        assert!(!matches!(first, '\u{307}' | '\u{200D}'), "{:?}", c.text);
        assert!(!c.text.ends_with('\u{200D}'), "{:?}", c.text);
    }
}

#[test]
fn flags_and_other_graphemes_are_never_cut() {
    let cfg = ChunkerConfig {
        target_tokens: 16,
        max_tokens: 16,
        overlap_tokens: 0,
    };
    let quarter = |t: &str| t.chars().count().div_ceil(4);
    let text = format!("a{}", "🇹🇷".repeat(40));
    let chunks = chunk(&doc(vec![p(&text)]), &cfg, &quarter);
    for c in &chunks {
        let flags = c.text.trim_start_matches('a');
        assert_eq!(flags.chars().count() % 2, 0, "{:?}", c.text);
        assert!(
            flags
                .chars()
                .all(|ch| ('\u{1F1E6}'..='\u{1F1FF}').contains(&ch))
        );
    }
}

#[test]
fn a_trimmed_breadcrumb_keeps_whole_letters() {
    let cfg = ChunkerConfig {
        target_tokens: 12,
        max_tokens: 12,
        overlap_tokens: 0,
    };
    let d = doc(vec![h(1, &"I\u{307}".repeat(4)), p("x")]);
    let crumb = &chunk(&d, &cfg, &chars)[0].breadcrumb;
    assert!(!crumb.is_empty() && !crumb.ends_with('I'), "{crumb:?}");
}

#[test]
fn the_breadcrumb_leaves_room_to_merge_under_a_small_target() {
    let cfg = ChunkerConfig {
        target_tokens: 20,
        max_tokens: 100,
        overlap_tokens: 0,
    };
    let heading: Vec<String> = (0..20).map(|i| format!("h{i}")).collect();
    let d = doc(vec![h(1, &heading.join(" ")), p("a"), p("b"), p("c")]);
    assert_eq!(chunk(&d, &cfg, &words).len(), 1);
}

#[test]
fn blank_headings_are_ignored() {
    let d = doc(vec![h(1, "Guide"), h(2, "  "), p("x")]);
    let c = &chunk(&d, &ChunkerConfig::default(), &words)[0];
    assert_eq!(
        (c.breadcrumb.as_str(), c.heading_path.clone()),
        ("Guide", vec!["Guide".to_string()])
    );
}

#[test]
fn repeated_body_text_is_at_most_half_the_body() {
    let cfg = ChunkerConfig {
        target_tokens: 16,
        max_tokens: 16,
        overlap_tokens: 7,
    };
    let body: Vec<String> = (0..40).map(|i| format!("w{i}")).collect();
    let d = doc(vec![h(1, "one two three four"), p(&body.join(" "))]);
    let chunks = chunk(&d, &cfg, &words);
    for pair in chunks.windows(2) {
        let shared = pair[1]
            .text
            .split(' ')
            .filter(|w| pair[0].text.split(' ').any(|v| v == *w))
            .count();
        assert!(
            shared * 2 <= words(&pair[0].text),
            "{:?} -> {:?}",
            pair[0].text,
            pair[1].text
        );
    }
}

#[test]
fn headings_without_a_body_are_still_indexed() {
    let only = chunk(
        &doc(vec![h(1, "Installation")]),
        &ChunkerConfig::default(),
        &words,
    );
    assert_eq!(
        only.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
        vec!["Installation"]
    );
    let d = doc(vec![
        h(1, "Guide"),
        h(2, "Deprecated"),
        h(2, "Next"),
        p("x"),
    ]);
    let texts: Vec<String> = chunk(&d, &ChunkerConfig::default(), &words)
        .into_iter()
        .map(|c| c.text)
        .collect();
    assert_eq!(
        texts,
        vec!["Deprecated", "x"],
        "Guide has subsections, so it is only a breadcrumb"
    );
}

#[test]
fn an_inconsistent_config_is_an_internal_error() {
    let cfg = ChunkerConfig {
        target_tokens: 9,
        max_tokens: 8,
        overlap_tokens: 0,
    };
    assert!(matches!(
        chunk_document(&doc(vec![p("a")]), &cfg, &words),
        Err(crate::error::OragError::Internal(_))
    ));
}

#[test]
fn merging_many_small_blocks_does_not_retokenize_each_time() {
    let counted = std::cell::Cell::new(0usize);
    let count = |t: &str| {
        counted.set(counted.get() + t.len());
        words(t)
    };
    let items: Vec<Block> = (0..5000)
        .map(|i| Block::ListItem(format!("item {i}")))
        .collect();
    let input: usize = items.iter().map(|b| b.text().len()).sum();
    chunk(&doc(items), &ChunkerConfig::default(), &count);
    assert!(
        counted.get() < input * 4,
        "tokenized {} bytes for {input}",
        counted.get()
    );
}

#[test]
fn text_without_spaces_still_overlaps() {
    let cfg = ChunkerConfig {
        target_tokens: 40,
        max_tokens: 60,
        overlap_tokens: 20,
    };
    let text = "情報検索".repeat(75);
    let chunks = chunk(&doc(vec![p(&text)]), &cfg, &chars);
    let total: usize = chunks.iter().map(|c| c.text.chars().count()).sum();
    assert!(total > 300, "windows overlap: {total} characters for 300");
}

#[test]
fn merging_counts_the_join_once_with_special_tokens() {
    // 2 special tokens per call, 1 per word, 1 per blank-line join.
    let special = |t: &str| 2 + t.split_whitespace().count() + t.matches("\n\n").count();
    let cfg = ChunkerConfig {
        target_tokens: 12,
        max_tokens: 12,
        overlap_tokens: 0,
    };
    let blocks: Vec<Block> = (0..8).map(|i| p(&format!("w{i}"))).collect();
    let plain = chunk(&doc(blocks.clone()), &cfg, &special);
    for c in &plain {
        assert!(c.token_count <= 12, "{c:?}");
        let last = c.ordinal as usize == plain.len() - 1;
        assert!(
            c.token_count >= 10 || last,
            "merged close to the target: {c:?}"
        );
    }
    let mut with_heading = vec![h(1, "H")];
    with_heading.extend(blocks);
    let under_heading = chunk(&doc(with_heading), &cfg, &special);
    assert!(
        under_heading
            .iter()
            .all(|c| c.token_count >= 10 || c.ordinal as usize == under_heading.len() - 1),
        "{under_heading:?}"
    );
}

#[test]
fn lexical_text_is_the_full_heading_path_and_the_body() {
    let path = vec!["Kargo".to_string(), "İade Koşulları".to_string()];
    assert_eq!(
        lexical_text(&path, "14 gün."),
        "Kargo > İade Koşulları\n\n14 gün."
    );
    assert_eq!(lexical_text(&[], "14 gün."), "14 gün.");
}
