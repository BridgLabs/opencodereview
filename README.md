<p align="center">
  <img src="assets/mascot.svg" alt="OpenCodeReview mascot" width="128" height="128">
</p>

# OpenCodeReview

[![License](https://img.shields.io/badge/license-PolyForm_Small_Business_1.0.0-blue.svg)](LICENSE)

AI code review for GitHub pull requests that runs on your own machine, using the Claude, ChatGPT or Gemini account you already have, or OpenCode's free models.

OpenCodeReview watches the repositories you pick, reviews each pull request with [Claude Code](https://docs.anthropic.com/en/docs/claude-code), [Codex](https://github.com/openai/codex), [Gemini CLI](https://github.com/google-gemini/gemini-cli) or [OpenCode](https://github.com/anomalyco/opencode) inside isolated Docker containers, checks every finding against the code, and posts the result on the pull request.

![The OpenCodeReview review queue](site/app-reviews.png)

## Features

- **Your own AI.** Reviews run on the Claude Code CLI (Claude Opus 5.5, medium effort), the Codex CLI or the Gemini CLI, signed in with your Claude, ChatGPT or Google account, or on OpenCode with its free models or your OpenCode Go key. There is no OpenCodeReview account.
- **Several accounts with failover.** Connect more than one account, of any provider, and order them. When an account hits its usage limit, the review continues on the next one, and the limited account comes back after its window resets. Settings shows how much of each Claude and ChatGPT plan is left.
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
2. **Your plan**: connect at least one account. Claude uses `claude setup-token` (you paste back the code it shows); ChatGPT uses the `codex login` browser flow or a device code; Gemini uses the Gemini CLI's Google sign-in (you paste back the code it shows); OpenCode works with no account on its free models, or takes an OpenCode Go key, which OpenCode checks before it is saved.
3. **Repositories**: choose the repositories to monitor. They are cloned and indexed in the background.
4. **First review**: pick an open pull request. After that, new pull requests in monitored repositories are reviewed automatically.

The first build compiles the Rust backend and can take several minutes. To skip it, run the images published with each release instead:

```bash
docker compose -f docker-compose.yml -f docker-compose.images.yml up -d
```

Set `OCR_VERSION` (for example `OCR_VERSION=0.2.0`) to stay on one release; the default is the newest.

## Requirements

- Docker Engine or Docker Desktop with Docker Compose v2 (the Compose file uses `additional_contexts`).
- A GitHub account with access to the repositories you want reviewed.
- At least one of:
  - a Claude subscription that works with Claude Code (Pro, Max, Team or Enterprise);
  - a ChatGPT plan that includes Codex;
  - a Google account that can use the Gemini CLI, or
  - nothing: OpenCode's free models need no account, and an OpenCode Go key unlocks its paid models. Local models through Ollama or similar are not supported: the review egress proxy refuses private networks by design.

You do not need Node, pnpm or Rust on the host to run OpenCodeReview. Every AI CLI ships in the image at a pinned, checksum-verified version.

## Configuration

Everything else is configured in the app under **Settings** (accounts, repository access, PR selection, integrations). Docker Compose reads these variables from your shell or from a `.env` file next to `docker-compose.yml`:

| Variable | Default | Purpose |
| --- | --- | --- |
| `OCR_GH_HOST` | empty (`github.com`) | GitHub Enterprise host, as a bare `host` or `host:port`. |
| `OCR_WEB_APP_URL` | `http://localhost:1421` | The origin the UI is served from. Requests from other origins are rejected; change it when you open the UI from another address. |
| `OCR_REVIEW_DEBUG_DIR` | empty | Directory where private review debug artifacts are written. Leave empty in normal use. |

For example, with GitHub Enterprise:

```bash
OCR_GH_HOST=github.example.com docker compose up --build
```

The full list of variables, including the ones Compose sets internally, is in [docs/configuration.md](docs/configuration.md).

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
| `review-runner` | Runs Claude Code, Codex, the Gemini CLI or OpenCode on a frozen snapshot of the pull request, one review stage per container at a time. 20 replicas by default (`OCR_REVIEW_RUNNERS`); idle ones use almost no memory. Each receives only a private socket and a read-only copy of the sign-in. |
| `review-egress` | Outbound proxy that allows only `api.anthropic.com`, `chatgpt.com`, `api.openai.com`, `auth.openai.com`, `oauth2.googleapis.com`, `cloudcode-pa.googleapis.com` and `opencode.ai`, on port 443 and public addresses only. |

A review fetches the live base and head of the pull request, selects the scope (full, incremental, base sync or stacked), builds context from the index, freezes the evidence into an archive, runs the AI in a runner (split into chunks for large PRs), validates the report and publishes it. On GitHub, OpenCodeReview posts a collapsed summary comment that it updates on each push, a result comment that replaces the previous one, and the validated findings as a single non-blocking review with inline comments. It never approves a pull request on its own.

More detail: [architecture](docs/architecture.md), [reviews](docs/reviews.md), [indexing](docs/indexing.md), [skills](docs/skills.md).

## Security and privacy

- **Your code goes only to your AI provider.** The pull request evidence is sent to Anthropic or OpenAI with your own sign-in, the same way their CLIs work when you use them directly. OpenCodeReview has no servers and sends no telemetry.
- **Sign-ins stay local.** GitHub credentials, AI sign-ins and keys are stored in the `ocr-data` Docker volume (AI sign-ins and keys with file mode `0600`). They are never logged or sent to the browser. **Sign out** in Settings deletes them.
- **Runners are sandboxed.** Read-only root filesystem, no Linux capabilities beyond what supervision needs, an unprivileged user for the AI process, no access to the database or repository cache, and provider-only network access.
- **The UI is meant for a trusted network.** The API is protected by an installation token that the web container injects into the UI, so anyone who can open the UI can use it. Do not expose port 1421 to the internet without an authenticated reverse proxy in front.

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

See [docs/development.md](docs/development.md) for the full setup, test suites and repository layout.

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request, and follow the [Code of Conduct](CODE_OF_CONDUCT.md). Changes are tracked in [CHANGELOG.md](CHANGELOG.md).

## License and attribution

Copyright 2026 BRIDG LABS LTDA.

Licensed under the [PolyForm Small Business License 1.0.0](LICENSE). Individuals and companies with fewer than 100 people (employees and contractors) and less than USD 1,000,000 in revenue in the prior tax year (2019 dollars, adjusted for inflation) may use, modify and distribute it for free. Larger companies, counted together with the organizations they control or are controlled by, need a separate commercial license from the copyright owner. Personal, non-commercial use and use by non-profit organizations are also free, whatever their size, as [NOTICE](NOTICE) states. For a commercial license, or help rolling it out across a company, contact [contato@bridglabs.com](mailto:contato@bridglabs.com).

See [NOTICE](NOTICE) for attribution and third-party notices.

OpenCodeReview is an independent source-available project and is not affiliated with, endorsed by, or sponsored by Anthropic, OpenAI, Google or OpenCode. Their product names are used only to describe compatibility.
