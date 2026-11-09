### Added

- Add OpenCode Go as a review provider with models and reasoning choices loaded from its official API,
  explicit private API key connection, runner credential projection and token attribution across
  account failover. Plan quota and reset times remain unknown; key presence does not establish
  entitlement or prevent provider Extra Usage charges.

### Fixed

- Keep account action menus open when a password field scrolls internally after losing focus.
- Attribute review stages to the model that actually ran instead of the legacy default.
- Separate chunk caches by provider, model, effort and frozen Go metadata; do not reuse mixed-engine results.
- Separate chunk execution time from time spent waiting for a runner, and show cancellation reasons.
- Allow review stages up to 30 minutes of inference with a fixed deadline that activity cannot extend.
- Keep supported Go models available when discovery includes unsupported entries, and accept successful retries.
- Read bounded windows from large evidence files without treating ordinary Unicode escape text as leaked credentials.
- Require a confirmed Go model before selecting an account for reviews, and report rejected model saves.
- Skip unavailable Go backups during failover without refreshing the review's frozen catalogue.
- Skip unprepared Go accounts during global preflight when another prepared review account is available.
- Identify evidence files skipped or interrupted by search limits while preserving matches already found.
- Show engine discovery failures with a retry action without removing provider connection choices.
- Preserve known review tokens and show unknown queue duration when execution has no recorded start time.

### Security

- Reject smoke-test temporary directories unless they are real directories owned by the current user with mode 0700.
