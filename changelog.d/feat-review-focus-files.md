### Added

- A `review_focus_files` setting (off by default). Above that many changed files, a review covers
  the highest-priority files, source code first and then the most changed lines, and its summary
  lists the files it left out. Files left out are not marked as covered, so a later review still
  includes them.
