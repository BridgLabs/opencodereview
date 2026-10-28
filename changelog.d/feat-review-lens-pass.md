### Added

- A `review_lens_pass` setting (off by default). Each review stage gets one more execution that
  looks for what a first pass usually leaves out: tests, stale docs and comments, maintainability,
  API shape and small slips. It adds only new findings, which are validated like any other; if it
  fails, the first report stands.
