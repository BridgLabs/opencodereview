### Added

- Index Ruby: methods, classes and modules, links between the methods and classes that call each
  other, `require` and `require_relative` links between files, `.rb`, `.rake`, `.gemspec` and `.ru`
  files, and a package for each `Gemfile` or `*.gemspec` with its gem dependencies. Reviews of
  Ruby pull requests now get callers and dependencies of the changed code, as for the other languages.

### Fixed

- Count `_spec.rb` and `_test.rb` files and `spec/` and `test/` directories as tests. Before, a Ruby
  pull request that changed specs was reported as "Source changed without nearby test updates".

### Changed

- Show test files as hollow circles in the indexing graphs, with a "test" entry in the legend.
- When an indexing graph is too large to draw, keep the best connected files and symbols, joined to
  each other. Before, it kept the first ones in path order and left many of them floating without
  edges.
- Ruby: call a `def` inside a class or module a method, stop a class from taking every call its
  methods make, stop a symbol from linking to the namespace it sits in, and link a file to the file
  that defines a constant it uses, as Rails loads code without `require`.
- A "Hide tests" option in the file and symbol graphs removes test files and their links, so large repositories fit more of the real code.
