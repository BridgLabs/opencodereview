-- The review archive index (it was index.sqlite next to the sealed archives). The archives
-- themselves stay files under OCR_DATA_DIR/review-archives; these tables say which exist, who
-- links them and what retention has queued for deletion.
CREATE SCHEMA archive;
GRANT USAGE ON SCHEMA archive TO PUBLIC;

CREATE TABLE archive.artifacts (
  id TEXT COLLATE "C" PRIMARY KEY,
  descriptor TEXT COLLATE "C" NOT NULL,
  bytes BIGINT NOT NULL
);

CREATE TABLE archive.uploads (
  owner TEXT COLLATE "C" PRIMARY KEY,
  id TEXT COLLATE "C" NOT NULL,
  descriptor TEXT COLLATE "C" NOT NULL,
  manifest BYTEA NOT NULL,
  deadline BIGINT NOT NULL,
  reservation BIGINT NOT NULL
);

CREATE INDEX idx_archive_uploads_id ON archive.uploads(id);
CREATE INDEX idx_archive_uploads_deadline ON archive.uploads(deadline);

CREATE TABLE archive.receipts (
  owner TEXT COLLATE "C" PRIMARY KEY,
  id TEXT COLLATE "C" NOT NULL
);

CREATE INDEX idx_archive_receipts_id ON archive.receipts(id);

CREATE TABLE archive.deletions (
  id TEXT COLLATE "C" PRIMARY KEY,
  bytes BIGINT NOT NULL,
  phase TEXT COLLATE "C" NOT NULL,
  created_at BIGINT NOT NULL
);

-- Archive retention's work: archives it decided to delete, and a record of each expiry.
CREATE TABLE archive.retention_queue (
  id TEXT COLLATE "C" PRIMARY KEY,
  bytes BIGINT NOT NULL
);

CREATE TABLE archive.retention_events (
  owner TEXT COLLATE "C" NOT NULL,
  id TEXT COLLATE "C" NOT NULL,
  expired_at BIGINT NOT NULL,
  bytes BIGINT NOT NULL,
  PRIMARY KEY (owner, id)
);
