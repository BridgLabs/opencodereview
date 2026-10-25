use super::*;

pub(crate) fn extract_symbols(file: &RepoIndexFileInput, content: &str) -> Vec<RepoIndexSymbolInput> {
    let Some(language) = file.language.as_deref() else {
        return Vec::new();
    };
    let lines: Vec<&str> = content.lines().collect();
    let parsed = match language {
        "typescript" | "typescriptreact" | "javascript" | "javascriptreact" => {
            extract_brace_symbols(&lines, ts_symbol_name, "//")
        }
        "rust" => extract_brace_symbols(&lines, rust_symbol_name, "//"),
        "go" => extract_brace_symbols(&lines, go_symbol_name, "//"),
        "python" => extract_python_symbols(&lines),
        _ => Vec::new(),
    };
    parsed
        .into_iter()
        .map(|parsed| RepoIndexSymbolInput {
            name: parsed.name,
            kind: parsed.kind,
            language: language.to_string(),
            path: file.path.clone(),
            package_path: file.package_path.clone(),
            start_line: parsed.start_line,
            end_line: parsed.end_line,
            signature: parsed.signature,
            is_exported: parsed.is_exported,
        })
        .collect()
}

pub(crate) fn extract_brace_symbols(
    lines: &[&str],
    parse: fn(&str) -> Option<(String, String, bool)>,
    comment_prefix: &str,
) -> Vec<ParsedSymbol> {
    let mut symbols = Vec::new();
    let mut brace_depth = 0i64;
    for (index, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        if brace_depth == 0 && !line.starts_with(comment_prefix) {
            if let Some((name, kind, exported)) = parse(line) {
                let start_line = index as i64 + 1;
                symbols.push(ParsedSymbol {
                    name,
                    kind,
                    start_line,
                    end_line: symbol_end_line(lines, index, start_line),
                    signature: Some(line.chars().take(240).collect()),
                    is_exported: exported,
                });
            }
        }
        brace_depth += line.chars().filter(|ch| *ch == '{').count() as i64;
        brace_depth -= line.chars().filter(|ch| *ch == '}').count() as i64;
        brace_depth = brace_depth.max(0);
    }
    symbols
}

pub(crate) fn extract_python_symbols(lines: &[&str]) -> Vec<ParsedSymbol> {
    let mut symbols = Vec::new();
    for (index, raw) in lines.iter().enumerate() {
        let indent = raw.chars().take_while(|ch| ch.is_whitespace()).count();
        let line = raw.trim();
        if indent == 0 && !line.starts_with('#') {
            if let Some((name, kind, exported)) = python_symbol_name(line) {
                let start_line = index as i64 + 1;
                symbols.push(ParsedSymbol {
                    name,
                    kind,
                    start_line,
                    end_line: python_symbol_end_line(lines, index, indent),
                    signature: Some(line.chars().take(240).collect()),
                    is_exported: exported,
                });
            }
        }
    }
    symbols
}

pub(crate) fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Python keywords and very common builtins that appear as `name(` but are not
/// meaningful call-graph targets.
pub(crate) fn is_python_call_noise(name: &str) -> bool {
    if name.starts_with("__") {
        return true;
    }
    matches!(
        name,
        "if" | "elif"
            | "while"
            | "for"
            | "return"
            | "and"
            | "or"
            | "not"
            | "in"
            | "is"
            | "with"
            | "assert"
            | "del"
            | "raise"
            | "yield"
            | "lambda"
            | "await"
            | "else"
            | "try"
            | "except"
            | "finally"
            | "import"
            | "from"
            | "as"
            | "pass"
            | "print"
            | "len"
            | "range"
            | "str"
            | "int"
            | "float"
            | "bool"
            | "list"
            | "dict"
            | "set"
            | "tuple"
            | "type"
            | "isinstance"
            | "issubclass"
            | "super"
            | "getattr"
            | "setattr"
            | "hasattr"
            | "delattr"
            | "enumerate"
            | "zip"
            | "map"
            | "filter"
            | "sorted"
            | "reversed"
            | "min"
            | "max"
            | "sum"
            | "any"
            | "all"
            | "abs"
            | "round"
            | "open"
            | "input"
            | "format"
            | "repr"
            | "id"
            | "hash"
            | "iter"
            | "next"
            | "vars"
            | "dir"
            | "bytes"
            | "bytearray"
            | "frozenset"
            | "property"
            | "staticmethod"
            | "classmethod"
            | "callable"
    )
}

