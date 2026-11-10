# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-11-10

### Added

- Support rootless Podman on Linux with the native podman-compose provider, using
  the same Compose stack and runner sandbox as Docker. Document provider selection
  and add container checks for startup and persistence without real AI calls.
- Review with OpenCode: sign in to the OpenCode console with a one-time code, like ChatGPT, or
  connect an OpenCode API key, which OpenCodeReview checks with OpenCode before saving it. The top bar
  lists OpenCode's models straight from opencode.ai, refreshed hourly: Go models on the Go plan and
  Zen models on the workspace credits, each with the efforts OpenCode offers for it, and only for
  the plans your OpenCode workspace has. Free models are left out. OpenCode accounts take part in failover like Claude and ChatGPT ones. Review runners run
  OpenCode 2.2.6 with a private server, no project configuration, plugins, MCP servers, skills, free
  models or web tools, and may reach `opencode.ai` only. Settings doesn't show OpenCode usage limits;
  check them in your OpenCode workspace, and turn off Go's Extra Usage there to keep Go reviews
  within the plan.
- Each review stage records how many model round trips and tool calls it took, the reasoning
  tokens the CLI reported and the time spent on the provider. The stage breakdown of a review
  returns them.

### Changed

- A review keeps the provider, model and effort chosen when it started. Switching the engine in
  the top bar no longer moves reviews that are already running; it applies to the next ones.
- OpenCode reviews read the prepared files several at a time instead of one small range per
  step, so each chunk takes far fewer model steps. Claude and Codex prompts are unchanged.
- OpenCode reviews get the prepared pull request text, skill, diff and changed files in their
  prompt instead of reading them step by step, and split into chunks of up to three files that
  run in parallel once a review has more than three. Claude and Codex reviews are unchanged.

### Fixed

- Wait for PostgreSQL's final TCP listener before starting the backend, rather than
  accepting the temporary initialization server as ready.
- Keep runtime smoke images separate from installed images, remove only the smoke's
  image tags after success or failure, and refuse projects with stopped containers.
- Reviews no longer fail with "Invalid operational runner configuration" after a few updates: the
  sockets that replaced review runners leave behind are now cleared before they are counted.
- A chunk reviewed with one provider, model or effort is no longer reused for a review that runs
  on another.
- An OpenCode or Codex review no longer fails when the model writes report fields beside the
  report, sends the report without its wrapper, leaves its JSON fence unclosed, or repeats the
  report's snapshot identity in every finding.
- An OpenCode or Codex review no longer fails when the model writes the contract version as a
  string.
- A review no longer fails when a finding cites code without its revision but the quote is the
  exact text at the pull request's head; the citation is bound to the head.
- Posting a review of a commit that already has a posted review now says so, instead of a generic
  "settings changed or publication in progress" error.

### Security

- Update the bundled SQLite used by `ocr-server import-sqlite` to a version without
  CVE-2026-119375 (RUSTSEC-2026-0459), which could expose page-cache memory when opening a
  database file together with a write-ahead log written by a less trusted party.

## [0.2.0] - 2026-11-04

### Changed

- Count only the files a review reads toward Maximum PR files. Build output, vendored code,
  minified bundles, source maps and generated code no longer count, so a pull request with a few
  source files and many generated ones is reviewed instead of skipped.
- Leave vendored code (`vendor/`), `generated/` directories, minified styles, source maps,
  protobuf and Go generated code, and the remaining lockfiles (`package-lock.json`,
  `npm-shrinkwrap.json`, `go.sum`, `Package.resolved`, `packages.lock.json`, `bun.lockb`) out of
  frozen review evidence, as `Cargo.lock`, `yarn.lock` and `pnpm-lock.yaml` already were. Their
  changes are no longer reviewed or counted toward Maximum PR files; dependency changes are still
  reviewed in `package.json`, `go.mod` and other manifests. Archives frozen before the update keep
  validating under the policy they were frozen with.

### Fixed

- Free a merged pull request's frozen review evidence as soon as the merge is confirmed. Before,
  it was kept for a day, so busy repositories filled the archive store and every review failed
  with "Frozen evidence exceeds its approved budget". Installs whose archive retention is 1 day,
  the old default, move to the new default of 0 days, including ones imported from SQLite.
- Post a new finding even when an earlier OpenCodeReview comment sits on the same line. Before,
  any finding on an already commented line was held back as "already posted".
- Recognize a finding that a later push moved to another line as already posted, by its title in
  the same file, instead of posting it again.
