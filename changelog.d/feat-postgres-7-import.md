### Changed

- OpenCodeReview stores its data in Postgres. Docker Compose runs a `postgres` service with
  generated passwords, on a network only the server can reach. An install from a pre-release SQLite
  build moves its data once, before starting this version, with
  `docker compose run --rm server ocr-server import-sqlite /data/opencodereview.sqlite`, which also
  imports the review archive index when present.
- Removing a repository from a workspace keeps its review history. Before, the history was
  deleted with it.

### Added

- Several server instances can share one database (`docker-compose.scale.yml`). Queued reviews start
  on whichever instance has a free slot, and background jobs run on one instance at a time.
