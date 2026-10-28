### Changed

- Reviews now read the review skill before the diff, then sweep every changed hunk for issues in
  every category (tests, stale docs and comments, maintainability, performance, accessibility as
  well as correctness) before verifying each one. On the ReviewBench test set this roughly tripled
  the issues found while precision stayed above 90%.
