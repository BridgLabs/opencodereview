use super::*;

pub(crate) fn extract_imports(path: &str, language: Option<&str>, content: &str) -> Vec<(String, String)> {
    if language == Some("go") || path.ends_with(".go") {
        return extract_go_imports(content);
    }
    if language == Some("rust") || path.ends_with(".rs") {
        return extract_rust_imports(content);
    }
    if language == Some("python") || path.ends_with(".py") {
        return extract_python_imports(content);
    }
    if language == Some("ruby") || path.ends_with(".rb") {
        return extract_ruby_imports(content);
    }
    extract_js_like_imports(content)
}

pub(crate) fn extract_js_like_imports(content: &str) -> Vec<(String, String)> {
    let mut imports = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('*') {
            continue;
        }
        let kind = if line.starts_with("import ") || line.starts_with("import(") {
            "import"
        } else if line.starts_with("export ") && line.contains(" from ") {
            "export"
        } else if line.contains("require(") {
            "require"
        } else if line.contains("import(") {
            "dynamic"
        } else {
            continue;
        };
        if let Some(spec) = extract_specifier(line) {
            imports.push((spec, kind.to_string()));
        }
    }
    imports
}

pub(crate) fn extract_rust_imports(content: &str) -> Vec<(String, String)> {
    let mut imports = Vec::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.starts_with("//") || line.starts_with("#") {
            continue;
        }
        let mut line = line;
        for prefix in ["pub ", "pub(crate) ", "pub(super) "] {
            if let Some(rest) = line.strip_prefix(prefix) {
                line = rest;
                break;
            }
        }
        if let Some(rest) = line.strip_prefix("mod ") {
            let name = identifier_prefix(rest.trim_end_matches(';').trim());
            if let Some(name) = name {
                imports.push((format!("self::{name}"), "mod".to_string()));
            }
        }
        if let Some(rest) = line.strip_prefix("use ") {
            let spec = rest.trim_end_matches(';').trim();
            if spec.starts_with("crate::") || spec.starts_with("self::") || spec.starts_with("super::") {
                imports.extend(
                    expand_rust_use_spec(spec)
                        .into_iter()
                        .map(|spec| (spec, "use".to_string())),
                );
            }
        }
    }
    imports
}

pub(crate) fn expand_rust_use_spec(spec: &str) -> Vec<String> {
    let clean = spec.trim().trim_end_matches(';').trim();
    if let Some(open) = clean.find("::{") {
        let prefix = &clean[..open];
        let tail = clean[open + 3..].trim_end_matches('}');
        return tail
            .split(',')
            .filter_map(|item| {
                let item = item.trim();
                if item.is_empty() || item == "self" {
                    None
                } else {
                    Some(format!("{prefix}::{item}"))
                }
            })
            .collect();
    }
    vec![clean.to_string()]
}

pub(crate) fn extract_python_imports(content: &str) -> Vec<(String, String)> {
    let mut imports = Vec::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("from ") {
            if let Some((module, names)) = rest.split_once(" import ") {
                let module = module.trim();
                imports.push((module.to_string(), "from".to_string()));
                for name in names.split(',') {
                    let name = name.split_whitespace().next().unwrap_or_default();
                    if !name.is_empty() && name != "*" {
                        imports.push((format!("{module}.{name}"), "from".to_string()));
                    }
                }
            }
        } else if let Some(rest) = line.strip_prefix("import ") {
            for item in rest.split(',') {
                let spec = item.split_whitespace().next().unwrap_or_default();
                if !spec.is_empty() {
                    imports.push((spec.to_string(), "import".to_string()));
                }
            }
        }
    }
    imports
}

pub(crate) fn extract_go_imports(content: &str) -> Vec<(String, String)> {
    let mut imports = Vec::new();
    let mut in_block = false;
    for raw in content.lines() {
        let line = raw.trim();
        if line.starts_with("//") {
            continue;
        }
        if line.starts_with("import (") || line == "import(" {
            in_block = true;
            continue;
        }
        if in_block && line.starts_with(')') {
            in_block = false;
            continue;
        }
        if in_block || line.starts_with("import ") {
            if let Some(spec) = extract_specifier(line) {
                imports.push((spec, "import".to_string()));
            }
        }
    }
    imports
}

