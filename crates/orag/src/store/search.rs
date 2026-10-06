//! Lexical (FTS5/BM25) and dense (sqlite-vec) search plus chunk hydration.

use std::collections::HashMap;
use std::sync::Arc;

use rusqlite::{Row, params};
use serde::Serialize;

use crate::domain::space::EmbeddingSpace;
use crate::domain::{ChunkId, CollectionId};
use crate::error::{OragError, Result};
use crate::infer::VectorIndex;
use crate::store::spaces::vec_table;
use crate::store::{Store, conversion_error};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChunkRecord {
    pub id: ChunkId,
    pub document_id: i64,
    pub collection_id: CollectionId,
    pub ordinal: u32,
    pub heading_path: Vec<String>,
    pub text: String,
    pub token_count: usize,
    pub document_title: Option<String>,
    pub filename: Option<String>,
}

/// Largest candidate count either search accepts; sqlite-vec 0.1.9 rejects
/// k above 4096.
pub const MAX_SEARCH_K: usize = 4096;

fn check_k(k: usize) -> Result<()> {
    if k > MAX_SEARCH_K {
        return Err(OragError::InvalidInput(format!(
            "at most {MAX_SEARCH_K} search candidates can be requested, got {k}"
        )));
    }
    Ok(())
}

pub(crate) fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn chunk_from_row(row: &Row<'_>) -> rusqlite::Result<ChunkRecord> {
    let heading_path: String = row.get(4)?;
    Ok(ChunkRecord {
        id: row.get(0)?,
        document_id: row.get(1)?,
        collection_id: row.get(2)?,
        ordinal: row.get(3)?,
        heading_path: serde_json::from_str(&heading_path)
            .map_err(|e| conversion_error(4, e.to_string()))?,
        text: row.get(5)?,
        token_count: row.get::<_, i64>(6)? as usize,
        document_title: row.get(7)?,
        filename: row.get(8)?,
    })
}

impl Store {
    /// BM25-ranked chunk ids within `collection_id`. `fts_query` must come from
    /// `domain::normalize::fts_query` (quoted terms only).
    ///
    /// Known limit: FTS5 keeps one index for all collections, so BM25 term
    /// statistics (IDF) include other collections' text. Results never
    /// leave the collection, but their order can shift with other content.
    pub fn lexical_search(
        &self,
        collection_id: CollectionId,
        fts_query: &str,
        limit: usize,
    ) -> Result<Vec<ChunkId>> {
        check_k(limit)?;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self.read()?;
        let mut stmt = conn.prepare(
            "SELECT c.id FROM chunks_fts JOIN chunks c ON c.id = chunks_fts.rowid \
             WHERE chunks_fts MATCH ?1 AND c.collection_id = ?2 \
             ORDER BY bm25(chunks_fts), c.id LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![fts_query, collection_id, limit as i64], |r| {
            r.get(0)
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Hydrates chunks of `collection_id` in the requested order; unknown ids
    /// and ids from other collections are skipped.
    pub fn get_chunks(
        &self,
        collection_id: CollectionId,
        ids: &[ChunkId],
    ) -> Result<Vec<ChunkRecord>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        check_k(ids.len())?;
        let conn = self.read()?;
        let placeholders = vec!["?"; ids.len()].join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT c.id, c.document_id, c.collection_id, c.ordinal, c.heading_path, c.text, \
             c.token_count, d.title, d.filename FROM chunks c JOIN documents d ON d.id = c.document_id \
             WHERE c.collection_id = ? AND c.id IN ({placeholders})"
        ))?;
        let params = std::iter::once(collection_id).chain(ids.iter().copied());
        let rows = stmt.query_map(rusqlite::params_from_iter(params), chunk_from_row)?;
        let mut by_id: HashMap<ChunkId, ChunkRecord> = HashMap::new();
        for row in rows {
            let record = row?;
            by_id.insert(record.id, record);
        }
        Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
    }
}

