//! Documents and their immutable source snapshots.

use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::domain::{CollectionId, DocumentId, JobId};
use crate::error::{OragError, Result};
use crate::ingest::format::SourceFormat;
use crate::store::{NOW_SQL, Store, conversion_error};

const MAX_FILENAME_CHARS: usize = 255;
pub const MAX_PAGE_SIZE: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DocumentStatus {
    Queued,
    Indexing,
    Ready,
    Failed,
}

impl DocumentStatus {
    pub(crate) fn parse(text: &str) -> Option<DocumentStatus> {
        match text {
            "queued" => Some(DocumentStatus::Queued),
            "indexing" => Some(DocumentStatus::Indexing),
            "ready" => Some(DocumentStatus::Ready),
            "failed" => Some(DocumentStatus::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DocumentRecord {
    pub id: DocumentId,
    pub collection_id: CollectionId,
    pub filename: Option<String>,
    pub format: String,
    pub title: Option<String>,
    pub status: DocumentStatus,
    pub error: Option<String>,
    pub chunk_count: i64,
    pub warnings: Vec<String>,
    pub source_sha256: String,
    pub size_bytes: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDocument {
    pub filename: Option<String>,
    pub format: SourceFormat,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Enqueued {
    pub document_id: DocumentId,
    // `duplicate` is true only when nothing new will run: a re-upload that
    // queues a retry is `false`, so the client follows the new job.
    /// The job that will (or did) ingest it; `None` only for an existing
    /// document whose jobs are gone and that needs no new one.
    pub job_id: Option<JobId>,
    pub duplicate: bool,
}

/// A page of documents after `after_id`; `limit` must be 1..=`MAX_PAGE_SIZE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub after_id: Option<i64>,
    pub limit: u32,
}

/// The columns `document_from_row` reads, in its order.
const DOCUMENT_COLUMNS: &str = "d.id, d.collection_id, d.filename, d.format, d.title, d.status, \
     d.error, d.chunk_count, d.warnings, d.source_sha256, s.size_bytes, d.created_at, d.updated_at";
const DOCUMENT_FROM: &str = "FROM documents d JOIN sources s ON s.sha256 = d.source_sha256";
/// Number of columns in `DOCUMENT_COLUMNS`; a query may append more after them.
const DOCUMENT_COLUMN_COUNT: usize = 13;

pub(crate) fn select_document() -> String {
    format!("SELECT {DOCUMENT_COLUMNS} {DOCUMENT_FROM}")
}

pub(crate) fn document_from_row(row: &Row<'_>) -> rusqlite::Result<DocumentRecord> {
    let status: String = row.get(5)?;
    let warnings: String = row.get(8)?;
    Ok(DocumentRecord {
        id: row.get(0)?,
        collection_id: row.get(1)?,
        filename: row.get(2)?,
        format: row.get(3)?,
        title: row.get(4)?,
        status: DocumentStatus::parse(&status)
            .ok_or_else(|| conversion_error(5, format!("unknown status {status}")))?,
        error: row.get(6)?,
        chunk_count: row.get(7)?,
        warnings: serde_json::from_str(&warnings)
            .map_err(|e| conversion_error(8, e.to_string()))?,
        source_sha256: row.get(9)?,
        size_bytes: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

/// NFC, only the last path component; rejects hidden characters.
fn clean_filename(filename: Option<String>) -> Result<Option<String>> {
    let Some(raw) = filename else { return Ok(None) };
    let raw: String = raw.nfc().collect();
    let base = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if base.is_empty() || base == "." || base == ".." {
        return Ok(None);
    }
    if base.chars().count() > MAX_FILENAME_CHARS || base.chars().any(is_hidden_char) {
        return Err(OragError::InvalidInput(
            "filename is too long or contains control, invisible or line-breaking characters"
                .into(),
        ));
    }
    Ok(Some(base))
}

/// Characters that make a displayed name differ from the real one: controls,
/// Unicode's default-ignorable set (bidi overrides, zero-width and tag
/// characters, fillers, variation selectors) and line/paragraph separators.
fn is_hidden_char(c: char) -> bool {
    c.is_control()
        || crate::domain::normalize::is_default_ignorable(c)
        || matches!(c, '\u{2028}' | '\u{2029}')
}

pub(crate) fn ensure_collection(conn: &Connection, collection_id: CollectionId) -> Result<()> {
    conn.query_row(
        "SELECT 1 FROM collections WHERE id = ?1",
        [collection_id],
        |_| Ok(()),
    )
    .optional()?
    .ok_or(OragError::NotFound {
        kind: "collection",
        id: collection_id,
    })
}

fn insert_job(tx: &Transaction<'_>, document_id: DocumentId) -> Result<JobId> {
    tx.execute(
        "INSERT INTO jobs (document_id, kind, status) VALUES (?1, 'ingest', 'queued')",
        [document_id],
    )?;
    Ok(tx.last_insert_rowid())
}

/// The document in `collection_id` with this content: id, status, latest job.
fn find_same_content(
    tx: &Transaction<'_>,
    collection_id: CollectionId,
    sha: &str,
) -> Result<Option<(DocumentId, DocumentStatus, Option<JobId>)>> {
    Ok(tx
        .query_row(
            "SELECT d.id, d.status, (SELECT MAX(j.id) FROM jobs j WHERE j.document_id = d.id) \
             FROM documents d WHERE d.collection_id = ?1 AND d.source_sha256 = ?2",
            params![collection_id, sha],
            |row| {
                let status: String = row.get(1)?;
                let status = DocumentStatus::parse(&status)
                    .ok_or_else(|| conversion_error(1, format!("unknown status {status}")))?;
                Ok((row.get(0)?, status, row.get(2)?))
            },
        )
        .optional()?)
}

/// Handles an upload whose content the collection already has. A failed
/// ingest is retried: the failed attempt's results are cleared (publishing
/// is atomic, so a failed document has no chunk rows) and the new upload's
/// format is used, with its name; a nameless upload keeps the old name only
/// if the format is the same, so name and format never disagree. A document still
/// queued without a job gets one; anything else is the existing document.
fn reuse_document(
    tx: &Transaction<'_>,
    (document_id, status, job_id): (DocumentId, DocumentStatus, Option<JobId>),
    filename: Option<&str>,
    format: SourceFormat,
) -> Result<Enqueued> {
    let requeue = match status {
        DocumentStatus::Failed => true,
        DocumentStatus::Queued => job_id.is_none(),
        DocumentStatus::Indexing | DocumentStatus::Ready => false,
    };
    if !requeue {
        return Ok(Enqueued {
            document_id,
            job_id,
            duplicate: true,
        });
    }
    tx.execute(
        &format!(
            "UPDATE documents SET status = 'queued', error = NULL, title = NULL, warnings = '[]', \
             chunk_count = 0, \
             filename = CASE WHEN ?2 IS NULL AND format = ?3 THEN filename ELSE ?2 END, format = ?3, \
             updated_at = {NOW_SQL} WHERE id = ?1"
        ),
        params![document_id, filename, format.as_str()],
    )?;
    Ok(Enqueued {
        document_id,
        job_id: Some(insert_job(tx, document_id)?),
        duplicate: false,
    })
}

impl Store {
    /// Stores the source snapshot and queues an ingest job. Identical content in
    /// the same collection returns the existing document (`duplicate: true`,
    /// or `false` with a new job when a failed ingest is retried);
    /// if its ingest failed, it is queued again (see `reuse_document`).
    pub fn enqueue_document(
        &self,
        collection_id: CollectionId,
        doc: NewDocument,
    ) -> Result<Enqueued> {
        if doc.bytes.is_empty() {
            return Err(OragError::InvalidInput("document is empty".into()));
        }
        let filename = clean_filename(doc.filename)?;
        let sha = hex::encode(Sha256::digest(&doc.bytes));
        self.write(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            ensure_collection(&tx, collection_id)?;
            let enqueued = match find_same_content(&tx, collection_id, &sha)? {
                Some(existing) => reuse_document(&tx, existing, filename.as_deref(), doc.format)?,
                None => {
                    tx.execute(
                        "INSERT OR IGNORE INTO sources (sha256, size_bytes, bytes) VALUES (?1, ?2, ?3)",
                        params![sha, doc.bytes.len() as i64, doc.bytes],
                    )?;
                    tx.execute(
                        "INSERT INTO documents (collection_id, source_sha256, filename, format, status) \
                         VALUES (?1, ?2, ?3, ?4, 'queued')",
                        params![collection_id, sha, filename, doc.format.as_str()],
                    )?;
                    let document_id = tx.last_insert_rowid();
                    Enqueued { document_id, job_id: Some(insert_job(&tx, document_id)?), duplicate: false }
                }
            };
            tx.commit()?;
            Ok(enqueued)
        })
    }

    pub fn get_document(
        &self,
        collection_id: CollectionId,
        document_id: DocumentId,
    ) -> Result<DocumentRecord> {
        let conn = self.read()?;
        ensure_collection(&conn, collection_id)?;
        conn.query_row(
            &format!(
                "{} WHERE d.id = ?1 AND d.collection_id = ?2",
                select_document()
            ),
            params![document_id, collection_id],
            document_from_row,
        )
        .optional()?
        .ok_or(OragError::NotFound {
            kind: "document",
            id: document_id,
        })
    }

    /// Documents after `page.after_id` in id order; a `limit` outside
    /// 1..=`MAX_PAGE_SIZE` is rejected rather than changed.
    pub fn list_documents(
        &self,
        collection_id: CollectionId,
        page: Page,
    ) -> Result<Vec<DocumentRecord>> {
        if !(1..=MAX_PAGE_SIZE).contains(&page.limit) {
            return Err(OragError::InvalidInput(format!(
                "limit must be 1-{MAX_PAGE_SIZE}"
            )));
        }
        let conn = self.read()?;
        ensure_collection(&conn, collection_id)?;
        let mut stmt = conn.prepare(&format!(
            "{} WHERE d.collection_id = ?1 AND d.id > ?2 ORDER BY d.id LIMIT ?3",
            select_document()
        ))?;
        let rows = stmt.query_map(
            params![collection_id, page.after_id.unwrap_or(0), page.limit],
            document_from_row,
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The document and its source bytes, read in one statement (one snapshot).
    pub fn load_source(&self, document_id: DocumentId) -> Result<(DocumentRecord, Vec<u8>)> {
        let conn = self.read()?;
        conn.query_row(
            &format!("SELECT {DOCUMENT_COLUMNS}, s.bytes {DOCUMENT_FROM} WHERE d.id = ?1"),
            [document_id],
            |row| {
                Ok((
                    document_from_row(row)?,
                    row.get::<_, Vec<u8>>(DOCUMENT_COLUMN_COUNT)?,
                ))
            },
        )
        .optional()?
        .ok_or(OragError::NotFound {
            kind: "document",
            id: document_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::testing::temp_store;

    fn md(name: &str, body: &str) -> NewDocument {
        NewDocument {
            filename: Some(name.into()),
            format: SourceFormat::Markdown,
            bytes: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn enqueue_creates_queued_document_and_job() {
        let (_dir, store) = temp_store();
        let e = store.enqueue_document(1, md("a.md", "# A")).unwrap();
        assert!(!e.duplicate);
        let doc = store.get_document(1, e.document_id).unwrap();
        assert_eq!(doc.status, DocumentStatus::Queued);
        assert_eq!(doc.filename.as_deref(), Some("a.md"));
        assert_eq!(doc.size_bytes, 3);
        assert_eq!(
            store.get_job(e.job_id.unwrap()).unwrap().status,
            crate::store::jobs::JobStatus::Queued
        );
    }

    #[test]
    fn same_content_in_same_collection_is_a_duplicate() {
        let (_dir, store) = temp_store();
        let first = store.enqueue_document(1, md("a.md", "same")).unwrap();
        let second = store.enqueue_document(1, md("renamed.md", "same")).unwrap();
        assert!(second.duplicate);
        assert_eq!(
            (second.document_id, second.job_id),
            (first.document_id, first.job_id)
        );
    }

    #[test]
    fn same_content_in_other_collection_shares_the_source() {
        let (_dir, store) = temp_store();
        let other = store.create_collection("other").unwrap();
        store.enqueue_document(1, md("a.md", "same")).unwrap();
        let e = store
            .enqueue_document(other.id, md("a.md", "same"))
            .unwrap();
        assert!(!e.duplicate);
        let sources: i64 = store
            .read()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM sources", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sources, 1);
    }

    #[test]
    fn filename_is_reduced_to_basename_and_validated() {
        let (_dir, store) = temp_store();
        let e = store
            .enqueue_document(1, md("../../etc/notes.md", "x"))
            .unwrap();
        assert_eq!(
            store
                .get_document(1, e.document_id)
                .unwrap()
                .filename
                .as_deref(),
            Some("notes.md")
        );
        let e = store
            .enqueue_document(1, md("C:\\Users\\a\\b.md", "y"))
            .unwrap();
        assert_eq!(
            store
                .get_document(1, e.document_id)
                .unwrap()
                .filename
                .as_deref(),
            Some("b.md")
        );
        assert!(store.enqueue_document(1, md("bad\u{7}.md", "z")).is_err());
    }

    #[test]
    fn empty_upload_and_unknown_collection_are_rejected() {
        let (_dir, store) = temp_store();
        assert!(matches!(
            store.enqueue_document(1, md("a.md", "")),
            Err(OragError::InvalidInput(_))
        ));
        assert!(matches!(
            store.enqueue_document(9, md("a.md", "x")),
            Err(OragError::NotFound {
                kind: "collection",
                ..
            })
        ));
    }

    #[test]
    fn documents_are_scoped_to_their_collection() {
        let (_dir, store) = temp_store();
        let other = store.create_collection("other").unwrap();
        let e = store.enqueue_document(1, md("a.md", "x")).unwrap();
        assert!(matches!(
            store.get_document(other.id, e.document_id),
            Err(OragError::NotFound {
                kind: "document",
                ..
            })
        ));
    }

    #[test]
    fn list_paginates_by_id() {
        let (_dir, store) = temp_store();
        let ids: Vec<i64> = (0..5)
            .map(|i| {
                store
                    .enqueue_document(1, md("a.md", &format!("doc {i}")))
                    .unwrap()
                    .document_id
            })
            .collect();
        let first = store
            .list_documents(
                1,
                Page {
                    after_id: None,
                    limit: 2,
                },
            )
            .unwrap();
        assert_eq!(first.iter().map(|d| d.id).collect::<Vec<_>>(), ids[..2]);
        let next = store
            .list_documents(
                1,
                Page {
                    after_id: Some(ids[1]),
                    limit: 10,
                },
            )
            .unwrap();
        assert_eq!(next.iter().map(|d| d.id).collect::<Vec<_>>(), ids[2..]);
    }

    #[test]
    fn load_source_returns_original_bytes() {
        let (_dir, store) = temp_store();
        let e = store.enqueue_document(1, md("a.md", "# Başlık")).unwrap();
        let (doc, bytes) = store.load_source(e.document_id).unwrap();
        assert_eq!(doc.id, e.document_id);
        assert_eq!(bytes, "# Başlık".as_bytes());
    }

    #[test]
    fn reuploading_a_failed_document_queues_it_again() {
        let (_dir, store) = temp_store();
        let first = store.enqueue_document(1, md("a.md", "same")).unwrap();
        let job = store.claim_next_job().unwrap().unwrap();
        store.fail_job(&job, "broken").unwrap();
        let again = store.enqueue_document(1, md("a.md", "same")).unwrap();
        assert!(
            !again.duplicate,
            "a new ingest was queued, so the client should follow it"
        );
        assert_eq!(again.document_id, first.document_id);
        assert_ne!(
            again.job_id, first.job_id,
            "a new job retries the failed ingest"
        );
        let doc = store.get_document(1, first.document_id).unwrap();
        assert_eq!((doc.status, doc.error), (DocumentStatus::Queued, None));
    }

    #[test]
    fn a_duplicate_without_any_job_gets_one() {
        let (_dir, store) = temp_store();
        let first = store.enqueue_document(1, md("a.md", "same")).unwrap();
        store
            .write(|conn| Ok(conn.execute("DELETE FROM jobs", [])?))
            .unwrap();
        let again = store.enqueue_document(1, md("a.md", "same")).unwrap();
        assert_eq!(again.document_id, first.document_id);
        assert_eq!(
            store.get_job(again.job_id.unwrap()).unwrap().status,
            crate::store::jobs::JobStatus::Queued
        );
    }

    #[test]
    fn invisible_and_bidi_characters_are_refused_in_filenames() {
        let (_dir, store) = temp_store();
        for bad in [
            "report\u{202E}dm.exe",
            "a\u{200B}b.md",
            "x\u{2066}y.md",
            "\u{FEFF}z.md",
            "report\u{2028}.md",
            "inv\u{E0065}x.md",
            "a\u{3164}.md",
            "b\u{FE0F}.md",
        ] {
            assert!(
                matches!(
                    store.enqueue_document(1, md(bad, bad)),
                    Err(OragError::InvalidInput(_))
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_retry_uses_the_new_uploads_name_and_format() {
        let (_dir, store) = temp_store();
        let first = store.enqueue_document(1, md("wrong.md", "same")).unwrap();
        let job = store.claim_next_job().unwrap().unwrap();
        store.fail_job(&job, "broken").unwrap();
        let fixed = NewDocument {
            filename: Some("right.txt".into()),
            format: SourceFormat::PlainText,
            bytes: b"same".to_vec(),
        };
        store.enqueue_document(1, fixed).unwrap();
        let doc = store.get_document(1, first.document_id).unwrap();
        assert_eq!(
            (doc.filename.as_deref(), doc.format.as_str()),
            (Some("right.txt"), "text")
        );
    }

    #[test]
    fn a_ready_document_without_jobs_is_returned_as_is() {
        let (_dir, store) = temp_store();
        let first = store.enqueue_document(1, md("a.md", "same")).unwrap();
        store
            .write(|conn| {
                conn.execute_batch("UPDATE documents SET status = 'ready'; DELETE FROM jobs;")?;
                Ok(())
            })
            .unwrap();
        let again = store.enqueue_document(1, md("a.md", "same")).unwrap();
        assert_eq!(
            (again.document_id, again.job_id, again.duplicate),
            (first.document_id, None, true)
        );
        assert_eq!(
            store.get_document(1, first.document_id).unwrap().status,
            DocumentStatus::Ready
        );
    }

    #[test]
    fn a_retry_clears_the_failed_attempts_results_and_keeps_a_known_name() {
        let (_dir, store) = temp_store();
        let first = store.enqueue_document(1, md("report.md", "same")).unwrap();
        let job = store.claim_next_job().unwrap().unwrap();
        store
            .write(|conn| {
                conn.execute_batch(
                    "UPDATE documents SET title = 'old', warnings = '[\"w\"]', chunk_count = 3",
                )?;
                Ok(())
            })
            .unwrap();
        store.fail_job(&job, "broken").unwrap();
        let nameless = NewDocument {
            filename: None,
            format: SourceFormat::Markdown,
            bytes: b"same".to_vec(),
        };
        store.enqueue_document(1, nameless).unwrap();
        let doc = store.get_document(1, first.document_id).unwrap();
        assert_eq!(doc.filename.as_deref(), Some("report.md"));
        assert_eq!(
            (doc.title, doc.warnings.len(), doc.chunk_count),
            (None, 0, 0)
        );
    }

    #[test]
    fn document_column_count_matches_the_column_list() {
        assert_eq!(DOCUMENT_COLUMNS.split(',').count(), DOCUMENT_COLUMN_COUNT);
    }

    #[test]
    fn page_limits_outside_the_range_are_rejected() {
        let (_dir, store) = temp_store();
        for limit in [0, MAX_PAGE_SIZE + 1] {
            let page = Page {
                after_id: None,
                limit,
            };
            assert!(
                matches!(
                    store.list_documents(1, page),
                    Err(OragError::InvalidInput(_))
                ),
                "{limit}"
            );
        }
    }

    #[test]
    fn a_nameless_retry_in_another_format_drops_the_old_name() {
        let (_dir, store) = temp_store();
        let first = store.enqueue_document(1, md("report.md", "same")).unwrap();
        let job = store.claim_next_job().unwrap().unwrap();
        store.fail_job(&job, "broken").unwrap();
        let text = NewDocument {
            filename: None,
            format: SourceFormat::PlainText,
            bytes: b"same".to_vec(),
        };
        store.enqueue_document(1, text).unwrap();
        let doc = store.get_document(1, first.document_id).unwrap();
        assert_eq!((doc.filename, doc.format.as_str()), (None, "text"));
    }

    #[test]
    fn filenames_are_stored_in_nfc() {
        let (_dir, store) = temp_store();
        let e = store
            .enqueue_document(1, md("Bas\u{0327}lik.md", "x"))
            .unwrap();
        let doc = store.get_document(1, e.document_id).unwrap();
        assert_eq!(doc.filename.as_deref(), Some("Başlik.md"));
    }

    #[test]
    fn a_missing_collection_is_not_found_when_listing_or_reading() {
        let (_dir, store) = temp_store();
        let page = Page {
            after_id: None,
            limit: 10,
        };
        assert!(matches!(
            store.list_documents(9, page),
            Err(OragError::NotFound {
                kind: "collection",
                ..
            })
        ));
        assert!(matches!(
            store.get_document(9, 1),
            Err(OragError::NotFound {
                kind: "collection",
                ..
            })
        ));
    }
}
