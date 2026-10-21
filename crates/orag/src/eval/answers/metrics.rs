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
    /// At least one `[n]`, and every one names a source that was given.
    pub valid_citations: bool,
    /// Answerable and not refused: an `expect` text occurs in a cited chunk.
    pub cited_evidence: bool,
}

/// Scores one answer: `citations` are its valid `[n]` numbers,
/// `invalid_citations` the count of the others, `cited_texts` the full texts
/// of the chunks it cites.
pub fn score_answer(
    probe: &AnswerProbe,
    answer: &str,
    abstained: bool,
    citations: usize,
    invalid_citations: usize,
    cited_texts: &[String],
) -> AnswerScore {
    let alternatives = |text: &str| text.split('|').map(normalized).collect::<Vec<_>>();
    let found = found_facts(probe, answer, abstained).len();
    let cited: Vec<String> = cited_texts.iter().map(|t| normalized(t)).collect();
    let cited_evidence = probe.expect.iter().any(|text| {
        alternatives(text)
            .iter()
            .any(|alt| cited.iter().any(|chunk| contains_term(chunk, alt)))
    });
    AnswerScore {
        found,
        // An answer without any `[n]` cites nothing, so it is not valid either.
        valid_citations: citations > 0 && invalid_citations == 0,
        // A refusal cites in passing at most; it does not cover the evidence.
        cited_evidence: probe.answerable && !abstained && cited_evidence,
    }
}

/// The indexes of the `expect` texts the answer contains (one of their `|`
/// alternatives); a refusal gives none, even when it quotes a fact in
/// passing. The v0.6 bench compares backends fact by fact with these.
pub fn found_facts(probe: &AnswerProbe, answer: &str, abstained: bool) -> Vec<usize> {
    if abstained {
        return Vec::new();
    }
    let answer = normalized(answer);
    probe
        .expect
        .iter()
        .enumerate()
        .filter(|(_, text)| {
            text.split('|')
                .map(normalized)
                .any(|alt| contains_term(&answer, &alt))
        })
        .map(|(index, _)| index)
        .collect()
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
    /// Answered without any valid `[n]` (the summary's `uncited`).
    pub uncited: bool,
    pub finish_reason: FinishReason,
}

