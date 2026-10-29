### Fixed

- A pull request that changes a file in another encoding, such as Latin-1, no longer fails before
  the review starts; the file's diff is read with the unreadable bytes replaced.
