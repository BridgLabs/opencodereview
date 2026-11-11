<p align="center">
  <img src="assets/mascot.svg" alt="OpenCodeReview mascot" width="128" height="128">
</p>

# OpenCodeReview

[![License](https://img.shields.io/badge/license-PolyForm_Small_Business_1.0.0-blue.svg)](LICENSE)

AI code review for GitHub pull requests that runs on your own machine, using the Claude, ChatGPT or OpenCode (Go or Zen) plan you already have.

OpenCodeReview watches the repositories you pick, reviews each pull request with [Claude Code](https://docs.anthropic.com/en/docs/claude-code), [Codex](https://github.com/openai/codex) or [OpenCode](https://opencode.ai) inside isolated containers, checks every finding against the code, and posts the result on the pull request.

![The OpenCodeReview review queue](site/app-reviews.png)

## Features

- **Your own AI.** Reviews run on the Claude Code CLI (Claude Opus 5.5 at medium effort by default), the Codex CLI, signed in with your Claude or ChatGPT account, or the OpenCode CLI signed in to your OpenCode workspace, on its Go plan or Zen credits. OpenCode's models are listed from its public Go and Zen model lists on opencode.ai, so new ones show up without an OpenCodeReview update. The top bar picks each provider's model and effort. There is no OpenCodeReview account.
- **Several accounts with failover.** Connect more than one account, of any provider, and pick the one new reviews start on. When an account hits its usage limit, the review continues on the next account of the same provider, then the other providers, and the limited account comes back after its window resets. Settings shows how much of each Claude and ChatGPT plan is left.
- **Codebase indexing.** Each selected repository is indexed locally: file metadata, a package and import graph, and lightweight symbols. Reviews get the callers, dependencies and tests of the changed code, plus your convention files (`AGENTS.md`, `CLAUDE.md`, `CONTRIBUTING.md`, `REVIEW.md` and others).
- **Usage.** See the tokens reviews used by day, pull request, repository, model and account, with findings, time and cache reuse. A pull request opens into each of its runs.
- **Review skills.** The review methodology is a Markdown skill you can edit, version and roll back in the app.
- **Evidence-checked findings.** Every finding must quote the exact code it is about and anchor to a changed line. Reports that fail validation are never published as clean.
- **Incremental and stacked PRs.** New pushes review only what changed since the last review; stacked PRs are reviewed against their immediate base.
- **Isolated execution.** The AI CLI runs in locked-down runner containers with a read-only snapshot of the code, no repository checkout, and network access limited to your AI provider.
- **Guided onboarding** at `http://localhost:1421`: sign in to GitHub, connect your plan, pick repositories and run a first review.

## Quick start

```bash
git clone https://github.com/BridgLabs/opencodereview && cd opencodereview && docker compose up --build
```

Open [http://localhost:1421](http://localhost:1421) and follow onboarding:

1. **GitHub**: sign in with the GitHub device flow.
2. **Your plan**: connect at least one account. Claude uses `claude setup-token` (you paste back the code it shows); ChatGPT uses the `codex login` browser flow or a device code; OpenCode signs in to the OpenCode console with a device code (you approve the code it shows), or takes an API key from your OpenCode workspace, which OpenCodeReview checks with OpenCode before saving it.
3. **Repositories**: choose the repositories to monitor. They are cloned and indexed in the background.
4. **First review**: pick an open pull request. After that, new pull requests in monitored repositories are reviewed automatically.

The first build compiles the Rust backend and can take several minutes. To skip it, run the images published with each release instead:

```bash
docker compose -f docker-compose.yml -f docker-compose.images.yml up -d
```

Set `OCR_VERSION` (for example `OCR_VERSION=0.1.0`) to stay on one release; the default is the newest.

## Requirements

- Docker Engine or Docker Desktop with Docker Compose v2 (the Compose file uses `additional_contexts`), or rootless Podman on Linux as described below.
- A GitHub account with access to the repositories you want reviewed.
- At least one of:
  - a Claude subscription that works with Claude Code (Pro, Max, Team or Enterprise);
  - a ChatGPT plan that includes Codex;
  - an OpenCode workspace with a Go plan (Go models) or Zen credits (Zen models). Go can bill past the plan through Extra Usage; OpenCodeReview can't stop that, so turn Extra Usage off in your Go workspace if you want reviews to stay within the plan.

Local models through Ollama or similar are not supported: the review egress proxy refuses private networks by design.

You do not need Node, pnpm or Rust on the host to run OpenCodeReview. Every AI CLI ships in the image at a pinned, checksum-verified version.

### Rootless Podman on Linux

Tested with Podman 5.8.9 and **podman-compose 1.6.0**, with cgroup v2, Netavark and
aardvark-dns. Use these versions or newer; older versions are not validated.
The native provider needs version 1.6.0 or newer for `service:` build contexts.
Podman on macOS and Windows is not supported.

Install podman-compose 1.6.0 or newer with your distribution package manager, or
run `pipx install podman-compose==1.6.0`. Select it explicitly: `podman compose`
may otherwise prefer Docker Compose even when podman-compose is installed:

```bash
podman-compose --version
export PODMAN_COMPOSE_PROVIDER="$(command -v podman-compose)"
podman compose up --build
```

Run the complete build, not just `build review-runner`: the server image must exist
before its runner context can be resolved. Keep the provider selected for later
`podman compose stop`, `start`, `restart` and `down` commands. Named volumes preserve
data across restarts; `down -v` deletes that data.

This native provider does not require the Podman API socket. The Docker Compose
provider needs `systemctl --user start podman.socket`, but starting the socket
alone does not fix its additional-context build failure on Podman. Keep Netavark
DNS enabled for the internal database and runner networks. The shared Compose file
labels its initialization bind mount for SELinux; do not disable SELinux or relax
runner isolation to work around startup failures.

## Configuration

Everything else is configured in the app under **Settings** (providers, repository access, PR selection, integrations). Compose reads these variables from your shell or from a `.env` file next to `docker-compose.yml`:

| Variable | Default | Purpose |
| --- | --- | --- |
| `OCR_GH_HOST` | empty (`github.com`) | GitHub Enterprise host, as a bare `host` or `host:port`. |
| `OCR_WEB_APP_URL` | `http://localhost:1421` | The origin the UI is served from. Requests from other origins are rejected; change it when you open the UI from another address. |
| `OCR_WEB_BIND` | `127.0.0.1` | Host address that publishes port 1421. The default keeps the UI on your own machine. Set `0.0.0.0` only on a trusted network, and put authentication in front of it (see Security). |
| `OCR_REVIEW_RUNNERS` | `20` | Review runner containers; idle ones use almost no memory. |
| `OCR_VERSION` | `latest` | Image tag used with `docker-compose.images.yml`, for example `0.1.0`. |
| `OCR_REVIEW_DEBUG_DIR` | empty | Path inside the server container (for example `/data/review-debug`) where private review debug artifacts, which contain pull request code, are written. Leave empty in normal use. |

For example, with GitHub Enterprise:

```bash
OCR_GH_HOST=github.example.com docker compose up --build
```

## How it works

```text
browser ──> web (nginx, :1421) ──/api──> server (Rust API, 127.0.0.1:8787)
                                            │  Git cache, gh CLI ──> postgres
                                            │  private IPC
                                            v
                              review-runner × 20 ──> review-egress ──> AI provider
```

| Service | Role |
| --- | --- |
| `web` | Serves the React UI and proxies `/api` to `server`. |
| `server` | Rust HTTP API: GitHub polling through the `gh` CLI, indexing, review orchestration, publication and background jobs. |
| `review-runner` | Runs Claude Code, Codex or OpenCode on a frozen snapshot of the pull request, one review stage per container at a time. 20 replicas by default (`OCR_REVIEW_RUNNERS`); idle ones use almost no memory. Each receives only a private socket and a read-only copy of the sign-in. |
| `review-egress` | Outbound proxy that allows only `api.anthropic.com`, `chatgpt.com`, `api.openai.com`, `auth.openai.com` and `opencode.ai`, on port 443 and public addresses only. |

A review fetches the live base and head of the pull request, selects the scope (full, incremental, base sync or stacked), builds context from the index, freezes the evidence into an archive, runs the AI in a runner (split into chunks for large PRs), validates the report and publishes it. On GitHub, OpenCodeReview posts a collapsed summary comment that it updates on each push, a result comment that replaces the previous one, and the validated findings as a single non-blocking review with inline comments. It never approves a pull request on its own.

## Security and privacy

- **Your code goes only to your AI provider.** The pull request evidence is sent to Anthropic, OpenAI or OpenCode with your own sign-in, the same way their CLIs work when you use them directly. OpenCodeReview has no servers and sends no telemetry.
- **Sign-ins stay local.** GitHub credentials, AI sign-ins and keys are stored in the `ocr-data` Docker volume (AI sign-ins and keys with file mode `0600`). They are never logged or sent to the browser. **Sign out** (GitHub, in Integrations) and **Remove account** (AI accounts, in Providers) delete them.
- **Runners are sandboxed.** Read-only root filesystem, no Linux capabilities beyond what supervision needs, an unprivileged user for the AI process, no access to the database or repository cache, and provider-only network access.
- **The UI is reachable from your machine only.** The API is protected by an installation token that the web container injects into the UI, so anyone who can open the UI can use it. Compose therefore publishes port 1421 on `127.0.0.1` only. To open it to other machines, set `OCR_WEB_BIND=0.0.0.0` and change `OCR_WEB_APP_URL` to the address you use. Docker's port rules bypass host firewalls such as ufw, so do this only on a trusted network, or put an authenticated reverse proxy in front. To reach a remote host, use an SSH tunnel instead.

To report a vulnerability, see [SECURITY.md](SECURITY.md).

## Development

```bash
pnpm install
pnpm debug:web        # Vite on :1421 against the Docker backend
pnpm typecheck
pnpm test
pnpm build
cargo test --workspace --locked
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full setup and checks.

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request, and follow the [Code of Conduct](CODE_OF_CONDUCT.md). Changes are tracked in [CHANGELOG.md](CHANGELOG.md).

## License and attribution

Copyright 2026 BRIDG LABS LTDA.

Licensed under the [PolyForm Small Business License 1.0.0](LICENSE). Individuals and companies with fewer than 100 people (employees and contractors) and less than USD 1,000,000 in revenue in the prior tax year (2019 dollars, adjusted for inflation) may use, modify and distribute it for free. Larger companies, counted together with the organizations they control or are controlled by, need a separate commercial license from the copyright owner. Personal, non-commercial use and use by non-profit organizations are also free, whatever their size, as [NOTICE](NOTICE) states. For a commercial license, or help rolling it out across a company, contact [contato@bridglabs.com](mailto:contato@bridglabs.com).

See [NOTICE](NOTICE) for attribution and third-party notices.

OpenCodeReview is an independent source-available project and is not affiliated with, endorsed by, or sponsored by Anthropic, OpenAI or OpenCode. Their product names are used only to describe compatibility.
