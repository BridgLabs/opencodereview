# AGENTS.md

Guide for contributors and AI coding agents working on OpenCodeReview. Read it before changing code. User-facing documentation lives in [README.md](README.md) and [docs/](docs).

## Repository map

```text
src/, src/platform/                      React frontend: onboarding, reviews, indexing, skills, settings
backend/src/providers/                   AI CLIs: claude.rs (fixed model and effort, tool policy), codex.rs, gemini.rs, opencode.rs
backend/src/accounts/                    AI account credentials, plan usage, sign-in projection into runners
backend/src/sandbox/                     ocr-sandbox: runner, egress proxy, read commands, tool guard
backend/src/review/                      Review orchestration, scope, coverage, evidence, findings, validation
backend/src/review_archive/              Frozen review archives, retention and purge
backend/src/github/, indexer/            GitHub calls through `gh`; repository indexer
backend/src/server/                      HTTP API, background jobs, publication, sign-in flows
backend/src/db/                          SQLite schema, migrations and queries
backend/src/assets/                      Built-in review skills
docker-compose.yml, docker/              Docker runtime, Dockerfiles, smoke and regression scripts
.agents/skills/                          Agent workflows: git, comments
```

## Agent commands

| Command | Workflow |
| --- | --- |
| `/git <intent>` | [.agents/skills/git/SKILL.md](.agents/skills/git/SKILL.md): commits, pushes and pull requests. |
| `/comments` | [.agents/skills/comments/SKILL.md](.agents/skills/comments/SKILL.md): resolve review feedback on the current branch's pull request. |

## Ground rules

- **pnpm only.** Do not add `package-lock.json`, `yarn.lock` or `bun.lock`.
- **Docker is the runtime.** OpenCodeReview runs only through `docker compose`. Do not add native desktop wrappers or non-Docker runtime configuration.
- **File size limits.** Frontend source files stay at or under 800 lines and Rust files at or under 2,500 lines (`pnpm check:file-size`). Split before you hit the limit.
- **Conventional Commits** for every commit and pull request title.
- **Pull requests only.** `main` is protected; every change lands through a squash-merged pull request reviewed by a code owner.
- **No AI attribution.** Do not add `Generated with` footers, AI `Co-Authored-By` trailers or agent signatures to commits, PR titles or PR bodies. OpenCodeReview's own finding markers in product code are not attribution and stay.
- **No secrets** in code, logs, docs, fixtures or commits. Never commit `.env` files or databases.

## Product rules

- OpenCodeReview is reviews, indexing and skills. Do not add unrelated product areas without a discussion in an issue first.
- Claude reviews run `claude --print` with one fixed model and effort (`MODEL` and `EFFORT` in `backend/src/providers/claude.rs`). Codex reviews run `codex exec` with medium reasoning effort and the CLI's default model. Gemini reviews run the Gemini CLI headless with its default model and web tools denied. OpenCode reviews run `opencode run` on a free model, or on an OpenCode Go model with the account's Go key. Do not add model pickers or other providers without a product discussion.
- All AI CLIs are baked into the image at pinned, checksum-verified versions. Do not add runtime CLI installs or auto-updates.
- Accounts are tried in priority order across providers (`pick_ai_account`); a usage limit rests the account and the same stage reruns on the next one.
- Usage is reported in tokens only. Do not add currency or cost estimates.
- GitHub access goes through the authenticated `gh` CLI. Do not add OpenCodeReview-managed GitHub tokens.
- Configuration lives in Settings (Accounts, Repository access, PR selection, Integrations). Review execution and publishing are fixed defaults, not settings.
- The index is file metadata, a package and import graph and lightweight symbols. Symbol extraction is not an AST parser; do not describe it as one.
- Never check out or switch a user's branch for PR context. Use fetch, diff and show against refs.
- Long-running work never runs synchronously in a request. Use background jobs with visible state and actionable errors, and clean up timers, subprocesses and file handles.
- Posted findings keep the OpenCodeReview `<!-- ocr-... -->` markers and the full finding and question templates.
- The built-in review skill is `backend/src/assets/review-skill.md`. Improve it there; local database skill versions are not shipped.

## Sandbox invariants

