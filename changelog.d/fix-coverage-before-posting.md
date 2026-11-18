### Fixed

- Keep a review's coverage when the server restarts while the review is being posted. Before, the
  review was still posted after the restart, but the next review of that pull request failed with
  "completed coverage for the previous review is unavailable" until you chose Re-run full. Reviews
  left that way are healed on the next start.
