### Added

- Support rootless Podman on Linux with the native podman-compose provider, using
  the same Compose stack and runner sandbox as Docker. Document provider selection
  and add container checks for startup and persistence without real AI calls.

### Fixed

- Wait for PostgreSQL's final TCP listener before starting the backend, rather than
  accepting the temporary initialization server as ready.
