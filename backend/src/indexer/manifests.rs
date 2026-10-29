use super::*;

pub(crate) fn is_package_manifest(path: &str) -> bool {
    matches!(
        path,
        "package.json" | "go.mod" | "Cargo.toml" | "pyproject.toml" | "setup.py" | "requirements.txt"
    ) || [
        "/package.json",
        "/go.mod",
        "/Cargo.toml",
        "/pyproject.toml",
        "/setup.py",
        "/requirements.txt",
    ]
    .iter()
    .any(|suffix| path.ends_with(suffix))
}

pub(crate) fn parse_manifest(path: &str, content: &[u8]) -> Option<ParsedManifest> {
    if path == "go.mod" || path.ends_with("/go.mod") {
        return parse_go_mod(content);
    }
    if path == "Cargo.toml" || path.ends_with("/Cargo.toml") {
        return parse_cargo_toml(content);
    }
    if path == "pyproject.toml" || path.ends_with("/pyproject.toml") {
        return parse_pyproject_toml(content);
    }
    if path == "setup.py" || path.ends_with("/setup.py") {
        return parse_setup_py(path, content);
    }
    if path == "requirements.txt" || path.ends_with("/requirements.txt") {
        return parse_requirements_txt(path, content);
    }
    let value: serde_json::Value = serde_json::from_slice(content).ok()?;
    let name = value.get("name").and_then(|name| name.as_str())?.to_string();
    let mut deps = Vec::new();
    for (key, dev) in [
        ("dependencies", false),
        ("devDependencies", true),
        ("peerDependencies", false),
    ] {
        if let Some(map) = value.get(key).and_then(|deps| deps.as_object()) {
            for dep_name in map.keys() {
                deps.push((dep_name.clone(), dev));
            }
        }
    }
    Some(ParsedManifest { name, deps })
}

pub(crate) fn parse_go_mod(content: &[u8]) -> Option<ParsedManifest> {
    let text = String::from_utf8_lossy(content);
    let mut name = None;
    let mut deps = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("module ") {
            name = line.split_whitespace().nth(1).map(str::to_string);
        } else if line.starts_with("require ") {
            if let Some(dep) = line.split_whitespace().nth(1) {
                deps.push((dep.to_string(), false));
            }
        } else if !line.starts_with("//") && !line.starts_with(')') && !line.is_empty() {
            let mut parts = line.split_whitespace();
            if let (Some(dep), Some(version)) = (parts.next(), parts.next()) {
                if version.starts_with('v') || version.starts_with("=>") {
                    deps.push((dep.to_string(), false));
                }
            }
        }
    }
    Some(ParsedManifest { name: name?, deps })
}

pub(crate) fn parse_cargo_toml(content: &[u8]) -> Option<ParsedManifest> {
    let text = String::from_utf8_lossy(content);
    let name = toml_string_value_in_section(&text, "package", "name")?;
    let mut deps = Vec::new();
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let dev = section != "dependencies";
        for dep in toml_table_keys_in_section(&text, section) {
            deps.push((dep, dev));
        }
    }
    Some(ParsedManifest { name, deps })
}

pub(crate) fn parse_pyproject_toml(content: &[u8]) -> Option<ParsedManifest> {
    let text = String::from_utf8_lossy(content);
    let name = toml_string_value_in_section(&text, "project", "name")
        .or_else(|| toml_string_value_in_section(&text, "tool.poetry", "name"))?;
    let mut deps = Vec::new();
    for dep in toml_array_strings_in_section(&text, "project", "dependencies") {
        deps.push((python_dep_name(&dep), false));
    }
    for dep in toml_table_keys_in_section(&text, "tool.poetry.dependencies") {
        if dep != "python" {
            deps.push((dep, false));
        }
    }
    for dep in toml_table_keys_in_section(&text, "tool.poetry.group.dev.dependencies") {
        deps.push((dep, true));
    }
    Some(ParsedManifest { name, deps })
}

pub(crate) fn parse_setup_py(path: &str, content: &[u8]) -> Option<ParsedManifest> {
    let text = String::from_utf8_lossy(content);
    let name = quoted_arg_after(&text, "name")
        .unwrap_or_else(|| manifest_dir(path).replace('/', "-"))
        .trim()
        .to_string();
    if name.is_empty() {
        return None;
    }
    Some(ParsedManifest { name, deps: Vec::new() })
}

pub(crate) fn parse_requirements_txt(path: &str, content: &[u8]) -> Option<ParsedManifest> {
    let text = String::from_utf8_lossy(content);
    let deps = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with('-'))
        .map(|line| (python_dep_name(line), false))
        .filter(|(name, _)| !name.is_empty())
        .collect();
    let dir = manifest_dir(path);
    Some(ParsedManifest {
        name: if dir.is_empty() {
            "python".to_string()
        } else {
            dir.replace('/', "-")
        },
        deps,
    })
}

