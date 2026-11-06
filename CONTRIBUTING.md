# Contributing to OpenCodeReview

Thanks for your interest in improving OpenCodeReview. This guide explains how to propose a change and get it merged.

By participating you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md). Report security issues privately as described in [SECURITY.md](SECURITY.md), not in public issues.

## Before you start

- **Bugs:** search the [existing issues](https://github.com/BridgLabs/opencodereview/issues) first, then open a bug report with steps to reproduce.
- **Features and larger changes:** open a feature request or discussion before writing code, so we can agree on the approach. Changes that add product areas, model providers or settings need a product decision first.
- **Small fixes** (typos, docs, obvious bugs) can go straight to a pull request.

## Workflow

`main` is protected. Nobody pushes to it directly, maintainers included. Every change lands through a pull request that:

- passes the required `check` status in CI,
- has one approving review from a code owner,
- has all review conversations resolved,
- is squash-merged, so the pull request title becomes the commit on `main` and, with your name, its line on the release page.

Steps:

1. Fork the repository and clone your fork.
2. Create a branch from `main` with a descriptive name, for example `fix/indexing-go-imports` or `feat/review-summary-links`.
3. Make your change with tests, and update the docs when behavior changes.
4. Run the checks below.
5. Commit using [Conventional Commits](https://www.conventionalcommits.org/).
6. Push your branch and open a pull request against `main`, filling in the template.
7. Address review feedback with new commits on the same branch. Do not force-push after review has started unless asked.

## Development setup

You need Docker with Compose v2 or [rootless Podman on Linux](README.md#rootless-podman-on-linux), Node 22, pnpm 10, a stable Rust toolchain and Python 3.

```bash
git clone https://github.com/<you>/opencodereview && cd opencodereview
pnpm install
docker compose up --build          # full stack on http://localhost:1421
pnpm debug:web                     # optional: hot-reload frontend against the Docker backend
```

Use pnpm only. Do not commit `package-lock.json`, `yarn.lock` or `bun.lock`.

`pnpm install` turns on the hooks in `.githooks/`. On each commit, pre-commit formats the staged files with Biome and rustfmt, validates staged release notes, and type-checks when TypeScript is staged. On each push, pre-push warns when the branch has no release notes in `changelog.d/`, which the Changelog workflow requires. CI runs everything else.

## Checks

Run the checks that match what you changed:

```bash
# Frontend
pnpm lint:fix
pnpm knip
pnpm typecheck
pnpm test
pnpm build

# Backend
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked

# Dependencies (when you change Cargo.lock or pnpm-lock.yaml)
cargo deny check
pnpm audit --audit-level high

# Repository
pnpm check:file-size
pnpm changelog:check
docker compose config --quiet
python3 -B docker/test_diagnostics.py
python3 -B docker/test-web-proxy.py
```

Rust tests need a Postgres where their role may create databases; each test gets its own, cloned from a migrated template:

```bash
docker run -d --name ocr-pg-dev -e POSTGRES_USER=ocr -e POSTGRES_PASSWORD=dev -p 127.0.0.1:55432:5432 postgres:17
export OCR_TEST_DATABASE_URL=postgres://ocr:dev@127.0.0.1:55432/postgres
```

If you changed the runner, egress or Dockerfiles, build the images and run `python3 -u docker/runner-smoke.py` (fake inference, no credentials needed).

For rootless Podman, select podman-compose 1.6.0 as documented in the README, then
use `podman compose` in place of `docker compose`. Run the container regressions
with `OCR_CONTAINER_ENGINE=podman`:

```bash
OCR_CONTAINER_ENGINE=podman python3 -B docker/test-web-proxy.py
OCR_CONTAINER_ENGINE=podman python3 -u docker/runner-smoke.py
OCR_CONTAINER_ENGINE=podman python3 -u docker/runtime-smoke.py
```

`runtime-smoke.py` builds a disposable Compose project with one runner, checks the
UI and authenticated settings API, restarts the stack, verifies persistence, and
removes only that project's containers and volumes. Stop any stack using ports
1421, 8787 or 1455 first. It runs no inference. Set `OCR_RUNTIME_PROJECT` to choose
its project name; an existing project is refused. The script also runs on Docker
with no engine override. For runner network checks against a running non-default
project, set `OCR_SMOKE_SERVER` to its server container name.

`python3 -B docker/test-runtime-smoke.py` checks the smoke CLI's stopped-project
guard and image ownership with a disposable engine fixture. Its successful HTTP
lifecycle runs inside a Linux user/network namespace (`unshare` and `ip` required),
so it does not use an existing installation's ports or API.

Do not include real AI calls in tests. Never commit credentials, tokens, `.env` files or database files and dumps, and keep test fixtures free of real customer or repository data.

## Commit messages

```text
<type>(<scope>): <summary>
```

Types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `build`, `ci`, `perf`, `style`, `revert`. Write the summary in the imperative mood without a trailing period, for example `fix(indexing): resolve tsconfig path aliases`. Mark breaking changes with `!` and a `BREAKING CHANGE:` footer. Because pull requests are squash-merged, the pull request title must follow the same format; the required `PR title` check enforces it.

Do not add AI tool attribution (such as `Generated with` footers or AI `Co-Authored-By` trailers) to commits or pull requests.

## Code guidelines

- Match the existing architecture and naming; read [AGENTS.md](AGENTS.md) for the project rules.
- Frontend source files stay at or under 800 lines and Rust files at or under 2,500 lines.
- Keep changes focused. Unrelated refactors belong in separate pull requests.
- Add or update tests for behavior changes.
- Write release notes for user-visible changes in `changelog.d/<branch>.md` (see [changelog.d/README.md](changelog.d/README.md)), not in `CHANGELOG.md`. A pull request with no user-visible change gets the `no-changelog` label.

## License

OpenCodeReview is licensed under the [PolyForm Small Business License 1.0.0](LICENSE). The maintainer also offers it under a commercial license to companies above its size limits.

To offer both, the maintainer must own the copyright in all of the code. Every contributor signs the [Contributor Assignment Agreement](CLA.md) once, which assigns the copyright in their contributions to the maintainer. On your first pull request a bot asks you to sign it by replying with a comment, and pull requests can only be merged after that.
