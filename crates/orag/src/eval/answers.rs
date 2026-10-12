//! Answer-level evaluation: runs the full answer path (retrieval, prompt,
//! generation) per sampler profile and checks what users see: expected facts
//! present, refusals, loops and answers cut by the output budget.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::domain::normalize::normalize_for_lexical;
use crate::error::{OragError, Result};
use crate::eval::answers::metrics::{AnswerMetrics, Outcome, aggregate, breakdown, score_answer};
use crate::eval::dataset::{SplitGuard, check_grouping};
use crate::eval::metrics::{percentile, squash};
use crate::eval::retrieval::index_corpus;
use crate::infer::{Embedder, Generator, SamplerProfile};
use crate::retrieval::answer::{AnswerEngine, AnswerEvent, AnswerSummary, FinishReason, SourceRef};
use crate::retrieval::hybrid::{RetrievalConfig, Retriever};
use crate::retrieval::repetition::line_key;
use crate::store::Store;
use crate::store::search::SqliteVecIndex;

pub mod judge;
pub mod metrics;

/// One question; `expect` lists texts a complete answer contains (compared
/// after lexical normalization, so case, `İ`/`ı` and the circumflex do not
/// matter). `|` separates alternatives: `600|altıyüz`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerProbe {
    pub id: String,
    pub lang: String,
    pub query: String,
    #[serde(default)]
    pub expect: Vec<String>,
    pub answerable: bool,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub split: Option<String>,
    #[serde(default)]
    pub family: Option<String>,
}

pub fn load_probes(path: &Path) -> Result<Vec<AnswerProbe>> {
    let text = std::fs::read_to_string(path)?;
    let mut probes = Vec::new();
    let mut ids = HashSet::new();
    let mut splits = SplitGuard::default();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let at = |msg: String| {
            OragError::InvalidInput(format!("{} line {}: {msg}", path.display(), index + 1))
        };
        let probe: AnswerProbe = serde_json::from_str(line).map_err(|e| at(e.to_string()))?;
        if probe.answerable == probe.expect.is_empty() {
            return Err(at(
                "answerable probes need `expect` texts; unanswerable ones must have none".into(),
            ));
        }
        // An empty text is contained in every answer and would always count as found.
        if probe
            .expect
            .iter()
            .any(|text| text.split('|').any(|alt| normalized(alt).is_empty()))
        {
            return Err(at(
                "`expect` texts and their `|` alternatives must not be empty".into(),
            ));
        }
        check_grouping(
            probe.category.as_deref(),
            probe.split.as_deref(),
            probe.answerable,
        )
        .map_err(at)?;
        splits
            .family(probe.family.as_deref(), probe.split.as_deref())
            .map_err(at)?;
        if !ids.insert(probe.id.clone()) {
            return Err(at(format!("duplicate id {}", probe.id)));
        }
        probes.push(probe);
    }
    // An empty set would print an all-zero report that reads like a regression.
    if !probes.iter().any(|p| p.answerable) {
        return Err(OragError::InvalidInput(format!(
            "{} has no answerable probes",
            path.display()
        )));
    }
    Ok(probes)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProbeResult {
    pub id: String,
    pub finish_reason: FinishReason,
    pub abstained: bool,
    pub found: usize,
    pub expected: usize,
    pub valid_citations: bool,
    pub cited_evidence: bool,
    /// Experimental (`--judge`): the generator's YES/NO on whether the answer
    /// only states what its cited sources say; `None` when not judged.
    pub judge: Option<bool>,
    pub distinct_line_ratio: f64,
    pub completion_tokens: usize,
    pub ms: f64,
    pub answer: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProfileReport {
    pub sampler: String,
    /// Mean share of `expect` texts found, over answerable probes.
    pub coverage: f64,
    /// Answerable probes with every `expect` text found.
    pub complete: usize,
    /// Answerable probes the model refused.
    pub refused_answerable: usize,
    /// Unanswerable probes the model refused (the wanted outcome).
    pub refused_unanswerable: usize,
    pub length_stops: usize,
    pub repetition_stops: usize,
    pub mean_distinct_line_ratio: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    /// Citation validity, cited evidence, expected facts, abstention and
    /// finish reasons (D-018), overall and per category and split.
    pub metrics: AnswerMetrics,
    pub by_category: BTreeMap<String, AnswerMetrics>,
    pub by_split: BTreeMap<String, AnswerMetrics>,
    /// Signals that no gate uses (D-018).
    pub experimental: Experimental,
    pub results: Vec<ProbeResult>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Experimental {
    /// YES share of the judged answers (`--judge`); `None` without it.
    pub judge_yes_rate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnswerReport {
    pub generator: String,
    pub answerable: usize,
    pub unanswerable: usize,
    pub profiles: Vec<ProfileReport>,
}

impl AnswerReport {
    pub fn to_markdown(&self) -> String {
        let mut out = format!(
            "Generator: `{}` · answerable: {} · unanswerable: {}\n\n\
             | sampler | expected facts | cited evidence | valid citations | abstention P / R | coverage | length stops | repetition stops | distinct lines | p50 ms | p95 ms |\n\
             |---|---|---|---|---|---|---|---|---|---|---|\n",
            self.generator, self.answerable, self.unanswerable
        );
        for p in &self.profiles {
            let m = &p.metrics;
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} / {} | {:.3} | {} | {} | {:.3} | {:.0} | {:.0} |\n",
                p.sampler,
                rate(m.expected_fact_rate),
                rate(m.cited_evidence_coverage),
                rate(m.valid_citation_rate),
                rate(m.abstention.precision),
                rate(m.abstention.recall),
                p.coverage,
                m.finish_reasons.length,
                m.finish_reasons.repetition,
                p.mean_distinct_line_ratio,
                p.p50_ms,
                p.p95_ms
            ));
        }
        for p in &self.profiles {
            for (title, groups) in [("category", &p.by_category), ("split", &p.by_split)] {
                if groups.is_empty() {
                    continue;
                }
                out.push_str(&format!(
                    "\n`{}` by {title}:\n\n| {title} | probes | expected facts | cited evidence | valid citations | abstention P / R |\n|---|---|---|---|---|---|\n",
                    p.sampler
                ));
                for (name, m) in groups {
                    out.push_str(&format!(
                        "| {name} | {} | {} | {} | {} | {} / {} |\n",
                        m.probes,
                        rate(m.expected_fact_rate),
                        rate(m.cited_evidence_coverage),
                        rate(m.valid_citation_rate),
                        rate(m.abstention.precision),
                        rate(m.abstention.recall)
                    ));
                }
            }
        }
        out
    }
}

