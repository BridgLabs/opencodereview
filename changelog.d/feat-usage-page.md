### Added

- A Usage page with the tokens reviews used over the last 7, 30 or 90 days, by day, pull request,
  repository, model and account, with findings found and posted, median review time, reused
  chunks and the tokens spent on reviews that were superseded, failed or cancelled. A pull request
  opens into each of its runs.
- Share a picture of your usage: an image of the period's totals to download or copy, without
  repository, pull request, account or model names.
- Each execution of a review stage records the account and model it ran on, including executions
  abandoned when an account hits its usage limit.

### Fixed

- Count every abandoned execution of a stage whose output was repaired after an account failover.
  Before, the repair's failover overwrote the first one's transcript and its tokens were lost.
