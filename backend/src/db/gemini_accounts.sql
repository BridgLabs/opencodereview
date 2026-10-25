-- Gemini (Google account) sign-ins join Claude and Codex as review accounts.
CREATE TABLE ai_accounts_next (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  provider TEXT NOT NULL CHECK (provider IN ('claude', 'codex', 'gemini')),
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
INSERT INTO ai_accounts_next (id, provider, label, priority, enabled, status, limited_until, last_error, last_used_at, created_at, updated_at)
  SELECT id, provider, label, priority, enabled, status, limited_until, last_error, last_used_at, created_at, updated_at FROM ai_accounts;
DROP TABLE ai_accounts;
ALTER TABLE ai_accounts_next RENAME TO ai_accounts;
