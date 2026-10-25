CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE workspaces (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  provider TEXT NOT NULL DEFAULT 'github',
  login TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL DEFAULT 'org',
  selected INTEGER NOT NULL DEFAULT 0,
  repo_count INTEGER NOT NULL DEFAULT 0,
  review_skill_repo_count INTEGER NOT NULL DEFAULT 0,
  refreshed_at INTEGER,
  monitor_all INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE repos (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  workspace_id INTEGER REFERENCES workspaces(id) ON DELETE SET NULL,
  owner TEXT NOT NULL,
  name TEXT NOT NULL,
  full_name TEXT NOT NULL UNIQUE,
  is_private INTEGER NOT NULL DEFAULT 0,
  default_branch TEXT,
  skill_path TEXT,
  skill_status TEXT NOT NULL DEFAULT 'unknown',
  skill_checked_at INTEGER,
  skill_error TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  poll_error TEXT,
  poll_error_url TEXT,
  poll_error_at INTEGER,
  description TEXT
);

CREATE TABLE subscriptions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo_id INTEGER NOT NULL UNIQUE REFERENCES repos(id) ON DELETE CASCADE,
  enabled INTEGER NOT NULL DEFAULT 1,
  review_mode TEXT NOT NULL DEFAULT 'manual',
  skip_draft_prs INTEGER NOT NULL DEFAULT 1,
  skip_self_prs INTEGER NOT NULL DEFAULT 1,
  last_polled_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE pull_requests (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
  number INTEGER NOT NULL,
  title TEXT NOT NULL,
  author TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'open',
  is_draft INTEGER NOT NULL DEFAULT 0,
  head_sha TEXT NOT NULL,
  base_ref TEXT,
  head_ref TEXT,
  url TEXT,
  review_lane TEXT NOT NULL DEFAULT 'open',
  last_synced_at INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  status_reason TEXT,
  author_avatar_url TEXT,
  last_decision_sha TEXT,
  last_reply_at INTEGER,
  ocr_review_id INTEGER,
  ocr_review_event TEXT,
  decided_by TEXT,
  base_sha TEXT,
  default_branch TEXT,
  is_stacked INTEGER NOT NULL DEFAULT 0,
  force_full_next_review INTEGER NOT NULL DEFAULT 0,
  review_scope_revision INTEGER NOT NULL DEFAULT 0,
  reprocess_next_review INTEGER NOT NULL DEFAULT 0,
  merged_at INTEGER,
  github_updated_at INTEGER,
  UNIQUE(repo_id, number)
);

CREATE TABLE review_runs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  head_sha TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  review_body TEXT,
  comments_json TEXT,
  outcome TEXT,
  event TEXT,
  decision TEXT,
  error_message TEXT,
  phase_json TEXT,
  tail TEXT,
  started_at INTEGER,
  finished_at INTEGER,
  decided_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  input_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens INTEGER NOT NULL DEFAULT 0,
  cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
  usage_model TEXT,
  model TEXT,
  risk_score INTEGER,
  risk_level TEXT,
  risk_json TEXT,
  reviewed_base_sha TEXT,
  reviewed_head_sha TEXT,
  context_index_sha TEXT,
  stacked_base_ref TEXT,
  stacked_base_sha TEXT,
  human_selection_required INTEGER NOT NULL DEFAULT 0,
  review_scope_revision INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE pending_actions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  type TEXT NOT NULL,
  repo_id INTEGER REFERENCES repos(id) ON DELETE CASCADE,
  pr_id INTEGER REFERENCES pull_requests(id) ON DELETE CASCADE,
  review_run_id INTEGER REFERENCES review_runs(id) ON DELETE CASCADE,
  status TEXT NOT NULL DEFAULT 'unseen',
  title TEXT NOT NULL,
  message TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  seen_at INTEGER,
  resolved_at INTEGER
);

