### Added

- Resolve the review threads of fixed findings automatically. When a new push fixes a finding
  OpenCodeReview published, the review that inspects the file now closes that finding's GitHub
  conversation, but only when it quotes the new head showing the problem gone. Threads started by
  people, questions, and findings that are still reported, only moved, unchanged or not inspected
  stay open. A GitHub failure is noted in the run's log and never fails the review; the next
  review tries again.
