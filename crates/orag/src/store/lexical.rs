//! The FTS index is derived data (D-006): `chunks` keeps every chunk's
//! heading path and text, so a lexical change (`LEXICAL_VERSION`) rebuilds the
//! index at startup. Vectors and embedding spaces are not touched.

use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::chunker::lexical_text;
use crate::domain::normalize::{LEXICAL_VERSION, normalize_for_lexical};
use crate::error::{OragError, Result};
use crate::store::Store;

const LEXICAL_VERSION_KEY: &str = "lexical_version";

fn stored_version(conn: &Connection) -> Result<Option<u32>> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [LEXICAL_VERSION_KEY],
            |r| r.get(0),
        )
        .optional()?;
    value
        .map(|v| {
            v.parse().map_err(|_| {
                OragError::Internal(format!("meta {LEXICAL_VERSION_KEY} is not a number: {v:?}"))
            })
        })
        .transpose()
}

/// An index from a newer release is not rebuilt down: two binaries would
/// otherwise keep rebuilding each other's index.
fn newer_index(version: u32) -> OragError {
    OragError::InvalidInput(format!(
        "the full-text index was built by a newer orag (lexical version {version}, this \
         release knows {LEXICAL_VERSION}); use that release or newer"
    ))
}

impl Store {
    /// The lexical version the FTS index was built with; `None` before the
    /// first build.
    pub fn lexical_version(&self) -> Result<Option<u32>> {
        stored_version(&self.read()?)
    }