pub(crate) fn extract_specifier(line: &str) -> Option<String> {
    // Find the last quoted segment on the line: '...' or "..."
    for quote in ['\'', '"'] {
        if let Some(start) = line.find(quote) {
            if let Some(end_rel) = line[start + 1..].find(quote) {
                let spec = &line[start + 1..start + 1 + end_rel];
                if !spec.is_empty() {
                    return Some(spec.to_string());
                }
            }
        }
    }
    None
}

/// Returns (resolved, to_path). `resolved` means the import points inside the repo
/// (relative file, workspace package, or tsconfig alias). `to_path` is the concrete
/// repo-root file path when one can be pinpointed (relative/alias); workspace-package
/// imports are resolved but have no single target file, so `to_path` is None.
pub(crate) fn resolve_spec(
    from_path: &str,
    language: Option<&str>,
    spec: &str,
    files: &HashSet<String>,
    package_names: &HashSet<String>,
    alias_rules: Option<&[AliasRule]>,
) -> (bool, Option<String>) {
    if language == Some("rust") || from_path.ends_with(".rs") {
        if let Some(path) = resolve_rust_spec(from_path, spec, files) {
            return (true, Some(path));
        }
    }
    if language == Some("python") || from_path.ends_with(".py") {
        if let Some(path) = resolve_python_spec(from_path, spec, files) {
            return (true, Some(path));
        }
    }
    if language == Some("ruby") || from_path.ends_with(".rb") {
        if let Some(path) = resolve_ruby_spec(spec, files) {
            return (true, Some(path));
        }
    }
    if spec.starts_with('.') {
        let dir = parent_dir(from_path);
        let path = resolve_candidate(&format!("{dir}/{spec}"), files);
        return (path.is_some(), path);
    }
    // workspace package name (exact or scoped subpath) — resolved, but no single file
    if package_names.contains(spec) {
        return (true, None);
    }
    for name in package_names {
        if spec == name {
            return (true, None);
        }
        if let Some(subpath) = spec.strip_prefix(&format!("{name}/")) {
            return (true, resolve_go_package_dir(subpath, files));
        }
    }
    // tsconfig path alias (e.g. "@/...", "@app/...")
    if let Some(rules) = alias_rules {
        if let Some(path) = resolve_alias(spec, rules, files) {
            return (true, Some(path));
        }
    }
    (false, None)
}

pub(crate) fn resolve_go_package_dir(dir: &str, files: &HashSet<String>) -> Option<String> {
    let prefix = format!("{}/", normalize_path(dir));
    files
        .iter()
        .filter(|file| file.starts_with(&prefix) && file.ends_with(".go") && !file.ends_with("_test.go"))
        .min()
        .cloned()
        .or_else(|| {
            files
                .iter()
                .filter(|file| file.starts_with(&prefix) && file.ends_with(".go"))
                .min()
                .cloned()
        })
}

pub(crate) fn resolve_rust_spec(from_path: &str, spec: &str, files: &HashSet<String>) -> Option<String> {
    let src_root = rust_src_root(from_path);
    let current_dir = parent_dir(from_path);
    if let Some(rest) = spec.strip_prefix("crate::") {
        return resolve_rust_module_path(&src_root, rest, files);
    }
    if let Some(rest) = spec.strip_prefix("self::") {
        return resolve_rust_module_path(&current_dir, rest, files);
    }
    if let Some(rest) = spec.strip_prefix("super::") {
        return resolve_rust_module_path(&parent_dir(&current_dir), rest, files);
    }
    None
}

pub(crate) fn rust_src_root(path: &str) -> String {
    if let Some((root, _)) = path.split_once("/src/") {
        format!("{root}/src")
    } else {
        "src".to_string()
    }
}

pub(crate) fn resolve_rust_module_path(base: &str, spec: &str, files: &HashSet<String>) -> Option<String> {
    let parts: Vec<&str> = spec
        .split("::")
        .filter(|part| !part.is_empty() && *part != "self")
        .collect();
    for end in (1..=parts.len()).rev() {
        let joined = normalize_path(&format!("{base}/{}", parts[..end].join("/")));
        if let Some(path) = resolve_candidate(&joined, files) {
            return Some(path);
        }
    }
    None
}

