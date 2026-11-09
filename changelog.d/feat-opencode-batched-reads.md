### Changed

- OpenCode reviews read the prepared files several at a time instead of one small range per
  step, so each chunk takes far fewer model steps. Claude and Codex prompts are unchanged.

### Fixed

- A chunk reviewed with one provider, model or effort is no longer reused for a review that runs
  on another.
