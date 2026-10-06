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

/// Moves each list's top hit (lexical #1, dense #1) to the front, keeping
/// fused order within the promoted and the remaining hits. RRF alone can
/// push a chunk that only one retriever finds, even at rank 1, below chunks
/// that are mediocre in both lists, so it never reaches the answer context
/// (measured on a PDF of the Turkish constitution, D-005). Scores are kept.
pub fn promote_list_leaders(mut hits: Vec<FusedHit>) -> Vec<FusedHit> {
    // Stable: `false` (leaders) sorts first, each group keeps fused order.
    hits.sort_by_key(|hit| !(hit.lexical_rank == Some(1) || hit.dense_rank == Some(1)));
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
        let promoted = promote_list_leaders(fused);
        assert_eq!(ids(&promoted), vec![4, 9, 1, 2, 3]);
        // Scores are untouched: only the order changes.
        assert!((promoted[1].score - 1.0 / 61.0).abs() < 1e-12);
    }

    #[test]
    fn promotion_keeps_order_when_the_leaders_already_lead() {
        let fused = reciprocal_rank_fusion(&[1, 2, 3], &[1, 3, 2]);
        assert_eq!(ids(&promote_list_leaders(fused.clone())), ids(&fused));
    }

    #[test]
    fn promotion_handles_a_single_list_and_empty_input() {
        let fused = reciprocal_rank_fusion(&[], &[5, 6]);
        assert_eq!(ids(&promote_list_leaders(fused)), vec![5, 6]);
        assert!(promote_list_leaders(Vec::new()).is_empty());
    }
}