pub(crate) fn manifest_dir(path: &str) -> String {
    for suffix in [
        "/package.json",
        "/go.mod",
        "/Cargo.toml",
        "/pyproject.toml",
        "/setup.py",
        "/requirements.txt",
    ] {
        if let Some(dir) = path.strip_suffix(suffix) {
            return dir.to_string();
        }
    }
    String::new()
}

pub(crate) fn toml_string_value_in_section(text: &str, section: &str, key: &str) -> Option<String> {
    let mut active = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            active = line.trim_matches(&['[', ']'][..]) == section;
            continue;
        }
        if active && line.starts_with(key) {
            let value = line.split_once('=')?.1.trim();
            return unquote(value.split('#').next()?.trim());
        }
    }
    None
}

pub(crate) fn toml_table_keys_in_section(text: &str, section: &str) -> Vec<String> {
    let mut active = false;
    let mut keys = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            active = line.trim_matches(&['[', ']'][..]) == section;
            continue;
        }
        if active && !line.is_empty() && !line.starts_with('#') {
            if let Some((key, _)) = line.split_once('=') {
                keys.push(key.trim().trim_matches('"').to_string());
            }
        }
    }
    keys
}

pub(crate) fn toml_array_strings_in_section(text: &str, section: &str, key: &str) -> Vec<String> {
    let mut active = false;
    let mut items = Vec::new();
    let mut collecting = false;
    let mut buffer = String::new();
    for raw in text.lines() {
        let line = without_toml_comment(raw).trim();
        if line.starts_with('[') && line.ends_with(']') && !collecting {
            active = line.trim_matches(&['[', ']'][..]) == section;
            continue;
        }
        if active && (collecting || line.starts_with(key)) {
            collecting = true;
            buffer.push_str(line);
            buffer.push('\n');
            if line.contains(']') {
                break;
            }
        }
    }
    for part in buffer.split(['[', ']', ',']) {
        if let Some(value) = unquote(part.trim()) {
            items.push(value);
        }
    }
    items
}

/// `line` up to a `#` that is outside a quoted string.
fn without_toml_comment(line: &str) -> &str {
    let mut quote = None;
    for (index, character) in line.char_indices() {
        match (quote, character) {
            (None, '"' | '\'') => quote = Some(character),
            (Some(open), _) if character == open => quote = None,
            (None, '#') => return &line[..index],
            _ => {}
        }
    }
    line
}

pub(crate) fn unquote(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_end_matches(',').trim();
    // A lone quote both starts and ends with one, but quotes nothing.
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        Some(trimmed[1..trimmed.len().saturating_sub(1)].to_string())
    } else {
        None
    }
}

pub(crate) fn quoted_arg_after(text: &str, key: &str) -> Option<String> {
    let idx = text.find(&format!("{key}="))?;
    let rest = &text[idx + key.len() + 1..];
    let quote = rest.chars().find(|c| *c == '\'' || *c == '"')?;
    let start = rest.find(quote)? + 1;
    let end = rest[start..].find(quote)?;
    Some(rest[start..start + end].to_string())
}

pub(crate) fn python_dep_name(dep: &str) -> String {
    dep.split(['=', '<', '>', '~', '!', ';', '['])
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

pub(crate) fn package_kind(package_path: &str) -> &'static str {
    if package_path.is_empty() {
        return "root";
    }
    if package_path.starts_with("apps/") {
        return "app";
    }
    if package_path.starts_with("packages/config") {
        return "config";
    }
    if package_path.starts_with("packages/") {
        return "lib";
    }
    if package_path.starts_with("tools/") || package_path.starts_with("scripts/") {
        return "tool";
    }
    "lib"
}

pub(crate) fn source_roots(package_path: &str, files: &HashSet<String>) -> Vec<String> {
    let prefix = if package_path.is_empty() {
        "src/".to_string()
    } else {
        format!("{package_path}/src/")
    };
    if files.iter().any(|file| file.starts_with(&prefix)) {
        vec![prefix.trim_end_matches('/').to_string()]
    } else {
        Vec::new()
    }
}

pub(crate) fn owning_package(file_path: &str, package_paths: &[String]) -> Option<String> {
    for package_path in package_paths {
        if package_path.is_empty() {
            continue;
        }
        if file_path == package_path || file_path.starts_with(&format!("{package_path}/")) {
            return Some(package_path.clone());
        }
    }
    // root package owns everything else if present
    if package_paths.iter().any(|path| path.is_empty()) {
        return Some(String::new());
    }
    None
}

pub(crate) fn detect_manager(files: &HashSet<String>) -> String {
    if files.contains("pnpm-workspace.yaml") || files.contains("pnpm-lock.yaml") {
        return "pnpm".to_string();
    }
    if files.contains("bun.lock") || files.contains("bun.lockb") {
        return "bun".to_string();
    }
    if files.contains("yarn.lock") {
        return "yarn".to_string();
    }
    if files.contains("package-lock.json") {
        return "npm".to_string();
    }
    if files.contains("Cargo.toml") {
        return "cargo".to_string();
    }
    if files.contains("go.mod") {
        return "go".to_string();
    }
    "unknown".to_string()
}
