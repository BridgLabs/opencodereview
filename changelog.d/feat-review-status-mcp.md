### Added

- Coding agents can follow a pull request's review through a read-only MCP endpoint at
  `/api/mcp`: `get_review_status` reports whether the review is queued, running (with its stage and
  chunk progress), posted or failed, and can wait until it finishes; `list_active_reviews` lists the
  reviews in progress.
