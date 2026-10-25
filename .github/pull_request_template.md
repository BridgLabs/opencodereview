<!--
The PR title becomes the squash commit on main, so use Conventional Commits:
feat(review): ..., fix(indexing): ..., docs: ...
-->

## Context

<!-- Why this change? The problem, who hits it, and links ("Closes #123"). -->

## What Changed

<!-- The resulting behavior, in a few bullets. Call out anything breaking. -->

## Failure modes

<!--
Remove this section unless the PR adds or changes tests. Write the list before the code: one line
per way the change can fail, with the test that catches it. A test that no line names is a test
written after the code.
-->

-

## Risk and security impact

<!--
Changes to the runner sandbox, the tool guard, egress hosts, credentials or sign-in projection,
GitHub permissions, publication, or what reaches logs and the browser. Write "None" when not
applicable.
-->

## Migration / Rollout

<!--
Anything an existing install needs: new database migration, Docker rebuild, changed
settings, egress hosts or CLI versions. Write "None" if a plain update is enough.
-->

## How to Review

<!--
Where to start and what deserves the closest look. Commands you ran and what you checked
by hand; say if anything was verified only with mocks or fake inference.
-->

## Screenshots

<!-- Before and after for UI changes, or a short recording. Delete this section otherwise. -->

---

- [ ] Checks pass for what I changed (`pnpm lint`, `pnpm knip`, `pnpm typecheck`, `pnpm test`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`).
- [ ] Release notes in `changelog.d/<branch>.md`, or the `no-changelog` label for a change users cannot see.
- [ ] No secrets, credentials, `.env` files, databases or real customer data.
- [ ] Database changes add a new migration instead of editing an applied one.