/// Collect candidate called-symbol names inside a Python symbol body (line range
/// 1-indexed, inclusive). Finds `name(` call sites, including the final segment
/// of `obj.method(`, excluding the symbol's own name, keywords/builtins, and
/// nested definition names.
pub(crate) fn python_symbol_calls(lines: &[&str], start_line: i64, end_line: i64, self_name: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    if lines.is_empty() {
        return names;
    }
    let start = start_line.max(1) as usize;
    let end = (end_line.max(start_line) as usize).min(lines.len());
    for line in &lines[start - 1..end] {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        let is_def = trimmed.starts_with("def ") || trimmed.starts_with("async def ") || trimmed.starts_with("class ");
        let bytes = line.as_bytes();
        for i in 0..bytes.len() {
            if bytes[i] != b'(' {
                continue;
            }
            let mut j = i;
            while j > 0 && bytes[j - 1] == b' ' {
                j -= 1;
            }
            let mut k = j;
            while k > 0 && is_ident_byte(bytes[k - 1]) {
                k -= 1;
            }
            if k >= j {
                continue;
            }
            let ident = &line[k..j];
            if !ident
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            {
                continue;
            }
            // Skip a definition's own name (the identifier right after def/class).
            let before = line[..k].trim_end();
            let is_def_name = is_def && (before.ends_with("def") || before.ends_with("class"));
            if is_def_name || ident == self_name || is_python_call_noise(ident) {
                continue;
            }
            if seen.insert(ident.to_string()) {
                names.push(ident.to_string());
            }
        }
    }
    names
}

/// Build symbol-level call/reference edges from per-symbol called-name
/// candidates. A called name resolves to a defined symbol: same-file
/// definitions win; otherwise repo-wide matches are used only when few enough
/// to stay unambiguous. Self-edges and duplicates are dropped.
pub(crate) fn build_symbol_edges(
    symbols: &[RepoIndexSymbolInput],
    symbol_refs: &[Vec<String>],
) -> Vec<RepoIndexSymbolEdgeInput> {
    const MAX_AMBIGUOUS: usize = 2;
    const MAX_EDGES_PER_SYMBOL: usize = 30;
    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (idx, symbol) in symbols.iter().enumerate() {
        by_name.entry(symbol.name.as_str()).or_default().push(idx);
    }
    let mut edges = Vec::new();
    for (from, refs) in symbol_refs.iter().enumerate() {
        let from_path = symbols[from].path.as_str();
        let from_lang = symbols[from].language.as_str();
        let mut seen: HashSet<usize> = HashSet::new();
        'refs: for name in refs {
            let Some(candidates) = by_name.get(name.as_str()) else {
                continue;
            };
            // Only link definitions in the same language: call refs are extracted
            // per language, so a same-named symbol in another language (e.g. a
            // Rust/TS `helper` vs a Python `helper()`) is not a real call target.
            let same_file: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|&j| j != from && symbols[j].language == from_lang && symbols[j].path == from_path)
                .collect();
            let targets = if !same_file.is_empty() {
                same_file
            } else {
                let repo: Vec<usize> = candidates
                    .iter()
                    .copied()
                    .filter(|&j| j != from && symbols[j].language == from_lang)
                    .collect();
                if repo.len() > MAX_AMBIGUOUS {
                    continue;
                }
                repo
            };
            for to in targets {
                if seen.insert(to) {
                    edges.push(RepoIndexSymbolEdgeInput {
                        from_symbol_index: from,
                        to_symbol_index: to,
                        kind: "calls".to_string(),
                    });
                    if seen.len() >= MAX_EDGES_PER_SYMBOL {
                        break 'refs;
                    }
                }
            }
        }
    }
    edges
}