    /// Rebuilds the FTS index when it was built with another `LEXICAL_VERSION`
    /// (or by a release that did not record one). Returns the number of
    /// chunks indexed, or `None` when the index was current. A current index
    /// is checked without the write lock, so opening never waits for another
    /// process's writes. The rebuild is one transaction: an interrupted one
    /// leaves the old index and version and runs again at the next start; the
    /// version is re-read under the write lock, so two processes never both
    /// rebuild.
    pub(crate) fn ensure_lexical_index(&self) -> Result<Option<usize>> {
        match self.lexical_version()? {
            Some(version) if version == LEXICAL_VERSION => return Ok(None),
            Some(version) if version > LEXICAL_VERSION => return Err(newer_index(version)),
            _ => {}
        }
        self.write(|conn| {
            // Waits like a migration: another process may be rebuilding a large index.
            let tx = crate::store::migrations::begin_immediate(conn)?;
            match stored_version(&tx)? {
                Some(version) if version == LEXICAL_VERSION => {
                    tx.commit()?;
                    return Ok(None);
                }
                Some(version) if version > LEXICAL_VERSION => return Err(newer_index(version)),
                _ => {}
            }
            // Contentless FTS5 has no 'rebuild': clear it, then insert every chunk.
            tx.execute(
                "INSERT INTO chunks_fts (chunks_fts) VALUES ('delete-all')",
                [],
            )?;
            let mut rows = 0;
            {
                let mut select = tx.prepare("SELECT id, heading_path, text FROM chunks")?;
                let mut insert =
                    tx.prepare("INSERT INTO chunks_fts (rowid, norm_text) VALUES (?1, ?2)")?;
                let mut chunks = select.query([])?;
                while let Some(row) = chunks.next()? {
                    let id: i64 = row.get(0)?;
                    let heading_path: Vec<String> =
                        serde_json::from_str(&row.get::<_, String>(1)?)?;
                    let text: String = row.get(2)?;
                    insert.execute(params![
                        id,
                        normalize_for_lexical(&lexical_text(&heading_path, &text))
                    ])?;
                    rows += 1;
                }
            }
            tx.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2) \
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![LEXICAL_VERSION_KEY, LEXICAL_VERSION.to_string()],
            )?;
            tx.commit()?;
            Ok(Some(rows))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::normalize::fts_query;
    use crate::infer::fake::FakeEmbedder;
    use crate::retrieval::testing::indexed;

    const DOC: &str = "# Garanti\n\n## Süre\n\nCihaz yirmi dört ay korunur.";

    fn hits(store: &Store, query: &str) -> usize {
        store
            .lexical_search(1, &fts_query(query).unwrap(), 10)
            .unwrap()
            .len()
    }

    /// Simulates an index written by an older release.
    fn make_stale(store: &Store, version: Option<&str>) {
        store
            .write(|conn| {
                conn.execute(
                    "INSERT INTO chunks_fts (chunks_fts) VALUES ('delete-all')",
                    [],
                )?;
                conn.execute("DELETE FROM meta", [])?;
                if let Some(v) = version {
                    conn.execute(
                        "INSERT INTO meta (key, value) VALUES (?1, ?2)",
                        params![LEXICAL_VERSION_KEY, v],
                    )?;
                }
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn a_new_database_records_the_current_version() {
        let (_dir, retriever) = indexed(FakeEmbedder::new(), &[]);
        assert_eq!(
            retriever.store.lexical_version().unwrap(),
            Some(LEXICAL_VERSION)
        );
    }

    #[test]
    fn an_older_index_is_rebuilt_at_open_with_heading_words() {
        for old in [None, Some("1"), Some("2")] {
            let (dir, retriever) = indexed(FakeEmbedder::new(), &[("g.md", DOC)]);
            make_stale(&retriever.store, old);
            assert_eq!(hits(&retriever.store, "korunur"), 0);
            drop(retriever);
            let store = Store::open(&dir.path().join("orag.db")).unwrap();
            assert_eq!(store.lexical_version().unwrap(), Some(LEXICAL_VERSION));
            assert_eq!(hits(&store, "korunur"), 1, "{old:?}");
            // The full heading path is indexed, also the top heading.
            assert_eq!(hits(&store, "garanti süre"), 1, "{old:?}");
        }
    }

    #[test]
    fn an_index_from_a_newer_release_is_refused_not_rebuilt() {
        let (dir, retriever) = indexed(FakeEmbedder::new(), &[("g.md", DOC)]);
        let newer = (LEXICAL_VERSION + 1).to_string();
        make_stale(&retriever.store, Some(&newer));
        drop(retriever);
        let err = Store::open(&dir.path().join("orag.db"))
            .err()
            .expect("refused");
        assert!(err.to_string().contains("newer orag"), "{err}");
    }

    #[test]
    fn a_current_index_is_left_alone() {
        let (dir, retriever) = indexed(FakeEmbedder::new(), &[("g.md", DOC)]);
        make_stale(&retriever.store, Some(&LEXICAL_VERSION.to_string()));
        drop(retriever);
        let store = Store::open(&dir.path().join("orag.db")).unwrap();
        assert_eq!(store.ensure_lexical_index().unwrap(), None);
        assert_eq!(
            hits(&store, "korunur"),
            0,
            "no rebuild when the version matches"
        );
    }

    #[test]
    fn a_rebuild_waits_for_another_writer_beyond_the_busy_timeout() {
        let (dir, retriever) = indexed(FakeEmbedder::new(), &[("g.md", DOC)]);
        make_stale(&retriever.store, Some("1"));
        drop(retriever);
        let path = dir.path().join("orag.db");
        let holder = rusqlite::Connection::open(&path).unwrap();
        holder.execute_batch("BEGIN IMMEDIATE;").unwrap();
        let release = std::thread::spawn(move || {
            // Longer than the connections' 5 s busy_timeout.
            std::thread::sleep(std::time::Duration::from_secs(6));
            holder.execute_batch("COMMIT;").unwrap();
        });
        let store = Store::open(&path).expect("waits for the writer, then rebuilds");
        release.join().unwrap();
        assert_eq!(hits(&store, "korunur"), 1);
    }

    #[test]
    fn a_rebuild_matches_what_ingestion_writes() {
        let (dir, retriever) = indexed(FakeEmbedder::new(), &[("g.md", DOC)]);
        let fresh = hits(&retriever.store, "garanti süre yirmi");
        make_stale(&retriever.store, Some("1"));
        drop(retriever);
        let store = Store::open(&dir.path().join("orag.db")).unwrap();
        assert_eq!(hits(&store, "garanti süre yirmi"), fresh);
    }
}
