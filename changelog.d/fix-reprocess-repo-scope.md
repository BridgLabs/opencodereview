### Fixed

- Re-running a review no longer fails with "previous scope belongs to another repository": review
  scopes recorded the run's temporary index instead of the repository, which also kept their
  coverage from being reused. Scopes recorded before the fix are recognised too.
