//! Per-question retrieval scores and their means over a group of questions.
//!
//! A question's labels are alternative evidence (any one is enough) unless it
//! is `require_all` (every label is needed, e.g. facts from two documents).
//! Recall, MRR, nDCG and the in-context check all follow that same rule.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::eval::metrics::ndcg_at;

/// The question counts as found within `k`: any one label, or every label
/// when `require_all`.
pub fn question_found(ranks: &[Option<usize>], k: usize, require_all: bool) -> bool {
    let within = |rank: &Option<usize>| rank.is_some_and(|r| r <= k);
    if require_all {
        !ranks.is_empty() && ranks.iter().all(within)
    } else {
        ranks.iter().any(within)
    }
}

/// The rank at which the question is answered: the best label, or for
/// `require_all` the last label needed. `None` when it is not answered.
fn answered_at(ranks: &[Option<usize>], require_all: bool) -> Option<usize> {
    if require_all {
        if ranks.is_empty() {
            return None;
        }
        ranks
            .iter()
            .copied()
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .max()
    } else {
        ranks.iter().flatten().copied().min()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuestionScore {
    pub found_at_5: bool,
    pub found_at_10: bool,
    pub in_context: bool,
    pub reciprocal_rank: f64,
    pub ndcg: f64,
}

/// Scores one question at depth 10; `context` is how many chunks the answer
/// prompt holds.
pub fn score_question(ranks: &[Option<usize>], context: usize, require_all: bool) -> QuestionScore {
    let answered = answered_at(ranks, require_all).filter(|&r| r <= 10);
    let ndcg = if require_all {
        // Coverage of every needed label.
        ndcg_at(ranks, 10)
    } else {
        // One label is the whole answer, so the ideal is that label at rank 1.
        answered.map_or(0.0, |r| 1.0 / (r as f64 + 1.0).log2())
    };
    QuestionScore {
        found_at_5: question_found(ranks, 5, require_all),
        found_at_10: question_found(ranks, 10, require_all),
        in_context: question_found(ranks, context, require_all),
        reciprocal_rank: answered.map_or(0.0, |r| 1.0 / r as f64),
        ndcg,
    }
}

/// Mean metrics over a group of questions (all, one category, one split).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GroupReport {
    pub queries: usize,
    pub recall_at_5: f64,
    pub recall_at_10: f64,
    pub mrr_at_10: f64,
    pub ndcg_at_10: f64,
    pub in_context: f64,
}

#[derive(Default)]
pub struct GroupSums {
    queries: usize,
    found_at_5: usize,
    found_at_10: usize,
    in_context: usize,
    reciprocal_rank: f64,
    ndcg: f64,
}

impl GroupSums {
    pub fn add(&mut self, score: &QuestionScore) {
        self.queries += 1;
        self.found_at_5 += usize::from(score.found_at_5);
        self.found_at_10 += usize::from(score.found_at_10);
        self.in_context += usize::from(score.in_context);
        self.reciprocal_rank += score.reciprocal_rank;
        self.ndcg += score.ndcg;
    }

    pub fn report(&self) -> GroupReport {
        let n = self.queries.max(1) as f64;
        GroupReport {
            queries: self.queries,
            recall_at_5: self.found_at_5 as f64 / n,
            recall_at_10: self.found_at_10 as f64 / n,
            mrr_at_10: self.reciprocal_rank / n,
            ndcg_at_10: self.ndcg / n,
            in_context: self.in_context as f64 / n,
        }
    }
}

/// Means per group key, in key order.
pub fn group_reports<'a>(
    scores: impl Iterator<Item = (&'a str, &'a QuestionScore)>,
) -> BTreeMap<String, GroupReport> {
    let mut sums: BTreeMap<String, GroupSums> = BTreeMap::new();
    for (key, score) in scores {
        sums.entry(key.to_string()).or_default().add(score);
    }
    sums.into_iter()
        .map(|(key, sums)| (key, sums.report()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_all_needs_every_label_and_otherwise_one_is_enough() {
        assert!(!question_found(&[Some(1), None], 10, true));
        assert!(question_found(&[Some(1), None], 10, false));
        assert!(question_found(&[Some(2), Some(9)], 10, true));
        assert!(!question_found(&[Some(2), Some(11)], 10, true));
        assert!(!question_found(&[None], 10, false));
        assert!(!question_found(&[], 10, false));
        assert!(!question_found(&[], 10, true));
    }

    #[test]
    fn a_half_found_require_all_question_scores_nothing() {
        let score = score_question(&[Some(1), None], 8, true);
        assert!(!score.found_at_10 && !score.in_context);
        assert_eq!(score.reciprocal_rank, 0.0);
        // Ranked by the last label it needs.
        let both = score_question(&[Some(1), Some(4)], 8, true);
        assert!((both.reciprocal_rank - 0.25).abs() < 1e-12);
    }

    #[test]
    fn alternative_labels_reach_a_perfect_score_with_one_label() {
        let score = score_question(&[Some(1), None], 8, false);
        assert_eq!((score.reciprocal_rank, score.ndcg), (1.0, 1.0));
        let later = score_question(&[None, Some(3)], 8, false);
        assert!((later.reciprocal_rank - 1.0 / 3.0).abs() < 1e-12);
        assert!((later.ndcg - 0.5).abs() < 1e-12);
        let outside = score_question(&[Some(11)], 8, false);
        assert_eq!((outside.reciprocal_rank, outside.ndcg), (0.0, 0.0));
    }

    #[test]
    fn groups_average_their_questions() {
        let hit = score_question(&[Some(1)], 8, false);
        let miss = score_question(&[None], 8, false);
        let reports = group_reports([("a", &hit), ("a", &miss), ("b", &hit)].into_iter());
        assert_eq!(reports["a"].queries, 2);
        assert!((reports["a"].recall_at_10 - 0.5).abs() < 1e-12);
        assert_eq!(reports["b"].in_context, 1.0);
    }
}
