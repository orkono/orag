//! Optional cross-encoder reranking of the first fused candidates (v0.4
//! candidate, D-005). Off unless a `Rerank` is configured; any scoring error
//! or an overrun of the time budget keeps the fused order.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::infer::Reranker;
use crate::retrieval::hybrid::Candidate;
use crate::retrieval::rerank_window::passage_window;

/// A reranker and how far into the fused list it reads.
#[derive(Clone)]
pub struct Rerank {
    pub reranker: Arc<dyn Reranker>,
    /// Fused candidates scored (the first `depth`); the rest keep their place.
    pub depth: usize,
    /// Keep each list's promoted leaders (D-005) in front, rerank the rest.
    pub pin_leaders: bool,
    /// Scoring that ends after this keeps the fused order.
    pub timeout: Duration,
    /// Score only the sentences around the query's terms, at most this many
    /// characters per chunk (step 65); `None` scores whole chunks.
    pub window_chars: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RerankTrace {
    pub model: String,
    /// Candidates the reranker scored (0 when it fell back).
    pub scored: usize,
    pub ms: u64,
    /// The fused order was kept: the reranker failed or ran out of time.
    pub fallback: bool,
    /// Scored passages read whole because no query key matched their chunk
    /// (only with a window).
    pub whole: usize,
}

impl Rerank {
    /// Re-orders the first `depth` candidates by score, after the `pinned`
    /// leading ones when leaders are pinned.
    pub fn apply(
        &self,
        query: &str,
        candidates: Vec<Candidate>,
        leaders: usize,
    ) -> (Vec<Candidate>, RerankTrace) {
        let pinned = if self.pin_leaders {
            candidates
                .iter()
                .take_while(|c| is_leader(c, leaders))
                .count()
        } else {
            0
        };
        let end = self.depth.min(candidates.len()).max(pinned);
        let mut whole = 0;
        let passages: Vec<String> = candidates[pinned..end]
            .iter()
            .map(|c| {
                let window = self
                    .window_chars
                    .and_then(|max| passage_window(query, &c.chunk.text, max));
                if self.window_chars.is_some() && window.is_none() {
                    whole += 1;
                }
                window.unwrap_or_else(|| c.chunk.text.clone())
            })
            .collect();
        let started = Instant::now();
        let scores = self
            .reranker
            .score(query, &passages, Some(started + self.timeout));
        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let mut trace = RerankTrace {
            model: self.reranker.model_id().to_string(),
            scored: 0,
            ms,
            fallback: false,
            whole,
        };
        match scores {
            Ok(scores) if scores.len() == passages.len() && started.elapsed() <= self.timeout => {
                trace.scored = scores.len();
                let order = order_by_score(&scores);
                let mut slots: Vec<Option<Candidate>> = candidates.into_iter().map(Some).collect();
                let head: Vec<Candidate> = slots[..pinned]
                    .iter_mut()
                    .filter_map(Option::take)
                    .collect();
                let reranked: Vec<Candidate> = order
                    .iter()
                    .filter_map(|&i| slots[pinned + i].take())
                    .collect();
                let tail: Vec<Candidate> =
                    slots[end..].iter_mut().filter_map(Option::take).collect();
                (
                    head.into_iter().chain(reranked).chain(tail).collect(),
                    trace,
                )
            }
            outcome => {
                if let Err(error) = outcome {
                    tracing::warn!(%error, "reranking failed; keeping the fused order");
                } else {
                    tracing::warn!(ms, "reranking ran out of time; keeping the fused order");
                }
                trace.fallback = true;
                (candidates, trace)
            }
        }
    }
}

/// A promoted leader: within the first `leaders` of either list.
fn is_leader(candidate: &Candidate, leaders: usize) -> bool {
    let within = |rank: Option<usize>| rank.is_some_and(|r| r <= leaders);
    within(candidate.hit.lexical_rank) || within(candidate.hit.dense_rank)
}

/// Indices by descending score; ties keep their order, NaN goes last.
fn order_by_score(scores: &[f32]) -> Vec<usize> {
    let key = |i: usize| {
        let s = scores[i];
        // `+ 0.0` turns -0.0 into 0.0, so the two tie as equal scores.
        if s.is_nan() {
            f32::NEG_INFINITY
        } else {
            s + 0.0
        }
    };
    let mut order: Vec<usize> = (0..scores.len()).collect();
    order.sort_by(|&a, &b| {
        key(b)
            .total_cmp(&key(a))
            .then_with(|| scores[a].is_nan().cmp(&scores[b].is_nan()))
    });
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{OragError, Result};
    use crate::infer::fake::{FakeEmbedder, FakeReranker};
    use crate::retrieval::hybrid::Strategy;
    use crate::retrieval::testing::indexed;

    #[test]
    fn scores_order_descending_with_stable_ties_and_nan_last() {
        assert_eq!(order_by_score(&[0.1, 0.9, 0.5]), vec![1, 2, 0]);
        assert_eq!(order_by_score(&[0.5, 0.5, 0.9]), vec![2, 0, 1]);
        assert_eq!(
            order_by_score(&[f32::NAN, 0.2, f32::NAN, 0.3]),
            vec![3, 1, 0, 2]
        );
        assert_eq!(order_by_score(&[f32::NEG_INFINITY, f32::NAN]), vec![0, 1]);
        assert_eq!(order_by_score(&[-0.0, 0.0, -0.0]), vec![0, 1, 2]);
    }

    struct Failing;
    impl Reranker for Failing {
        fn model_id(&self) -> &str {
            "failing"
        }
        fn score(&self, _: &str, _: &[String], _: Option<Instant>) -> Result<Vec<f32>> {
            Err(OragError::InvalidInput("boom".into()))
        }
    }

    struct Slow;
    impl Reranker for Slow {
        fn model_id(&self) -> &str {
            "slow"
        }
        fn score(&self, _: &str, passages: &[String], _: Option<Instant>) -> Result<Vec<f32>> {
            std::thread::sleep(Duration::from_millis(30));
            Ok((0..passages.len()).map(|i| i as f32).collect())
        }
    }

    /// Scores passages holding `word` 1, the rest 0.
    struct Prefer(&'static str);
    impl Reranker for Prefer {
        fn model_id(&self) -> &str {
            "prefer"
        }
        fn score(&self, _: &str, passages: &[String], _: Option<Instant>) -> Result<Vec<f32>> {
            Ok(passages
                .iter()
                .map(|p| f32::from(u8::from(p.contains(self.0))))
                .collect())
        }
    }

    fn rerank(reranker: Arc<dyn Reranker>, depth: usize, pin_leaders: bool) -> Rerank {
        Rerank {
            reranker,
            depth,
            pin_leaders,
            timeout: Duration::from_secs(5),
            window_chars: None,
        }
    }

    /// Scores 0 and records the longest passage it was given.
    struct Longest(std::sync::Mutex<usize>);
    impl Reranker for Longest {
        fn model_id(&self) -> &str {
            "longest"
        }
        fn score(&self, _: &str, passages: &[String], _: Option<Instant>) -> Result<Vec<f32>> {
            let mut longest = self.0.lock().unwrap();
            for p in passages {
                *longest = (*longest).max(p.chars().count());
            }
            Ok(vec![0.0; passages.len()])
        }
    }

    const DOCS: [(&str, &str); 5] = [
        ("a.md", "# A\n\nfiltre kartuşu değişimi"),
        ("b.md", "# B\n\nfiltre gövdesi plastik"),
        ("c.md", "# C\n\nfiltre basınç kaybı"),
        ("d.md", "# D\n\nfiltre temizliği"),
        ("e.md", "# E\n\nyedek parça listesi"),
    ];

    fn titles(candidates: &[Candidate]) -> Vec<String> {
        candidates
            .iter()
            .map(|c| c.chunk.document_title.clone().unwrap_or_default())
            .collect()
    }

    #[test]
    fn without_a_reranker_the_order_is_the_fused_order() {
        let (_dir, retriever) = indexed(FakeEmbedder::new(), &DOCS);
        let plain = retriever
            .retrieve(1, "filtre kartuşu", Strategy::Hybrid, 3)
            .unwrap();
        assert!(plain.trace.rerank.is_none());
        let again = retriever
            .retrieve(1, "filtre kartuşu", Strategy::Hybrid, 3)
            .unwrap();
        assert_eq!(titles(&plain.candidates), titles(&again.candidates));
    }

    #[test]
    fn a_candidate_beyond_the_limit_but_within_depth_can_come_first() {
        let (_dir, mut retriever) = indexed(FakeEmbedder::new(), &DOCS);
        let query = "filtre kartuşu";
        let fused = retriever.retrieve(1, query, Strategy::Hybrid, 5).unwrap();
        assert_eq!(
            titles(&fused.candidates).last().unwrap(),
            "E",
            "E is fused last"
        );
        retriever.reranker = Some(rerank(Arc::new(Prefer("yedek")), 5, false));
        let reranked = retriever.retrieve(1, query, Strategy::Hybrid, 2).unwrap();
        assert_eq!(
            titles(&reranked.candidates),
            vec!["E", titles(&fused.candidates)[0].as_str()]
        );
        let trace = reranked.trace.rerank.unwrap();
        assert_eq!(
            (trace.model.as_str(), trace.scored, trace.fallback),
            ("prefer", 5, false)
        );
        let again = retriever.retrieve(1, query, Strategy::Hybrid, 2).unwrap();
        assert_eq!(titles(&reranked.candidates), titles(&again.candidates));
    }

    #[test]
    fn only_hybrid_retrieval_is_reranked() {
        let (_dir, mut retriever) = indexed(FakeEmbedder::new(), &DOCS);
        retriever.reranker = Some(rerank(Arc::new(FakeReranker), 5, false));
        let lexical = retriever
            .retrieve(1, "filtre kartuşu", Strategy::Lexical, 3)
            .unwrap();
        assert!(lexical.trace.rerank.is_none());
    }

    #[test]
    fn pinned_leaders_stay_in_front() {
        let (_dir, mut retriever) = indexed(FakeEmbedder::new(), &DOCS);
        let query = "filtre kartuşu";
        retriever.config.max_context_chunks = 2; // one leader per list
        let fused = retriever.retrieve(1, query, Strategy::Hybrid, 5).unwrap();
        let leaders = fused
            .candidates
            .iter()
            .take_while(|c| is_leader(c, 1))
            .count();
        retriever.reranker = Some(rerank(Arc::new(FakeReranker), 5, true));
        let reranked = retriever.retrieve(1, query, Strategy::Hybrid, 5).unwrap();
        assert!(leaders >= 1);
        assert_eq!(
            titles(&reranked.candidates)[..leaders],
            titles(&fused.candidates)[..leaders]
        );
    }

    #[test]
    fn a_failing_or_slow_reranker_keeps_the_fused_order() {
        let (_dir, mut retriever) = indexed(FakeEmbedder::new(), &DOCS);
        let query = "filtre kartuşu";
        let fused = retriever.retrieve(1, query, Strategy::Hybrid, 5).unwrap();
        retriever.reranker = Some(rerank(Arc::new(Failing), 5, false));
        let failed = retriever.retrieve(1, query, Strategy::Hybrid, 5).unwrap();
        assert_eq!(titles(&failed.candidates), titles(&fused.candidates));
        let trace = failed.trace.rerank.unwrap();
        assert!(trace.fallback && trace.scored == 0, "{trace:?}");
        retriever.reranker = Some(Rerank {
            timeout: Duration::from_millis(1),
            ..rerank(Arc::new(Slow), 5, false)
        });
        let slow = retriever.retrieve(1, query, Strategy::Hybrid, 5).unwrap();
        assert_eq!(titles(&slow.candidates), titles(&fused.candidates));
        assert!(slow.trace.rerank.unwrap().fallback);
    }

    #[test]
    fn a_window_shortens_what_the_reranker_reads() {
        let long = format!(
            "# L\n\n{} filtre kartuşu burada.",
            "Uzun bir giriş cümlesi. ".repeat(20)
        );
        let docs = [("l.md", long.as_str()), DOCS[0], DOCS[1]];
        let (_dir, mut retriever) = indexed(FakeEmbedder::new(), &docs);
        let whole = Arc::new(Longest(std::sync::Mutex::new(0)));
        retriever.reranker = Some(rerank(whole.clone(), 8, false));
        retriever
            .retrieve(1, "filtre kartuşu", Strategy::Hybrid, 3)
            .unwrap();
        assert!(*whole.0.lock().unwrap() > 100);
        let windowed = Arc::new(Longest(std::sync::Mutex::new(0)));
        retriever.reranker = Some(Rerank {
            window_chars: Some(100),
            ..rerank(windowed.clone(), 8, false)
        });
        retriever
            .retrieve(1, "filtre kartuşu", Strategy::Hybrid, 3)
            .unwrap();
        assert!(*windowed.0.lock().unwrap() <= 100);
        let english = retriever
            .retrieve(1, "spare cartridge", Strategy::Hybrid, 3)
            .unwrap();
        assert!(english.trace.rerank.unwrap().whole >= 1);
    }
}
