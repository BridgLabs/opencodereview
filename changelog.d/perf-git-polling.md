### Changed

- Reviews and indexing run git faster: each git call is checked after 1ms instead of 50 to 80ms,
  and the wait grows only for long calls.
