### Changed

- The review details panel shows less chrome and repeats less. A stage's timeline starts open and short
  output shows inline. Tokens only appear for stages that call a model, and every expandable section
  uses the same flat style. The Context stage shows one number per kind of file (changed, related,
  tests, conventions, packages); clicking a number lists those files.
- In Review result, each finding shows its title once, without the "Automated review by OpenCodeReview"
  line GitHub comments carry, and without the diff side and the "AI: selected" tag. Finding groups are
  filter chips instead of a second row of tabs, and the original AI summary folds below the findings.
  The connection status only shows while a review is running.
