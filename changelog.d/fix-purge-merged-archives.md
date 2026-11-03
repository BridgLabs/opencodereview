### Fixed

- Free a merged pull request's frozen review evidence as soon as the merge is confirmed. Before,
  it was kept for a day, so busy repositories filled the archive store and every review failed
  with "Frozen evidence exceeds its approved budget".
