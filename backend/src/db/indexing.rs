use super::*;

impl Database {
    pub fn related_index_files(
        &self,
        repo_id: i64,
        paths: &[String],
        limit: i64,
    ) -> Result<Vec<(String, Option<String>, String)>, String> {
        let mut out = Vec::new();
        for path in paths {
            let mut stmt = self.conn.prepare(
                "SELECT f.path, f.language, CASE WHEN e.from_path = $2 THEN 'imports seed' ELSE 'imported by seed' END FROM repo_index_edges e JOIN repo_index_files f ON f.repo_id = e.repo_id AND f.path = CASE WHEN e.from_path = $2 THEN e.to_path ELSE e.from_path END WHERE e.repo_id = $1 AND e.resolved = 1 AND (e.from_path = $2 OR e.to_path = $2) LIMIT $3",
            ).map_err(|error| error.to_string())?;
            let rows = stmt
                .query_map(params![repo_id, path, limit.clamp(1, 100)], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            out.extend(rows);
        }
        out.sort_by(|left: &(String, Option<String>, String), right| left.0.cmp(&right.0));
        out.dedup_by(|left, right| left.0 == right.0);
        out.truncate(limit.max(0) as usize);
        Ok(out)
    }

    pub fn repo_index_status(&self, repo_id: i64) -> Result<RepoIndexStatus, String> {
        let mut stmt = self.conn.prepare(
      "SELECT status, stage, base_ref, base_sha, remote_base_sha, spec_version, file_count, package_count, edge_count,
              (SELECT COUNT(*) FROM repo_index_symbols WHERE repo_id = $1),
              skipped_count, total_count, started_at, indexed_at, duration_ms, error_message, updated_at
       FROM repo_index_state WHERE repo_id = $1",
    ).map_err(|error| error.to_string())?;
        let mut rows = stmt.query(params![repo_id]).map_err(|error| error.to_string())?;
        if let Some(row) = rows.next().map_err(|error| error.to_string())? {
            Ok(RepoIndexStatus {
                repo_id,
                status: row.get(0).map_err(|error| error.to_string())?,
                stage: row.get(1).map_err(|error| error.to_string())?,
                base_ref: row.get(2).map_err(|error| error.to_string())?,
                base_sha: row.get(3).map_err(|error| error.to_string())?,
                remote_base_sha: row.get(4).map_err(|error| error.to_string())?,
                spec_version: row.get(5).map_err(|error| error.to_string())?,
                file_count: row.get(6).map_err(|error| error.to_string())?,
                package_count: row.get(7).map_err(|error| error.to_string())?,
                edge_count: row.get(8).map_err(|error| error.to_string())?,
                symbol_count: row.get(9).map_err(|error| error.to_string())?,
                skipped_count: row.get(10).map_err(|error| error.to_string())?,
                total_count: row.get(11).map_err(|error| error.to_string())?,
                started_at: row.get(12).map_err(|error| error.to_string())?,
                indexed_at: row.get(13).map_err(|error| error.to_string())?,
                duration_ms: row.get(14).map_err(|error| error.to_string())?,
                error_message: row.get(15).map_err(|error| error.to_string())?,
                updated_at: row.get(16).map_err(|error| error.to_string())?,
            })
        } else {
            Ok(RepoIndexStatus {
                repo_id,
                status: "not_indexed".to_string(),
                stage: None,
                base_ref: None,
                base_sha: None,
                remote_base_sha: None,
                spec_version: 0,
                file_count: 0,
                package_count: 0,
                edge_count: 0,
                symbol_count: 0,
                skipped_count: 0,
                total_count: 0,
                started_at: None,
                indexed_at: None,
                duration_ms: None,
                error_message: None,
                updated_at: now_millis(),
            })
        }
    }

    /// The internal package dependency graph for the interactive visualization:
    /// nodes = indexed packages, edges = internal package→package deps (a dep whose
    /// `to_package_name` matches another indexed package's name; external/npm deps and
    /// self-loops are dropped).
    pub fn index_graph(&self, repo_id: i64) -> Result<IndexGraph, String> {
        let mut node_stmt = self
            .conn
            .prepare("SELECT id, name, package_path, kind FROM repo_index_packages WHERE repo_id = $1 ORDER BY name")
            .map_err(|error| error.to_string())?;
        let raw_nodes: Vec<(i64, String, String, String)> = node_stmt
            .query_map(params![repo_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let name_to_id: std::collections::HashMap<String, i64> =
            raw_nodes.iter().map(|(id, name, _, _)| (name.clone(), *id)).collect();

        let mut dep_stmt = self
            .conn
            .prepare("SELECT from_package_id, to_package_name, dev FROM repo_index_pkg_deps WHERE repo_id = $1")
            .map_err(|error| error.to_string())?;
        let raw_deps: Vec<(i64, String, i64)> = dep_stmt
            .query_map(params![repo_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;

        let edges: Vec<IndexGraphEdge> = raw_deps
            .into_iter()
            .filter_map(|(from_id, to_name, dev)| {
                let to_id = *name_to_id.get(&to_name)?;
                if to_id == from_id {
                    return None;
                } // drop self-loops
                Some(IndexGraphEdge {
                    source: from_id.to_string(),
                    target: to_id.to_string(),
                    dev: dev != 0,
                })
            })
            .collect();
        let nodes = raw_nodes
            .into_iter()
            .map(|(id, name, path, kind)| IndexGraphNode {
                id: id.to_string(),
                name,
                path,
                kind,
            })
            .collect();
        Ok(IndexGraph { nodes, edges })
    }

    /// Symbol-level call/reference graph: nodes are indexed functions/classes,
    /// edges are `repo_index_symbol_edges` (e.g. "calls"). Node id = symbol id.
    pub fn symbol_graph(&self, repo_id: i64) -> Result<IndexGraph, String> {
        let mut node_stmt = self
            .conn
            .prepare(
                "SELECT id, name, path, kind FROM repo_index_symbols
                 WHERE repo_id = $1 ORDER BY path, start_line",
            )
            .map_err(|error| error.to_string())?;
        let nodes: Vec<IndexGraphNode> = node_stmt
            .query_map(params![repo_id], |row| {
                Ok(IndexGraphNode {
                    id: row.get::<_, i64>(0)?.to_string(),
                    name: row.get(1)?,
                    path: row.get(2)?,
                    kind: row.get(3)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let mut edge_stmt = self
            .conn
            .prepare(
                "SELECT from_symbol_id, to_symbol_id FROM repo_index_symbol_edges
                 WHERE repo_id = $1",
            )
            .map_err(|error| error.to_string())?;
        let edges: Vec<IndexGraphEdge> = edge_stmt
            .query_map(params![repo_id], |row| {
                Ok(IndexGraphEdge {
                    source: row.get::<_, i64>(0)?.to_string(),
                    target: row.get::<_, i64>(1)?.to_string(),
                    dev: false,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(IndexGraph { nodes, edges })
    }

    /// File-level graph WITHIN one package (drill-down): nodes = the package's files
    /// (id/path = repo-relative path, kind = language), edges = resolved import edges
    /// whose BOTH ends are files of this package (internal structure only).
    pub fn package_file_graph(&self, repo_id: i64, package_id: i64) -> Result<IndexGraph, String> {
        let mut fstmt = self.conn.prepare(
      "SELECT path, COALESCE(language, '') FROM repo_index_files WHERE repo_id = $1 AND package_id = $2 ORDER BY path",
    ).map_err(|error| error.to_string())?;
        let files: Vec<(String, String)> = fstmt
            .query_map(params![repo_id, package_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let paths: std::collections::HashSet<String> = files.iter().map(|(path, _)| path.clone()).collect();

        let mut estmt = self.conn.prepare(
      "SELECT from_path, to_path FROM repo_index_edges WHERE repo_id = $1 AND resolved = 1 AND to_path IS NOT NULL",
    ).map_err(|error| error.to_string())?;
        let raw: Vec<(String, String)> = estmt
            .query_map(params![repo_id], |row| Ok((row.get(0)?, row.get::<_, String>(1)?)))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let edges: Vec<IndexGraphEdge> = raw
            .into_iter()
            .filter(|(from, to)| from != to && paths.contains(from) && paths.contains(to))
            .map(|(from, to)| IndexGraphEdge {
                source: from,
                target: to,
                dev: false,
            })
            .collect();
        let nodes = files
            .into_iter()
            .map(|(path, language)| {
                let name = path.rsplit('/').next().unwrap_or(&path).to_string();
                let kind = if language.is_empty() {
                    "file".to_string()
                } else {
                    language
                };
                IndexGraphNode {
                    id: path.clone(),
                    name,
                    path,
                    kind,
                }
            })
            .collect();
        Ok(IndexGraph { nodes, edges })
    }

    pub fn list_index_queue(&self) -> Result<Vec<(Repo, RepoIndexStatus)>, String> {
        let repos = self.monitored_repos()?;
        repos
            .into_iter()
            .map(|repo| self.repo_index_status(repo.id).map(|status| (repo, status)))
            .collect()
    }

    pub fn enqueue_repo_index(&self, repo_id: i64) -> Result<RepoIndexStatus, String> {
        let now = now_millis();
        self.conn.execute(
      "INSERT INTO repo_index_state (repo_id, status, stage, spec_version, file_count, package_count, edge_count, skipped_count, total_count, error_message, updated_at)
       VALUES ($1, 'queued', 'queued', $2, 0, 0, 0, 0, 0, NULL, $3)
       ON CONFLICT(repo_id) DO UPDATE SET
         status = CASE WHEN repo_index_state.status = 'indexing' THEN repo_index_state.status ELSE 'queued' END,
         stage = CASE WHEN repo_index_state.status = 'indexing' THEN repo_index_state.stage ELSE 'queued' END,
         spec_version = $2,
         error_message = NULL,
         updated_at = $3",
      params![repo_id, INDEX_SPEC_VERSION, now],
    ).map_err(|error| error.to_string())?;
        self.repo_index_status(repo_id)
    }

    /// Removes a review's head-commit index from `scratch`. Names the schema outright, so it is
    /// safe on any connection and can never reach a repository's own index.
    pub fn drop_snapshot_index(&self, run_id: i64) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(|error| error.to_string())?;
        for table in SNAPSHOT_INDEX_TABLES {
            tx.execute(
                &format!("DELETE FROM scratch.{table} WHERE repo_id = $1"),
                params![run_id],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.execute("DELETE FROM scratch.repos WHERE id = $1", params![run_id])
            .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())
    }

    /// Removes head-commit indexes whose review is no longer active: left behind when the server
    /// stopped mid-review.
    pub fn reap_snapshot_indexes(&self) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(|error| error.to_string())?;
        for table in SNAPSHOT_INDEX_TABLES {
            tx.execute(
                &format!(
                    "DELETE FROM scratch.{table} WHERE repo_id NOT IN
                     (SELECT id FROM review_runs WHERE status IN ('pending', 'running', 'posting', 'queued', 'cancelling'))"
                ),
                [],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.execute(
            "DELETE FROM scratch.repos WHERE id NOT IN
             (SELECT id FROM review_runs WHERE status IN ('pending', 'running', 'posting', 'queued', 'cancelling'))",
            [],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())
    }

    pub fn begin_repo_index(&self, repo_id: i64, base_ref: Option<&str>) -> Result<RepoIndexStatus, String> {
        let now = now_millis();
        self.conn.execute(
      "INSERT INTO repo_index_state (repo_id, status, stage, spec_version, base_ref, file_count, package_count, edge_count, skipped_count, total_count, started_at, error_message, updated_at, owner_instance)
       VALUES ($1, 'indexing', 'preparing', $2, $3, 0, 0, 0, 0, 0, $4, NULL, $4, $5)
       ON CONFLICT(repo_id) DO UPDATE SET
         status = 'indexing', stage = 'preparing', spec_version = excluded.spec_version,
         base_ref = excluded.base_ref, file_count = 0, package_count = 0, edge_count = 0,
         skipped_count = 0, total_count = 0, started_at = excluded.started_at, indexed_at = NULL,
         duration_ms = NULL, error_message = NULL, updated_at = excluded.updated_at, owner_instance = excluded.owner_instance",
      params![repo_id, INDEX_SPEC_VERSION, base_ref, now, self.db.instance_id()],
    ).map_err(|error| error.to_string())?;
        self.repo_index_status(repo_id)
    }

    pub fn update_repo_index_stage(
        &self,
        repo_id: i64,
        stage: &str,
        file_count: i64,
        total_count: i64,
        package_count: i64,
        edge_count: i64,
        skipped_count: i64,
    ) -> Result<(), String> {
        let now = now_millis();
        self.conn.execute(
      "UPDATE repo_index_state SET stage = $1, file_count = $2, total_count = $3, package_count = $4, edge_count = $5, skipped_count = $6, updated_at = $7
       WHERE repo_id = $8 AND status = 'indexing'",
      params![stage, file_count, total_count, package_count, edge_count, skipped_count, now, repo_id],
    ).map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn finish_repo_index(&self, repo_id: i64, write: &RepoIndexWrite) -> Result<RepoIndexStatus, String> {
        self.write_repo_index(repo_id, write)?;
        // Index is fresh again → clear any "index outdated" pending.
        self.resolve_repo_pending("index_stale", repo_id)?;
        self.repo_index_status(repo_id)
    }

    /// Stores a finished index without touching the repository's pending actions: also how a
    /// review stores the index of its head commit in `scratch`, which has no pending actions.
    pub fn write_repo_index(&self, repo_id: i64, write: &RepoIndexWrite) -> Result<RepoIndexStatus, String> {
        let now = now_millis();
        let tx = self.conn.unchecked_transaction().map_err(|error| error.to_string())?;
        tx.execute(
            "DELETE FROM repo_index_symbol_edges WHERE repo_id = $1",
            params![repo_id],
        )
        .map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM repo_index_symbols WHERE repo_id = $1", params![repo_id])
            .map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM repo_index_files WHERE repo_id = $1", params![repo_id])
            .map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM repo_index_packages WHERE repo_id = $1", params![repo_id])
            .map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM repo_index_pkg_deps WHERE repo_id = $1", params![repo_id])
            .map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM repo_index_edges WHERE repo_id = $1", params![repo_id])
            .map_err(|error| error.to_string())?;

        // One statement per table (arrays through UNNEST): a large repository has tens of
        // thousands of rows, and a round trip each would take minutes.
        let package_ids: HashMap<String, i64> = tx
            .prepare(
                "INSERT INTO repo_index_packages (repo_id, package_path, name, kind, manager, source_roots_json, test_globs_json, indexed_at)
                 SELECT $1, package_path, name, kind, manager, source_roots_json, test_globs_json, $2
                 FROM UNNEST($3::TEXT[], $4::TEXT[], $5::TEXT[], $6::TEXT[], $7::TEXT[], $8::TEXT[])
                   AS p(package_path, name, kind, manager, source_roots_json, test_globs_json)
                 RETURNING package_path, id",
            )
            .and_then(|mut stmt| {
                let packages = &write.packages;
                stmt.query_map(
                    params![
                        repo_id,
                        now,
                        packages.iter().map(|p| p.package_path.as_str()).collect::<Vec<_>>(),
                        packages.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
                        packages.iter().map(|p| p.kind.as_str()).collect::<Vec<_>>(),
                        packages.iter().map(|p| p.manager.as_str()).collect::<Vec<_>>(),
                        packages.iter().map(|p| p.source_roots_json.as_str()).collect::<Vec<_>>(),
                        packages.iter().map(|p| p.test_globs_json.as_str()).collect::<Vec<_>>()
                    ],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?
                .collect()
            })
            .map_err(|error| error.to_string())?;
        let package_id = |path: &Option<String>| path.as_ref().and_then(|path| package_ids.get(path)).copied();
        let files = &write.files;
        tx.execute(
            "INSERT INTO repo_index_files (repo_id, path, language, size, blob_sha, package_id, is_binary, is_generated, indexed_at)
             SELECT $1, path, language, size, blob_sha, package_id, is_binary, is_generated, $2
             FROM UNNEST($3::TEXT[], $4::TEXT[], $5::BIGINT[], $6::TEXT[], $7::BIGINT[], $8::BIGINT[], $9::BIGINT[])
               AS f(path, language, size, blob_sha, package_id, is_binary, is_generated)",
            params![
                repo_id,
                now,
                files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
                files.iter().map(|f| f.language.as_deref()).collect::<Vec<_>>(),
                files.iter().map(|f| f.size).collect::<Vec<_>>(),
                files.iter().map(|f| f.blob_sha.as_str()).collect::<Vec<_>>(),
                files.iter().map(|f| package_id(&f.package_path)).collect::<Vec<_>>(),
                files.iter().map(|f| i64::from(f.is_binary)).collect::<Vec<_>>(),
                files.iter().map(|f| i64::from(f.is_generated)).collect::<Vec<_>>()
            ],
        )
        .map_err(|error| error.to_string())?;
        // Symbol edges point at symbols by position, so the ids are drawn first and inserted
        // explicitly. The sequence resolves through the connection's search_path (public or
        // scratch).
        let symbols = &write.symbols;
        let symbol_ids: Vec<i64> = tx
            .prepare("SELECT nextval(pg_get_serial_sequence('repo_index_symbols', 'id')) FROM generate_series(1, $1::BIGINT)")
            .and_then(|mut stmt| {
                stmt.query_map(params![symbols.len() as i64], |row| row.get(0))?
                    .collect()
            })
            .map_err(|error| error.to_string())?;
        tx.execute(
            "INSERT INTO repo_index_symbols (id, repo_id, name, kind, language, path, package_id, start_line, end_line, signature, is_exported, indexed_at)
             SELECT id, $1, name, kind, language, path, package_id, start_line, end_line, signature, is_exported, $2
             FROM UNNEST($3::BIGINT[], $4::TEXT[], $5::TEXT[], $6::TEXT[], $7::TEXT[], $8::BIGINT[], $9::BIGINT[], $10::BIGINT[], $11::TEXT[], $12::BIGINT[])
               AS s(id, name, kind, language, path, package_id, start_line, end_line, signature, is_exported)",
            params![
                repo_id,
                now,
                symbol_ids,
                symbols.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
                symbols.iter().map(|s| s.kind.as_str()).collect::<Vec<_>>(),
                symbols.iter().map(|s| s.language.as_str()).collect::<Vec<_>>(),
                symbols.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(),
                symbols.iter().map(|s| package_id(&s.package_path)).collect::<Vec<_>>(),
                symbols.iter().map(|s| s.start_line).collect::<Vec<_>>(),
                symbols.iter().map(|s| s.end_line).collect::<Vec<_>>(),
                symbols.iter().map(|s| s.signature.as_deref()).collect::<Vec<_>>(),
                symbols.iter().map(|s| i64::from(s.is_exported)).collect::<Vec<_>>()
            ],
        )
        .map_err(|error| error.to_string())?;
        let symbol_edges: Vec<(i64, i64, &str)> = write
            .symbol_edges
            .iter()
            .filter_map(|edge| {
                let from = symbol_ids.get(edge.from_symbol_index)?;
                let to = symbol_ids.get(edge.to_symbol_index)?;
                Some((*from, *to, edge.kind.as_str()))
            })
            .collect();
        tx.execute(
            "INSERT INTO repo_index_symbol_edges (repo_id, from_symbol_id, to_symbol_id, kind)
             SELECT $1, from_symbol_id, to_symbol_id, kind
             FROM UNNEST($2::BIGINT[], $3::BIGINT[], $4::TEXT[]) AS e(from_symbol_id, to_symbol_id, kind)",
            params![
                repo_id,
                symbol_edges.iter().map(|edge| edge.0).collect::<Vec<_>>(),
                symbol_edges.iter().map(|edge| edge.1).collect::<Vec<_>>(),
                symbol_edges.iter().map(|edge| edge.2).collect::<Vec<_>>()
            ],
        )
        .map_err(|error| error.to_string())?;
        let deps: Vec<(i64, &str, i64)> = write
            .deps
            .iter()
            .filter_map(|dep| {
                let from = package_ids.get(&dep.from_package_path)?;
                Some((*from, dep.to_package_name.as_str(), i64::from(dep.dev)))
            })
            .collect();
        tx.execute(
            "INSERT INTO repo_index_pkg_deps (repo_id, from_package_id, to_package_name, dev)
             SELECT $1, from_package_id, to_package_name, dev
             FROM UNNEST($2::BIGINT[], $3::TEXT[], $4::BIGINT[]) AS d(from_package_id, to_package_name, dev)",
            params![
                repo_id,
                deps.iter().map(|dep| dep.0).collect::<Vec<_>>(),
                deps.iter().map(|dep| dep.1).collect::<Vec<_>>(),
                deps.iter().map(|dep| dep.2).collect::<Vec<_>>()
            ],
        )
        .map_err(|error| error.to_string())?;
        let edges = &write.edges;
        tx.execute(
            "INSERT INTO repo_index_edges (repo_id, from_path, to_ref, to_path, kind, resolved)
             SELECT $1, from_path, to_ref, to_path, kind, resolved
             FROM UNNEST($2::TEXT[], $3::TEXT[], $4::TEXT[], $5::TEXT[], $6::BIGINT[]) AS e(from_path, to_ref, to_path, kind, resolved)",
            params![
                repo_id,
                edges.iter().map(|e| e.from_path.as_str()).collect::<Vec<_>>(),
                edges.iter().map(|e| e.to_ref.as_str()).collect::<Vec<_>>(),
                edges.iter().map(|e| e.to_path.as_deref()).collect::<Vec<_>>(),
                edges.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
                edges.iter().map(|e| i64::from(e.resolved)).collect::<Vec<_>>()
            ],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
      "UPDATE repo_index_state SET status = 'ready', stage = 'ready', spec_version = $1, base_ref = $2, base_sha = $3, remote_base_sha = $3,
         file_count = $4, package_count = $5, edge_count = $6, skipped_count = $7, total_count = $8,
         indexed_at = $9, duration_ms = $10, error_message = NULL, updated_at = $9
       WHERE repo_id = $11",
      params![
        INDEX_SPEC_VERSION, write.base_ref, write.base_sha,
        write.files.len() as i64, write.packages.len() as i64, write.edges.len() as i64,
        write.skipped_count, write.total_count, now, write.duration_ms, repo_id
      ],
    ).map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
        self.repo_index_status(repo_id)
    }

    pub fn cancel_repo_index(&self, repo_id: i64) -> Result<RepoIndexStatus, String> {
        let now = now_millis();
        self.conn.execute(
      "UPDATE repo_index_state SET status = 'cancelled', stage = 'cancelled', error_message = 'Indexing cancelled by user.',
         duration_ms = CASE WHEN started_at IS NULL THEN NULL ELSE $1 - started_at END, updated_at = $1
       WHERE repo_id = $2 AND status = 'indexing'",
      params![now, repo_id],
    ).map_err(|error| error.to_string())?;
        self.repo_index_status(repo_id)
    }

    pub fn fail_repo_index(&self, repo_id: i64, error_message: &str) -> Result<RepoIndexStatus, String> {
        let now = now_millis();
        self.conn.execute(
      "INSERT INTO repo_index_state (repo_id, status, stage, error_message, started_at, duration_ms, updated_at)
       VALUES ($1, 'failed', 'failed', $2, $3, 0, $3)
       ON CONFLICT(repo_id) DO UPDATE SET status = 'failed', stage = 'failed', error_message = excluded.error_message,
         duration_ms = CASE WHEN repo_index_state.started_at IS NULL THEN NULL ELSE excluded.updated_at - repo_index_state.started_at END,
         updated_at = excluded.updated_at",
      params![repo_id, error_message, now],
    ).map_err(|error| error.to_string())?;
        self.repo_index_status(repo_id)
    }

    /// The repository has no commits yet: nothing to index, and nothing wrong.
    pub fn mark_repo_index_empty(&self, repo_id: i64) -> Result<RepoIndexStatus, String> {
        let now = now_millis();
        self.conn
            .execute(
                "INSERT INTO repo_index_state (repo_id, status, stage, error_message, updated_at)
       VALUES ($1, 'empty', 'empty', NULL, $2)
       ON CONFLICT(repo_id) DO UPDATE SET status = 'empty', stage = 'empty', error_message = NULL,
         duration_ms = NULL, updated_at = excluded.updated_at",
                params![repo_id, now],
            )
            .map_err(|error| error.to_string())?;
        self.repo_index_status(repo_id)
    }

    pub fn mark_repo_index_stale(&self, repo_id: i64, remote_base_sha: &str) -> Result<RepoIndexStatus, String> {
        let now = now_millis();
        self.conn
            .execute(
                "UPDATE repo_index_state SET status = 'stale', stage = 'stale', remote_base_sha = $1,
         error_message = 'Default branch changed. Reindex required.', updated_at = $2 WHERE repo_id = $3",
                params![remote_base_sha, now, repo_id],
            )
            .map_err(|error| error.to_string())?;
        // Raise a repo-level pending so it's visible/alerted until reindexed.
        self.upsert_repo_pending_action(
            "index_stale",
            repo_id,
            "Index outdated",
            "Default branch changed — reindex to keep reviews accurate.",
        )?;
        self.repo_index_status(repo_id)
    }
}

/// The `scratch` tables holding a review's head-commit index, children before parents.
const SNAPSHOT_INDEX_TABLES: [&str; 7] = [
    "repo_index_symbol_edges",
    "repo_index_symbols",
    "repo_index_edges",
    "repo_index_pkg_deps",
    "repo_index_files",
    "repo_index_packages",
    "repo_index_state",
];