/// The answer-level metrics of a set of probes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnswerMetrics {
    pub probes: usize,
    /// Answered (not refused) probes with at least one `[n]`, all naming a
    /// given source. `None` when no probe was answered.
    pub valid_citation_rate: Option<f64>,
    /// Answerable probes with an expected text in a cited chunk (`None`
    /// without answerable probes).
    pub cited_evidence_coverage: Option<f64>,
    /// Answerable, not refused probes whose answer holds every expected text (`None`
    /// without answerable probes).
    pub expected_fact_rate: Option<f64>,
    pub abstention: Abstention,
    /// Answered probes without any valid `[n]` (step 67).
    pub uncited: usize,
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
        valid_citation_rate: share(
            answered.iter().filter(|o| o.score.valid_citations).count(),
            answered.len(),
        ),
        cited_evidence_coverage: share(
            answerable.iter().filter(|o| o.score.cited_evidence).count(),
            answerable.len(),
        ),
        expected_fact_rate: share(
            answerable
                .iter()
                .filter(|o| o.score.found == o.probe.expect.len())
                .count(),
            answerable.len(),
        ),
        abstention: confusion(&pairs),
        uncited: outcomes.iter().filter(|o| o.uncited).count(),
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
            false,
            1,
            0,
            &sources(&["kargo ücretsiz"]),
        );
        assert!(!r.cited_evidence);
        assert_eq!(r.found, 1, "the fact is in the answer itself");
        let r = score_answer(
            &p,
            "İade süresi 14 gündür [2].",
            false,
            1,
            0,
            &sources(&["14 gün içinde iade"]),
        );
        assert!(r.cited_evidence);
        let unanswerable = AnswerProbe {
            answerable: false,
            expect: Vec::new(),
            ..p.clone()
        };
        assert!(
            !score_answer(&unanswerable, "x [1]", false, 1, 0, &sources(&["14 gün"]))
                .cited_evidence
        );
        // A refusal that cites the evidence chunk in passing does not cover it.
        let refused = score_answer(
            &p,
            "Kaynaklar süreyi söylemiyor; [1] iadeden söz eder.",
            true,
            1,
            0,
            &sources(&["14 gün içinde iade"]),
        );
        assert!(!refused.cited_evidence);
        // Nor does it hold the fact it quotes while saying it is not there.
        assert_eq!(refused.found, 0);
        let quoting = score_answer(
            &p,
            "Süre doğrudan belirtilmemiştir; belgede 14 gün geçer.",
            true,
            0,
            0,
            &[],
        );
        assert_eq!(quoting.found, 0);
    }

    #[test]
    fn an_answer_without_citations_is_not_validly_cited() {
        let p = probe("14 gün");
        assert!(!score_answer(&p, "İade süresi 14 gündür.", false, 0, 0, &[]).valid_citations);
        assert!(
            score_answer(
                &p,
                "İade süresi 14 gündür [1].",
                false,
                1,
                0,
                &sources(&["14 gün"])
            )
            .valid_citations
        );
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
        let good = score_answer(&p, "14 gün [1].", false, 1, 0, &sources(&["14 gün"]));
        // The answer cites [9] while three sources were given.
        let bad = score_answer(&p, "14 gün [9].", false, 0, 1, &[]);
        assert!(good.valid_citations && !bad.valid_citations);
        let outcome = |score| Outcome {
            probe: &p,
            score,
            abstained: false,
            uncited: !score.valid_citations,
            finish_reason: FinishReason::Stop,
        };
        let m = aggregate(&[outcome(good), outcome(bad)]);
        assert_eq!(m.uncited, 1);
        assert_eq!(m.valid_citation_rate, Some(0.5));
        assert_eq!(m.expected_fact_rate, Some(1.0));
        assert_eq!(m.cited_evidence_coverage, Some(0.5));
    }

    #[test]
    fn abstained_answers_do_not_count_for_citations_and_reasons_are_tallied() {
        let p = probe("x");
        let refused = Outcome {
            probe: &p,
            score: score_answer(&p, "Bilgi yok [7].", true, 0, 1, &[]),
            abstained: true,
            uncited: false,
            finish_reason: FinishReason::Length,
        };
        let m = aggregate(&[refused]);
        assert_eq!(m.uncited, 0);
        assert_eq!(
            m.valid_citation_rate, None,
            "no answered probe: nothing to measure"
        );
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
                score: score_answer(p, "x", false, 0, 0, &[]),
                abstained: false,
                uncited: true,
                finish_reason: FinishReason::Stop,
            })
            .collect();
        let by = breakdown(&outcomes, |p| p.category.as_deref());
        assert_eq!(by.keys().collect::<Vec<_>>(), ["factual", "numeric"]);
        assert_eq!(by["factual"].expected_fact_rate, Some(1.0));
        assert_eq!(by["numeric"].expected_fact_rate, Some(0.0));
        assert_eq!(by["numeric"].uncited, 1);
        // A group of unanswerable probes has no expected facts to rate.
        let none = AnswerProbe {
            answerable: false,
            expect: Vec::new(),
            ..probe("w")
        };
        let only_unanswerable = [Outcome {
            probe: &none,
            score: score_answer(&none, "x", true, 0, 0, &[]),
            abstained: true,
            uncited: false,
            finish_reason: FinishReason::Stop,
        }];
        assert_eq!(aggregate(&only_unanswerable).expected_fact_rate, None);
    }
}

#[cfg(test)]
mod found_fact_tests {
    use super::*;

    fn probe(expect: &[&str]) -> AnswerProbe {
        AnswerProbe {
            id: "p".into(),
            lang: "tr".into(),
            query: "q".into(),
            expect: expect.iter().map(|s| s.to_string()).collect(),
            answerable: true,
            category: None,
            split: None,
            family: None,
        }
    }

    #[test]
    fn found_facts_names_which_expect_texts_were_found() {
        let p = probe(&["2009", "18 TL|18 lira"]);
        assert_eq!(found_facts(&p, "since 2009 [1]", false), vec![0]);
        assert_eq!(found_facts(&p, "18 lira, not 2010 [1]", false), vec![1]);
        assert_eq!(found_facts(&p, "2009 and 18 TL", false), vec![0, 1]);
        // A refusal gives no fact, as in `score_answer`.
        assert!(found_facts(&p, "2009 is not in the sources", true).is_empty());
        let score = score_answer(&p, "2009 and 18 TL", false, 1, 0, &[]);
        assert_eq!(score.found, found_facts(&p, "2009 and 18 TL", false).len());
    }
}
