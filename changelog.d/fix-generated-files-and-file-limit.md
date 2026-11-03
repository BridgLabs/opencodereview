### Changed

- Count only the files a review reads toward Maximum PR files. Build output, vendored code,
  minified bundles, source maps and generated code no longer count, so a pull request with a few
  source files and many generated ones is reviewed instead of skipped.

### Fixed

- Recognize more generated files and lockfiles in reviews, risk and indexing: `vendor/`,
  `generated/`, `.min.css`, source maps, protobuf and Go generated code, `package-lock.json`,
  `go.sum` and other package manager lockfiles. Before, each part used its own shorter list.