/// A rate with three decimals, or `–` when there was nothing to measure.
fn rate(value: Option<f64>) -> String {
    value.map_or_else(|| "–".to_string(), |v| format!("{v:.3}"))
}

pub fn run_answer_eval(
    embedder: Arc<dyn Embedder>,
    generator: Arc<dyn Generator>,
    corpus: &Path,
    probes: &[AnswerProbe],
    samplers: &[SamplerProfile],
    work_dir: &Path,
    with_judge: bool,
) -> Result<AnswerReport> {
    let db = work_dir.join("eval.db");
    if db.exists() {
        return Err(OragError::InvalidInput(format!(
            "{} already exists; use an empty work dir",
            db.display()
        )));
    }
    let store = Arc::new(Store::open(&db)?);
    index_corpus(&store, embedder.clone(), corpus)?;
    let profiles = samplers
        .iter()
        .map(|&sampler| {
            let engine = AnswerEngine {
                retriever: Retriever {
                    index: Arc::new(SqliteVecIndex::new(store.clone())),
                    store: store.clone(),
                    embedder: embedder.clone(),
                    config: RetrievalConfig::default(),
                },
                generator: generator.clone(),
                sampler,
            };
            evaluate_profile(&engine, probes, with_judge)
        })
        .collect::<Result<Vec<_>>>()?;
    let answerable = probes.iter().filter(|p| p.answerable).count();
    Ok(AnswerReport {
        generator: generator.model_id().to_string(),
        answerable,
        unanswerable: probes.len() - answerable,
        profiles,
    })
}