impl Store {
    /// Benchmark-only: inserts vectors with consecutive rowids starting at `first_rowid`,
    /// without chunk rows. Never call on a user database.
    pub(crate) fn bulk_insert_vectors(
        &self,
        space: &EmbeddingSpace,
        collection_id: CollectionId,
        first_rowid: i64,
        vectors: &[Vec<f32>],
    ) -> Result<()> {
        self.write(|conn| {
            let tx = conn.transaction()?;
            {
                let mut insert = tx.prepare(&format!(
                    "INSERT INTO {} (rowid, collection_id, embedding) VALUES (?1, ?2, ?3)",
                    vec_table(space.id)
                ))?;
                for (offset, vector) in vectors.iter().enumerate() {
                    insert.execute(params![
                        first_rowid + offset as i64,
                        collection_id,
                        f32_bytes(vector)
                    ])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Every chunk of `collection_id` as `(filename, text)`, in document and
    /// chunk order. For offline tools such as the evaluation harness.
    pub fn chunk_texts(
        &self,
        collection_id: CollectionId,
    ) -> Result<Vec<(Option<String>, String)>> {
        let conn = self.read()?;
        let mut stmt = conn.prepare(
            "SELECT d.filename, c.text FROM chunks c JOIN documents d ON d.id = c.document_id \
             WHERE c.collection_id = ?1 ORDER BY c.document_id, c.ordinal",
        )?;
        let rows = stmt.query_map([collection_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

fn space_exists(conn: &rusqlite::Connection, space_id: i64) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM embedding_spaces WHERE id = ?1)",
        [space_id],
        |r| r.get(0),
    )?)
}

/// Exact k-NN over sqlite-vec `vec0` (brute force in 0.1.9).
pub struct SqliteVecIndex {
    store: Arc<Store>,
}

impl SqliteVecIndex {
    pub fn new(store: Arc<Store>) -> Self {
        SqliteVecIndex { store }
    }
}

impl VectorIndex for SqliteVecIndex {
    fn search(
        &self,
        space: &EmbeddingSpace,
        collection_id: CollectionId,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<(ChunkId, f32)>> {
        if query.len() != space.dimensions {
            return Err(OragError::InvalidInput(format!(
                "query has {} dimensions, space expects {}",
                query.len(),
                space.dimensions
            )));
        }
        check_k(k)?;
        if k == 0 {
            return Ok(Vec::new());
        }
        // Stored vectors are unit length (publish checks it); normalizing the
        // query makes the distance below convert to the true cosine.
        let query = crate::infer::l2_normalize(query)?;
        let conn = self.store.read()?;
        let sql = format!(
            "SELECT rowid, distance FROM {} WHERE embedding MATCH ?1 AND k = ?2 AND collection_id = ?3 ORDER BY distance",
            vec_table(space.id)
        );
        let mut stmt = match conn.prepare(&sql) {
            Ok(stmt) => stmt,
            // The collection was deleted after `query_space`, taking its space with it.
            Err(_) if !space_exists(&conn, space.id)? => {
                return Err(OragError::NotFound {
                    kind: "collection",
                    id: collection_id,
                });
            }
            Err(e) => return Err(e.into()),
        };
        let rows = stmt.query_map(params![f32_bytes(&query), k as i64, collection_id], |row| {
            let distance: f64 = row.get(1)?;
            // Vectors are unit length: cos = 1 - L2² / 2.
            Ok((
                row.get::<_, i64>(0)?,
                (1.0 - distance * distance / 2.0) as f32,
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::domain::chunker::ChunkDraft;
    use crate::domain::normalize::{fts_query, normalize_for_lexical};
    use crate::domain::space::SpaceDescriptor;
    use crate::error::OragError;
    use crate::infer::VectorIndex;
    use crate::ingest::format::SourceFormat;
    use crate::store::documents::NewDocument;
    use crate::store::publish::{PreparedChunk, PublishOutcome};
    use crate::store::testing::temp_store;

    fn desc(model: &str) -> SpaceDescriptor {
        SpaceDescriptor {
            model_id: model.into(),
            model_sha256: "a".repeat(64),
            pooling: "last".into(),
            query_prefix: String::new(),
            document_prefix: String::new(),
            dimensions: 4,
            normalized: true,
            max_tokens: 512,
            require_trailing_eos: false,
        }
    }

    fn chunk(ordinal: u32, text: &str, embedding: [f32; 4]) -> PreparedChunk {
        PreparedChunk {
            draft: ChunkDraft {
                ordinal,
                heading_path: vec!["H".into()],
                breadcrumb: "H".into(),
                text: text.into(),
                token_count: 3,
            },
            norm_text: normalize_for_lexical(text),
            embedding: crate::infer::l2_normalize(&embedding).expect("non-zero test vector"),
        }
    }

    /// Enqueues, claims and publishes one document; returns its id.
    fn index(store: &Store, collection: i64, body: &str, chunks: &[PreparedChunk]) -> i64 {
        let e = store
            .enqueue_document(
                collection,
                NewDocument {
                    filename: Some("d.md".into()),
                    format: SourceFormat::Markdown,
                    bytes: body.into(),
                },
            )
            .unwrap();
        let job = store.claim_next_job().unwrap().unwrap();
        let space = store.bind_space(collection, &desc("m")).unwrap();
        let outcome = store
            .publish_document(&job, &space, Some("Title"), &[], chunks)
            .unwrap();
        assert_eq!(
            outcome,
            PublishOutcome::Published {
                chunk_count: chunks.len()
            }
        );
        e.document_id
    }

    fn count(store: &Store, sql: &str) -> i64 {
        store
            .read()
            .unwrap()
            .query_row(sql, [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn spaces_are_shared_by_fingerprint_and_mismatch_requires_reindex() {
        let (_dir, store) = temp_store();
        let other = store.create_collection("other").unwrap();
        assert_eq!(store.query_space(1, &desc("m")).unwrap(), None);
        let a = store.bind_space(1, &desc("m")).unwrap();
        let b = store.bind_space(other.id, &desc("m")).unwrap();
        assert_eq!(a, b);
        assert_eq!(store.query_space(1, &desc("m")).unwrap(), Some(a));
        // Mismatch matters once the collection holds chunks.
        index(&store, 1, "doc", &[chunk(0, "a", [1.0, 0.0, 0.0, 0.0])]);
        assert!(matches!(
            store.bind_space(1, &desc("other-model")),
            Err(OragError::ReindexRequired { collection_id: 1 })
        ));
        assert!(matches!(
            store.query_space(1, &desc("other-model")),
            Err(OragError::ReindexRequired { .. })
        ));
    }

    #[test]
    fn lexical_search_matches_normalized_turkish_text() {
        let (_dir, store) = temp_store();
        index(
            &store,
            1,
            "doc",
            &[
                chunk(0, "İade süresi 14 gündür", [1.0, 0.0, 0.0, 0.0]),
                chunk(1, "Kargo ücreti", [0.0, 1.0, 0.0, 0.0]),
            ],
        );
        let hits = store
            .lexical_search(1, &fts_query("iade SÜRESİ").unwrap(), 50)
            .unwrap();
        let chunks = store.get_chunks(1, &hits).unwrap();
        assert_eq!(chunks[0].text, "İade süresi 14 gündür");
        assert_eq!(chunks[0].document_title.as_deref(), Some("Title"));
        assert_eq!(chunks[0].heading_path, vec!["H"]);
    }

    #[test]
    fn lexical_search_survives_hostile_query() {
        let (_dir, store) = temp_store();
        index(
            &store,
            1,
            "doc",
            &[chunk(0, "near text column", [1.0, 0.0, 0.0, 0.0])],
        );
        let query = fts_query(r#"norm_text:"x" NEAR(near* text) -column ^ OR AND NOT"#).unwrap();
        assert!(store.lexical_search(1, &query, 50).is_ok());
    }

    #[test]
    fn dense_search_returns_nearest_with_cosine() {
        let (_dir, store) = temp_store();
        let store = Arc::new(store);
        index(
            &store,
            1,
            "doc",
            &[
                chunk(0, "a", [1.0, 0.0, 0.0, 0.0]),
                chunk(1, "b", [0.0, 1.0, 0.0, 0.0]),
            ],
        );
        let space = store.query_space(1, &desc("m")).unwrap().unwrap();
        let hits = SqliteVecIndex::new(store.clone())
            .search(&space, 1, &[0.9, 0.1, 0.0, 0.0], 2)
            .unwrap();
        let first = store.get_chunks(1, &[hits[0].0]).unwrap();
        assert_eq!(first[0].text, "a");
        assert!(hits[0].1 > hits[1].1);
        // The query is normalized first, so the score is the true cosine.
        let expected = 0.9 / (0.82f32).sqrt();
        assert!(
            (hits[0].1 - expected).abs() < 1e-4,
            "{} vs {expected}",
            hits[0].1
        );
    }

    #[test]
    fn both_searches_are_scoped_to_the_collection() {
        let (_dir, store) = temp_store();
        let store = Arc::new(store);
        let other = store.create_collection("other").unwrap();
        // Skewed data: collection 1 holds three vectors closer to the query than
        // the only vector in `other`. With k = 2, a filter applied *after* top-k
        // would return nothing for `other`; vec0's partition key filters before.
        index(
            &store,
            1,
            "doc-a",
            &[
                chunk(0, "ortak kelime", [1.0, 0.0, 0.0, 0.0]),
                chunk(1, "ortak kelime", [0.99, 0.1, 0.0, 0.0]),
                chunk(2, "ortak kelime", [0.98, 0.2, 0.0, 0.0]),
            ],
        );
        index(
            &store,
            other.id,
            "doc-b",
            &[chunk(0, "ortak kelime", [0.0, 1.0, 0.0, 0.0])],
        );
        let lexical = store
            .lexical_search(other.id, &fts_query("ortak").unwrap(), 50)
            .unwrap();
        let space = store.query_space(other.id, &desc("m")).unwrap().unwrap();
        let dense = SqliteVecIndex::new(store.clone())
            .search(&space, other.id, &[1.0, 0.0, 0.0, 0.0], 2)
            .unwrap();
        for chunk in store.get_chunks(other.id, &lexical).unwrap() {
            assert_eq!(chunk.collection_id, other.id);
        }
        assert_eq!(
            dense.len(),
            1,
            "the collection filter must apply before top-k"
        );
        assert_eq!(
            store.get_chunks(other.id, &[dense[0].0]).unwrap()[0].collection_id,
            other.id
        );
    }

    #[test]
    fn get_chunks_preserves_requested_order_and_skips_missing() {
        let (_dir, store) = temp_store();
        index(
            &store,
            1,
            "doc",
            &[
                chunk(0, "x", [1.0, 0.0, 0.0, 0.0]),
                chunk(1, "y", [0.0, 1.0, 0.0, 0.0]),
            ],
        );
        let ids: Vec<i64> = store
            .get_chunks(1, &[1, 2])
            .unwrap()
            .iter()
            .map(|c| c.id)
            .collect();
        let reversed: Vec<i64> = store
            .get_chunks(1, &[ids[1], 999, ids[0]])
            .unwrap()
            .iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(reversed, vec![ids[1], ids[0]]);
    }

    #[test]
    fn publish_after_document_deleted_is_discarded() {
        let (_dir, store) = temp_store();
        let e = store
            .enqueue_document(
                1,
                NewDocument {
                    filename: None,
                    format: SourceFormat::PlainText,
                    bytes: "x".into(),
                },
            )
            .unwrap();
        let job = store.claim_next_job().unwrap().unwrap();
        let space = store.bind_space(1, &desc("m")).unwrap();
        store.delete_document(1, e.document_id).unwrap();
        let outcome = store
            .publish_document(
                &job,
                &space,
                None,
                &[],
                &[chunk(0, "x", [1.0, 0.0, 0.0, 0.0])],
            )
            .unwrap();
        assert_eq!(outcome, PublishOutcome::Discarded);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM chunks"), 0);
    }

    #[test]
    fn stale_job_cannot_touch_a_later_upload() {
        use crate::store::documents::DocumentStatus;
        use crate::store::jobs::JobStatus;
        let (_dir, store) = temp_store();
        let new = |body: &str| NewDocument {
            filename: None,
            format: SourceFormat::PlainText,
            bytes: body.into(),
        };
        let a = store.enqueue_document(1, new("first")).unwrap();
        let job_a = store.claim_next_job().unwrap().unwrap();
        store.delete_document(1, a.document_id).unwrap();
        let b = store.enqueue_document(1, new("second")).unwrap();
        assert_ne!(
            (b.document_id, b.job_id),
            (a.document_id, a.job_id),
            "ids must never be reused"
        );
        store.fail_job(&job_a, "late failure").unwrap();
        assert_eq!(
            store.get_job(b.job_id.unwrap()).unwrap().status,
            JobStatus::Queued
        );
        assert_eq!(
            store.get_document(1, b.document_id).unwrap().status,
            DocumentStatus::Queued
        );
        let space = store.bind_space(1, &desc("m")).unwrap();
        let outcome = store
            .publish_document(
                &job_a,
                &space,
                None,
                &[],
                &[chunk(0, "x", [1.0, 0.0, 0.0, 0.0])],
            )
            .unwrap();
        assert_eq!(outcome, PublishOutcome::Discarded);
    }

    #[test]
    fn delete_document_removes_fts_and_vectors() {
        let (_dir, store) = temp_store();
        let doc = index(
            &store,
            1,
            "doc",
            &[chunk(0, "silinecek metin", [1.0, 0.0, 0.0, 0.0])],
        );
        let space = store.query_space(1, &desc("m")).unwrap().unwrap();
        store.delete_document(1, doc).unwrap();
        assert_eq!(count(&store, "SELECT COUNT(*) FROM chunks"), 0);
        assert_eq!(
            count(
                &store,
                &format!(
                    "SELECT COUNT(*) FROM {}",
                    crate::store::spaces::vec_table(space.id)
                )
            ),
            0
        );
        assert!(
            store
                .lexical_search(1, &fts_query("silinecek").unwrap(), 50)
                .unwrap()
                .is_empty()
        );
        assert_eq!(count(&store, "SELECT COUNT(*) FROM sources"), 0);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM jobs"), 0);
    }

    #[test]
    fn delete_collection_removes_everything_and_protects_default() {
        let (_dir, store) = temp_store();
        let other = store.create_collection("other").unwrap();
        index(
            &store,
            other.id,
            "doc",
            &[chunk(0, "metin", [1.0, 0.0, 0.0, 0.0])],
        );
        store.delete_collection(other.id).unwrap();
        assert_eq!(count(&store, "SELECT COUNT(*) FROM chunks"), 0);
        assert_eq!(count(&store, "SELECT COUNT(*) FROM documents"), 0);
        assert!(matches!(
            store.get_collection(other.id),
            Err(OragError::NotFound { .. })
        ));
        for table in ["chunks_fts", "jobs", "sources", "embedding_spaces"] {
            assert_eq!(
                count(&store, &format!("SELECT COUNT(*) FROM {table}")),
                0,
                "{table}"
            );
        }
        // The space no longer has a collection, so its vec0 table is dropped.
        assert_eq!(
            count(
                &store,
                "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'vec_space_%'"
            ),
            0
        );
        assert!(matches!(
            store.delete_collection(1),
            Err(OragError::Conflict(_))
        ));
    }

    #[test]
    fn deleting_a_collection_keeps_a_space_another_collection_uses() {
        let (_dir, store) = temp_store();
        let other = store.create_collection("other").unwrap();
        index(&store, 1, "doc-a", &[chunk(0, "a", [1.0, 0.0, 0.0, 0.0])]);
        index(
            &store,
            other.id,
            "doc-b",
            &[chunk(0, "b", [0.0, 1.0, 0.0, 0.0])],
        );
        store.delete_collection(other.id).unwrap();
        assert_eq!(count(&store, "SELECT COUNT(*) FROM embedding_spaces"), 1);
        let space = store.query_space(1, &desc("m")).unwrap().unwrap();
        let hits = SqliteVecIndex::new(Arc::new(store))
            .search(&space, 1, &[1.0, 0.0, 0.0, 0.0], 5)
            .unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn an_empty_collection_follows_a_model_change() {
        let (_dir, store) = temp_store();
        let doc = index(&store, 1, "doc", &[chunk(0, "a", [1.0, 0.0, 0.0, 0.0])]);
        // With chunks, a different model needs a reindex.
        assert!(matches!(
            store.query_space(1, &desc("new")),
            Err(OragError::ReindexRequired { .. })
        ));
        store.delete_document(1, doc).unwrap();
        // Empty, the collection is unbound for queries and rebinds on ingest.
        assert!(store.query_space(1, &desc("new")).unwrap().is_none());
        let space = store.bind_space(1, &desc("new")).unwrap();
        assert_eq!(space.fingerprint, desc("new").fingerprint());
        // The old space lost its last user, so it is gone.
        assert_eq!(count(&store, "SELECT COUNT(*) FROM embedding_spaces"), 1);
    }

    #[test]
    fn get_chunks_rejects_too_many_ids() {
        let (_dir, store) = temp_store();
        let ids: Vec<i64> = (0..=MAX_SEARCH_K as i64).collect();
        assert!(matches!(
            store.get_chunks(1, &ids),
            Err(OragError::InvalidInput(_))
        ));
    }

    #[test]
    fn searching_a_dropped_space_reports_the_collection_missing() {
        let (_dir, store) = temp_store();
        let store = Arc::new(store);
        let other = store.create_collection("other").unwrap();
        index(
            &store,
            other.id,
            "doc",
            &[chunk(0, "a", [1.0, 0.0, 0.0, 0.0])],
        );
        let space = store.query_space(other.id, &desc("m")).unwrap().unwrap();
        store.delete_collection(other.id).unwrap();
        assert!(matches!(
            SqliteVecIndex::new(store).search(&space, other.id, &[1.0, 0.0, 0.0, 0.0], 5),
            Err(OragError::NotFound {
                kind: "collection",
                ..
            })
        ));
    }

    #[test]
    fn get_chunks_ignores_ids_from_other_collections() {
        let (_dir, store) = temp_store();
        let other = store.create_collection("other").unwrap();
        index(
            &store,
            other.id,
            "doc-b",
            &[chunk(0, "gizli", [0.0, 1.0, 0.0, 0.0])],
        );
        let id = count(&store, "SELECT id FROM chunks");
        assert!(store.get_chunks(1, &[id]).unwrap().is_empty());
        assert_eq!(store.get_chunks(other.id, &[id]).unwrap().len(), 1);
    }

    #[test]
    fn search_sizes_are_bounded() {
        let (_dir, store) = temp_store();
        let store = Arc::new(store);
        index(&store, 1, "doc", &[chunk(0, "metin", [1.0, 0.0, 0.0, 0.0])]);
        let space = store.query_space(1, &desc("m")).unwrap().unwrap();
        let index = SqliteVecIndex::new(store.clone());
        assert!(
            index
                .search(&space, 1, &[1.0, 0.0, 0.0, 0.0], 0)
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            index.search(&space, 1, &[1.0, 0.0, 0.0, 0.0], MAX_SEARCH_K + 1),
            Err(OragError::InvalidInput(_))
        ));
        let q = fts_query("metin").unwrap();
        assert!(store.lexical_search(1, &q, 0).unwrap().is_empty());
        assert!(matches!(
            store.lexical_search(1, &q, MAX_SEARCH_K + 1),
            Err(OragError::InvalidInput(_))
        ));
        assert!(matches!(
            index.search(&space, 1, &[f32::NAN, 0.0, 0.0, 0.0], 5),
            Err(OragError::Model(_))
        ));
    }

    #[test]
    fn publish_rejects_vectors_that_are_not_unit_length() {
        let (_dir, store) = temp_store();
        store
            .enqueue_document(
                1,
                NewDocument {
                    filename: Some("d.md".into()),
                    format: SourceFormat::Markdown,
                    bytes: b"x".to_vec(),
                },
            )
            .unwrap();
        let job = store.claim_next_job().unwrap().unwrap();
        let space = store.bind_space(1, &desc("m")).unwrap();
        let mut bad = chunk(0, "a", [1.0, 0.0, 0.0, 0.0]);
        bad.embedding = vec![2.0, 0.0, 0.0, 0.0];
        assert!(matches!(
            store.publish_document(&job, &space, None, &[], &[bad]),
            Err(OragError::Internal(_))
        ));
        assert_eq!(count(&store, "SELECT COUNT(*) FROM chunks"), 0);
    }

    #[test]
    fn wrong_dimension_query_is_rejected() {
        let (_dir, store) = temp_store();
        let store = Arc::new(store);
        index(&store, 1, "doc", &[chunk(0, "a", [1.0, 0.0, 0.0, 0.0])]);
        let space = store.query_space(1, &desc("m")).unwrap().unwrap();
        assert!(
            SqliteVecIndex::new(store)
                .search(&space, 1, &[1.0, 0.0], 5)
                .is_err()
        );
    }
}
