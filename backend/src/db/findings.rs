use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use super::{now_millis, Database};
use crate::review::findings::{self, FindingAudit};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingAuditView {
    pub run_id: i64,
    pub pr_id: i64,
    pub revision: i64,
    pub requires_confirmation: bool,
    pub selection_saved: bool,
    pub editable: bool,
    pub audit: Option<FindingAudit>,
}

impl FindingAuditView {
    pub(super) fn can_auto_post(&self) -> bool {
        self.editable
            && self.selection_saved
            && self.audit.as_ref().is_some_and(|audit| {
                audit.candidates.iter().any(|candidate| candidate.selected)
                    && audit.candidates.iter().all(|candidate| {
                        candidate.selected == (candidate.default_selected && candidate.blocked_reason.is_none())
                    })
            })
    }
}

impl Database {
    pub(crate) fn review_evidence_warnings(
        &self,
        run_id: i64,
    ) -> Result<Vec<crate::review::warnings::EvidenceWarning>, String> {
        Ok(self
            .load_finding_audit(run_id)?
            .and_then(|audit| audit.evidence_warnings)
            .unwrap_or_default())
    }

    pub fn review_requires_finding_selection(&self, run_id: i64) -> Result<bool, String> {
        self.conn
            .query_row(
                "SELECT human_selection_required FROM review_runs WHERE id = ?1",
                params![run_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| error.to_string())
    }

    pub fn load_finding_audit(&self, run_id: i64) -> Result<Option<FindingAudit>, String> {
        let raw: Option<String> = self
            .conn
            .query_row(
                "SELECT audit_json FROM review_finding_audits WHERE run_id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(|error| error.to_string()))
            .transpose()
    }

    pub fn clear_finding_evidence(&self, run_id: i64) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE review_finding_audits SET audit_json = json_remove(audit_json, '$.diff') WHERE run_id = ?1",
                params![run_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn store_finding_audit(&self, run_id: i64, audit: &FindingAudit) -> Result<(), String> {
        let json = serde_json::to_string(audit).map_err(|error| error.to_string())?;
        self.conn.execute(
            "INSERT INTO review_finding_audits (run_id, audit_json, revision, selection_saved, updated_at)
             SELECT id, ?2, 0, 0, ?3 FROM review_runs WHERE id = ?1 AND status = 'running'
             ON CONFLICT(run_id) DO UPDATE SET audit_json = excluded.audit_json, revision = review_finding_audits.revision + 1, selection_saved = 0, updated_at = excluded.updated_at
             WHERE EXISTS (SELECT 1 FROM review_runs WHERE id = ?1 AND status = 'running')",
            params![run_id, json, now_millis()],
        ).map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn finding_audit(&self, pr_id: i64, run_id: i64) -> Result<FindingAuditView, String> {
        let run = self
            .review_run_by_id(run_id)?
            .filter(|run| run.pr_id == pr_id)
            .ok_or("Review run does not belong to this PR.")?;
        let requires_confirmation = self.review_requires_finding_selection(run_id)?;
        let stored: Option<(String, i64, bool)> = self
            .conn
            .query_row(
                "SELECT audit_json, revision, selection_saved FROM review_finding_audits WHERE run_id = ?1",
                params![run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let (mut audit, revision, selection_saved) = match stored {
            Some((json, revision, saved)) => (
                Some(serde_json::from_str::<FindingAudit>(&json).map_err(|error| error.to_string())?),
                revision,
                saved,
            ),
            None => (None, 0, false),
        };
        let current: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM pull_requests pr WHERE pr.id = ?1 AND pr.head_sha = ?2 AND LOWER(pr.state) = 'open'
             AND ?3 = (SELECT id FROM review_runs WHERE pr_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 1))",
            params![pr_id, run.head_sha, run_id], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        let editable = requires_confirmation
            && current
            && run.status == "preview"
            && audit
                .as_ref()
                .is_some_and(|audit| audit.complete && audit.head_sha == run.head_sha);
        if let Some(audit) = audit.as_mut() {
            audit.diff = None;
            for candidate in &mut audit.candidates {
                if candidate.kind.is_empty() {
                    candidate.kind = findings::point_kind(&candidate.body).into();
                }
            }
        }
        Ok(FindingAuditView {
            run_id,
            pr_id,
            revision,
            requires_confirmation,
            selection_saved,
            editable,
            audit,
        })
    }

    pub fn save_finding_selection(
        &self,
        pr_id: i64,
        run_id: i64,
        revision: i64,
        selected_ids: &[String],
    ) -> Result<FindingAuditView, String> {
        let tx = self.conn.unchecked_transaction().map_err(|error| error.to_string())?;
        let view = self.finding_audit(pr_id, run_id)?;
        if !view.editable {
            return Err(
                "Finding selection is read-only for this run. Only the current, unpublished review can be changed."
                    .to_string(),
            );
        }
        if view.revision != revision {
            return Err("Finding selection changed. Reload it before saving.".to_string());
        }
        let mut audit = view.audit.ok_or("Finding audit is unavailable.")?;
        findings::select(&mut audit, selected_ids)?;
        let (body, comments) = findings::selection_payload(&audit);
        let json = serde_json::to_string(&audit).map_err(|error| error.to_string())?;
        let audit_changed = self.conn.execute("UPDATE review_finding_audits SET audit_json = ?1, revision = revision + 1, selection_saved = 1, updated_at = ?2 WHERE run_id = ?3 AND revision = ?4", params![json, now_millis(), run_id, revision]).map_err(|error| error.to_string())?;
        if audit_changed != 1 {
            return Err("Finding selection changed. Reload before saving.".to_string());
        }
        let changed = self.conn.execute("UPDATE review_runs SET review_body = ?1, comments_json = ?2, updated_at = ?3 WHERE id = ?4 AND status = 'preview'", params![body, comments, now_millis(), run_id]).map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("Review is no longer editable.".to_string());
        }
        let saved = self.finding_audit(pr_id, run_id)?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(saved)
    }

    pub fn auto_confirm_generated_finding_selection(&self, pr_id: i64, run_id: i64) -> Result<Option<i64>, String> {
        let tx = self.conn.unchecked_transaction().map_err(|error| error.to_string())?;
        let view = self.finding_audit(pr_id, run_id)?;
        if !view.editable {
            return Err("Automatic finding confirmation is unavailable for this review.".to_string());
        }
        if view.selection_saved {
            let revision = view.can_auto_post().then_some(view.revision);
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(revision);
        }
        let mut audit = view.audit.ok_or("Finding audit is unavailable.")?;
        let selected_ids = audit
            .candidates
            .iter()
            .filter(|candidate| candidate.default_selected && candidate.blocked_reason.is_none())
            .map(|candidate| candidate.id.clone())
            .collect::<Vec<_>>();
        if selected_ids.is_empty() {
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(None);
        }
        findings::select(&mut audit, &selected_ids)?;
        let (body, comments) = findings::selection_payload(&audit);
        let json = serde_json::to_string(&audit).map_err(|error| error.to_string())?;
        let changed = self.conn.execute(
            "UPDATE review_finding_audits SET audit_json = ?1, revision = revision + 1, selection_saved = 1, updated_at = ?2 WHERE run_id = ?3 AND revision = ?4",
            params![json, now_millis(), run_id, view.revision],
        ).map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("Finding selection changed before automatic confirmation.".to_string());
        }
        let changed = self.conn.execute(
            "UPDATE review_runs SET review_body = ?1, comments_json = ?2, updated_at = ?3 WHERE id = ?4 AND status = 'preview'",
            params![body, comments, now_millis(), run_id],
        ).map_err(|error| error.to_string())?;
        if changed != 1 {
            return Err("Review is no longer editable for automatic confirmation.".to_string());
        }
        tx.commit().map_err(|error| error.to_string())?;
        Ok(Some(view.revision + 1))
    }

    pub fn guard_finding_decision<T>(
        &self,
        pr_id: i64,
        run_id: i64,
        revision: Option<i64>,
        operation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let tx = self.conn.unchecked_transaction().map_err(|error| error.to_string())?;
        let view = self.finding_audit(pr_id, run_id)?;
        let selection_required = view.requires_confirmation
            && match view.audit.as_ref() {
                Some(audit) => !audit.complete || !audit.candidates.is_empty(),
                None => true,
            };
        if view.requires_confirmation && !view.editable {
            return Err(
                "Confirm and save the current finding selection before deciding this review. Reload if it changed."
                    .to_string(),
            );
        }
        if selection_required && (!view.selection_saved || revision != Some(view.revision)) {
            return Err(
                "Confirm and save the current finding selection before deciding this review. Reload if it changed."
                    .to_string(),
            );
        }
        let run = self.review_run_by_id(run_id)?.ok_or("Review run not found.")?;
        if run.status != "preview" {
            return Err("Review is no longer awaiting a decision.".to_string());
        }
        let result = operation()?;
        tx.commit().map_err(|error| error.to_string())?;
        Ok(result)
    }
}
