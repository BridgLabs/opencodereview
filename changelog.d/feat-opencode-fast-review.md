### Changed

- OpenCode reviews get the prepared pull request text, skill, diff and changed files in their
  prompt instead of reading them step by step, and split into chunks of up to three files that
  run in parallel once a review has more than three. Claude and Codex reviews are unchanged.
