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
