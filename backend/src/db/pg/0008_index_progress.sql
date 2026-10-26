-- Indexing progress lives in the database, so any instance can report an index another one runs.
ALTER TABLE repo_index_state ADD COLUMN progress_json TEXT;
-- The review index copies share the index code, so they get the column too.
ALTER TABLE scratch.repo_index_state ADD COLUMN progress_json TEXT;