fn evaluate_profile(
    engine: &AnswerEngine,
    probes: &[AnswerProbe],
    with_judge: bool,
) -> Result<ProfileReport> {
    let results = probes
        .iter()
        .map(|probe| run_probe(engine, probe, with_judge))
        .collect::<Result<Vec<_>>>()?;
    let answerable: Vec<(&AnswerProbe, &ProbeResult)> = probes
        .iter()
        .zip(&results)
        .filter(|(p, _)| p.answerable)
        .collect();
    let coverage = mean(
        answerable
            .iter()
            .map(|(_, r)| r.found as f64 / r.expected as f64),
    );
    let ms: Vec<f64> = results.iter().map(|r| r.ms).collect();
    let outcomes: Vec<Outcome> = probes
        .iter()
        .zip(&results)
        .map(|(probe, r)| Outcome {
            probe,
            score: metrics::AnswerScore {
                found: r.found,
                valid_citations: r.valid_citations,
                cited_evidence: r.cited_evidence,
            },
            abstained: r.abstained,
            finish_reason: r.finish_reason,
        })
        .collect();
    Ok(ProfileReport {
        sampler: engine.sampler.name(),
        coverage,
        complete: answerable
            .iter()
            .filter(|(_, r)| r.found == r.expected)
            .count(),
        refused_answerable: answerable.iter().filter(|(_, r)| r.abstained).count(),
        refused_unanswerable: probes
            .iter()
            .zip(&results)
            .filter(|(p, r)| !p.answerable && r.abstained)
            .count(),
        length_stops: count_finish(&results, FinishReason::Length),
        repetition_stops: count_finish(&results, FinishReason::Repetition),
        mean_distinct_line_ratio: mean(results.iter().map(|r| r.distinct_line_ratio)),
        p50_ms: percentile(&ms, 50.0),
        p95_ms: percentile(&ms, 95.0),
        metrics: aggregate(&outcomes),
        by_category: breakdown(&outcomes, |p| p.category.as_deref()),
        by_split: breakdown(&outcomes, |p| p.split.as_deref()),
        experimental: Experimental {
            // `None` without `--judge`, and when no probe got a verdict.
            judge_yes_rate: with_judge
                .then(|| {
                    let judged: Vec<bool> = results.iter().filter_map(|r| r.judge).collect();
                    let yes = judged.iter().filter(|&&yes| yes).count();
                    (!judged.is_empty()).then(|| yes as f64 / judged.len() as f64)
                })
                .flatten(),
        },
        results,
    })
}

fn run_probe(engine: &AnswerEngine, probe: &AnswerProbe, with_judge: bool) -> Result<ProbeResult> {
    let started = Instant::now();
    let mut summary: Option<AnswerSummary> = None;
    let mut sources: Vec<SourceRef> = Vec::new();
    engine.answer(1, &probe.query, &mut |event| {
        match event {
            AnswerEvent::Sources(given) => sources = given,
            AnswerEvent::Done(done) => summary = Some(*done),
            AnswerEvent::Token(_) => {}
        }
        ControlFlow::Continue(())
    })?;
    let summary = summary.ok_or_else(|| {
        OragError::Internal(format!("{}: the answer ended without a summary", probe.id))
    })?;
    // Excerpts are cut short; the cited chunks' full texts come from the store.
    let cited = cited_sources(&sources, &summary.citations);
    let ids: Vec<i64> = cited.iter().map(|&(_, chunk_id)| chunk_id).collect();
    let texts: HashMap<i64, String> = engine
        .retriever
        .store
        .get_chunks(1, &ids)?
        .into_iter()
        .map(|chunk| (chunk.id, chunk.text))
        .collect();
    let cited: Vec<(usize, String)> = cited
        .into_iter()
        .filter_map(|(number, id)| texts.get(&id).map(|text| (number, text.clone())))
        .collect();
    // Only answers that cite something can be judged against their sources.
    // The judge is experimental: if it cannot run (its prompt too long for
    // the context), the probe stays unjudged and the run goes on.
    let judged = with_judge && probe.answerable && !summary.abstained && !cited.is_empty();
    let judge = if judged {
        judge::judge(engine.generator.as_ref(), &summary.answer, &cited).unwrap_or_else(|err| {
            tracing::warn!(probe = %probe.id, error = %err, "the judge could not run; left unjudged");
            None
        })
    } else {
        None
    };
    let cited_texts: Vec<String> = cited.into_iter().map(|(_, text)| text).collect();
    let score = score_answer(
        probe,
        &summary.answer,
        summary.abstained,
        summary.citations.len(),
        summary.invalid_citations.len(),
        &cited_texts,
    );
    Ok(ProbeResult {
        id: probe.id.clone(),
        finish_reason: summary.finish_reason,
        abstained: summary.abstained,
        found: score.found,
        expected: probe.expect.len(),
        valid_citations: score.valid_citations,
        cited_evidence: score.cited_evidence,
        judge,
        distinct_line_ratio: distinct_line_ratio(&summary.answer),
        completion_tokens: summary.trace.completion_tokens,
        ms: started.elapsed().as_secs_f64() * 1000.0,
        answer: summary.answer,
    })
}

