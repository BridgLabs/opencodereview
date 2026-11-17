-- Cursor accounts: reviews may now run on a Cursor subscription or API key.
ALTER TABLE ai_accounts DROP CONSTRAINT ai_accounts_provider_check;
ALTER TABLE ai_accounts ADD CONSTRAINT ai_accounts_provider_check
  CHECK (provider IN ('claude', 'codex', 'gemini', 'opencode', 'cursor'));