CREATE TABLE repo_index_state (
  repo_id INTEGER PRIMARY KEY REFERENCES repos(id) ON DELETE CASCADE,
  status TEXT NOT NULL,
  stage TEXT,
  spec_version INTEGER NOT NULL DEFAULT 0,
  base_ref TEXT,
  base_sha TEXT,
  remote_base_sha TEXT,
  file_count INTEGER NOT NULL DEFAULT 0,
  package_count INTEGER NOT NULL DEFAULT 0,
  edge_count INTEGER NOT NULL DEFAULT 0,
  skipped_count INTEGER NOT NULL DEFAULT 0,
  total_count INTEGER NOT NULL DEFAULT 0,
  started_at INTEGER,
  indexed_at INTEGER,
  duration_ms INTEGER,
  error_message TEXT,
  updated_at INTEGER NOT NULL
);

CREATE TABLE repo_index_packages (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
  package_path TEXT NOT NULL,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  manager TEXT NOT NULL,
  source_roots_json TEXT,
  test_globs_json TEXT,
  indexed_at INTEGER NOT NULL,
  UNIQUE(repo_id, package_path)
);

CREATE TABLE repo_index_files (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
  path TEXT NOT NULL,
  language TEXT,
  size INTEGER NOT NULL,
  blob_sha TEXT NOT NULL,
  package_id INTEGER REFERENCES repo_index_packages(id) ON DELETE SET NULL,
  is_binary INTEGER NOT NULL DEFAULT 0,
  is_generated INTEGER NOT NULL DEFAULT 0,
  indexed_at INTEGER NOT NULL,
  UNIQUE(repo_id, path)
);

CREATE TABLE repo_index_pkg_deps (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
  from_package_id INTEGER NOT NULL REFERENCES repo_index_packages(id) ON DELETE CASCADE,
  to_package_name TEXT NOT NULL,
  dev INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE repo_index_edges (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
  from_path TEXT NOT NULL,
  to_ref TEXT NOT NULL,
  kind TEXT NOT NULL,
  resolved INTEGER NOT NULL DEFAULT 0,
  to_path TEXT
);

CREATE INDEX idx_index_files_repo_path ON repo_index_files(repo_id, path);

CREATE INDEX idx_index_files_package ON repo_index_files(repo_id, package_id);

CREATE INDEX idx_index_edges_from ON repo_index_edges(repo_id, from_path);

CREATE INDEX idx_index_pkg_deps_from ON repo_index_pkg_deps(repo_id, from_package_id);

CREATE INDEX idx_index_edges_to ON repo_index_edges(repo_id, to_path);

CREATE TABLE review_context_snapshots (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
  pr_id INTEGER REFERENCES pull_requests(id) ON DELETE CASCADE,
  review_run_id INTEGER REFERENCES review_runs(id) ON DELETE SET NULL,
  head_sha TEXT,
  base_sha TEXT,
  dir_path TEXT,
  manifest_json TEXT NOT NULL,
  changed_count INTEGER NOT NULL DEFAULT 0,
  related_count INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);

CREATE TABLE review_run_model_usage (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id INTEGER NOT NULL REFERENCES review_runs(id) ON DELETE CASCADE,
  pr_id INTEGER NOT NULL,
  model TEXT NOT NULL,
  input_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens INTEGER NOT NULL DEFAULT 0,
  cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
  UNIQUE(run_id, model)
);

CREATE INDEX idx_run_model_usage_model ON review_run_model_usage(model);

CREATE TABLE review_threads (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo_id INTEGER,
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  gh_id TEXT NOT NULL,
  path TEXT,
  line INTEGER,
  is_resolved INTEGER NOT NULL DEFAULT 0,
  is_outdated INTEGER NOT NULL DEFAULT 0,
  comment_count INTEGER NOT NULL DEFAULT 0,
  last_author TEXT,
  last_at INTEGER,
  comments_json TEXT NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(pr_id, gh_id)
);

CREATE INDEX idx_review_threads_pr ON review_threads(pr_id);

CREATE TABLE skill_versions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  version INTEGER NOT NULL,
  content TEXT NOT NULL,
  note TEXT,
  parent_id INTEGER REFERENCES skill_versions(id) ON DELETE SET NULL,
  source_run_id INTEGER,
  applied_insight_ids TEXT,
  created_at INTEGER NOT NULL,
  deleted_at INTEGER
);

