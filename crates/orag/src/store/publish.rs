//! Atomic publication of a document's chunks, FTS rows and vectors (D-008).

use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use crate::domain::chunker::ChunkDraft;
use crate::domain::space::EmbeddingSpace;
use crate::error::{OragError, Result};
use crate::store::jobs::JobRecord;
use crate::store::search::f32_bytes;
use crate::store::spaces::{collection_space_id, vec_table};
use crate::store::{NOW_SQL, Store};

#[derive(Debug, Clone, PartialEq)]
pub struct PreparedChunk {
    pub draft: ChunkDraft,
    pub norm_text: String,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOutcome {
    Published {
        chunk_count: usize,
    },
    /// The document or collection was deleted while the job ran.
    Discarded,
}

pub(crate) enum ChunkScope {
    Document(i64),
    Collection(i64),
}

/// Deletes chunk rows plus their FTS and vector rows. Call inside a transaction.
pub(crate) fn delete_chunk_rows(
    tx: &Transaction<'_>,
    space_id: Option<i64>,
    scope: ChunkScope,
) -> Result<usize> {
    let (column, id) = match scope {
        ChunkScope::Document(id) => ("document_id", id),
        ChunkScope::Collection(id) => ("collection_id", id),
    };
    let owned = format!("SELECT id FROM chunks WHERE {column} = ?1");
    tx.execute(
        &format!("DELETE FROM chunks_fts WHERE rowid IN ({owned})"),
        [id],
    )?;
    if let Some(space) = space_id {
        tx.execute(
            &format!("DELETE FROM {} WHERE rowid IN ({owned})", vec_table(space)),
            [id],
        )?;
    }
    Ok(tx.execute(&format!("DELETE FROM chunks WHERE {column} = ?1"), [id])?)
}

/// Deletes the given sources once no document refers to them. Call inside a
/// write transaction.
pub(crate) fn delete_orphan_sources(tx: &Transaction<'_>, shas: &[String]) -> Result<()> {
    let mut orphan = tx.prepare(
        "DELETE FROM sources WHERE sha256 = ?1 \
         AND NOT EXISTS (SELECT 1 FROM documents WHERE source_sha256 = ?1)",
    )?;
    for sha in shas {
        orphan.execute([sha])?;
    }
    Ok(())
}

impl Store {
    pub fn publish_document(
        &self,
        job: &JobRecord,
        space: &EmbeddingSpace,
        title: Option<&str>,
        warnings: &[String],
        chunks: &[PreparedChunk],
    ) -> Result<PublishOutcome> {
        if let Some(bad) = chunks
            .iter()
            .find(|c| c.embedding.len() != space.dimensions)
        {
            return Err(OragError::Internal(format!(
                "embedding has {} dimensions, space expects {}",
                bad.embedding.len(),
                space.dimensions
            )));
        }
        if let Some(bad) = chunks.iter().find(|c| !is_unit(&c.embedding)) {
            return Err(OragError::Internal(format!(
                "embedding of chunk {} is not unit length; dense scores assume it is",
                bad.draft.ordinal
            )));
        }
        self.write(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            // Publish only for the claim that still owns the job and an existing, indexing document.
            let status: Option<(String, String)> = tx
                .query_row(
                    "SELECT d.status, j.status FROM jobs j JOIN documents d ON d.id = j.document_id \
                     WHERE j.id = ?1 AND d.id = ?2",
                    params![job.id, job.document_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if status.as_ref().map(|(d, j)| (d.as_str(), j.as_str())) != Some(("indexing", "running")) {
                tx.commit()?;
                return Ok(PublishOutcome::Discarded);
            }
            // The space changed under the job. Returning the error rolls back;
            // the worker records it with `fail_job`, as for any job error.
            if collection_space_id(&tx, job.collection_id)? != Some(space.id) {
                return Err(OragError::ReindexRequired { collection_id: job.collection_id });
            }
            delete_chunk_rows(&tx, Some(space.id), ChunkScope::Document(job.document_id))?;
            insert_chunks(&tx, job, space, chunks)?;
            tx.execute(
                &format!(
                    "UPDATE documents SET status = 'ready', error = NULL, title = ?2, chunk_count = ?3, \
                     warnings = ?4, updated_at = {NOW_SQL} WHERE id = ?1"
                ),
                params![job.document_id, title, chunks.len() as i64, serde_json::to_string(warnings)?],
            )?;
            tx.execute(
                &format!("UPDATE jobs SET status = 'succeeded', error = NULL, updated_at = {NOW_SQL} WHERE id = ?1"),
                [job.id],
            )?;
            tx.commit()?;
            Ok(PublishOutcome::Published { chunk_count: chunks.len() })
        })
    }
}

/// Finite, with an L2 norm of 1 (within f32 rounding).
fn is_unit(vector: &[f32]) -> bool {
    let norm = vector
        .iter()
        .map(|v| f64::from(*v).powi(2))
        .sum::<f64>()
        .sqrt();
    norm.is_finite() && (norm - 1.0).abs() < 1e-3
}

fn insert_chunks(
    tx: &Transaction<'_>,
    job: &JobRecord,
    space: &EmbeddingSpace,
    chunks: &[PreparedChunk],
) -> Result<()> {
    let mut insert_chunk = tx.prepare(
        "INSERT INTO chunks (document_id, collection_id, ordinal, heading_path, text, token_count) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    let mut insert_fts = tx.prepare("INSERT INTO chunks_fts (rowid, norm_text) VALUES (?1, ?2)")?;
    let mut insert_vec = tx.prepare(&format!(
        "INSERT INTO {} (rowid, collection_id, embedding) VALUES (?1, ?2, ?3)",
        vec_table(space.id)
    ))?;
    for chunk in chunks {
        insert_chunk.execute(params![
            job.document_id,
            job.collection_id,
            chunk.draft.ordinal,
            serde_json::to_string(&chunk.draft.heading_path)?,
            chunk.draft.text,
            chunk.draft.token_count as i64,
        ])?;
        let chunk_id = tx.last_insert_rowid();
        insert_fts.execute(params![chunk_id, chunk.norm_text])?;
        insert_vec.execute(params![
            chunk_id,
            job.collection_id,
            f32_bytes(&chunk.embedding)
        ])?;
    }
    Ok(())
}
