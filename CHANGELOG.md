# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/BridgLabs/opencodereview/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/BridgLabs/opencodereview/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/BridgLabs/opencodereview/releases/tag/v0.1.0
