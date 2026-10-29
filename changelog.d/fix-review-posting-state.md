### Fixed

- PR summaries on private repositories fetch the pull request with the GitHub token, like every
  other fetch, instead of failing with "could not read Username".
- A review waiting for a decision because automatic publication failed now says why above the
  decision panel.

### Changed

- The server logs one line per finished review saying what automatic publication did with it
  (posted, failed and why, left for a decision, or not attempted and why).
