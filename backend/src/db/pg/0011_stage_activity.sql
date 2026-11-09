-- What each stage's executions did, read from their transcripts: model round trips, tool calls,
-- reasoning tokens and time spent on the provider. Zero where the CLI does not report the value
-- and for stages recorded before this migration.
ALTER TABLE review_run_stages
  ADD COLUMN turns BIGINT NOT NULL DEFAULT 0,
  ADD COLUMN tool_calls BIGINT NOT NULL DEFAULT 0,
  ADD COLUMN reasoning_tokens BIGINT NOT NULL DEFAULT 0,
  ADD COLUMN model_ms BIGINT NOT NULL DEFAULT 0;
