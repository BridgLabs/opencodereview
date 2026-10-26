### Changed

- OpenCodeReview stores its data in Postgres instead of SQLite. Docker Compose runs a `postgres`
  service with generated passwords, on a network only the server can reach. Existing installs move
  their data once with `ocr-server import-sqlite`; follow [docs/upgrading.md](../docs/upgrading.md)
  before starting the new version.
- Removing a repository from a workspace keeps its review history. Before, the history was
  deleted with it.

### Added

- Several server instances can share one database (`docker-compose.scale.yml`). Queued reviews start
  on whichever instance has a free slot, and background jobs run on one instance at a time.
