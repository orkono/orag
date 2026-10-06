//! Query-time pipeline: deterministic hybrid retrieval (D-005) and grounded answers.

pub mod answer;
pub mod hybrid;
pub mod repetition;

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Arc;

    use crate::domain::chunker::ChunkerConfig;
    use crate::infer::Embedder;
    use crate::infer::fake::FakeEmbedder;
    use crate::ingest::format::SourceFormat;
    use crate::ingest::worker::{IngestContext, run_once};
    use crate::retrieval::hybrid::{RetrievalConfig, Retriever};
    use crate::store::Store;
    use crate::store::documents::NewDocument;
    use crate::store::search::SqliteVecIndex;

    /// Temp store with `docs` (name, markdown) indexed into the default collection.
    pub(crate) fn indexed(
        embedder: FakeEmbedder,
        docs: &[(&str, &str)],
    ) -> (tempfile::TempDir, Retriever) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("orag.db")).unwrap());
        let embedder: Arc<dyn Embedder> = Arc::new(embedder);
        let ctx = IngestContext {
            store: store.clone(),
            embedder: embedder.clone(),
            chunker: ChunkerConfig::default(),
        };
        for (name, body) in docs {
            store
                .enqueue_document(
                    1,
                    NewDocument {
                        filename: Some((*name).into()),
                        format: SourceFormat::Markdown,
                        bytes: body.as_bytes().to_vec(),
                    },
                )
                .unwrap();
            assert!(run_once(&ctx).unwrap());
        }
        let retriever = Retriever {
            index: Arc::new(SqliteVecIndex::new(store.clone())),
            store,
            embedder,
            config: RetrievalConfig::default(),
        };
        (dir, retriever)
    }
}
