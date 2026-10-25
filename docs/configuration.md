# Configuration

Most configuration lives in the app under **Settings**. Environment variables cover deployment details: where data lives, which GitHub host to use and how the containers find each other.

## Settings

| Section | What it controls |
| --- | --- |
| Accounts | Claude, ChatGPT, Gemini and OpenCode accounts, their priority order, remaining plan usage (Claude and ChatGPT) and which account the next review uses. |
| Repository access | GitHub organizations and repositories to monitor. Only selected repositories are indexed, polled and reviewed. An organization can be monitored as a whole, including repositories created later. |
| PR selection | Review scope (`Assigned` to you or your teams, or `All PRs`), the maximum number of files per PR (default 100; larger PRs are skipped before any AI call), poll interval and title keywords that skip a PR (for example `renovate`, `chore(deps)`). |
| Integrations | GitHub and AI CLI status and versions, re-check and sign out. |

Edits stay pending until **Save changes**, which applies them atomically. Review execution and publishing use fixed defaults: automatic reviews on, up to 10 reviews at once (and at most three AI processes at a time), Auto depth, summary and result comments posted, findings posted as a non-blocking review, stale approvals dismissed. See [reviews.md](reviews.md).

**Author activation:** the pull request author can comment `/ocr review` to queue a first review, including on a draft PR.

## Environment variables

Set these in your shell or in a `.env` file next to `docker-compose.yml`. Compose forwards the first three to the `server` container:

| Variable | Default | Purpose |
| --- | --- | --- |
| `OCR_GH_HOST` | empty (`github.com`) | GitHub Enterprise host as a bare `host` or `host:port`. Values with a scheme, path, credentials or whitespace are rejected. Read once at startup; restart after changing it. `GH_HOST` is used as a fallback. |
| `OCR_WEB_APP_URL` | `http://localhost:1421` | The trusted UI origin. State-changing API requests from other origins are rejected, and links in onboarding use it. Set it to the address you open the UI from. |
| `OCR_REVIEW_DEBUG_DIR` | empty | When set, private review debug artifacts are copied here. Treat them as sensitive. |

The remaining variables are set by `docker-compose.yml` or have working defaults. To change one, add it to the service in a `docker-compose.override.yml`.

### `server`

| Variable | Compose value | Purpose |
| --- | --- | --- |
| `OCR_BIND` | `0.0.0.0:8787` | API listen address (default outside Docker: `127.0.0.1:8787`). |
| `OCR_DATA_DIR` | `/data` | Data root for accounts, sign-ins and archives. |
| `OCR_DB_PATH` | `/data/opencodereview.sqlite` | SQLite database. |
| `OCR_REPOS_DIR` | `/data/repos` | Git cache for monitored repositories. |
| `OCR_AUTO_REINDEX` | `true` | Keep indexes fresh when the default branch moves. `false` or `0` disables it. |
| `OCR_POLL_INTERVAL_SECONDS` | `300` | Initial polling interval; Settings can change it. |
| `OCR_API_TOKEN` | generated | Bearer token for the API. When unset, a random token is generated and stored at `/data/api-token`. |
| `OCR_GH_PR_LIMIT` | `1000` | Maximum open pull requests listed per repository on each poll. |
| `OCR_REVIEW_RUNNERS` | `20` | Runner replicas Docker Compose starts: how many review stages can run at once. Read by `docker-compose.yml`. |
| `OCR_REVIEW_RUNNER_DIR` | `/review-ipc` | Directory where every runner replica listens on its own socket. |
| `OCR_REVIEW_RUNNER_SOCKETS` | unset | Comma-separated runner sockets (up to 64 absolute paths); overrides `OCR_REVIEW_RUNNER_DIR`, for diagnostics. |
| `OCR_REVIEW_CLAUDE_AUTH_DIR` | `/review-claude-auth/app-managed` | Where sign-ins are projected for runners. |
| `OCR_ONBOARDING_HTML_PATH` | `/app/assets/onboarding.html` | Static onboarding page served by the API. |
| `OCR_CLAUDE_TOKEN_PATH` | `/data/claude/oauth-token` | Location of a single legacy Claude token; if present it becomes the first account. |
| `OCR_CODEX_CALLBACK_BIND` | `0.0.0.0:1456` | Listener for the ChatGPT browser sign-in callback (published on host `127.0.0.1:1455`). |
| `GH_TOKEN` | unset | Development fallback for GitHub access. Normal installs sign in through onboarding. |

### Runners and egress

| Variable | Compose value | Purpose |
| --- | --- | --- |
| `OCR_RUNNER_AUTH_ROOT` | `/claude-auth/app-managed` | Read-only sign-in projection inside runners. |
| `OCR_RUNNER_PROXY` | `http://review-egress:8790` | Egress proxy used by the AI CLIs. |
| `OCR_EGRESS_MAX_CONNECTIONS` | `48` | Concurrent tunnels allowed by `review-egress` (1 to 512). |

### Maintenance

| Variable | Default | Purpose |
| --- | --- | --- |
| `OCR_DOCKER_CACHE_KEEP` | `168h` | Age threshold for [`docker/ocr-docker-cleanup`](../docker/ocr-docker-cleanup), which prunes old build cache, images and stopped containers. |

## Remote access

The browser only talks to the `web` port. nginx proxies `/api` to `server`, which stays bound to the host loopback. To open the UI from another machine, set `OCR_WEB_APP_URL` to that origin and put an authenticated reverse proxy in front: the API token is embedded in the UI, so anyone who can load the page can use the API. If `server` is recreated and its container address changes, run `docker compose restart web`.

## Ports

| Host port | Service | Purpose |
| --- | --- | --- |
| `1421` | `web` | UI and `/api` proxy. |
| `127.0.0.1:8787` | `server` | API, loopback only. |
| `127.0.0.1:1455` | `server` | ChatGPT browser sign-in callback, forwarded only while a sign-in runs. If the port is busy, use **Sign in with a code** instead. |
