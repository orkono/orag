-- Schema version 3. Never edit after release; add 0004_*.sql instead.
-- The physical PDF pages a chunk came from (first and last, 1-based), for
-- citations. NULL for formats without pages and for chunks indexed before
-- this version (a reindex fills them). Provenance only: no index, no FTS or
-- embedding change.
ALTER TABLE chunks ADD COLUMN page_start INTEGER;
ALTER TABLE chunks ADD COLUMN page_end INTEGER;
