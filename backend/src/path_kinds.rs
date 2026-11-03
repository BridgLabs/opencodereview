//! Path kinds shared by indexing, review scope and risk: files a tool wrote, and lockfiles.

/// Directories whose files are build output, dependencies or code generation.
const GENERATED_DIRS: &[&str] = &[
    "node_modules",
    "dist",
    "build",
    ".next",
    "coverage",
    "__generated__",
    "generated",
    "vendor",
];

/// File name endings of minified bundles, source maps and protobuf or Go code generation.
const GENERATED_SUFFIXES: &[&str] = &[
    "generated.ts",
    "generated.js",
    ".min.js",
    ".min.css",
    ".js.map",
    ".css.map",
    ".pb.go",
    ".pb.h",
    ".pb.cc",
    "_pb2.py",
    "_pb2_grpc.py",
    "_gen.go",
];

const LOCKFILES: &[&str] = &[
    "package-lock.json",
    "npm-shrinkwrap.json",
    "go.sum",
    "package.resolved",
    "packages.lock.json",
];

/// Whether a tool wrote this file: build output, vendored dependencies, minified bundles, source
/// maps or generated code.
pub(crate) fn is_generated_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let mut parts = lower.split('/').collect::<Vec<_>>();
    let name = parts.pop().unwrap_or_default();
    parts.iter().any(|part| GENERATED_DIRS.contains(part))
        || name.contains(".generated.")
        || name.starts_with("zz_generated")
        || GENERATED_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
}

/// Whether this file pins dependency versions for a package manager.
pub(crate) fn is_lockfile(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or_default();
    name.ends_with(".lock") || name.ends_with("lock.yaml") || name.ends_with(".lockb") || LOCKFILES.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_paths_cover_build_output_bundles_maps_and_codegen() {
        for path in [
            "dist/app.js",
            "web/node_modules/react/index.js",
            "packages/ui/build/index.js",
            "vendor/github.com/pkg/errors/errors.go",
            "src/api/__generated__/types.ts",
            "src/schema.generated.ts",
            "src/graphql/generated.ts",
            "static/app.min.css",
            "static/app.js.map",
            "api/v1/service.pb.go",
            "proto/service_pb2_grpc.py",
            "pkg/apis/zz_generated.deepcopy.go",
            "internal/mock_gen.go",
        ] {
            assert!(is_generated_path(path), "{path}");
        }
        for path in [
            "src/build.rs",
            "src/generator.ts",
            "docs/vendoring.md",
            "src/distance.rs",
            "proto/service.proto",
            "src/app.js",
        ] {
            assert!(!is_generated_path(path), "{path}");
        }
    }

    #[test]
    fn lockfiles_cover_every_package_manager_name() {
        for path in [
            "Cargo.lock",
            "web/yarn.lock",
            "pnpm-lock.yaml",
            "package-lock.json",
            "npm-shrinkwrap.json",
            "go.sum",
            "Package.resolved",
            "src/App/packages.lock.json",
            "bun.lockb",
        ] {
            assert!(is_lockfile(path), "{path}");
        }
        for path in ["package.json", "go.mod", "src/lock.rs", "docs/locking.md"] {
            assert!(!is_lockfile(path), "{path}");
        }
    }
}
