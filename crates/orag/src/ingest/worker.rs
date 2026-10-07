//! Durable ingestion worker (D-008): parse → chunk → embed outside the write
//! lock, then publish atomically.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::domain::chunker::{ChunkDraft, ChunkerConfig, chunk_document, lexical_text};
use crate::domain::normalize::normalize_for_lexical;
use crate::error::{OragError, Result};
use crate::infer::Embedder;
use crate::ingest::format::SourceFormat;
use crate::ingest::parse::parse_document_until;
use crate::store::Store;
use crate::store::jobs::JobRecord;
use crate::store::publish::{PreparedChunk, PublishOutcome};

pub const NO_TEXT_WARNING: &str = "no_text: the document contains no extractable text";
const EMBED_BATCH: usize = 16;
const IDLE_POLL: Duration = Duration::from_secs(5);
/// How often a running job checks that its claim still stands.
const CLAIM_CHECK_INTERVAL: Duration = Duration::from_millis(500);

/// Notices, between embedding batches and while a parser runs, that a reindex
/// closed the job (D-022), so its work stops early instead of being discarded
/// at publish. Store errors count as "still running": publish decides then.
struct ClaimCheck<'a> {
    store: &'a Store,
    job_id: i64,
    last: std::cell::Cell<Option<Instant>>,
    lost: std::cell::Cell<bool>,
}

impl<'a> ClaimCheck<'a> {
    fn new(store: &'a Store, job_id: i64) -> Self {
        ClaimCheck {
            store,
            job_id,
            last: Default::default(),
            lost: Default::default(),
        }
    }

    /// True once the job is no longer `running`; checks the store at most
    /// every `CLAIM_CHECK_INTERVAL`.
    fn lost(&self) -> bool {
        if self.lost.get() {
            return true;
        }
        if self
            .last
            .get()
            .is_some_and(|at| at.elapsed() < CLAIM_CHECK_INTERVAL)
        {
            return false;
        }
        self.last.set(Some(Instant::now()));
        let lost = matches!(self.store.job_is_running(self.job_id), Ok(false));
        self.lost.set(lost);
        lost
    }
}

pub struct IngestContext {
    pub store: Arc<Store>,
    pub embedder: Arc<dyn Embedder>,
    pub chunker: ChunkerConfig,
}

impl IngestContext {
    /// Checks the chunker against the embedding model once at startup, so a
    /// misconfiguration stops the server instead of failing every upload.
    pub fn check(&self) -> Result<()> {
        effective_chunker(&self.chunker, self.embedder.as_ref()).map(|_| ())
    }
}

/// Processes one claimed job. A document (or collection) deleted at any point
/// while the job runs gives `Discarded`, not an error.
pub fn process_job(ctx: &IngestContext, job: &JobRecord) -> Result<PublishOutcome> {
    Ok(process_until(ctx, job, &|| false)?.unwrap_or(PublishOutcome::Discarded))
}

/// `Ok(None)`: `stop` turned true between embedding batches; the job is left
/// `running` and is requeued on the next start.
fn process_until(
    ctx: &IngestContext,
    job: &JobRecord,
    stop: &dyn Fn() -> bool,
) -> Result<Option<PublishOutcome>> {
    match index(ctx, job, stop) {
        Err(OragError::NotFound { .. }) => Ok(Some(PublishOutcome::Discarded)),
        other => other,
    }
}

fn index(
    ctx: &IngestContext,
    job: &JobRecord,
    stop: &dyn Fn() -> bool,
) -> Result<Option<PublishOutcome>> {
    let (doc, bytes) = ctx.store.load_source(job.document_id)?;
    // Fail fast when the collection needs a reindex, before any embedding work.
    ctx.store
        .query_space(job.collection_id, ctx.embedder.descriptor())?;
    let parsed = parse_document_until(SourceFormat::from_name(&doc.format)?, &bytes, stop)?;
    let chunker = effective_chunker(&ctx.chunker, ctx.embedder.as_ref())?;
    let drafts = chunk_document(&parsed, &chunker, &|text| ctx.embedder.count_tokens(text))?;
    let mut warnings = parsed.warnings;
    if drafts.is_empty() {
        warnings.push(NO_TEXT_WARNING.to_string());
    }
    let Some(prepared) = prepare_chunks(ctx.embedder.as_ref(), drafts, stop)? else {
        return Ok(None);
    };
    let space = ctx
        .store
        .bind_space(job.collection_id, ctx.embedder.descriptor())?;
    let title = parsed.title.or(doc.filename);
    ctx.store
        .publish_document(job, &space, title.as_deref(), &warnings, &prepared)
        .map(Some)
}

