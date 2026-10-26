-- Per-review index of the pull request's head commit (it was a SQLite file per run). The tables
-- copy the repository index tables, are UNLOGGED (rebuilt on demand, never worth WAL), and key
-- each review by its run id in place of a repository id. The server reaches them only through
-- connections whose search_path is `scratch` alone, so an index query can never touch the
-- operational tables, and those connections need no schema changes per review.
CREATE SCHEMA scratch;

-- No unique full_name: two reviews of the same repository index at the same time.
CREATE UNLOGGED TABLE scratch.repos (LIKE public.repos INCLUDING DEFAULTS INCLUDING IDENTITY);
ALTER TABLE scratch.repos ADD PRIMARY KEY (id);

CREATE UNLOGGED TABLE scratch.repo_index_state (LIKE public.repo_index_state INCLUDING ALL);
CREATE UNLOGGED TABLE scratch.repo_index_packages (LIKE public.repo_index_packages INCLUDING ALL);
CREATE UNLOGGED TABLE scratch.repo_index_files (LIKE public.repo_index_files INCLUDING ALL);
CREATE UNLOGGED TABLE scratch.repo_index_pkg_deps (LIKE public.repo_index_pkg_deps INCLUDING ALL);
CREATE UNLOGGED TABLE scratch.repo_index_edges (LIKE public.repo_index_edges INCLUDING ALL);
CREATE UNLOGGED TABLE scratch.repo_index_symbols (LIKE public.repo_index_symbols INCLUDING ALL);
CREATE UNLOGGED TABLE scratch.repo_index_symbol_edges (LIKE public.repo_index_symbol_edges INCLUDING ALL);

-- Rows come and go with every review; vacuum them sooner than the defaults would.
ALTER TABLE scratch.repo_index_files SET (autovacuum_vacuum_scale_factor = 0.02);
ALTER TABLE scratch.repo_index_edges SET (autovacuum_vacuum_scale_factor = 0.02);
ALTER TABLE scratch.repo_index_symbols SET (autovacuum_vacuum_scale_factor = 0.02);
ALTER TABLE scratch.repo_index_symbol_edges SET (autovacuum_vacuum_scale_factor = 0.02);
