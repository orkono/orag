//! Durable ingestion job queue (D-008).

use rusqlite::{OptionalExtension, Row, TransactionBehavior, params};
use serde::Serialize;

use crate::domain::{CollectionId, DocumentId, JobId};
use crate::error::{OragError, Result};
use crate::store::{NOW_SQL, Store, conversion_error};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
}

impl JobStatus {
    fn parse(text: &str) -> Option<JobStatus> {
        match text {
            "queued" => Some(JobStatus::Queued),
            "running" => Some(JobStatus::Running),
            "succeeded" => Some(JobStatus::Succeeded),
            "failed" => Some(JobStatus::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JobRecord {
    pub id: JobId,
    pub document_id: DocumentId,
    pub collection_id: CollectionId,
    pub status: JobStatus,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

const SELECT_JOB: &str = "SELECT j.id, j.document_id, d.collection_id, j.status, j.error, \
     j.created_at, j.updated_at FROM jobs j JOIN documents d ON d.id = j.document_id";

fn job_from_row(row: &Row<'_>) -> rusqlite::Result<JobRecord> {
    let status: String = row.get(3)?;
    Ok(JobRecord {
        id: row.get(0)?,
        document_id: row.get(1)?,
        collection_id: row.get(2)?,
        status: JobStatus::parse(&status)
            .ok_or_else(|| conversion_error(3, format!("unknown status {status}")))?,
        error: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

impl Store {
    pub fn get_job(&self, job_id: JobId) -> Result<JobRecord> {
        let conn = self.read()?;
        conn.query_row(
            &format!("{SELECT_JOB} WHERE j.id = ?1"),
            [job_id],
            job_from_row,
        )
        .optional()?
        .ok_or(OragError::NotFound {
            kind: "job",
            id: job_id,
        })
    }

    /// Atomically moves the oldest queued job to `running` and its document to `indexing`.
    pub fn claim_next_job(&self) -> Result<Option<JobRecord>> {
        self.write(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let next: Option<(JobId, DocumentId)> = tx
                .query_row(
                    "SELECT id, document_id FROM jobs WHERE status = 'queued' ORDER BY id LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((job_id, document_id)) = next else {
                tx.commit()?;
                return Ok(None);
            };
            tx.execute(&format!("UPDATE jobs SET status = 'running', updated_at = {NOW_SQL} WHERE id = ?1"), [job_id])?;
            tx.execute(
                &format!("UPDATE documents SET status = 'indexing', error = NULL, updated_at = {NOW_SQL} WHERE id = ?1"),
                [document_id],
            )?;
            let job = tx.query_row(&format!("{SELECT_JOB} WHERE j.id = ?1"), [job_id], job_from_row)?;
            tx.commit()?;
            Ok(Some(job))
        })
    }

    /// Records a user-safe failure message on the job and its document and
    /// says whether it did. Only a job still `running` is changed; one process
    /// owns the database (`orag.lock`), so a running job belongs to its claim.
    pub fn fail_job(&self, job: &JobRecord, message: &str) -> Result<bool> {
        self.write(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let owned = tx.execute(
                &format!(
                    "UPDATE jobs SET status = 'failed', error = ?2, updated_at = {NOW_SQL} \
                     WHERE id = ?1 AND status = 'running'"
                ),
                params![job.id, message],
            )?;
            if owned == 1 {
                tx.execute(
                    &format!(
                        "UPDATE documents SET status = 'failed', error = ?2, updated_at = {NOW_SQL} \
                         WHERE id = ?1 AND status = 'indexing'"
                    ),
                    params![job.document_id, message],
                )?;
            }
            tx.commit()?;
            Ok(owned == 1)
        })
    }

    /// Restart recovery: jobs left `running` by a crash go back to `queued`.
    pub fn requeue_running_jobs(&self) -> Result<usize> {
        self.write(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let moved = tx.execute(
                &format!("UPDATE jobs SET status = 'queued', updated_at = {NOW_SQL} WHERE status = 'running'"),
                [],
            )?;
            tx.execute(
                &format!("UPDATE documents SET status = 'queued', updated_at = {NOW_SQL} WHERE status = 'indexing'"),
                [],
            )?;
            tx.commit()?;
            Ok(moved)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::format::SourceFormat;
    use crate::store::documents::{DocumentStatus, NewDocument};
    use crate::store::testing::temp_store;

    fn enqueue(store: &crate::store::Store, body: &str) -> crate::store::documents::Enqueued {
        store
            .enqueue_document(
                1,
                NewDocument {
                    filename: None,
                    format: SourceFormat::PlainText,
                    bytes: body.into(),
                },
            )
            .unwrap()
    }

    #[test]
    fn claim_takes_oldest_and_marks_running_and_indexing() {
        let (_dir, store) = temp_store();
        let first = enqueue(&store, "one");
        enqueue(&store, "two");
        let job = store.claim_next_job().unwrap().unwrap();
        assert_eq!(Some(job.id), first.job_id);
        assert_eq!(job.status, JobStatus::Running);
        assert_eq!(job.collection_id, 1);
        assert_eq!(
            store.get_document(1, first.document_id).unwrap().status,
            DocumentStatus::Indexing
        );
    }

    #[test]
    fn claim_returns_none_when_queue_is_empty() {
        let (_dir, store) = temp_store();
        assert!(store.claim_next_job().unwrap().is_none());
    }

    #[test]
    fn fail_marks_job_and_document() {
        let (_dir, store) = temp_store();
        let e = enqueue(&store, "x");
        let job = store.claim_next_job().unwrap().unwrap();
        store.fail_job(&job, "parser exploded").unwrap();
        let job = store.get_job(e.job_id.unwrap()).unwrap();
        assert_eq!(
            (job.status, job.error.as_deref()),
            (JobStatus::Failed, Some("parser exploded"))
        );
        let doc = store.get_document(1, e.document_id).unwrap();
        assert_eq!(
            (doc.status, doc.error.as_deref()),
            (DocumentStatus::Failed, Some("parser exploded"))
        );
    }

    #[test]
    fn requeue_moves_running_jobs_back_to_queue() {
        let (_dir, store) = temp_store();
        let e = enqueue(&store, "x");
        store.claim_next_job().unwrap().unwrap();
        assert_eq!(store.requeue_running_jobs().unwrap(), 1);
        assert_eq!(
            store.get_job(e.job_id.unwrap()).unwrap().status,
            JobStatus::Queued
        );
        assert_eq!(
            store.get_document(1, e.document_id).unwrap().status,
            DocumentStatus::Queued
        );
    }

    #[test]
    fn failing_a_job_that_is_not_running_reports_it() {
        let (_dir, store) = temp_store();
        enqueue(&store, "x");
        let job = store.claim_next_job().unwrap().unwrap();
        assert!(store.fail_job(&job, "first").unwrap());
        assert!(
            !store.fail_job(&job, "again").unwrap(),
            "already failed: nothing recorded"
        );
    }
}