/// Tokens kept free for the document prefix's merge with the text and for BOS/EOS.
const SPECIAL_TOKEN_RESERVE: usize = 4;

/// Caps the configured chunker to what the embedding model accepts, so every
/// chunk (breadcrumb + body + document prefix + special tokens) fits exactly.
/// The configured values are validated first and kept unchanged when the
/// model has room for them.
pub fn effective_chunker(
    configured: &ChunkerConfig,
    embedder: &dyn Embedder,
) -> Result<ChunkerConfig> {
    configured.validate()?;
    let descriptor = embedder.descriptor();
    let reserve = embedder.count_tokens(&descriptor.document_prefix) + SPECIAL_TOKEN_RESERVE;
    let limit = descriptor.max_tokens.saturating_sub(reserve);
    if configured.max_tokens <= limit {
        return Ok(*configured);
    }
    if limit == 0 {
        return Err(OragError::Model(format!(
            "embedding model {} accepts {} tokens, less than its document prefix needs",
            descriptor.model_id, descriptor.max_tokens
        )));
    }
    let capped = ChunkerConfig {
        max_tokens: limit,
        target_tokens: configured.target_tokens.min(limit),
        overlap_tokens: configured.overlap_tokens.min(limit / 4),
    };
    capped.validate()?;
    Ok(capped)
}

fn prepare_chunks(
    embedder: &dyn Embedder,
    drafts: Vec<ChunkDraft>,
    stop: &dyn Fn() -> bool,
) -> Result<Option<Vec<PreparedChunk>>> {
    let mut prepared = Vec::with_capacity(drafts.len());
    let mut drafts = drafts.into_iter().peekable();
    while drafts.peek().is_some() {
        if stop() {
            return Ok(None);
        }
        let batch: Vec<ChunkDraft> = drafts.by_ref().take(EMBED_BATCH).collect();
        let texts: Vec<String> = batch.iter().map(ChunkDraft::embedding_text).collect();
        let vectors = embedder.embed_documents(&texts)?;
        if vectors.len() != batch.len() {
            return Err(OragError::Internal(format!(
                "embedder returned {} vectors for {} texts",
                vectors.len(),
                batch.len()
            )));
        }
        for (draft, embedding) in batch.into_iter().zip(vectors) {
            prepared.push(PreparedChunk {
                norm_text: normalize_for_lexical(&lexical_text(&draft.heading_path, &draft.text)),
                draft,
                embedding,
            });
        }
    }
    Ok(Some(prepared))
}

/// Message stored on failed jobs/documents: actionable for user errors,
/// generic for internal ones (details go to the log only).
pub fn user_message(err: &OragError) -> String {
    match err {
        OragError::InvalidInput(_)
        | OragError::UnsupportedFormat(_)
        | OragError::ReindexRequired { .. } => err.to_string(),
        _ => "internal error while indexing; see the server log".to_string(),
    }
}

/// Claims and processes one job. Returns `Ok(false)` when the queue is empty.
///
/// A panic while processing (a parser or backend bug) fails the job like any
/// other error, so its document never stays `indexing`. Storage errors here
/// are not retried: the one process that holds `orag.lock` owns the single
/// writer connection, so a busy database only comes from outside tools; a
/// job left `running` by one is requeued on the next start.
pub fn run_once(ctx: &IngestContext) -> Result<bool> {
    run_once_until(ctx, &|| false)
}

