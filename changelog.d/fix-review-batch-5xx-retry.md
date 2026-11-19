### Fixed

- When GitHub returns a temporary server error while publishing a review, check whether it already
  received that part before sending it again, so inline comments are not posted twice.
