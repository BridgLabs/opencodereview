### Fixed

- A pull request that changes a file in another encoding, such as Latin-1, no longer fails before
  the review starts; the file's diff is read with the unreadable bytes replaced.
- A finding whose text mentions a code fence (```` ``` ````) no longer makes the final answer read
  as an unclosed fence and fail the review; only a fence before the report object counts.
- A chunk whose every finding still fails validation after the output repair no longer fails the
  whole review: its points are dropped and listed as a warning, and the other chunks' findings are
  posted. The warning now says a point was dropped because its evidence did not validate.
- A pull request in a very large repository, such as FreeCAD or Node.js, no longer fails before the
  review starts because the repository exceeds the frozen snapshot's size budget. The changed,
  related, test and convention files are always frozen first; files that no longer fit are marked
  excluded and cannot be cited.
