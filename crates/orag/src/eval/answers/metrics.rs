//! Answer-level metrics (Task 43, D-018): citation validity, evidence in the
//! cited chunks, expected facts, abstention as a confusion matrix, and finish
//! reasons, overall and per category and split.
//!
//! `cited_evidence` says the answer's cited chunks contain an expected text.
//! It is evidence coverage, not faithfulness: a cited chunk can hold the fact
//! while the answer still states something else.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::retrieval::answer::FinishReason;

use super::{AnswerProbe, contains_term, normalized};

/// What one answer earned, before it is aggregated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AnswerScore {
    /// `expect` texts found in the answer.
    pub found: usize,
    /// No `[n]` points at a source that was not given.
    pub valid_citations: bool,
    /// Answerable only: an `expect` text occurs in a cited chunk.
    pub cited_evidence: bool,
}

/// Scores one answer; `cited_texts` are the full texts of the chunks the
/// answer cites (an `invalid_citations` entry has none).
pub fn score_answer(
    probe: &AnswerProbe,
    answer: &str,
    invalid_citations: usize,
    cited_texts: &[String],
) -> AnswerScore {
    let answer = normalized(answer);
    let alternatives = |text: &str| text.split('|').map(normalized).collect::<Vec<_>>();
    let found = probe
        .expect
        .iter()
        .filter(|text| {
            alternatives(text)
                .iter()
                .any(|alt| contains_term(&answer, alt))
        })
        .count();
    let cited: Vec<String> = cited_texts.iter().map(|t| normalized(t)).collect();
    let cited_evidence = probe.expect.iter().any(|text| {
        alternatives(text)
            .iter()
            .any(|alt| cited.iter().any(|chunk| contains_term(chunk, alt)))
    });
    AnswerScore {
        found,
        valid_citations: invalid_citations == 0,
        cited_evidence: probe.answerable && cited_evidence,
    }
}

/// Abstention with "the question has no answer in the corpus" as positive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Abstention {
    /// Unanswerable and refused.
    pub true_positive: usize,
    /// Answerable but refused.
    pub false_positive: usize,
    /// Answerable and answered.
    pub true_negative: usize,
    /// Unanswerable but answered.
    pub false_negative: usize,
    /// `None` with no refusal at all (nothing to be precise about).
    pub precision: Option<f64>,
    /// `None` with no unanswerable probe (nothing to recall).
    pub recall: Option<f64>,
}

/// `(answerable, abstained)` per probe.
pub fn confusion(outcomes: &[(bool, bool)]) -> Abstention {
    let mut c = Abstention::default();
    for &(answerable, abstained) in outcomes {
        match (answerable, abstained) {
            (false, true) => c.true_positive += 1,
            (true, true) => c.false_positive += 1,
            (true, false) => c.true_negative += 1,
            (false, false) => c.false_negative += 1,
        }
    }
    c.precision = share(c.true_positive, c.true_positive + c.false_positive);
    c.recall = share(c.true_positive, c.true_positive + c.false_negative);
    c
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct FinishReasons {
    pub stop: usize,
    pub length: usize,
    pub repetition: usize,
}

/// One probe's outcome, as the aggregates need it.
#[derive(Debug, Clone, Copy)]
pub struct Outcome<'a> {
    pub probe: &'a AnswerProbe,
    pub score: AnswerScore,
    pub abstained: bool,
    pub finish_reason: FinishReason,
}

/// The answer-level metrics of a set of probes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnswerMetrics {
    pub probes: usize,
    /// Answered (not abstained) probes whose every `[n]` names a given source.
    pub valid_citation_rate: f64,
    /// Answerable probes with an expected text in a cited chunk.
    pub cited_evidence_coverage: f64,
    /// Answerable probes whose answer holds every expected text.
    pub expected_fact_rate: f64,
    pub abstention: Abstention,
    pub finish_reasons: FinishReasons,
}

pub fn aggregate(outcomes: &[Outcome]) -> AnswerMetrics {
    let answered: Vec<&Outcome> = outcomes.iter().filter(|o| !o.abstained).collect();
    let answerable: Vec<&Outcome> = outcomes.iter().filter(|o| o.probe.answerable).collect();
    let pairs: Vec<(bool, bool)> = outcomes
        .iter()
        .map(|o| (o.probe.answerable, o.abstained))
        .collect();
    let mut finish = FinishReasons::default();
    for o in outcomes {
        match o.finish_reason {
            FinishReason::Stop => finish.stop += 1,
            FinishReason::Length => finish.length += 1,
            FinishReason::Repetition => finish.repetition += 1,
        }
    }
    AnswerMetrics {
        probes: outcomes.len(),
        valid_citation_rate: ratio(
            answered.iter().filter(|o| o.score.valid_citations).count(),
            answered.len(),
        ),
        cited_evidence_coverage: ratio(
            answerable.iter().filter(|o| o.score.cited_evidence).count(),
            answerable.len(),
        ),
        expected_fact_rate: ratio(
            answerable
                .iter()
                .filter(|o| o.score.found == o.probe.expect.len())
                .count(),
            answerable.len(),
        ),
        abstention: confusion(&pairs),
        finish_reasons: finish,
    }
}

