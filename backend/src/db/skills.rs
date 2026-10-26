use super::*;

impl Database {
    /// Active version id (0 = built-in baseline). Stored as a settings pointer so
    /// rollback never overwrites a snapshot.
    pub fn active_skill_version_id(&self) -> i64 {
        self.setting("active_skill_version")
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    /// The skill text reviews should use: the active version's content,
    /// or the built-in embedded skill when the baseline (v0) is active or missing.
    pub fn active_skill_content(&self) -> String {
        let id = self.active_skill_version_id();
        if id == GENERIC_SKILL_ID {
            return crate::review::REVIEW_SKILL_GENERIC.to_string();
        }
        if id <= 0 {
            return crate::review::REVIEW_SKILL.to_string();
        }
        self.conn
            .query_row(
                "SELECT content FROM skill_versions WHERE id = $1 AND deleted_at IS NULL",
                params![id],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_else(|_| crate::review::REVIEW_SKILL.to_string())
    }

    /// Content of a specific version; the generic built-in id → the generic
    /// skill; id 0 (or unknown) → the default built-in skill.
    pub fn skill_version_content(&self, id: i64) -> String {
        if id == GENERIC_SKILL_ID {
            return crate::review::REVIEW_SKILL_GENERIC.to_string();
        }
        if id <= 0 {
            return crate::review::REVIEW_SKILL.to_string();
        }
        self.conn
            .query_row("SELECT content FROM skill_versions WHERE id = $1", params![id], |row| {
                row.get::<_, String>(0)
            })
            .unwrap_or_else(|_| crate::review::REVIEW_SKILL.to_string())
    }

    /// Baseline (v0) + every stored snapshot, newest first, with the active flag.
    pub fn list_skill_versions(&self) -> Result<Vec<SkillVersion>, String> {
        let active = self.active_skill_version_id();
        let mut out = Vec::new();
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, version, note, length(content), created_at
       FROM skill_versions WHERE deleted_at IS NULL ORDER BY version DESC",
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SkillVersion {
                    id: row.get(0)?,
                    version: row.get(1)?,
                    note: row.get(2)?,
                    char_count: row.get(3)?,
                    active: row.get::<_, i64>(0)? == active,
                    created_at: row.get(4)?,
                })
            })
            .map_err(|error| error.to_string())?;
        for v in rows {
            out.push(v.map_err(|error| error.to_string())?);
        }
        // Always include the two built-ins: the default (v0) and the generic
        // alternative. The default is active for any non-generic baseline value.
        out.push(SkillVersion {
            id: 0,
            version: 0,
            note: Some("Built-in review skill (default)".to_string()),
            char_count: crate::review::REVIEW_SKILL.chars().count() as i64,
            active: active <= 0 && active != GENERIC_SKILL_ID,
            created_at: 0,
        });
        out.push(SkillVersion {
            id: GENERIC_SKILL_ID,
            version: 0,
            note: Some("Built-in generic review skill (for other codebases)".to_string()),
            char_count: crate::review::REVIEW_SKILL_GENERIC.chars().count() as i64,
            active: active == GENERIC_SKILL_ID,
            created_at: 0,
        });
        Ok(out)
    }

    /// Create an immutable snapshot and make it active. Returns the new version.
    pub fn create_skill_version(&self, content: &str, note: Option<&str>) -> Result<SkillVersion, String> {
        let now = now_millis();
        let next_version: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) + 1 FROM skill_versions WHERE deleted_at IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap_or(1);
        let parent_id = {
            let id = self.active_skill_version_id();
            (id > 0).then_some(id)
        };
        let id: i64 = self
            .conn
            .query_row(
                "INSERT INTO skill_versions (version, content, note, parent_id, created_at)
       VALUES ($1, $2, $3, $4, $5) RETURNING id",
                params![next_version, content, note, parent_id, now],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        self.save_setting_value("active_skill_version", &id.to_string())?;
        Ok(SkillVersion {
            id,
            version: next_version,
            note: note.map(str::to_string),
            char_count: content.chars().count() as i64,
            active: true,
            created_at: now,
        })
    }

    pub fn delete_skill_version(&self, id: i64) -> Result<Vec<SkillVersion>, String> {
        if id <= 0 {
            return Err("baseline skill cannot be deleted".to_string());
        }
        let active = self.active_skill_version_id() == id;
        let deleted = self
            .conn
            .execute(
                "UPDATE skill_versions SET deleted_at = $2 WHERE id = $1 AND deleted_at IS NULL",
                params![id, now_millis()],
            )
            .map_err(|error| error.to_string())?;
        if deleted == 0 {
            return Err("skill version not found".to_string());
        }
        if active {
            let fallback = self
                .conn
                .query_row(
                    "SELECT id FROM skill_versions WHERE deleted_at IS NULL ORDER BY version DESC LIMIT 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0);
            self.save_setting_value("active_skill_version", &fallback.to_string())?;
        }
        self.list_skill_versions()
    }

    /// Switch the active version (rollback). id 0 = built-in baseline.
    pub fn set_active_skill_version(&self, id: i64) -> Result<(), String> {
        if id > 0 {
            let exists: bool = self
                .conn
                .query_row(
                    "SELECT 1 FROM skill_versions WHERE id = $1 AND deleted_at IS NULL",
                    params![id],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            if !exists {
                return Err("skill version not found".to_string());
            }
        }
        // Preserve the generic built-in sentinel; clamp any other negative to the
        // default baseline (0).
        let normalized = if id == GENERIC_SKILL_ID {
            GENERIC_SKILL_ID
        } else {
            id.max(0)
        };
        self.save_setting_value("active_skill_version", &normalized.to_string())
    }
}