pub(crate) fn resolve_python_spec(from_path: &str, spec: &str, files: &HashSet<String>) -> Option<String> {
    if spec.starts_with('.') {
        let dot_count = spec.chars().take_while(|ch| *ch == '.').count();
        let rest = spec.trim_start_matches('.').replace('.', "/");
        let mut base = parent_dir(from_path);
        for _ in 1..dot_count {
            base = parent_dir(&base);
        }
        return resolve_candidate(&format!("{base}/{rest}"), files);
    }
    let path = spec.replace('.', "/");
    resolve_candidate(&path, files)
}

/// Resolve a candidate module path (no extension) to a tracked file, trying
/// common source extensions and index/module files. Normalizes `.`/`..` segments.
pub(crate) fn resolve_candidate(raw: &str, files: &HashSet<String>) -> Option<String> {
    let joined = normalize_path(raw);
    let candidates = [
        joined.clone(),
        format!("{joined}.ts"),
        format!("{joined}.tsx"),
        format!("{joined}.d.ts"),
        format!("{joined}.js"),
        format!("{joined}.jsx"),
        format!("{joined}.mjs"),
        format!("{joined}.cjs"),
        format!("{joined}.go"),
        format!("{joined}.rs"),
        format!("{joined}.py"),
        format!("{joined}.rb"),
        format!("{joined}/index.ts"),
        format!("{joined}/index.tsx"),
        format!("{joined}/index.js"),
        format!("{joined}/index.jsx"),
        format!("{joined}/mod.rs"),
        format!("{joined}/__init__.py"),
    ];
    candidates.into_iter().find(|candidate| files.contains(candidate))
}

pub(crate) fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(index) => path[..index].to_string(),
        None => String::new(),
    }
}

/// Collapse `.` and `..` segments into a clean repo-root-relative path.
pub(crate) fn normalize_path(raw: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for segment in raw.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

pub(crate) fn is_tsconfig(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.starts_with("tsconfig") && name.ends_with(".json")
}

pub(crate) fn config_dir(path: &str) -> String {
    parent_dir(path)
}

pub(crate) fn nearest_alias_rules<'a>(
    file_path: &str,
    tsconfig_rules: &'a [(String, Vec<AliasRule>)],
) -> Option<&'a [AliasRule]> {
    // tsconfig_rules is sorted by dir length desc, so first prefix match is the nearest config.
    for (dir, rules) in tsconfig_rules {
        if dir.is_empty() || file_path == dir || file_path.starts_with(&format!("{dir}/")) {
            return Some(rules.as_slice());
        }
    }
    None
}

pub(crate) fn resolve_alias(spec: &str, rules: &[AliasRule], files: &HashSet<String>) -> Option<String> {
    for rule in rules {
        if rule.has_star {
            if let Some(captured) = spec.strip_prefix(&rule.prefix) {
                for target in &rule.targets {
                    if let Some(found) = resolve_candidate(&format!("{target}/{captured}"), files) {
                        return Some(found);
                    }
                }
            }
        } else if spec == rule.prefix {
            for target in &rule.targets {
                if let Some(found) = resolve_candidate(target, files) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Parse tsconfig JSON (JSONC tolerant) and build alias rules from `compilerOptions.paths`,
/// with targets resolved against `baseUrl` relative to the config directory.
pub(crate) fn parse_tsconfig_aliases(content: &str, config_dir: &str) -> Option<Vec<AliasRule>> {
    let value: serde_json::Value = serde_json::from_str(&strip_jsonc(content)).ok()?;
    let options = value.get("compilerOptions")?;
    let base_url = options.get("baseUrl").and_then(|value| value.as_str()).unwrap_or(".");
    let base_dir = normalize_path(&format!("{config_dir}/{base_url}"));
    let paths = options.get("paths").and_then(|value| value.as_object())?;
    let mut rules = Vec::new();
    for (pattern, targets) in paths {
        let target_list: Vec<&str> = targets
            .as_array()
            .map(|array| array.iter().filter_map(|value| value.as_str()).collect())
            .unwrap_or_default();
        if target_list.is_empty() {
            continue;
        }
        let has_star = pattern.contains('*');
        let prefix = pattern.split('*').next().unwrap_or(pattern).to_string();
        let resolved_targets: Vec<String> = target_list
            .iter()
            .map(|target| {
                let target_prefix = target.split('*').next().unwrap_or(target);
                normalize_path(&format!("{base_dir}/{target_prefix}"))
            })
            .collect();
        rules.push(AliasRule {
            prefix,
            has_star,
            targets: resolved_targets,
        });
    }
    Some(rules)
}