CREATE TABLE review_run_stages (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id INTEGER NOT NULL REFERENCES review_runs(id) ON DELETE CASCADE,
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  stage TEXT NOT NULL,
  chunk_id INTEGER,
  model TEXT,
  status TEXT NOT NULL,
  cache_key TEXT,
  cache_hit INTEGER NOT NULL DEFAULT 0,
  potential_cache_hit INTEGER NOT NULL DEFAULT 0,
  potential_savings_tokens INTEGER NOT NULL DEFAULT 0,
  input_tokens INTEGER NOT NULL DEFAULT 0,
  output_tokens INTEGER NOT NULL DEFAULT 0,
  cache_read_tokens INTEGER NOT NULL DEFAULT 0,
  cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
  prompt_bytes INTEGER NOT NULL DEFAULT 0,
  diff_bytes INTEGER NOT NULL DEFAULT 0,
  context_bytes INTEGER NOT NULL DEFAULT 0,
  file_count INTEGER NOT NULL DEFAULT 0,
  related_file_count INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  finished_at INTEGER
);

CREATE INDEX idx_review_run_stages_run ON review_run_stages(run_id, stage, chunk_id);

CREATE INDEX idx_review_run_stages_cache ON review_run_stages(cache_key);

CREATE TABLE review_chunk_cache (
  cache_key TEXT PRIMARY KEY,
  repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  run_id INTEGER NOT NULL REFERENCES review_runs(id) ON DELETE CASCADE,
  chunk_id INTEGER NOT NULL,
  head_sha TEXT NOT NULL,
  prior_sha TEXT,
  model TEXT NOT NULL,
  prompt_version TEXT NOT NULL,
  skill_hash TEXT NOT NULL,
  index_spec_version INTEGER NOT NULL,
  chunk_json TEXT NOT NULL,
  summary TEXT NOT NULL,
  comments_json TEXT,
  usage_json TEXT NOT NULL,
  prompt_bytes INTEGER NOT NULL DEFAULT 0,
  diff_bytes INTEGER NOT NULL DEFAULT 0,
  context_bytes INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  last_seen_at INTEGER NOT NULL,
  seen_count INTEGER NOT NULL DEFAULT 1,
  last_hit_at INTEGER
);

CREATE INDEX idx_review_chunk_cache_repo ON review_chunk_cache(repo_id, last_seen_at);

CREATE INDEX idx_review_chunk_cache_run ON review_chunk_cache(run_id);

CREATE TABLE pr_summaries (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  pr_id INTEGER NOT NULL,
  head_sha TEXT NOT NULL,
  github_comment_id INTEGER,
  status TEXT NOT NULL,
  body TEXT,
  summary_json TEXT,
  error_message TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  posted_at INTEGER
);

CREATE INDEX idx_pr_summaries_pr_head ON pr_summaries(pr_id, head_sha, status);

CREATE TABLE repo_index_symbols (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  language TEXT NOT NULL,
  path TEXT NOT NULL,
  package_id INTEGER REFERENCES repo_index_packages(id) ON DELETE SET NULL,
  start_line INTEGER NOT NULL,
  end_line INTEGER NOT NULL,
  signature TEXT,
  is_exported INTEGER NOT NULL DEFAULT 0,
  indexed_at INTEGER NOT NULL
);

CREATE INDEX idx_repo_index_symbols_repo_name ON repo_index_symbols(repo_id, name);

CREATE INDEX idx_repo_index_symbols_repo_kind ON repo_index_symbols(repo_id, kind);

CREATE INDEX idx_repo_index_symbols_repo_path ON repo_index_symbols(repo_id, path);

