# Repository indexing

Every monitored repository gets a local context index that reviews use to find related code. The index lives in SQLite and is built from a Git cache under `/data/repos`. The Indexing page (`/platform/indexing`) shows each repository's status and an interactive graph.

## What is indexed

| Part | Contents | Used for |
| --- | --- | --- |
| Files | Path, language, size, blob SHA and package of source, convention and manifest files | Context selection |
| Package and import graph | Packages from manifests, internal package dependencies, file import edges | Finding callers, dependencies and tests of a change |
| Symbols | Top-level declarations with kind, location, signature and export flag | Symbol-centered context |

All three are written together by one index job, so the UI shows a single status per repository.

### Packages

| Ecosystem | Manifest | Package name |
| --- | --- | --- |
| JavaScript / TypeScript | `package.json` | `name` |
| Go | `go.mod` | `module` |
| Rust | `Cargo.toml` | `[package].name` |
| Python | `pyproject.toml`, `setup.py`, `requirements.txt` | project name, with fallbacks |

Manifests larger than 2,000,000 bytes fail indexing with their path and size instead of producing an incomplete graph.

### Imports

| Language | Resolved imports |
| --- | --- |
| JavaScript / TypeScript | `import`, `export ... from`, `require`, dynamic `import()`, including `tsconfig` path aliases |
| Go | `import "..."` and grouped imports, resolved to module subpaths |
| Others | Indexed as files and packages; import resolution is limited |

### Symbols

Symbol extraction scans source text for top-level declarations. It is intentionally lightweight and is **not** an AST parser.

| Language | Symbols | Exported when |
| --- | --- | --- |
| JavaScript / TypeScript | `function`, `class`, `interface`, `type`, `enum`, exported `const`/`let`/`var` | `export` keyword |
| Rust | `fn`, `struct`, `enum`, `trait`, `type`, `mod`, `const`, `static` | `pub` |
| Python | top-level `def`, `async def`, `class` | name does not start with `_` |
| Go | functions, methods, struct and interface types | identifier starts with an uppercase letter |

## How indexing runs

1. Selecting a repository in Settings prepares its Git cache and queues an index.
2. A background worker fetches the default branch, lists the tree and fetches only indexable blobs in bounded batches (binary and generated files are never downloaded just to be listed).
3. Files, packages, imports and symbols are written in one transaction, and the index is marked ready with the indexed commit.
4. With `OCR_AUTO_REINDEX=true` (the default), the worker compares the indexed commit with the remote default branch and re-indexes when it moves or when the index format changes.

**Re-index** on the Indexing page runs the same full pipeline on demand.

Reviews do not depend on the default-branch index being fresh: each review builds a temporary graph at the pull request's pinned head and removes it before the AI runs.

Source: `backend/src/indexer/`.