/// The metrics per value of `key` (category or split); probes without one
/// are left out.
pub fn breakdown(
    outcomes: &[Outcome],
    key: impl Fn(&AnswerProbe) -> Option<&str>,
) -> BTreeMap<String, AnswerMetrics> {
    let mut groups: BTreeMap<String, Vec<Outcome>> = BTreeMap::new();
    for o in outcomes {
        if let Some(k) = key(o.probe) {
            groups.entry(k.to_string()).or_default().push(*o);
        }
    }
    groups
        .into_iter()
        .map(|(k, group)| (k, aggregate(&group)))
        .collect()
}

/// `part / whole`, 0 for an empty whole (the report says how many probes).
fn ratio(part: usize, whole: usize) -> f64 {
    share(part, whole).unwrap_or(0.0)
}

/// `part / whole`, `None` for an empty whole.
fn share(part: usize, whole: usize) -> Option<f64> {
    (whole > 0).then(|| part as f64 / whole as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(expect: &str) -> AnswerProbe {
        AnswerProbe {
            id: "p".into(),
            lang: "tr".into(),
            query: "q".into(),
            expect: vec![expect.into()],
            answerable: true,
            category: Some("factual".into()),
            split: Some("dev".into()),
            family: None,
        }
    }

    fn sources(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn cited_evidence_coverage_counts_only_cited_chunks() {
        let p = probe("14 gün");
        // The expected text is in source 2, the answer cites only [1].
        let r = score_answer(
            &p,
            "İade süresi 14 gündür [1].",
            0,
            &sources(&["kargo ücretsiz"]),
        );
        assert!(!r.cited_evidence);
        assert_eq!(r.found, 1, "the fact is in the answer itself");
        let r = score_answer(
            &p,
            "İade süresi 14 gündür [2].",
            0,
            &sources(&["14 gün içinde iade"]),
        );
        assert!(r.cited_evidence);
        let unanswerable = AnswerProbe {
            answerable: false,
            expect: Vec::new(),
            ..p
        };
        assert!(!score_answer(&unanswerable, "x", 0, &sources(&["14 gün"])).cited_evidence);
    }

    #[test]
    fn abstention_confusion_is_counted() {
        // (answerable, abstained)
        let c = confusion(&[(false, true), (false, false), (true, true), (true, false)]);
        assert_eq!(
            (
                c.true_positive,
                c.false_negative,
                c.false_positive,
                c.true_negative
            ),
            (1, 1, 1, 1)
        );
        assert_eq!((c.precision, c.recall), (Some(0.5), Some(0.5)));
        // Answerable probes only, none refused: nothing to measure, not 0.
        let none = confusion(&[(true, false), (true, false)]);
        assert_eq!((none.precision, none.recall), (None, None));
    }

    #[test]
    fn invalid_citation_lowers_valid_citation_rate() {
        let p = probe("14 gün");
        let good = score_answer(&p, "14 gün [1].", 0, &sources(&["14 gün"]));
        // The answer cites [9] while three sources were given.
        let bad = score_answer(&p, "14 gün [9].", 1, &[]);
        assert!(good.valid_citations && !bad.valid_citations);
        let outcome = |score| Outcome {
            probe: &p,
            score,
            abstained: false,
            finish_reason: FinishReason::Stop,
        };
        let m = aggregate(&[outcome(good), outcome(bad)]);
        assert_eq!(m.valid_citation_rate, 0.5);
        assert_eq!(m.expected_fact_rate, 1.0);
        assert_eq!(m.cited_evidence_coverage, 0.5);
    }

    #[test]
    fn abstained_answers_do_not_count_for_citations_and_reasons_are_tallied() {
        let p = probe("x");
        let refused = Outcome {
            probe: &p,
            score: score_answer(&p, "Bilgi yok [7].", 1, &[]),
            abstained: true,
            finish_reason: FinishReason::Length,
        };
        let m = aggregate(&[refused]);
        assert_eq!(m.valid_citation_rate, 0.0, "no answered probe");
        assert_eq!(
            m.finish_reasons,
            FinishReasons {
                stop: 0,
                length: 1,
                repetition: 0
            }
        );
    }

    #[test]
    fn breakdown_groups_by_category_and_skips_untagged() {
        let a = probe("x");
        let b = AnswerProbe {
            category: Some("numeric".into()),
            ..probe("y")
        };
        let c = AnswerProbe {
            category: None,
            ..probe("z")
        };
        let outcomes: Vec<Outcome> = [&a, &b, &c]
            .into_iter()
            .map(|p| Outcome {
                probe: p,
                score: score_answer(p, "x", 0, &[]),
                abstained: false,
                finish_reason: FinishReason::Stop,
            })
            .collect();
        let by = breakdown(&outcomes, |p| p.category.as_deref());
        assert_eq!(by.keys().collect::<Vec<_>>(), ["factual", "numeric"]);
        assert_eq!(by["factual"].expected_fact_rate, 1.0);
        assert_eq!(by["numeric"].expected_fact_rate, 0.0);
    }
}