CREATE TABLE repo_index_symbol_edges (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
  from_symbol_id INTEGER NOT NULL REFERENCES repo_index_symbols(id) ON DELETE CASCADE,
  to_symbol_id INTEGER NOT NULL REFERENCES repo_index_symbols(id) ON DELETE CASCADE,
  kind TEXT NOT NULL
);

CREATE INDEX idx_repo_index_symbol_edges_from ON repo_index_symbol_edges(repo_id, from_symbol_id);

CREATE INDEX idx_repo_index_symbol_edges_to ON repo_index_symbol_edges(repo_id, to_symbol_id);

CREATE TABLE review_results (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  installation_id TEXT NOT NULL,
  marker_key TEXT NOT NULL,
  github_comment_id INTEGER,
  run_id INTEGER REFERENCES review_runs(id) ON DELETE SET NULL,
  observed_head_sha TEXT NOT NULL,
  reviewed_head_sha TEXT NOT NULL,
  prior_result_state TEXT,
  prior_result_head_sha TEXT,
  observation TEXT NOT NULL,
  effective_state TEXT NOT NULL,
  publication_status TEXT NOT NULL,
  body TEXT,
  body_hash TEXT,
  github_review_id INTEGER,
  github_review_url TEXT,
  reviewer_version TEXT NOT NULL,
  skill_version TEXT NOT NULL,
  error_message TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  posted_at INTEGER,
  actionable_run_id INTEGER REFERENCES review_runs(id) ON DELETE SET NULL,
  UNIQUE(pr_id, installation_id)
);

CREATE INDEX idx_review_results_publication
  ON review_results(publication_status, updated_at);

CREATE TABLE review_finding_audits (
  run_id INTEGER PRIMARY KEY REFERENCES review_runs(id) ON DELETE CASCADE,
  audit_json TEXT NOT NULL,
  revision INTEGER NOT NULL DEFAULT 0,
  selection_saved INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL
);

CREATE TABLE pr_review_file_counts (
  pr_id INTEGER PRIMARY KEY REFERENCES pull_requests(id) ON DELETE CASCADE,
  changed_file_count INTEGER,
  updated_at INTEGER NOT NULL
);

CREATE INDEX idx_pr_review_file_counts_updated ON pr_review_file_counts(updated_at);

CREATE TABLE review_coverage (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
  run_id INTEGER NOT NULL REFERENCES review_runs(id) ON DELETE CASCADE,
  status TEXT NOT NULL,
  base_ref TEXT NOT NULL,
  base_sha TEXT NOT NULL,
  merge_base_sha TEXT NOT NULL,
  head_sha TEXT NOT NULL,
  changed_files_json TEXT NOT NULL,
  covered_patch_ids_json TEXT NOT NULL,
  diff_hash TEXT NOT NULL,
  diff_text TEXT NOT NULL,
  source_json TEXT,
  created_at INTEGER NOT NULL,
  validated_at INTEGER
);

CREATE INDEX idx_review_coverage_repo_status ON review_coverage(repo_id, status, created_at);

CREATE INDEX idx_review_coverage_pr_status ON review_coverage(pr_id, status, created_at);

CREATE TABLE review_request_restorations (
  run_id INTEGER PRIMARY KEY REFERENCES review_runs(id) ON DELETE CASCADE,
  snapshot_json TEXT NOT NULL,
  marker TEXT NOT NULL,
  review_id INTEGER,
  status TEXT NOT NULL DEFAULT 'pending',
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt_at INTEGER NOT NULL,
  lease_until INTEGER NOT NULL DEFAULT 0,
  last_error TEXT
);

CREATE INDEX idx_review_request_restorations_pending ON review_request_restorations(status, next_attempt_at);

CREATE TABLE comment_scan_cursors (
  repo_id INTEGER PRIMARY KEY REFERENCES repos(id) ON DELETE CASCADE,
  enabled_at INTEGER NOT NULL,
  checkpoint INTEGER NOT NULL
);

