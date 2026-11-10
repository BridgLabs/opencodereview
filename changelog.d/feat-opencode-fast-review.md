### Changed

- OpenCode reviews get the prepared pull request text, skill, diff and changed files in their
  prompt instead of reading them step by step, and split into chunks of up to three files that
  run in parallel once a review has more than three. Claude and Codex reviews are unchanged.

### Fixed

- An OpenCode or Codex review no longer fails when the model writes the contract version as a
  string.
- A review no longer fails when a finding cites code without its revision but the quote is the
  exact text at the pull request's head; the citation is bound to the head.
