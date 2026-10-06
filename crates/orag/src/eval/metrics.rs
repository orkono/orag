//! Retrieval metrics over ranked chunk lists.

use crate::eval::dataset::Relevant;

pub(crate) fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// For each label, the 1-based rank of the first `(document, chunk_text)` that
/// comes from the labeled document and contains the labeled substring.
pub fn label_ranks(ranked: &[(String, String)], labels: &[Relevant]) -> Vec<Option<usize>> {
    labels
        .iter()
        .map(|label| {
            let needle = squash(&label.contains);
            ranked
                .iter()
                .position(|(document, text)| {
                    *document == label.document && squash(text).contains(&needle)
                })
                .map(|index| index + 1)
        })
        .collect()
}

pub fn recall_at(ranks: &[Option<usize>], k: usize) -> f64 {
    if ranks.is_empty() {
        return 0.0;
    }
    ranks.iter().filter(|r| r.is_some_and(|r| r <= k)).count() as f64 / ranks.len() as f64
}

pub fn mrr_at(ranks: &[Option<usize>], k: usize) -> f64 {
    ranks
        .iter()
        .flatten()
        .filter(|r| **r <= k)
        .min()
        .map_or(0.0, |r| 1.0 / *r as f64)
}

/// Evidence-coverage nDCG@k over labels (not chunks), so one label found in
/// two overlapping chunks counts once and one chunk covering two labels cannot
/// exceed the ideal. Sorted label ranks r₁ ≤ r₂ ≤ … are placed at effective
/// positions pⱼ = max(rⱼ, j); gain 1/log₂(pⱼ + 1) when pⱼ ≤ k. Always in [0, 1].
pub fn ndcg_at(ranks: &[Option<usize>], k: usize) -> f64 {
    let mut found: Vec<usize> = ranks.iter().flatten().copied().collect();
    found.sort_unstable();
    let dcg: f64 = found
        .iter()
        .enumerate()
        .map(|(j, &rank)| rank.max(j + 1))
        .filter(|&position| position <= k)
        .map(|position| 1.0 / (position as f64 + 1.0).log2())
        .sum();
    let ideal: f64 = (1..=ranks.len().min(k))
        .map(|position| 1.0 / (position as f64 + 1.0).log2())
        .sum();
    if ideal == 0.0 { 0.0 } else { dcg / ideal }
}

/// Nearest-rank percentile; 0.0 for an empty slice.
pub fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted[rank.min(sorted.len()) - 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(document: &str, contains: &str) -> Relevant {
        Relevant {
            document: document.into(),
            contains: contains.into(),
        }
    }

    #[test]
    fn label_ranks_find_first_matching_chunk() {
        let ranked = vec![
            ("a.md".to_string(), "x  foo\ny".to_string()),
            ("b.md".to_string(), "bar".to_string()),
        ];
        let ranks = label_ranks(
            &ranked,
            &[rel("a.md", "foo y"), rel("b.md", "bar"), rel("c.md", "z")],
        );
        assert_eq!(ranks, vec![Some(1), Some(2), None]);
    }

    #[test]
    fn recall_mrr_ndcg_and_percentile() {
        let ranks = [Some(1), Some(2), None];
        assert!((recall_at(&ranks, 1) - 1.0 / 3.0).abs() < 1e-9);
        assert!((recall_at(&ranks, 5) - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(mrr_at(&ranks, 10), 1.0);
        assert!((mrr_at(&[None, Some(3)], 10) - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(mrr_at(&[Some(3)], 2), 0.0);
        assert!((ndcg_at(&[Some(2)], 10) - 1.0 / 3f64.log2()).abs() < 1e-9);
        assert_eq!(ndcg_at(&[Some(1)], 10), 1.0);
        assert_eq!(ndcg_at(&[None], 10), 0.0);
    }

    #[test]
    fn ndcg_counterexamples_stay_within_bounds() {
        // Two labels both satisfied by the first chunk: perfect, not > 1.
        assert!((ndcg_at(&[Some(1), Some(1)], 10) - 1.0).abs() < 1e-9);
        // One label whose evidence appears in two chunks is counted once (label_ranks keeps the first).
        let ranked = vec![
            ("a.md".to_string(), "foo".to_string()),
            ("a.md".to_string(), "foo again".to_string()),
        ];
        let ranks = label_ranks(&ranked, &[rel("a.md", "foo")]);
        assert_eq!(ndcg_at(&ranks, 10), 1.0);
        assert_eq!(percentile(&[10.0, 20.0, 30.0, 40.0], 50.0), 20.0);
        assert_eq!(percentile(&[10.0, 20.0, 30.0, 40.0], 95.0), 40.0);
        assert_eq!(percentile(&[], 95.0), 0.0);
    }
}
