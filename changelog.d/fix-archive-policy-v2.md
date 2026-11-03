### Changed

- Leave vendored code (`vendor/`), `generated/` directories, minified styles, source maps,
  protobuf and Go generated code, and the remaining lockfiles (`package-lock.json`,
  `npm-shrinkwrap.json`, `go.sum`, `Package.resolved`, `packages.lock.json`, `bun.lockb`) out of
  frozen review evidence, as `Cargo.lock`, `yarn.lock` and `pnpm-lock.yaml` already were. Their
  changes are no longer reviewed or counted toward Maximum PR files; dependency changes are still
  reviewed in `package.json`, `go.mod` and other manifests. Archives frozen before the update keep
  validating under the policy they were frozen with.

### Fixed

- Skip a pull request that only changes generated files, lockfiles, binaries or oversized files,
  marked "Not reviewed", instead of failing with an evidence preflight error. A pull request that
  only changes private files (such as `.env`) still fails explicitly.
- Leave a link in the pull request to a generated file or lockfile out of the evidence instead of
  failing preparation.