/// The sources the answer cites, as `(number, chunk id)` in number order.
fn cited_sources(sources: &[SourceRef], citations: &[usize]) -> Vec<(usize, i64)> {
    let mut cited: Vec<(usize, i64)> = sources
        .iter()
        .filter(|s| citations.contains(&s.number))
        .map(|s| (s.number, s.chunk_id))
        .collect();
    cited.sort_unstable();
    cited
}

/// Lexical normalization, then whitespace collapsed (normalization itself can
/// leave double spaces, e.g. from private-use PDF glyphs).
fn normalized(text: &str) -> String {
    squash(&normalize_for_lexical(text))
}

/// Whether `needle` occurs in `haystack` starting at a word boundary, so `600`
/// is not found in `1600` nor `laik` in `alaika`. The end is open, since
/// Turkish adds suffixes (`türkçe` in `türkçedir`, `onan` in `onanmasına`),
/// except after a digit: `600` is not found in `6000`.
/// A dot between digits (`1.600`, `01.02.2016`) belongs to the number.
fn contains_term(haystack: &str, needle: &str) -> bool {
    let is_word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    let digit = |c: Option<char>| c.is_some_and(|c| c.is_ascii_digit());
    let starts_in_digit = digit(needle.chars().next());
    let ends_in_digit = digit(needle.chars().next_back());
    haystack.match_indices(needle).any(|(start, _)| {
        let mut before = haystack[..start].chars().rev();
        let mut after = haystack[start + needle.len()..].chars();
        let (b1, b2) = (before.next(), before.next());
        let (a1, a2) = (after.next(), after.next());
        let number_goes_on_before = starts_in_digit && b1 == Some('.') && digit(b2);
        let number_goes_on_after = ends_in_digit && (digit(a1) || (a1 == Some('.') && digit(a2)));
        !is_word(b1) && !number_goes_on_before && !number_goes_on_after
    })
}

/// Unique non-empty lines over non-empty lines, compared like the repetition
/// guard does (list numbers and markers ignored); 1.0 for an empty answer.
pub fn distinct_line_ratio(answer: &str) -> f64 {
    let lines: Vec<String> = answer
        .lines()
        .map(line_key)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return 1.0;
    }
    let unique: HashSet<&String> = lines.iter().collect();
    unique.len() as f64 / lines.len() as f64
}

