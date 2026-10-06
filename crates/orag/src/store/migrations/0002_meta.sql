-- Schema version 2. Never edit after release; add 0003_*.sql instead.
-- Settings of the database itself. `lexical_version` records what the FTS
-- index holds (domain::normalize::LEXICAL_VERSION), so a lexical change
-- rebuilds that index from `chunks` at startup instead of re-embedding.
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;
