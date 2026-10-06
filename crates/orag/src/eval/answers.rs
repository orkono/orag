//! Answer-level evaluation: runs the full answer path (retrieval, prompt,
//! generation) per sampler profile and checks what users see: expected facts
//! present, refusals, loops and answers cut by the output budget.

use std::collections::HashSet;
use std::ops::ControlFlow;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::domain::normalize::normalize_for_lexical;
use crate::error::{OragError, Result};
use crate::eval::metrics::{percentile, squash};
use crate::eval::retrieval::index_corpus;
use crate::infer::{Embedder, Generator, SamplerProfile};
use crate::retrieval::answer::{AnswerEngine, AnswerEvent, AnswerSummary, FinishReason};
use crate::retrieval::hybrid::{RetrievalConfig, Retriever};
use crate::retrieval::repetition::line_key;
use crate::store::Store;
use crate::store::search::SqliteVecIndex;

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
}

pub fn load_probes(path: &Path) -> Result<Vec<AnswerProbe>> {
    let text = std::fs::read_to_string(path)?;
    let mut probes = Vec::new();
    let mut ids = HashSet::new();
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
    pub results: Vec<ProbeResult>,
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
             | sampler | coverage | complete | refused answerable | refused unanswerable | length stops | repetition stops | distinct lines | p50 ms | p95 ms |\n\
             |---|---|---|---|---|---|---|---|---|---|\n",
            self.generator, self.answerable, self.unanswerable
        );
        for p in &self.profiles {
            out.push_str(&format!(
                "| {} | {:.3} | {} | {} | {} | {} | {} | {:.3} | {:.0} | {:.0} |\n",
                p.sampler,
                p.coverage,
                p.complete,
                p.refused_answerable,
                p.refused_unanswerable,
                p.length_stops,
                p.repetition_stops,
                p.mean_distinct_line_ratio,
                p.p50_ms,
                p.p95_ms
            ));
        }
        out
    }
}

pub fn run_answer_eval(
    embedder: Arc<dyn Embedder>,
    generator: Arc<dyn Generator>,
    corpus: &Path,
    probes: &[AnswerProbe],
    samplers: &[SamplerProfile],
    work_dir: &Path,
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
            evaluate_profile(&engine, probes)
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

fn evaluate_profile(engine: &AnswerEngine, probes: &[AnswerProbe]) -> Result<ProfileReport> {
    let results = probes
        .iter()
        .map(|probe| run_probe(engine, probe))
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
        results,
    })
}

fn run_probe(engine: &AnswerEngine, probe: &AnswerProbe) -> Result<ProbeResult> {
    let started = Instant::now();
    let mut summary: Option<AnswerSummary> = None;
    engine.answer(1, &probe.query, &mut |event| {
        if let AnswerEvent::Done(done) = event {
            summary = Some(*done);
        }
        ControlFlow::Continue(())
    })?;
    let summary = summary.ok_or_else(|| {
        OragError::Internal(format!("{}: the answer ended without a summary", probe.id))
    })?;
    let answer = normalized(&summary.answer);
    let found = probe
        .expect
        .iter()
        .filter(|text| {
            text.split('|')
                .any(|alt| contains_term(&answer, &normalized(alt)))
        })
        .count();
    Ok(ProbeResult {
        id: probe.id.clone(),
        finish_reason: summary.finish_reason,
        abstained: summary.abstained,
        found,
        expected: probe.expect.len(),
        distinct_line_ratio: distinct_line_ratio(&summary.answer),
        completion_tokens: summary.trace.completion_tokens,
        ms: started.elapsed().as_secs_f64() * 1000.0,
        answer: summary.answer,
    })
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
            },
            AnswerProbe {
                id: "p2".into(),
                lang: "tr".into(),
                query: "Şirketin kuruluş yılı nedir?".into(),
                expect: vec![],
                answerable: false,
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
        assert!(report.to_markdown().contains("| dry |"));
    }
}
