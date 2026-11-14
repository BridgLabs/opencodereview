use super::*;

/// Strip `//` and `/* */` comments and trailing commas so serde_json can parse JSONC tsconfig files.
pub(crate) fn strip_jsonc(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut in_string = false;
    let mut escaped = false;
    while i < bytes.len() {
        let byte = bytes[i];
        if in_string {
            out.push(byte as char);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
            out.push('"');
            i += 1;
            continue;
        }
        if byte == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if byte == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        out.push(byte as char);
        i += 1;
    }
    // remove trailing commas: `,` followed by whitespace then } or ]
    let mut cleaned = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    let mut j = 0;
    while j < chars.len() {
        if chars[j] == ',' {
            let mut k = j + 1;
            while k < chars.len() && chars[k].is_whitespace() {
                k += 1;
            }
            if k < chars.len() && (chars[k] == '}' || chars[k] == ']') {
                j += 1;
                continue;
            }
        }
        cleaned.push(chars[j]);
        j += 1;
    }
    cleaned
}

pub(crate) fn is_code_language(language: &str) -> bool {
    matches!(
        language,
        "typescript" | "typescriptreact" | "javascript" | "javascriptreact" | "rust" | "python" | "go" | "ruby"
    )
}

pub(crate) fn language_for_path(path: &str) -> Option<&'static str> {
    let ext = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "ts" => Some("typescript"),
        "tsx" => Some("typescriptreact"),
        "js" | "mjs" | "cjs" => Some("javascript"),
        "jsx" => Some("javascriptreact"),
        "rs" => Some("rust"),
        "py" => Some("python"),
        "go" => Some("go"),
        "rb" | "rake" | "gemspec" | "ru" => Some("ruby"),
        "css" | "less" | "scss" => Some("styles"),
        "md" | "mdx" => Some("markdown"),
        "json" => Some("json"),
        "toml" => Some("toml"),
        "yaml" | "yml" => Some("yaml"),
        "html" | "hbs" => Some("markup"),
        _ => None,
    }
}

pub(crate) fn is_fts_eligible_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with("pnpm-lock.yaml")
        || lower.ends_with("package-lock.json")
        || lower.ends_with("yarn.lock")
        || lower.ends_with("cargo.lock")
    {
        return false;
    }
    matches!(
        Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some(
            "ts" | "tsx"
                | "js"
                | "jsx"
                | "rs"
                | "py"
                | "go"
                | "rb"
                | "rake"
                | "gemspec"
                | "ru"
                | "css"
                | "scss"
                | "md"
                | "mdx"
                | "json"
                | "toml"
                | "yaml"
                | "yml"
                | "html"
        )
    )
}

pub(crate) fn is_binary_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".pdf", ".zip", ".ttf", ".otf", ".woff", ".woff2", ".mp3",
        ".mp4", ".wasm", ".bin",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

pub(crate) fn is_generated_path(path: &str) -> bool {
    crate::path_kinds::is_generated_path(path) || crate::path_kinds::is_lockfile(path)
}
