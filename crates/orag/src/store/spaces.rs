//! Embedding-space registry and per-space vec0 tables (D-009).

use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::CollectionId;
use crate::domain::space::{EmbeddingSpace, SpaceDescriptor};
use crate::error::{OragError, Result};
use crate::store::Store;

const MAX_DIMENSIONS: usize = 8192;

/// Name of the vec0 table for a space. Built only from an integer id.
pub(crate) fn vec_table(space_id: i64) -> String {
    format!("vec_space_{space_id}")
}

pub(crate) fn load_space(conn: &Connection, space_id: i64) -> rusqlite::Result<EmbeddingSpace> {
    conn.query_row(
        "SELECT id, fingerprint, dimensions FROM embedding_spaces WHERE id = ?1",
        [space_id],
        |row| {
            Ok(EmbeddingSpace {
                id: row.get(0)?,
                fingerprint: row.get(1)?,
                dimensions: row.get::<_, i64>(2)? as usize,
            })
        },
    )
}

pub(crate) fn collection_space_id(
    conn: &Connection,
    collection_id: CollectionId,
) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT embedding_space_id FROM collections WHERE id = ?1",
        [collection_id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or(OragError::NotFound {
        kind: "collection",
        id: collection_id,
    })
}

/// Drops a space's vec0 table and registry row once no collection uses it.
/// Call inside a write transaction.
pub(crate) fn drop_unused_space(conn: &Connection, space_id: i64) -> Result<()> {
    let users: i64 = conn.query_row(
        "SELECT COUNT(*) FROM collections WHERE embedding_space_id = ?1",
        [space_id],
        |r| r.get(0),
    )?;
    if users == 0 {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS {}", vec_table(space_id)))?;
        conn.execute("DELETE FROM embedding_spaces WHERE id = ?1", [space_id])?;
    }
    Ok(())
}

fn has_chunks(conn: &Connection, collection_id: CollectionId) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM chunks WHERE collection_id = ?1)",
        [collection_id],
        |r| r.get(0),
    )
}

impl Store {
    /// Space for ingestion: binds the collection on first use, creating the
    /// space and its vec0 table if needed. Mismatch → `ReindexRequired`.
    pub fn bind_space(
        &self,
        collection_id: CollectionId,
        desc: &SpaceDescriptor,
    ) -> Result<EmbeddingSpace> {
        if desc.dimensions == 0 || desc.dimensions > MAX_DIMENSIONS {
            return Err(OragError::InvalidInput(format!(
                "embedding dimensions must be 1-{MAX_DIMENSIONS}"
            )));
        }
        let fingerprint = desc.fingerprint();
        self.write(|conn| {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let bound = collection_space_id(&tx, collection_id)?;
            if let Some(space_id) = bound {
                let space = load_space(&tx, space_id)?;
                if desc.accepts(&space.fingerprint) {
                    tx.commit()?;
                    return Ok(space);
                }
                // A collection with no chunks has nothing to reindex: it simply
                // follows the new model.
                if has_chunks(&tx, collection_id)? {
                    return Err(OragError::ReindexRequired { collection_id });
                }
            }
            let existing: Option<i64> = tx
                .query_row("SELECT id FROM embedding_spaces WHERE fingerprint = ?1", [&fingerprint], |r| r.get(0))
                .optional()?;
            let space_id = match existing {
                Some(id) => id,
                None => {
                    tx.execute(
                        "INSERT INTO embedding_spaces (fingerprint, model_id, dimensions, descriptor) VALUES (?1, ?2, ?3, ?4)",
                        params![fingerprint, desc.model_id, desc.dimensions as i64, serde_json::to_string(desc)?],
                    )?;
                    let id = tx.last_insert_rowid();
                    tx.execute_batch(&format!(
                        "CREATE VIRTUAL TABLE {} USING vec0(collection_id integer partition key, embedding float[{}])",
                        vec_table(id),
                        desc.dimensions
                    ))?;
                    id
                }
            };
            tx.execute("UPDATE collections SET embedding_space_id = ?1 WHERE id = ?2", params![space_id, collection_id])?;
            if let Some(old) = bound {
                drop_unused_space(&tx, old)?;
            }
            let space = load_space(&tx, space_id)?;
            tx.commit()?;
            Ok(space)
        })
    }

    /// Space for querying: `None` if the collection has nothing indexed under
    /// it (never indexed, or emptied before a model change).
    pub fn query_space(
        &self,
        collection_id: CollectionId,
        desc: &SpaceDescriptor,
    ) -> Result<Option<EmbeddingSpace>> {
        let conn = self.read()?;
        let Some(space_id) = collection_space_id(&conn, collection_id)? else {
            return Ok(None);
        };
        let space = load_space(&conn, space_id)?;
        if !desc.accepts(&space.fingerprint) {
            // Empty: nothing to search, and ingest will rebind it.
            if !has_chunks(&conn, collection_id)? {
                return Ok(None);
            }
            return Err(OragError::ReindexRequired { collection_id });
        }
        Ok(Some(space))
    }
}

#[cfg(test)]
mod tests {
    use crate::error::OragError;
    use crate::infer::Embedder;
    use crate::infer::fake::FakeEmbedder;
    use crate::retrieval::testing::indexed;

    #[test]
    fn a_space_stored_with_a_legacy_fingerprint_keeps_serving() {
        let (_dir, retriever) = indexed(FakeEmbedder::new(), &[("a.md", "# A\n\nalpha")]);
        let store = &retriever.store;
        let desc = FakeEmbedder::new().descriptor().clone();
        let set_stored = |fingerprint: &str| {
            store
                .write(|conn| {
                    conn.execute(
                        "UPDATE embedding_spaces SET fingerprint = ?1",
                        [fingerprint],
                    )?;
                    Ok(())
                })
                .unwrap();
        };
        let bound = store.query_space(1, &desc).unwrap().unwrap();
        for normalizer in [1, 2] {
            set_stored(&desc.legacy_fingerprint(normalizer));
            let space = store
                .query_space(1, &desc)
                .unwrap()
                .expect("legacy space is served");
            assert_eq!(space.id, bound.id);
            assert_eq!(store.bind_space(1, &desc).unwrap().id, bound.id);
        }
        set_stored(&"0".repeat(64));
        assert!(matches!(
            store.query_space(1, &desc),
            Err(OragError::ReindexRequired { collection_id: 1 })
        ));
        assert!(matches!(
            store.bind_space(1, &desc),
            Err(OragError::ReindexRequired { collection_id: 1 })
        ));
    }
}