/// `run_once` that stops a running job between embedding batches once `stop`
/// turns true (shutdown), leaving it for the next start's requeue.
fn run_once_until(ctx: &IngestContext, stop: &dyn Fn() -> bool) -> Result<bool> {
    let Some(job) = ctx.store.claim_next_job()? else {
        return Ok(false);
    };
    let claim = ClaimCheck::new(&ctx.store, job.id);
    let stop_or_lost = || stop() || claim.lost();
    let outcome = catch_unwind(AssertUnwindSafe(|| process_until(ctx, &job, &stop_or_lost)))
        .unwrap_or_else(|_| {
            Err(OragError::Internal(format!(
                "ingest job {} panicked",
                job.id
            )))
        });
    if claim.lost.get() {
        info!(
            job = job.id,
            "job no longer claimed (document deleted or reindexed); stopped"
        );
        return Ok(true);
    }
    match outcome {
        Ok(None) => info!(job = job.id, "shutting down; job resumes on the next start"),
        Ok(Some(PublishOutcome::Published { chunk_count })) => {
            info!(
                job = job.id,
                document = job.document_id,
                chunk_count,
                "document indexed"
            );
        }
        Ok(Some(PublishOutcome::Discarded)) => {
            info!(
                job = job.id,
                "document deleted or job superseded during indexing; discarded"
            )
        }
        Err(OragError::Interrupted) => info!(
            job = job.id,
            "indexing interrupted by shutdown; will resume after restart"
        ),
        Err(err) => {
            warn!(job = job.id, error = %err, "ingest job failed");
            if !ctx.store.fail_job(&job, &user_message(&err))? {
                info!(job = job.id, "job no longer running; failure not recorded");
            }
        }
    }
    Ok(true)
}

