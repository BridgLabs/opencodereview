### Added

- Index Ruby: methods, classes and modules, links between the methods and classes that call each
  other, `require` and `require_relative` links between files, `.rb`, `.rake`, `.gemspec` and `.ru`
  files, and a package for each `Gemfile` or `*.gemspec` with its gem dependencies. Reviews of
  Ruby pull requests now get callers and dependencies of the changed code, as for the other languages.
- Link a Ruby file to the file that defines a constant it uses, as Rails loads code without
  `require`. A qualified name such as `Aws::S3::Client` links only to a file that also declares
  its namespace, so a constant from a gem does not link to a local class of the same name.
- A "Hide tests" option in the file and symbol graphs removes test files and their links, so large
  repositories fit more of the real code.

### Fixed

- Count `_spec.rb` and `_test.rb` files, `spec/` and `test/` directories, and Jasmine `_spec.js`
  files as tests. Before, a Ruby pull request that changed specs was reported as "Source changed
  without nearby test updates".

### Changed

- Show test files as hollow circles in the indexing graphs, with a "test" entry in the legend.
- When an indexing graph is too large to draw, keep the best connected files and symbols, joined to
  each other. Before, it kept the first ones in path order and left many of them floating without
  edges.