- Recognize more generated files and lockfiles in reviews, risk and indexing: `vendor/`,
  `generated/`, `.min.css`, source maps, protobuf and Go generated code, `package-lock.json`,
  `go.sum` and other package manager lockfiles. Before, each part used its own shorter list.
- Skip a pull request that only changes generated files, lockfiles, binaries or oversized files,
  marked "Not reviewed", instead of failing with an evidence preflight error. A later push that
  only adds such files keeps the previous review outcome. A pull request that only changes private
  files (such as `.env`) still fails explicitly.
- Leave a link in the pull request to a generated, lockfile or binary file out of the evidence,
  without checking or fetching its repository, instead of failing preparation.
- Publish reviews whose findings hold very long words or tokens without stalling. Redacting
  credentials scanned a token again from each of its characters, so a 30,000-character token took
  minutes.

### Security

- Never quote a private or internal repository linked from a pull request, or used as its
  submodule, in a review it could leak from. Linked code is accepted when it is public, or when
  the reviewed repository is private or internal and has the same owner; otherwise the review
  stops and says which link to remove. Before, any repository your GitHub session could read was
  frozen as evidence, so an outside contributor could link your private code into a public review.
- Accept `/ocr review` only from pull request authors who are an owner, member or collaborator of
  the repository. Before, any author could start a review on your AI accounts, even with
  automatic reviews limited to assigned pull requests.
- Redact credential-shaped text (GitHub, GitLab, AWS, Anthropic, OpenAI, Slack, Stripe, Google
  and npm tokens, private key blocks and URL passwords) from published findings, questions and
  suggestions.
- Neutralize Markdown images and `@` mentions in published review text, so a comment never makes
  GitHub fetch a URL chosen by the model or notify a user or team.

## [0.1.1] - 2026-11-02

### Security

- The web image moves to nginx 1.30 and takes Alpine's security updates at build time, fixing a critical
  brotli vulnerability and high ones in curl, OpenSSL, expat, pcre2 and other base packages. The server
  and runner images take Debian's security updates at build time too.
- Update pulldown-cmark to 0.13.6 (RUSTSEC-2026-0433). OpenCodeReview did not enable the affected
  wikilink parsing.

## [0.1.0] - 2026-11-02

### Added

- First public release of OpenCodeReview: AI code review for GitHub pull requests that runs locally in Docker.
- Reviews with the Claude Code CLI or the Codex CLI, signed in with your own Claude or ChatGPT account. Claude
  reviews use Claude Opus 5.5 at medium effort by default; the top bar picks each provider's model and effort.
- Several accounts with failover. Pick the account new reviews start on from the top bar, which shows how much of
  each plan is left and when it resets. At a usage limit, a review continues on the next account of the same
  provider, then the other provider.
- Codebase indexing: file metadata, a package and import graph and lightweight symbols. Reviews get the callers,
  dependencies and tests of the changed code, plus your convention files (`AGENTS.md`, `CLAUDE.md`, `REVIEW.md`
  and others), including committed agent folders such as `.claude` and `.agents`.
- Evidence-checked findings. Each finding quotes the exact code, anchors to a changed line, carries a severity
  (high, medium or low) and names its trigger; a defect whose trigger cannot be named is asked as a question. A
  point that fails validation is dropped with a warning, and such a review is never treated as clean.
- Full, incremental, base-sync and stacked pull request reviews. Large pull requests are split into chunks
  reviewed in parallel and checked by a final pass.
- Publication on GitHub: a collapsed pull request summary, a result comment and one non-blocking review with
  inline comments. Lower-priority suggestions (tests, docs, naming) are posted only when Settings → PR selection →
  Review comments → Post suggestions is on. OpenCodeReview never approves a pull request.
- A Reviews list grouped into Needs you, AI reviewing, Completed, Drafts and skipped, and Closed and merged, with a
  details panel for each run.
- A Usage page with the tokens reviews used over 7, 30 or 90 days, by day, pull request, repository, model and
  account, and a picture of it to share.
- Editable, versioned review skills.
- Guided onboarding at `http://localhost:1421`.
- Data in Postgres. Several server instances can share one database (`docker-compose.scale.yml`).
- Prebuilt images for amd64 and arm64 at `ghcr.io/bridglabs/opencodereview-server`, `-runner` and `-web`
  (`docker-compose.images.yml`).
- Source-available project files: PolyForm Small Business 1.0.0 license, NOTICE, contributing guide, code of
  conduct, security policy, issue and pull request templates.

[Unreleased]: https://github.com/BridgLabs/opencodereview/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/BridgLabs/opencodereview/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/BridgLabs/opencodereview/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/BridgLabs/opencodereview/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/BridgLabs/opencodereview/releases/tag/v0.1.0
