-- Schema version 1. Never edit after release; add 0002_*.sql instead.
-- AUTOINCREMENT on mutable entities: ids are never reused after deletion, so a
-- stale job or citation can never address a newer row.

CREATE TABLE embedding_spaces (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    fingerprint TEXT NOT NULL UNIQUE,
    model_id TEXT NOT NULL,
    dimensions INTEGER NOT NULL CHECK (dimensions > 0),
    descriptor TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE collections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    embedding_space_id INTEGER REFERENCES embedding_spaces(id),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Immutable source snapshots, shared across collections by content hash.
CREATE TABLE sources (
    sha256 TEXT PRIMARY KEY,
    size_bytes INTEGER NOT NULL,
    bytes BLOB NOT NULL
);

CREATE TABLE documents (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    collection_id INTEGER NOT NULL REFERENCES collections(id),
    source_sha256 TEXT NOT NULL REFERENCES sources(sha256),
    filename TEXT,
    format TEXT NOT NULL,
    title TEXT,
    status TEXT NOT NULL CHECK (status IN ('queued', 'indexing', 'ready', 'failed')),
    error TEXT,
    chunk_count INTEGER NOT NULL DEFAULT 0,
    warnings TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (collection_id, source_sha256),
    -- Target of the chunks (document_id, collection_id) foreign key.
    UNIQUE (id, collection_id)
);
CREATE INDEX documents_collection ON documents(collection_id, id);

CREATE TABLE jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('ingest')),
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed')),
    error TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX jobs_queue ON jobs(status, id);
-- Cascade from documents and the sources FK check would otherwise scan.
CREATE INDEX jobs_document ON jobs(document_id);
CREATE INDEX documents_source ON documents(source_sha256);

-- No ON DELETE CASCADE: chunk deletion must go through store code so FTS and
-- vector rows are removed in the same transaction (the FK makes forgetting fail).
CREATE TABLE chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id INTEGER NOT NULL,
    collection_id INTEGER NOT NULL, -- checked through the composite FK below
    ordinal INTEGER NOT NULL,
    heading_path TEXT NOT NULL,
    text TEXT NOT NULL,
    token_count INTEGER NOT NULL,
    UNIQUE (document_id, ordinal),
    -- A chunk is always filed under its document's collection, so
    -- collection-scoped search can never return another collection's text.
    FOREIGN KEY (document_id, collection_id) REFERENCES documents (id, collection_id)
);
CREATE INDEX chunks_collection ON chunks(collection_id);

-- Normalized shadow text only (D-006); rowid == chunks.id.
CREATE VIRTUAL TABLE chunks_fts USING fts5(
    norm_text,
    content = '',
    contentless_delete = 1,
    tokenize = 'unicode61 remove_diacritics 0'
);

INSERT INTO collections (name) VALUES ('default');

-- Marks the file as an orag database ('ORAG'); Store::open checks it.
PRAGMA application_id = 1330790727;