fn count_finish(results: &[ProbeResult], reason: FinishReason) -> usize {
    results.iter().filter(|r| r.finish_reason == reason).count()
}

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let (sum, count) = values.fold((0.0, 0usize), |(s, n), v| (s + v, n + 1));
    if count == 0 { 0.0 } else { sum / count as f64 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infer::fake::{FakeEmbedder, FakeGenerator};

    fn write(dir: &Path, name: &str, text: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn probes_load_and_inconsistent_rows_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let ok = write(
            dir.path(),
            "ok.jsonl",
            "{\"id\":\"a\",\"lang\":\"tr\",\"query\":\"q\",\"expect\":[\"x\"],\"answerable\":true}\n\n\
             {\"id\":\"b\",\"lang\":\"tr\",\"query\":\"q\",\"answerable\":false}\n",
        );
        assert_eq!(load_probes(&ok).unwrap().len(), 2);
        let bad = write(
            dir.path(),
            "bad.jsonl",
            "{\"id\":\"a\",\"lang\":\"tr\",\"query\":\"q\",\"answerable\":true}\n",
        );
        assert!(
            load_probes(&bad)
                .unwrap_err()
                .to_string()
                .contains("line 1")
        );
        for expect in ["[\"\"]", "[\"600|\"]", "[\" | x\"]"] {
            let row = format!(
                "{{\"id\":\"a\",\"lang\":\"tr\",\"query\":\"q\",\"expect\":{expect},\"answerable\":true}}\n"
            );
            let path = write(dir.path(), "empty.jsonl", &row);
            let err = load_probes(&path).unwrap_err().to_string();
            assert!(err.contains("must not be empty"), "{expect}: {err}");
        }
    }

    #[test]
    fn only_the_cited_sources_are_read_under_their_own_numbers() {
        let source = |number: usize, chunk_id: i64| SourceRef {
            number,
            chunk_id,
            document_id: 1,
            title: None,
            filename: None,
            heading_path: Vec::new(),
            pages: None,
            ordinal: 0,
            excerpt: String::new(),
            rank_score: 0.0,
            lexical_rank: None,
            dense_rank: None,
        };
        let given = [source(1, 10), source(2, 20), source(3, 30)];
        assert_eq!(cited_sources(&given, &[3, 1]), [(1, 10), (3, 30)]);
        assert!(cited_sources(&given, &[]).is_empty());
        assert!(
            cited_sources(&given, &[9]).is_empty(),
            "an invalid number names no source"
        );
    }

    #[test]
    fn expected_texts_match_whole_words_only() {
        assert!(contains_term("tbmm 600 milletvekili", "600"));
        assert!(contains_term("600.", "600"));
        assert!(!contains_term("1600 tl", "600"));
        assert!(!contains_term("6000 gün", "600"));
        assert!(contains_term("1600 ve 600", "600"));
        assert!(contains_term("hukuk devleti [1]", "hukuk devleti"));
        assert!(!contains_term("alaika", "laik"));
        assert!(contains_term("dili türkçedir", "türkçe"));
        assert!(contains_term("onanmasina", "onan"));
        assert!(contains_term("600'dür", "600"));
        assert!(!contains_term("1.600 tl", "600"));
        assert!(!contains_term("600.000 kişi", "600"));
        assert!(contains_term("sayısı 600.", "600"));
        assert!(contains_term("tarih 02.02.2016 idi", "02.02.2016"));
    }

    #[test]
    fn a_set_without_answerable_probes_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let empty = write(dir.path(), "e.jsonl", "\n");
        assert!(
            load_probes(&empty)
                .unwrap_err()
                .to_string()
                .contains("no answerable probes")
        );
        let only = write(
            dir.path(),
            "u.jsonl",
            "{\"id\":\"b\",\"lang\":\"tr\",\"query\":\"q\",\"answerable\":false}\n",
        );
        assert!(load_probes(&only).is_err());
    }

    #[test]
    fn distinct_line_ratio_counts_repeated_lines() {
        assert_eq!(distinct_line_ratio(""), 1.0);
        assert_eq!(distinct_line_ratio("a\n\nb\n"), 1.0);
        assert_eq!(distinct_line_ratio("a\na\n a \nb"), 0.5);
        assert_eq!(distinct_line_ratio("1. aynı madde\n2. aynı madde"), 0.5);
    }

    #[test]
    fn answers_are_scored_per_sampler_profile() {
        let corpus = tempfile::tempdir().unwrap();
        write(
            corpus.path(),
            "kargo.md",
            "# İade\n\nÜrünler 14 gün içinde iade edilebilir.",
        );
        let probes = vec![
            AnswerProbe {
                id: "p1".into(),
                lang: "tr".into(),
                query: "İade süresi kaç gün?".into(),
                expect: vec!["on dört|14 GÜN".into(), "kargo ücretsiz".into()],
                answerable: true,
                category: Some("factual".into()),
                split: Some("dev".into()),
                family: None,
            },
            AnswerProbe {
                id: "p2".into(),
                lang: "tr".into(),
                query: "Şirketin kuruluş yılı nedir?".into(),
                expect: vec![],
                answerable: false,
                category: Some("unanswerable".into()),
                split: Some("dev".into()),
                family: None,
            },
        ];
        let work = tempfile::tempdir().unwrap();
        let report = run_answer_eval(
            Arc::new(FakeEmbedder::new()),
            Arc::new(FakeGenerator::new("İade süresi 14 gün [1].")),
            corpus.path(),
            &probes,
            &[SamplerProfile::Greedy, SamplerProfile::Dry],
            work.path(),
            false,
        )
        .unwrap();
        assert_eq!((report.answerable, report.unanswerable), (1, 1));
        let names: Vec<&str> = report.profiles.iter().map(|p| p.sampler.as_str()).collect();
        assert_eq!(names, vec!["greedy", "dry"]);
        let greedy = &report.profiles[0];
        assert_eq!(
            (greedy.results[0].found, greedy.results[0].expected),
            (1, 2)
        );
        assert!((greedy.coverage - 0.5).abs() < 1e-12);
        assert_eq!(greedy.complete, 0);
        // The fake answers every question, so the unanswerable one is not refused.
        assert_eq!(greedy.refused_unanswerable, 0);
        assert_eq!(greedy.length_stops + greedy.repetition_stops, 0);
        // [1] is the İade chunk, which holds "14 gün": cited evidence, valid citations.
        let m = &greedy.metrics;
        assert!(greedy.results[0].cited_evidence && greedy.results[0].valid_citations);
        assert_eq!(
            (m.cited_evidence_coverage, m.valid_citation_rate),
            (Some(1.0), Some(1.0)),
            "both answers cite [1], the one chunk retrieved for both questions"
        );
        assert_eq!(
            m.expected_fact_rate,
            Some(0.0),
            "one of two expected texts is missing"
        );
        assert_eq!(
            (m.abstention.false_negative, m.abstention.true_negative),
            (1, 1)
        );
        assert_eq!(m.finish_reasons.stop, 2);
        assert_eq!(
            greedy.by_category.keys().collect::<Vec<_>>(),
            ["factual", "unanswerable"]
        );
        assert_eq!(greedy.by_split["dev"].probes, 2);
        let markdown = report.to_markdown();
        assert!(
            markdown.contains("| dry |") && markdown.contains("by category"),
            "{markdown}"
        );
    }

    #[test]
    fn the_experimental_judge_is_reported_only_when_asked() {
        let corpus = tempfile::tempdir().unwrap();
        write(
            corpus.path(),
            "kargo.md",
            "# İade\n\nÜrünler 14 gün içinde iade edilebilir.",
        );
        let probes = vec![AnswerProbe {
            id: "p1".into(),
            lang: "tr".into(),
            query: "İade süresi kaç gün?".into(),
            expect: vec!["14 gün".into()],
            answerable: true,
            category: None,
            split: None,
            family: None,
        }];
        let run = |with_judge| {
            let work = tempfile::tempdir().unwrap();
            // The fake says "YES [1]." both as the answer and as the verdict.
            run_answer_eval(
                Arc::new(FakeEmbedder::new()),
                Arc::new(FakeGenerator::new("YES [1].")),
                corpus.path(),
                &probes,
                &[SamplerProfile::Greedy],
                work.path(),
                with_judge,
            )
            .unwrap()
        };
        let judged = run(true);
        assert_eq!(judged.profiles[0].results[0].judge, Some(true));
        assert_eq!(judged.profiles[0].experimental.judge_yes_rate, Some(1.0));
        let plain = run(false);
        assert_eq!(plain.profiles[0].results[0].judge, None);
        assert_eq!(plain.profiles[0].experimental.judge_yes_rate, None);
    }

    #[test]
    fn v02_fields_load_for_probes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jsonl");
        std::fs::write(
            &path,
            r#"{"id":"a","lang":"tr","category":"table","split":"holdout","family":"sulama","query":"q","expect":["x"],"answerable":true}"#,
        )
        .unwrap();
        let probes = load_probes(&path).unwrap();
        assert_eq!(probes[0].split.as_deref(), Some("holdout"));
        std::fs::write(
            &path,
            r#"{"id":"a","lang":"tr","category":"misc","query":"q","expect":["x"],"answerable":true}"#,
        )
        .unwrap();
        let err = load_probes(&path).unwrap_err().to_string();
        assert!(err.contains("line 1") && err.contains("misc"), "{err}");
    }

    #[test]
    fn v02_probes_load_with_the_planned_mix() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../eval/datasets/v02-answers.jsonl");
        let probes = load_probes(&path).unwrap();
        assert_eq!(probes.len(), 60);
        assert_eq!(probes.iter().filter(|p| p.answerable).count(), 40);
        assert!(
            probes
                .iter()
                .all(|p| p.category.is_some() && p.split.is_some() && p.family.is_some())
        );
    }
}
