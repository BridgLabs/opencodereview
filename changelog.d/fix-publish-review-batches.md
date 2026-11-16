### Fixed

- Publish reviews with many findings without hitting GitHub's secondary rate limit. A review with
  more than 40 inline comments now goes out in several reviews about a minute apart, with the
  summary and verdict in the last one, and a review GitHub refuses with that limit is sent again
  after a wait instead of failing at once.
