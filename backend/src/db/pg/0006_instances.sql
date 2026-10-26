-- Several server instances can share one database. Each records itself and refreshes a heartbeat;
-- reviews and indexing record the instance running them, so work whose instance stopped is
-- reclaimed instead of all work at every start.
CREATE TABLE server_instances (
  id TEXT COLLATE "C" PRIMARY KEY,
  host TEXT COLLATE "C" NOT NULL,
  started_at BIGINT NOT NULL,
  heartbeat_at BIGINT NOT NULL
);

CREATE INDEX idx_server_instances_host ON server_instances(host);

ALTER TABLE review_runs ADD COLUMN owner_instance TEXT COLLATE "C";
ALTER TABLE repo_index_state ADD COLUMN owner_instance TEXT COLLATE "C";
-- The review index copies (`scratch`) were created LIKE the repository index; they get the column
-- too, since the same index code writes both.
ALTER TABLE scratch.repo_index_state ADD COLUMN owner_instance TEXT COLLATE "C";
