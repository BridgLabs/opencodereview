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

Rust tests need a Postgres where their role may create databases; each test gets its own, cloned from a migrated template:

```bash
docker run -d --name ocr-pg-dev -e POSTGRES_USER=ocr -e POSTGRES_PASSWORD=dev -p 127.0.0.1:55432:5432 postgres:17
export OCR_TEST_DATABASE_URL=postgres://ocr:dev@127.0.0.1:55432/postgres
```

Run a subset of Rust tests with a filter, for example `cargo test -p opencodereview-server review_scope --lib --locked`.

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
  src/db/                Postgres access, migrations (pg/) and queries
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

A release is one button: Actions → **Release** → **Run workflow**. Everything merged since the last tag ships as one version, so several pull requests make one release. The [Release workflow](../.github/workflows/release.yml) reads the Conventional Commits since the last `v*` tag with `scripts/release-version.mjs`:

- `feat` releases a minor version; `fix`, `perf` and `revert` a patch version; a breaking change (`!` or `BREAKING CHANGE:`) a major version, or a minor one before 1.0.
- `build`, `chore`, `ci`, `docs`, `refactor`, `style` and `test` release nothing on their own, unless the pull request left notes in `changelog.d/`.
- The **bump** input (`patch`, `minor` or `major`) raises the version to at least that level; `auto`, the default, uses only the commits.

When there is something to release, the workflow moves the `changelog.d/` notes into a new `CHANGELOG.md` section (or, when no pull request wrote notes, writes them from the commit subjects), sets `package.json` to the version and pushes a `release: vX.Y.Z` commit to a `release/vX.Y.Z` branch. It builds the `server`, `runner` and `web` images for `amd64` and `arm64` from that commit; only when every image builds does it fast-forward `main` to the commit, push the `vX.Y.Z` tag, publish the images to `ghcr.io/bridglabs/opencodereview-*` as `X.Y.Z` and `latest`, and create the GitHub release from the `CHANGELOG.md` section. A failed build leaves `main` and the tags untouched.

The push uses the `RELEASE_DEPLOY_KEY` deploy key, which the `main` ruleset lets bypass the pull request rule. With **dry-run** on, the workflow shows the next version and its notes and builds the server and web images without publishing anything. `node scripts/release-version.mjs` prints the next version locally.

GHCR creates packages private, even for a public repository. After the first release, open each package (`opencodereview-server`, `-runner`, `-web`), set its visibility to **Public** and check it is linked to the repository; the workflow's last step fails until users can pull the images without a login.

## Landing page

`site/` is a static page with no build step, served at <https://opencodereview.run> as Cloudflare Workers static assets (`site/wrangler.jsonc`). The [Site workflow](../.github/workflows/site.yml) deploys it whenever a push to `main` changes `site/`, and can also be run by hand. It needs the `CLOUDFLARE_API_TOKEN` (from the "Edit Cloudflare Workers" token template) and `CLOUDFLARE_ACCOUNT_ID` repository secrets. To deploy from your machine: `npx wrangler deploy --config site/wrangler.jsonc`.

### Search, AI assistants and link previews

The page is written to be found by search engines and cited by AI assistants:

- `site/index.html` carries the Open Graph and Twitter tags and one JSON-LD block (Organization, WebSite, SoftwareApplication and FAQPage). `pnpm site:build` writes the JSON-LD; its FAQPage is copied from the page's FAQ section, so edit the FAQ and rerun it. CI fails when the block is out of date (`scripts/site/seo.test.ts`).
- `site/robots.txt` allows every crawler, names the AI ones, and sets Cloudflare's `Content-Signal` to allow search, AI answers and training. `site/sitemap.xml` lists the page and its images.
- `site/llms.txt` is a hand-written summary for language models ([llmstxt.org](https://llmstxt.org)). `site/llms-full.txt` joins the README and `docs/` into one file; it is not committed, because the Site workflow builds it (and the sitemap's `lastmod`) before each deploy and also runs when the README or `docs/` change.
- `site/_headers` sets content types and caching for those files.
- The share image `site/og.png` (1200×630) and the PNG icons are rendered from `scripts/og/` with Playwright: `pnpm exec playwright install chromium-headless-shell` once, then `pnpm site:images`. When the image changes, bump `?v=` in its two meta tags so previews refetch it.

The Cloudflare zone for `opencodereview.run` must not undo this. Keep **AI Crawl Control → Block AI bots** off, and **Manage robots.txt** off, since it would prepend its own rules to ours. Keep **Bot Fight Mode** off; it challenges crawlers that are not on Cloudflare's verified list. Keep **Crawler Hints** on, which tells Bing and other IndexNow search engines when the page changes. To check from outside: `curl -A GPTBot -I https://opencodereview.run/` should answer 200, and `https://opencodereview.run/robots.txt` should match `site/robots.txt`.
