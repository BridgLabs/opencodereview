### Changed

- OpenCodeReview stores its data in Postgres. Docker Compose runs a `postgres` service with
  generated passwords, on a network only the server can reach. An install from a pre-release SQLite
  build moves its data once: stop it (`docker compose stop`) and keep a copy of
  `/data/opencodereview.sqlite*` and `/data/review-archives/index.sqlite*` (the `-wal` and `-shm`
  files too), update, run `docker compose up -d postgres`, then
  `docker compose run --rm server ocr-server import-sqlite /data/opencodereview.sqlite`, which also
  imports the review archive index, and start the stack. To go back to a pre-release build, restore
  both copies first: without its archive index, it deletes every stored archive. Gemini and OpenCode
  accounts and their saved sign-ins are removed on upgrade; reviews run on Claude and ChatGPT
  accounts.
- Removing a repository from a workspace keeps its review history. Before, the history was
  deleted with it.

### Added

- Several server instances can share one database (`docker-compose.scale.yml`). Queued reviews start
  on whichever instance has a free slot, and background jobs run on one instance at a time.