pub(crate) fn ts_symbol_name(line: &str) -> Option<(String, String, bool)> {
    let exported = line.starts_with("export ");
    let mut rest = line.strip_prefix("export ").unwrap_or(line).trim_start();
    rest = rest.strip_prefix("default ").unwrap_or(rest).trim_start();
    for (prefix, kind) in [
        ("async function ", "function"),
        ("function ", "function"),
        ("class ", "class"),
        ("interface ", "interface"),
        ("type ", "type"),
        ("enum ", "enum"),
    ] {
        if let Some(after) = rest.strip_prefix(prefix) {
            return Some((identifier_prefix(after)?, kind.to_string(), exported));
        }
    }
    if exported {
        for prefix in ["const ", "let ", "var "] {
            if let Some(after) = rest.strip_prefix(prefix) {
                return Some((identifier_prefix(after)?, "const".to_string(), true));
            }
        }
    }
    None
}

pub(crate) fn rust_symbol_name(line: &str) -> Option<(String, String, bool)> {
    let exported = line.starts_with("pub ") || line.starts_with("pub(");
    let rest = if let Some(rest) = line.strip_prefix("pub ") {
        rest
    } else if let Some(rest) = line.strip_prefix("pub(") {
        rest.split_once(')')?.1
    } else {
        line
    }
    .trim_start();
    let rest = rest.strip_prefix("async ").unwrap_or(rest).trim_start();
    for (prefix, kind) in [
        ("fn ", "function"),
        ("struct ", "struct"),
        ("enum ", "enum"),
        ("trait ", "trait"),
        ("type ", "type"),
        ("mod ", "module"),
        ("const ", "const"),
        ("static ", "static"),
    ] {
        if let Some(after) = rest.strip_prefix(prefix) {
            return Some((identifier_prefix(after)?, kind.to_string(), exported));
        }
    }
    None
}

pub(crate) fn go_symbol_name(line: &str) -> Option<(String, String, bool)> {
    if let Some(after) = line.strip_prefix("func ") {
        let after = after
            .strip_prefix('(')
            .and_then(|rest| rest.split_once(')').map(|(_, tail)| tail.trim_start()))
            .unwrap_or(after);
        let name = identifier_prefix(after)?;
        let exported = name.chars().next().map(char::is_uppercase).unwrap_or(false);
        return Some((name, "function".to_string(), exported));
    }
    if let Some(after) = line.strip_prefix("type ") {
        let name = identifier_prefix(after)?;
        let exported = name.chars().next().map(char::is_uppercase).unwrap_or(false);
        let tail = after[name.len()..].trim_start();
        let kind = if tail.starts_with("interface") {
            "interface"
        } else if tail.starts_with("struct") {
            "struct"
        } else {
            "type"
        };
        return Some((name, kind.to_string(), exported));
    }
    None
}

pub(crate) fn python_symbol_name(line: &str) -> Option<(String, String, bool)> {
    for (prefix, kind) in [("async def ", "function"), ("def ", "function"), ("class ", "class")] {
        if let Some(after) = line.strip_prefix(prefix) {
            let name = identifier_prefix(after)?;
            return Some((name.clone(), kind.to_string(), !name.starts_with('_')));
        }
    }
    None
}

pub(crate) fn identifier_prefix(text: &str) -> Option<String> {
    let name: String = text
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '$')
        .collect();
    (!name.is_empty()).then_some(name)
}

pub(crate) fn symbol_end_line(lines: &[&str], start: usize, fallback: i64) -> i64 {
    let mut depth = 0i64;
    let mut seen_open = false;
    for (index, line) in lines.iter().enumerate().skip(start) {
        for ch in line.chars() {
            if ch == '{' {
                depth += 1;
                seen_open = true;
            } else if ch == '}' {
                depth -= 1;
                if seen_open && depth <= 0 {
                    return index as i64 + 1;
                }
            } else if ch == ';' && !seen_open {
                return index as i64 + 1;
            }
        }
    }
    fallback
}

pub(crate) fn python_symbol_end_line(lines: &[&str], start: usize, indent: usize) -> i64 {
    for (index, raw) in lines.iter().enumerate().skip(start + 1) {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let current = raw.chars().take_while(|ch| ch.is_whitespace()).count();
        if current <= indent {
            return index as i64;
        }
    }
    lines.len() as i64
}