- Runners get only their IPC socket and the read-only sign-in projection. Never mount `ocr-data`, repositories or SQLite into a runner, and never expose credential bytes to the browser or logs.
- All Claude CLI arguments live in `backend/src/providers/claude.rs`. Runners expose `Read`, `Grep`, `Glob`, `Bash` and `Write`; deny `Agent`, `Task`, `WebFetch`, `WebSearch` and `mcp__*`; and run the `ocr-sandbox tool-guard` `PreToolUse` hook on every call.
- Egress admits only the AI provider hosts in `backend/src/sandbox/egress.rs`.
- Backend and runner images share an executor fingerprint; deploy them together. Never add an in-process fallback that runs the AI inside `server`.
- Never automatically re-run inference after a startup failure. A successful process exit still requires report validation.
- Paid AI calls (real review diagnostics) always require explicit authorization from a maintainer.

## Review invariants

- Scope uses the live immediate base, head and merge-base. A failed fetch fails preparation; never fall back to stale refs or the default branch.
- Evidence is frozen and validated for every stage before any inference. Findings must quote the snapshot exactly and anchor to changed lines; invalid reports fail, they are never relabelled clean.
- Output repair allows at most one extra execution per stage.
- Publication is idempotent and bound to the exact validated selection. Nothing approves automatically.
- Archive retention deletes only frozen code of merged pull requests; never delete review results, audits, Git data or coverage.

## Runtime

```bash
docker compose up --build
```

Open `http://localhost:1421`. Services: `web` (nginx on 1421, proxying `/api`), `server` (API on `127.0.0.1:8787`), `review-runner` (20 replicas by default, `OCR_REVIEW_RUNNERS`) and `review-egress`. State lives in the `ocr-data` volume. `GH_TOKEN` is only a development fallback.

## Development commands

```bash
pnpm install
pnpm debug:web                     # Vite on 1421 against the Docker backend
pnpm lint:fix                      # Biome: lint, imports, format; CI runs pnpm lint
pnpm knip                          # unused files, exports and dependencies
pnpm typecheck
pnpm test
pnpm build
pnpm check:file-size
cargo fmt --all                    # rustfmt; CI runs cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked    # or: cargo test -p opencodereview-server <filter> --lib --locked
cargo deny check                   # when Cargo.lock changes: advisories, licenses, sources
pnpm changelog:check               # release notes in changelog.d/
python3 -B docker/test_diagnostics.py
python3 -B docker/test-web-proxy.py
docker compose config --quiet
```

After building images, `python3 -u docker/runner-smoke.py` validates the sandbox with fake inference (no credentials, no paid calls). If SQLite-backed Rust tests fail with `database is locked`, rerun with `-- --test-threads=1`; do not weaken production locking. Zero matching tests is not a pass.

## Database

The schema starts from `backend/src/db/schema.sql` plus the migrations in `backend/src/db/migrations.rs`. Append new migrations at the end; never edit an applied one. A database newer than the app fails explicitly.

## UI rules

- Style with Tailwind utilities and the theme tokens in `src/styles/globals.css`. No raw hex colors in components.
- Reuse the primitives in `src/components/DesignSystem.tsx` before adding new ones.
- `src/styles/components.css` is only for third-party internals (React Flow, rendered Markdown, TipTap). Do not add stylesheet rules for app components.
- Loading states use `src/components/BrandLoading.tsx`, not skeletons or generic spinners.
- Empty states say which action fills the area. Do not add placeholder tabs.
- At most two visible buttons per action area, primary last; secondary actions go in a `DSMenu`.
- An unreachable API shows actionable Docker guidance, not "Failed to fetch".

## Code style

- Match the existing architecture and naming. Prefer compact, idiomatic code with minimal nesting.
- Add comments only where the code needs them to be understood.
- Check existing dependencies before adding new ones.
- Vitest uses jsdom for frontend interaction tests. Keep `@types/node` explicit.

## Release notes

User-visible changes get a release note in `changelog.d/<branch>.md` ([changelog.d/README.md](changelog.d/README.md)); do not edit `CHANGELOG.md` directly. Internal changes get the `no-changelog` label instead.

## Before you finish

Run the checks for the scope you changed, check `git status --short`, and keep generated artifacts (`node_modules/`, `dist/`, `target/`) and runtime state (`*.sqlite`, `.env`) out of Git.
