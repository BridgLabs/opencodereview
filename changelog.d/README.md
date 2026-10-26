# Unreleased notes

Each pull request writes its release notes in its own file here, so two pull requests never change
the same lines of [CHANGELOG.md](../CHANGELOG.md). Name the file after the branch, with `-` for `/`:
`changelog.d/fix-indexing-go-imports.md`.

Write for people running OpenCodeReview, under the [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
groups `Added`, `Changed`, `Deprecated`, `Removed`, `Fixed` and `Security`. A file can have more
than one group, and an item can continue on lines indented by two spaces:

```md
### Fixed

- Keep the review queue order after the server restarts. Before, reviews went back to the order
  the pull requests were opened.
```

`pnpm changelog:check` validates the files; the pre-commit hook and the Changelog workflow run it
too. The Release workflow moves every item into a new version section of `CHANGELOG.md` after the
merge, in the order the pull requests merged, and deletes the files.

A pull request with no change that a user can see gets the `no-changelog` label instead.
