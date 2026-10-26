# Review skills

A review skill is the Markdown methodology the AI follows when it reviews a pull request: what to look for, how to gather evidence, when to ask a question instead of asserting a defect, and how to classify severity. OpenCodeReview handles everything else (fetching, context, chunking, validation and posting), so a skill only describes how to reason about the evidence.

Manage skills on the Skills page (`/platform/skills`).

## Built-in skill

**Default** ([`backend/src/assets/review-skill.md`](../backend/src/assets/review-skill.md)) is the full, repository-agnostic review strategy, active on a new install. It is embedded in the binary and cannot be edited in place.

## Versions

- Open any skill, edit it in the visual editor or as Markdown, and compare it with the default using **Diff vs default**.
- **Save new version** stores a snapshot (`v1`, `v2`, ...) and makes it active. The skill you started from stays unchanged.
- Select any version and activate it to roll back. Saved versions can be deleted.
- Exactly one skill is active at a time. Every new review stage copies the active skill into its context pack as `skill/SKILL.md`, and the skill is part of the cache key, so changing it invalidates cached chunk results.

## Project conventions

You do not need a custom skill to teach OpenCodeReview a repository's rules. Reviews automatically include these files from the repository root at the pull request's base commit: `REVIEW.md`, `AGENTS.md`, `CONVENTIONS.md`, `CONTRIBUTING.md`, `CLAUDE.md`, `.cursorrules` and `README.md`. The built-in skills defer to them. Use a custom skill to change the review strategy itself.

## Writing a skill

- Write for the evidence OpenCodeReview provides: `PR.md`, `PR_DIFF.patch`, `CONTEXT.md`, and the `changed/`, `related/` and `conventions/` folders. The AI cannot reach the network or run project commands.
- Do not ask the AI to post comments, run Git or call GitHub; OpenCodeReview owns publication and renders the final comment format itself.
- Keep fenced code blocks well-formed. The editor preserves raw HTML and markers as plain content.

## Shipping a better default

To improve the default for every new install, open a pull request that updates `backend/src/assets/review-skill.md`. Skill versions saved in a local database are not shared.
