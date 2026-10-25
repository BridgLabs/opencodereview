# Review rules

OpenCodeReview reviews its own pull requests: the maintainer's install monitors this repository, and every review reads this file from the pull request's base commit. [AGENTS.md](AGENTS.md) holds the project rules; this file says which breaks of those rules to report and which findings to leave out. CI already runs Biome, knip, `tsc`, rustfmt, clippy with `-D warnings`, cargo-deny, the file-size guard and the release-note check, so do not report what those tools catch.

## Sandbox and credentials

Report, as the highest severity, a change that:

- mounts `ocr-data`, a repository, the SQLite database or a writable credential path into a runner, or adds a path from a runner to the server;
- adds a tool to `RUNNER_TOOLS`, removes one from the deny list, or calls the AI CLI without the `ocr-sandbox tool-guard` hook;
- adds a host to the egress allowlist in `backend/src/sandbox/egress.rs` that is not an AI provider's API;
- lets credential bytes, tokens or `auth.json` contents reach a log, an HTTP response, an IPC result or the browser;
- runs inference inside `server`, or reruns inference automatically after a startup failure;
- installs or updates an AI CLI at runtime instead of pinning it, with checksums, in `docker/Dockerfile`.

## Review pipeline

- Scope must use the live base, head and merge-base. Report a fallback to stale refs or to the default branch after a failed fetch.
- A report that fails validation must fail the stage. Report code that relabels it clean, drops invalid findings silently, or allows more than one repair execution per stage.
- Publication must stay idempotent and bound to the validated selection. Report anything that approves a pull request or posts without the `<!-- ocr-... -->` markers.
- Report a change to the default skill (`backend/src/assets/review-skill.md`) that asks the AI to reach the network, run Git, call GitHub or post comments.

## Database

- Report an edit to a migration that has already shipped; changes go in a new migration at the end of `backend/src/db/migrations.rs`.
- A new migration needs a test that opens a database at the previous version and upgrades it. A schema change without one is a finding.
- Report a deletion of review results, audits, Git data or coverage in archive retention.

## Background work

- Report long-running work started synchronously inside a request handler.
- Report a background job, timer, subprocess or file handle that is not cleaned up on cancel, failure and shutdown.

## Tests

Report a finding when the diff:

- adds a test whose assertions all still hold if the feature under test is deleted;
- asserts only that a mock was called, when the observable result could be asserted instead;
- asserts on a value the same test just built, so it restates the setup;
- makes a real AI call outside the explicitly authorized diagnostics in `docker/`.

Do not report a missing test unless you can name the regression it would catch, and that regression can lose data, break the sandbox, leak a secret or publish a wrong review.

## UI

- Report raw hex colors in components, new stylesheet rules for app components in `src/styles/components.css`, and loading states that do not use `BrandLoading`.
- Report an action area with more than two visible buttons, or an unreachable API shown as "Failed to fetch" instead of Docker guidance.

## Leave out

- Style and formatting, import order and naming preferences that match the surrounding code.
- Suggestions to add model pickers, providers, settings or currency estimates: those need a product decision in an issue first.
- Findings about generated files, `pnpm-lock.yaml`, `Cargo.lock` or the license texts.
