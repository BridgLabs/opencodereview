---
name: git
description: Commit, push and pull request operations for OpenCodeReview, using Conventional Commits and the repository PR template.
argument-hint: '<natural-language git request>'
triggers:
  - user
---

Read `AGENTS.md` first. This file is the source of truth for Git and pull request work in this repository.

Git request: $ARGUMENTS

## Contract

| Question | Answer |
| --- | --- |
| Scope | Current branch worktree, branch diff versus `main`, and the associated PR when requested |
| Evidence | `git status`, diffs, logs, the PR template and PR metadata |
| Boundary | User-initiated only; no destructive Git; no broad staging; push only when explicitly requested |
| Verification | Relevant checks for the changed scope before push or PR when practical |

Supported requests include `plan commits`, `commit these changes`, `commit everything, separated by context`, `push`, `create PR`, `update PR description` and `commit, push, and update PR`. Resolve the intent from the user's words and ask before mutating Git state when it is ambiguous. Derive commit messages, splits, PR titles and bodies from the diff, branch and template; ask only when inspection leaves real ambiguity.

## Core rules

- `main` is protected. Every change lands through a pull request; never push to `main`.
- Use Conventional Commits.
- Do not add AI or tool authorship signatures to commit messages, PR titles or bodies: no `Generated with` footers, no AI `Co-Authored-By` trailers, no agent signatures. Inspect the final message before running the mutation. Preserve real human attribution and OpenCodeReview's own finding/question markers.
- Do not use interactive Git commands.
- Do not force-push, rewrite history, delete branches, reset, restore, clean, stash, rebase, merge, pull or checkout over changes without explicit confirmation.
- Never commit secrets, runtime state (`*.sqlite`, `.env`), or generated artifacts (`node_modules/`, `dist/`, `target/`).
- Stage specific files only; never `git add .`, `git add -A` or broad pathspecs, and never stage unrelated changes.

## Commit planning

Group changes into logical commit units before staging:

- Keep product behavior with its tests.
- Keep frontend and backend changes separate when each is reviewable on its own.
- Keep Docker, CI and build changes separate from product behavior.
- Keep permission, security and configuration changes separate.
- Keep large removals separate from replacement implementations when that helps review.
- Keep fixes for PR review comments in dedicated commits.

When there is more than one unit, print the plan first:

```markdown
Commit Plan:

1. <message>
   - Files: <paths>
   - Rationale: <why these belong together>
```

## Commit execution

1. Inspect state: `git status --short --branch`, `git diff --stat`, `git diff`, `git log --oneline --decorate -10`.
2. Stage each group precisely: `git add -- path/one path/two`.
3. Review the staged diff: `git diff --cached`.
4. Run the relevant checks:
   - Frontend: `pnpm typecheck`, `pnpm test`, `pnpm build`.
   - Backend: `cargo check --workspace --locked`, `cargo test --workspace --locked`.
   - Docs only: inspect the diff.
5. Commit with a Conventional Commit message, then repeat for the next unit.
6. Push only when requested.
7. Finish with the branch, commits, PR URL if any, validation status and working-tree status.

## Commit format

```text
<type>(<scope>): <summary>

<body explaining why, when useful>
```

Types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `build`, `ci`, `perf`, `style`, `revert`. The summary is imperative with no trailing period. Use a scope when clear, for example `feat(review): ...`, `fix(indexing): ...`. Mark breaking changes with `!` and a `BREAKING CHANGE:` footer.

## Pull requests

Use [`.github/pull_request_template.md`](../../../.github/pull_request_template.md) as the source of truth. Keep its sections in order, remove instruction comments and replace placeholders. Explain the problem, the resulting behavior and the risks that matter to reviewers. The PR title becomes the squash commit, so write it as a Conventional Commit.

Before creating or updating a PR:

```bash
git status --short --branch
git diff --stat main...HEAD
git log --oneline main..HEAD
gh pr status
```

Write the body to a temporary file outside the repository, inspect it, then:

```bash
gh pr create --title "<conventional title>" --body-file "$TMPDIR/pr-body.md"
```

Use `--draft` only when requested. When updating, keep reviewer-added context and edit with `gh pr edit --body-file`.
