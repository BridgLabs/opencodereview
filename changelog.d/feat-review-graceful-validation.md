### Changed

- One invalid finding no longer fails a whole review. After output repair, a finding that still
  fails validation is dropped and the summary says so; the findings that passed are kept. A review
  whose every finding is invalid still fails.
- A finding the final verification pass leaves without a decision keeps the version validated in
  its chunk, and the summary marks it as not confirmed by the final pass.
- A final integration batch whose output still fails validation after its repair no longer fails the
  review: its findings stay as validated in their chunks, marked unconfirmed.
