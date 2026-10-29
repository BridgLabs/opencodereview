### Changed

- Reviews post only defects as inline comments: wrong results, crashes, security, data loss,
  concurrency and broken contracts. Lower-priority points, such as missing tests, stale test
  names or docs, duplication and style, are still found and checked against the code, but are
  listed together in a Suggestions section of the review summary, so readers see what breaks
  first. A review with only suggestions posts them as a comment and never requests changes.
