//! Reciprocal Rank Fusion: score(d) = Σ 1 / (k + rank_i(d)), rank 1-based.

use std::collections::HashMap;

use serde::Serialize;

use crate::domain::ChunkId;

pub const RRF_K: f64 = 60.0;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FusedHit {
    pub chunk_id: ChunkId,
    pub score: f64,
    /// 1-based rank among the lexical list's distinct chunks. The store
    /// returns each chunk once, so this equals its position in that list.
    pub lexical_rank: Option<usize>,
    /// 1-based rank among the dense list's distinct chunks.
    pub dense_rank: Option<usize>,
}

/// Fuses two ranked lists with `k = RRF_K`. A chunk repeated within one
/// list counts once, and ranks are positions among a list's distinct
/// chunks. Sorted by score descending, ties by chunk id ascending.
pub fn reciprocal_rank_fusion(lexical: &[ChunkId], dense: &[ChunkId]) -> Vec<FusedHit> {
    let capacity = lexical.len() + dense.len();
    let mut hits: Vec<FusedHit> = Vec::with_capacity(capacity);
    let mut slots: HashMap<ChunkId, usize> = HashMap::with_capacity(capacity);
    for (list, is_lexical) in [(lexical, true), (dense, false)] {
        let mut rank = 0;
        for &chunk_id in list {
            let slot = *slots.entry(chunk_id).or_insert_with(|| {
                hits.push(FusedHit {
                    chunk_id,
                    score: 0.0,
                    lexical_rank: None,
                    dense_rank: None,
                });
                hits.len() - 1
            });
            let hit = &mut hits[slot];
            let rank_slot = if is_lexical {
                &mut hit.lexical_rank
            } else {
                &mut hit.dense_rank
            };
            if rank_slot.is_some() {
                continue; // repeated within this list: keep its first rank
            }
            rank += 1;
            *rank_slot = Some(rank);
            hit.score += 1.0 / (RRF_K + rank as f64);
        }
    }
    // Chunk ids are unique, so an unstable sort gives a deterministic order.
    hits.sort_unstable_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.chunk_id.cmp(&b.chunk_id))
    });
    hits
}

/// Moves each list's #1 to the front, then the rest of each list's top
/// `per_list`, keeping fused order within each tier, so a context that fits
/// fewer chunks still gets both leaders first. Each list's #1 always leads
/// (D-005), so `per_list` 0 and 1 behave the same. RRF alone can push a
/// chunk that only one retriever finds, even near the top, below chunks that
/// are mediocre in both lists, so it never reaches the answer context: on the
/// PDF of the Turkish constitution (lexical #1, D-005) and on cross-language
/// questions, where the lexical list matches incidental words in wrong chunks
/// and the dense list finds the answer (D-005, 0.3.0-alpha.1). Scores are kept.
pub fn promote_list_leaders(mut hits: Vec<FusedHit>, per_list: usize) -> Vec<FusedHit> {
    // Stable: each tier keeps fused order.
    hits.sort_by_cached_key(|hit| {
        let best = [hit.lexical_rank, hit.dense_rank]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(usize::MAX);
        match best {
            1 => 0,
            rank if rank <= per_list => 1,
            _ => 2,
        }
    });
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(hits: &[FusedHit]) -> Vec<i64> {
        hits.iter().map(|hit| hit.chunk_id).collect()
    }

    #[test]
    fn items_in_both_lists_rank_highest() {
        let hits = reciprocal_rank_fusion(&[1, 2, 3], &[3, 1]);
        assert_eq!(ids(&hits), vec![1, 3, 2]);
        let one = &hits[0];
        assert_eq!((one.lexical_rank, one.dense_rank), (Some(1), Some(2)));
        assert!((one.score - (1.0 / 61.0 + 1.0 / 62.0)).abs() < 1e-12);
    }

    #[test]
    fn duplicates_within_a_list_count_once_at_best_rank() {
        let hits = reciprocal_rank_fusion(&[7, 7, 8], &[]);
        assert_eq!(ids(&hits), vec![7, 8]);
        assert!((hits[0].score - 1.0 / 61.0).abs() < 1e-12);
        // 8 is the second distinct hit, so it ranks 2nd, not 3rd.
        assert_eq!(hits[1].lexical_rank, Some(2));
        assert!((hits[1].score - 1.0 / 62.0).abs() < 1e-12);
    }

    #[test]
    fn ties_break_by_chunk_id() {
        assert_eq!(ids(&reciprocal_rank_fusion(&[5], &[4])), vec![4, 5]);
    }

    #[test]
    fn empty_inputs_give_empty_output() {
        assert!(reciprocal_rank_fusion(&[], &[]).is_empty());
    }

    #[test]
    fn each_list_leader_is_promoted_to_the_front() {
        // 9 is lexical #1 but missing from the dense list; RRF ranks it below
        // chunks that are mediocre in both lists.
        let lexical = [9, 1, 2, 3];
        let dense = [4, 1, 2, 3];
        let fused = reciprocal_rank_fusion(&lexical, &dense);
        assert_eq!(ids(&fused), vec![1, 2, 3, 4, 9]);
        let promoted = promote_list_leaders(fused, 1);
        assert_eq!(ids(&promoted), vec![4, 9, 1, 2, 3]);
        // Scores are untouched: only the order changes.
        assert!((promoted[1].score - 1.0 / 61.0).abs() < 1e-12);
    }

    #[test]
    fn each_list_keeps_its_top_hits_ahead_of_the_rest() {
        // A cross-language question: the lexical list matches incidental
        // words in wrong chunks, the dense list finds the answer (30) at #4.
        // Chunks in both lists at middling ranks outscore it under RRF.
        let lexical = [11, 12, 13, 14, 20, 21, 22, 23];
        let dense = [20, 21, 22, 30, 23];
        let fused = reciprocal_rank_fusion(&lexical, &dense);
        let before = ids(&fused);
        assert!(before.iter().position(|&id| id == 30) > Some(7));
        let promoted = promote_list_leaders(fused, 4);
        // Both #1s lead, then lexical #2-4 and dense #2-4 in fused order.
        assert_eq!(ids(&promoted), vec![20, 11, 21, 22, 12, 13, 14, 30, 23]);
    }

    #[test]
    fn list_leaders_stay_first_when_more_hits_are_promoted() {
        // A small context (two chunks) still gets lexical #1 and dense #1.
        let lexical = [9, 1, 2, 3];
        let dense = [4, 1, 2, 3];
        let promoted = promote_list_leaders(reciprocal_rank_fusion(&lexical, &dense), 4);
        assert_eq!(ids(&promoted)[..2], [4, 9]);
    }

    #[test]
    fn promotion_keeps_order_when_the_leaders_already_lead() {
        let fused = reciprocal_rank_fusion(&[1, 2, 3], &[1, 3, 2]);
        assert_eq!(ids(&promote_list_leaders(fused.clone(), 4)), ids(&fused));
    }

    #[test]
    fn promotion_handles_a_single_list_and_empty_input() {
        let fused = reciprocal_rank_fusion(&[], &[5, 6]);
        assert_eq!(ids(&promote_list_leaders(fused, 4)), vec![5, 6]);
        assert!(promote_list_leaders(Vec::new(), 4).is_empty());
    }
}
