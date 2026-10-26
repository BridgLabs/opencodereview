# Development

Docker is the only supported runtime. For development you also need the frontend and Rust toolchains on the host so you can run tests and the Vite dev server.

## Toolchain

- Docker with Compose v2
- Node 22 and pnpm 10 (`corepack enable` picks up the version pinned in `package.json`). Use pnpm only; do not add `package-lock.json` or `bun.lock`.
- Rust stable with `cargo` on your `PATH`
- Python 3 for the Docker smoke and regression scripts

The pre-commit hook in `.githooks/` turns on with `pnpm install`. It formats staged files with Biome and rustfmt, validates staged release notes in `changelog.d/`, and type-checks when TypeScript is staged.

## Running locally

Start the full stack:

```bash
docker compose up --build
```

For hot-reloading frontend work against the Docker backend:

```bash
pnpm install
pnpm debug:web
```

`debug:web` stops the Docker `web` service, serves Vite on port 1421 using the API token from the running `server` container, and restores `web` when you exit. Backend changes still need a rebuilt image: `docker compose up --build server`.

`pnpm dev:web` runs Vite alone, without the Docker API.

## Checks

These are the checks CI runs. Run the ones that match what you changed before opening a pull request.

```bash
# Frontend
pnpm lint
pnpm knip                      # unused files, exports and dependencies
pnpm typecheck
pnpm test
pnpm build
python3 -B docker/test-web-proxy.py

# Backend
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python3 -B docker/test_diagnostics.py

# Dependencies
cargo deny check               # advisories, licenses and sources (deny.toml)
pnpm audit --audit-level high

# Repository
pnpm check:file-size
pnpm changelog:check
docker compose config --quiet
docker compose build && python3 -u docker/runner-smoke.py
```

CI also checks the links and heading anchors between Markdown files with lychee.

Run a subset of Rust tests with a filter, for example `cargo test -p opencodereview-server review_scope --lib --locked`. If SQLite-backed tests fail with `database is locked`, rerun with `-- --test-threads=1`.

After building the images, the runner smoke test checks archive IPC and sandboxing with fake inference. It uses no credentials and makes no AI calls:

```bash
python3 -u docker/runner-smoke.py
```

Scripts that make real AI calls (`docker/run-real-review-diagnostic.py`, `docker/run-concurrent-review-diagnostic.py`) use your plan's quota and require explicit flags to run.

## File size limits

`scripts/check-file-sizes.mjs` enforces maximum line counts on tracked files: 800 for frontend TypeScript, 2,500 for Rust, 3,500 for CSS and 300 for scripts in `scripts/`. Split a file before it reaches the limit.

## Repository layout

```text
src/                     React frontend (React 19, TypeScript, Vite, Tailwind)
  pages/                 Onboarding
  platform/              Reviews, indexing, skills and settings pages, components, hooks
  services/              HTTP API clients
  components/            Shared UI primitives (DesignSystem.tsx)
  styles/                Theme tokens and base styles
backend/                 Rust crate opencodereview-server
  src/main_server.rs     ocr-server binary
  src/main_sandbox.rs    ocr-sandbox binary (runner, egress, tool guard)
  src/db/                SQLite schema, migrations and queries
  src/assets/            Built-in review skills
assets/                  Mascot, favicon and static onboarding page
docker/                  Dockerfiles, nginx config, smoke and diagnostic scripts
scripts/                 Dev and CI helper scripts
site/                    Static project landing page
docs/                    Documentation
docker-compose.yml       The runtime
```

## Database changes

The schema starts from `backend/src/db/schema.sql` and the migrations listed in `backend/src/db/migrations.rs`. Add a new migration at the end of the list; never edit one that has already shipped. Never commit a database file.

## Upgrading the AI CLIs

The Claude Code and Codex CLI and Node.js versions are pinned in `docker/Dockerfile` with SHA-256 checksums for `amd64` and `arm64`; the Gemini CLI is pinned to one npm package with its SHA-256; the OpenCode CLI is a pinned release archive with per-architecture SHA-256 checksums. Update a version and its checksums together, rebuild, and run `docker/runner-smoke.py` before using the new image.

The [Provider CLI canary](../.github/workflows/provider-cli-canary.yml) builds the image with the newest release of each CLI every six hours. It checks that each CLI still accepts the flags its provider passes, runs the runner smoke, and sends one prompt to OpenCode's free model. Its run summary lists the pinned and newest versions, and `python3 -B docker/provider-cli-canary.py resolve` prints the newest versions with their `amd64` checksums. It opens an issue when it fails twice in a row.

## Releases

`pnpm changelog:release <version>` moves the notes in `changelog.d/` into a new `CHANGELOG.md` section. Commit that, then push the tag `v<version>`: the [Release workflow](../.github/workflows/release.yml) publishes the `server`, `runner` and `web` images for `amd64` and `arm64` to `ghcr.io/bridglabs/opencodereview-*`.

## Landing page

`site/` is a static page with no build step. The [Site workflow](../.github/workflows/site.yml) publishes it to the Cloudflare Pages project `opencodereview`, served at <https://opencodereview.run>, whenever a push to `main` changes `site/`; it can also be run by hand. It needs the `CLOUDFLARE_API_TOKEN` (with Cloudflare Pages edit access) and `CLOUDFLARE_ACCOUNT_ID` repository secrets.
