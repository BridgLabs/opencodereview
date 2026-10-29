### Added

- A `review_focus_files` setting (off by default). Above that many changed files, a review covers
  the highest-priority files, source code first and then the most changed lines, and its summary
  lists the files it left out. Such a review carries a warning: it is never treated as clean and
  records no coverage, so a later review starts from the base again.