CREATE TRIGGER comment_scan_enable_insert AFTER INSERT ON subscriptions WHEN NEW.enabled = 1 BEGIN
  INSERT INTO comment_scan_cursors VALUES (NEW.repo_id, NEW.updated_at, NEW.updated_at)
  ON CONFLICT(repo_id) DO UPDATE SET enabled_at = excluded.enabled_at, checkpoint = excluded.checkpoint;
END;

CREATE TRIGGER comment_scan_enable_update AFTER UPDATE OF enabled ON subscriptions WHEN NEW.enabled = 1 AND OLD.enabled = 0 BEGIN
  INSERT INTO comment_scan_cursors VALUES (NEW.repo_id, NEW.updated_at, NEW.updated_at)
  ON CONFLICT(repo_id) DO UPDATE SET enabled_at = excluded.enabled_at, checkpoint = excluded.checkpoint;
END;

CREATE TABLE comment_activation_receipts (
  repo_id INTEGER NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
  comment_id INTEGER NOT NULL,
  PRIMARY KEY (repo_id, comment_id)
);

CREATE TABLE pr_comment_activations (
  pr_id INTEGER PRIMARY KEY REFERENCES pull_requests(id) ON DELETE CASCADE,
  command_id INTEGER NOT NULL,
  head_sha TEXT NOT NULL,
  marker TEXT NOT NULL,
  body TEXT NOT NULL,
  confirmation_id INTEGER,
  last_error TEXT,
  confirmation_attempts INTEGER NOT NULL DEFAULT 0,
  next_confirmation_at INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE queue_acknowledgements (
  pr_id INTEGER PRIMARY KEY REFERENCES pull_requests(id) ON DELETE CASCADE,
  generation INTEGER NOT NULL,
  head_sha TEXT NOT NULL,
  scope_revision INTEGER NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('pending','posted','retiring','retired')),
  comment_id INTEGER,
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt_at INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  post_uncertain_at INTEGER
);

CREATE TABLE ai_accounts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  provider TEXT NOT NULL CHECK (provider IN ('claude', 'codex')),
  label TEXT NOT NULL,
  priority INTEGER NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  status TEXT NOT NULL DEFAULT 'ok' CHECK (status IN ('ok', 'limited', 'auth_failed')),
  limited_until INTEGER,
  last_error TEXT,
  last_used_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

INSERT INTO settings (key, value, updated_at) VALUES
  ('auto_post_mode', 'comment', strftime('%s','now') * 1000),
  ('auto_skip_keywords', '["bump","deps","deps-dev"]', strftime('%s','now') * 1000),
  ('dismiss_stale_approvals', 'true', strftime('%s','now') * 1000),
  ('font_size', '14', strftime('%s','now') * 1000),
  ('index_strategy', 'graph', strftime('%s','now') * 1000),
  ('mission_index_done', 'false', strftime('%s','now') * 1000),
  ('mission_workspace_configured', 'false', strftime('%s','now') * 1000),
  ('poll_interval_seconds', '60', strftime('%s','now') * 1000),
  ('pr_summary_comment_enabled', 'true', strftime('%s','now') * 1000),
  ('review_archive_retention_days', '1', strftime('%s','now') * 1000),
  ('review_auto_enabled', 'true', strftime('%s','now') * 1000),
  ('review_chunk_size', '5', strftime('%s','now') * 1000),
  ('review_concurrency', '20', strftime('%s','now') * 1000),
  ('review_deep_threshold', '10', strftime('%s','now') * 1000),
  ('review_depth_mode', 'auto', strftime('%s','now') * 1000),
  ('review_max_files', '100', strftime('%s','now') * 1000),
  ('review_parallel_chunks', '6', strftime('%s','now') * 1000),
  ('review_result_mode', 'result_comment', strftime('%s','now') * 1000),
  ('sidebar_collapsed', 'false', strftime('%s','now') * 1000),
  ('theme', 'system', strftime('%s','now') * 1000);
