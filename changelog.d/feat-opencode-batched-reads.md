### Changed

- OpenCode reviews read the prepared files several at a time instead of one small range per
  step, so each chunk takes far fewer model steps. Claude and Codex prompts are unchanged.

### Fixed

- A chunk reviewed with one provider, model or effort is no longer reused for a review that runs
  on another.
- An OpenCode or Codex review no longer fails when the model writes report fields beside the
  report, sends the report without its wrapper, leaves its JSON fence unclosed, or repeats the
  report's snapshot identity in every finding.
