### Fixed

- A review whose final answer had a stray closing bracket no longer fails; the extra bracket is
  dropped and the report is validated as usual.
- The single output repair attempt now also runs when a review reported excluded or missing
  supporting evidence. That evidence stays as a warning and the repair cannot change it.
- A pull request that deletes or renames a file whose name has brackets, such as `[key].ts`, no
  longer fails before the review starts. The deleted file used to be read back as an empty file.
- When the output repair also rewords a finding, the original wording is kept and only its line and
  quote corrections are used, instead of failing the whole review.
- Final verification splits a batch with more than 30 candidates across files into smaller
  batches, so a large review no longer runs past the stage time limit.
- A chunk that writes a context question as an object instead of text no longer fails the review;
  the question's text is used.
- An output repair that only rewords the reason of a final-pass decision no longer fails the
  review: the reason first given is kept.
- Evidence cited with a path missing its leading directories (`AppDelegate.swift` for
  `Pika/AppDelegate.swift`) no longer fails the review when exactly one file at that revision ends
  with it and holds the quoted text.
- A final response that puts the report's `contractVersion` and `snapshotId` beside the report,
  as Sonnet sometimes does, is read instead of failing the stage.
- The review prompt now says every finding needs at least one counter-evidence reference, which
  models left empty and then needed an output repair for.
- A finding anchored to a line number of the pull request's diff file, instead of the changed
  file's own line number, is moved to the right line instead of being dropped or repaired.
- A final answer that runs a sentence straight into the report object, with no line break before
  it, is read instead of failing the review.
