### Changed

- Large reviews finish sooner: a review now runs up to six chunks at once instead of three, and
  the final pass checks its batches side by side instead of one after another.

### Fixed

- Reviews no longer lose turns to rejected snapshot reads: reading 201 lines with `sed` (for
  example `200,400p`) and limiting a search with `head -n` below 80 are now allowed.