/// Runs jobs until `shutdown` becomes true or its sender is dropped; a job in
/// progress stops at its next embedding batch.
/// Uploads wake the worker with `notify_one`, which stores a permit, so a
/// wake-up sent while a job runs is not lost.
pub fn spawn_worker(
    ctx: Arc<IngestContext>,
    wake: Arc<Notify>,
    mut shutdown: watch::Receiver<bool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if *shutdown.borrow() {
                break;
            }
            let job_ctx = Arc::clone(&ctx);
            let stopping = shutdown.clone();
            let stop = move || *stopping.borrow();
            match tokio::task::spawn_blocking(move || run_once_until(&job_ctx, &stop)).await {
                Ok(Ok(true)) => continue,
                Ok(Ok(false)) => {}
                Ok(Err(err)) => error!(error = %err, "ingest worker storage error"),
                Err(err) => error!(error = %err, "ingest worker task failed"),
            }
            tokio::select! {
                () = wake.notified() => {}
                () = tokio::time::sleep(IDLE_POLL) => {}
                changed = shutdown.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::domain::normalize::fts_query;
    use crate::domain::space::SpaceDescriptor;
    use crate::infer::fake::FakeEmbedder;
    use crate::ingest::format::SourceFormat;
    use crate::store::documents::{DocumentStatus, NewDocument};

    fn context(store: Arc<Store>) -> IngestContext {
        IngestContext {
            store,
            embedder: Arc::new(FakeEmbedder::new()),
            chunker: ChunkerConfig::default(),
        }
    }

    fn temp() -> (tempfile::TempDir, Arc<Store>) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("orag.db")).unwrap());
        (dir, store)
    }

    fn enqueue(
        store: &Store,
        format: SourceFormat,
        body: &[u8],
    ) -> crate::store::documents::Enqueued {
        store
            .enqueue_document(
                1,
                NewDocument {
                    filename: Some("k.md".into()),
                    format,
                    bytes: body.to_vec(),
                },
            )
            .unwrap()
    }

    const DOC: &str = "# Kargo Politikası\n\n## İade\n\nÜrünler 14 gün içinde iade edilebilir.\n";

    #[test]
    fn indexes_markdown_into_searchable_chunks() {
        let (_dir, store) = temp();
        let ctx = context(store.clone());
        let e = enqueue(&store, SourceFormat::Markdown, DOC.as_bytes());
        assert!(run_once(&ctx).unwrap());
        let doc = store.get_document(1, e.document_id).unwrap();
        assert_eq!(doc.status, DocumentStatus::Ready);
        assert_eq!(doc.title.as_deref(), Some("Kargo Politikası"));
        assert_eq!(doc.chunk_count, 1);
        let hits = store
            .lexical_search(1, &fts_query("iade").unwrap(), 10)
            .unwrap();
        assert_eq!(hits.len(), 1);
        // Headings are searchable through the breadcrumb in the shadow text.
        assert_eq!(
            store
                .lexical_search(1, &fts_query("kargo politikasi").unwrap(), 10)
                .unwrap(),
            hits
        );
    }

    #[test]
    fn run_once_returns_false_on_empty_queue() {
        let (_dir, store) = temp();
        assert!(!run_once(&context(store)).unwrap());
    }

    #[test]
    fn invalid_utf8_marks_document_failed_with_message() {
        let (_dir, store) = temp();
        let e = enqueue(&store, SourceFormat::PlainText, &[b'o', 0xC3, 0x28]);
        run_once(&context(store.clone())).unwrap();
        let doc = store.get_document(1, e.document_id).unwrap();
        assert_eq!(doc.status, DocumentStatus::Failed);
        assert!(doc.error.unwrap().contains("UTF-8"));
    }

    #[test]
    fn whitespace_only_document_is_ready_with_warning() {
        let (_dir, store) = temp();
        let e = enqueue(&store, SourceFormat::PlainText, b"  \n\n \n");
        run_once(&context(store.clone())).unwrap();
        let doc = store.get_document(1, e.document_id).unwrap();
        assert_eq!(doc.status, DocumentStatus::Ready);
        assert_eq!(doc.warnings, vec![NO_TEXT_WARNING.to_string()]);
    }

    #[test]
    fn restart_requeues_running_job_and_indexes_once() {
        let (dir, store) = temp();
        let e = enqueue(&store, SourceFormat::Markdown, DOC.as_bytes());
        store.claim_next_job().unwrap().unwrap(); // simulate a crash mid-job
        drop(store);
        let store = Arc::new(Store::open(&dir.path().join("orag.db")).unwrap());
        assert_eq!(store.requeue_running_jobs().unwrap(), 1);
        assert!(run_once(&context(store.clone())).unwrap());
        let doc = store.get_document(1, e.document_id).unwrap();
        assert_eq!(doc.status, DocumentStatus::Ready);
        let rows: i64 = store
            .read()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, doc.chunk_count);
    }

    /// Embedder that deletes the document mid-job, simulating a concurrent DELETE.
    struct DeletingEmbedder {
        inner: FakeEmbedder,
        store: Arc<Store>,
        document_id: i64,
    }

    impl Embedder for DeletingEmbedder {
        fn descriptor(&self) -> &SpaceDescriptor {
            self.inner.descriptor()
        }
        fn count_tokens(&self, text: &str) -> usize {
            self.inner.count_tokens(text)
        }
        fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.store.delete_document(1, self.document_id)?;
            self.inner.embed_documents(texts)
        }
        fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
            self.inner.embed_query(text)
        }
    }

    #[test]
    fn publish_after_delete_is_discarded() {
        let (_dir, store) = temp();
        let e = enqueue(&store, SourceFormat::Markdown, DOC.as_bytes());
        let embedder = DeletingEmbedder {
            inner: FakeEmbedder::new(),
            store: store.clone(),
            document_id: e.document_id,
        };
        let ctx = IngestContext {
            store: store.clone(),
            embedder: Arc::new(embedder),
            chunker: ChunkerConfig::default(),
        };
        let job = store.claim_next_job().unwrap().unwrap();
        assert_eq!(process_job(&ctx, &job).unwrap(), PublishOutcome::Discarded);
        let conn = store.read().unwrap();
        let chunks: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(chunks, 0);
        assert!(
            store
                .lexical_search(1, &fts_query("iade").unwrap(), 10)
                .unwrap()
                .is_empty()
        );
    }

    /// Fake embedder with its own model id, i.e. a different embedding space.
    struct Renamed(FakeEmbedder, SpaceDescriptor);

    impl Renamed {
        fn new(model_id: &str) -> Self {
            let inner = FakeEmbedder::new();
            let mut desc = inner.descriptor().clone();
            desc.model_id = model_id.into();
            Self(inner, desc)
        }
    }

    impl Embedder for Renamed {
        fn descriptor(&self) -> &SpaceDescriptor {
            &self.1
        }
        fn count_tokens(&self, text: &str) -> usize {
            self.0.count_tokens(text)
        }
        fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.0.embed_documents(texts)
        }
        fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
            self.0.embed_query(text)
        }
    }

    #[test]
    fn embedding_model_change_fails_job_with_reindex_message() {
        let (_dir, store) = temp();
        // The collection already holds chunks embedded by a previous model.
        let old = IngestContext {
            store: store.clone(),
            embedder: Arc::new(Renamed::new("previous-model")),
            chunker: ChunkerConfig::default(),
        };
        enqueue(&store, SourceFormat::Markdown, DOC.as_bytes());
        assert!(run_once(&old).unwrap());
        let e = enqueue(&store, SourceFormat::PlainText, b"Kargo ucretsizdir.");
        run_once(&context(store.clone())).unwrap();
        let doc = store.get_document(1, e.document_id).unwrap();
        assert_eq!(doc.status, DocumentStatus::Failed);
        assert!(doc.error.unwrap().contains("reindex"));
    }

    #[test]
    fn reindex_waits_for_indexing_then_reuses_the_stored_source() {
        let (_dir, store) = temp();
        let old = IngestContext {
            store: store.clone(),
            embedder: Arc::new(Renamed::new("previous-model")),
            chunker: ChunkerConfig::default(),
        };
        let first = enqueue(&store, SourceFormat::Markdown, DOC.as_bytes());
        assert!(run_once(&old).unwrap());
        let second = enqueue(&store, SourceFormat::PlainText, b"Kargo ucretsizdir.");
        assert!(
            matches!(store.reindex_collection(1), Err(OragError::Conflict(_))),
            "a queued document blocks a reindex"
        );
        assert!(run_once(&old).unwrap());
        assert_eq!(store.reindex_collection(1).unwrap(), 2);
        let current = context(store.clone());
        while run_once(&current).unwrap() {}
        for id in [first.document_id, second.document_id] {
            let doc = store.get_document(1, id).unwrap();
            assert_eq!(doc.status, DocumentStatus::Ready, "{doc:?}");
            assert!(doc.chunk_count > 0);
        }
        assert!(
            store
                .query_space(1, current.embedder.descriptor())
                .unwrap()
                .is_some(),
            "the collection follows the current model"
        );
        let conn = store.read().unwrap();
        let spaces: i64 = conn
            .query_row("SELECT COUNT(*) FROM embedding_spaces", [], |r| r.get(0))
            .unwrap();
        assert_eq!(spaces, 1, "the old space was dropped");
    }

    #[test]
    fn a_lost_claim_is_noticed() {
        let (_dir, store) = temp();
        let e = enqueue(&store, SourceFormat::Markdown, DOC.as_bytes());
        let job = store.claim_next_job().unwrap().unwrap();
        let claim = ClaimCheck::new(&store, job.id);
        assert!(!claim.lost());
        // Deleting the document while it is indexed takes its job with it.
        store.delete_document(1, e.document_id).unwrap();
        assert!(!claim.lost(), "checked at most every CLAIM_CHECK_INTERVAL");
        claim.last.set(None);
        assert!(claim.lost());
    }

    #[test]
    fn chunker_is_capped_by_the_embedding_model_limit() {
        let embedder = FakeEmbedder::new();
        let roomy = effective_chunker(&ChunkerConfig::default(), &embedder).unwrap();
        assert_eq!(roomy, ChunkerConfig::default());
        let tight = ChunkerConfig {
            target_tokens: 384,
            max_tokens: 512,
            overlap_tokens: 48,
        };
        let mut small = embedder.descriptor().clone();
        small.max_tokens = 104;
        struct Small(FakeEmbedder, SpaceDescriptor);
        impl Embedder for Small {
            fn descriptor(&self) -> &SpaceDescriptor {
                &self.1
            }
            fn count_tokens(&self, t: &str) -> usize {
                self.0.count_tokens(t)
            }
            fn embed_documents(&self, t: &[String]) -> Result<Vec<Vec<f32>>> {
                self.0.embed_documents(t)
            }
            fn embed_query(&self, t: &str) -> Result<Vec<f32>> {
                self.0.embed_query(t)
            }
        }
        let capped = effective_chunker(&tight, &Small(FakeEmbedder::new(), small)).unwrap();
        assert_eq!(
            capped,
            ChunkerConfig {
                target_tokens: 100,
                max_tokens: 100,
                overlap_tokens: 25
            }
        );
    }

    #[test]
    fn internal_errors_are_not_leaked_to_users() {
        let message = user_message(&OragError::Internal("secret path /Users/x".into()));
        assert!(!message.contains("/Users/x"));
    }

    #[test]
    fn a_roomy_model_keeps_the_configured_chunker() {
        let wide = ChunkerConfig {
            target_tokens: 384,
            max_tokens: 512,
            overlap_tokens: 200,
        };
        assert_eq!(
            effective_chunker(&wide, &FakeEmbedder::new()).unwrap(),
            wide
        );
    }

    #[test]
    fn invalid_chunker_config_is_rejected_at_startup() {
        let (_dir, store) = temp();
        let bad = ChunkerConfig {
            target_tokens: 600,
            max_tokens: 512,
            overlap_tokens: 48,
        };
        assert!(effective_chunker(&bad, &FakeEmbedder::new()).is_err());
        let ctx = IngestContext {
            store,
            embedder: Arc::new(FakeEmbedder::new()),
            chunker: bad,
        };
        assert!(ctx.check().is_err());
        let (_dir2, store2) = temp();
        assert!(context(store2).check().is_ok());
    }

    /// Embedder that panics, standing in for a parser or backend bug.
    struct PanickingEmbedder(FakeEmbedder);

    impl Embedder for PanickingEmbedder {
        fn descriptor(&self) -> &SpaceDescriptor {
            self.0.descriptor()
        }
        fn count_tokens(&self, text: &str) -> usize {
            self.0.count_tokens(text)
        }
        fn embed_documents(&self, _: &[String]) -> Result<Vec<Vec<f32>>> {
            panic!("backend bug")
        }
        fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
            self.0.embed_query(text)
        }
    }

    #[test]
    fn a_panicking_job_is_failed_not_left_running() {
        let (_dir, store) = temp();
        let e = enqueue(&store, SourceFormat::Markdown, DOC.as_bytes());
        let ctx = IngestContext {
            store: store.clone(),
            embedder: Arc::new(PanickingEmbedder(FakeEmbedder::new())),
            chunker: ChunkerConfig::default(),
        };
        assert!(run_once(&ctx).unwrap());
        let doc = store.get_document(1, e.document_id).unwrap();
        assert_eq!(doc.status, DocumentStatus::Failed);
        assert!(doc.error.unwrap().contains("internal error"));
    }

    #[test]
    fn a_document_deleted_before_processing_is_discarded() {
        let (_dir, store) = temp();
        let e = enqueue(&store, SourceFormat::Markdown, DOC.as_bytes());
        let job = store.claim_next_job().unwrap().unwrap();
        store.delete_document(1, e.document_id).unwrap();
        assert_eq!(
            process_job(&context(store), &job).unwrap(),
            PublishOutcome::Discarded
        );
    }

    #[tokio::test]
    async fn worker_stops_when_the_shutdown_sender_is_dropped() {
        let (_dir, store) = temp();
        let (tx, rx) = watch::channel(false);
        let handle = spawn_worker(Arc::new(context(store)), Arc::new(Notify::new()), rx);
        drop(tx);
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("worker exits")
            .unwrap();
    }

    #[tokio::test]
    async fn worker_stops_on_shutdown() {
        let (_dir, store) = temp();
        let (tx, rx) = watch::channel(false);
        let handle = spawn_worker(Arc::new(context(store)), Arc::new(Notify::new()), rx);
        tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("worker exits")
            .unwrap();
    }

    #[test]
    fn reindex_is_detected_before_any_embedding_work() {
        let (_dir, store) = temp();
        let old = IngestContext {
            store: store.clone(),
            embedder: Arc::new(Renamed::new("previous-model")),
            chunker: ChunkerConfig::default(),
        };
        enqueue(&store, SourceFormat::Markdown, DOC.as_bytes());
        assert!(run_once(&old).unwrap());
        let e = enqueue(&store, SourceFormat::PlainText, b"Kargo ucretsizdir.");
        // This embedder panics if asked to embed: the check must come first.
        let ctx = IngestContext {
            store: store.clone(),
            embedder: Arc::new(PanickingEmbedder(FakeEmbedder::new())),
            chunker: ChunkerConfig::default(),
        };
        run_once(&ctx).unwrap();
        let doc = store.get_document(1, e.document_id).unwrap();
        assert!(doc.error.unwrap().contains("reindex"));
    }

    #[test]
    fn shutdown_stops_a_running_job_between_batches() {
        let (_dir, store) = temp();
        let e = enqueue(&store, SourceFormat::Markdown, DOC.as_bytes());
        assert!(run_once_until(&context(store.clone()), &|| true).unwrap());
        let doc = store.get_document(1, e.document_id).unwrap();
        // Left for the next start's requeue; nothing was published.
        assert_eq!(doc.status, DocumentStatus::Indexing);
        assert_eq!(doc.chunk_count, 0);
        assert_eq!(store.requeue_running_jobs().unwrap(), 1);
    }
}
