use super::*;

/// Ruby has no braces, so blocks are tracked by the keyword that opens them and the `end` that
/// closes them. The scan is line based and approximate, like the other lightweight extractors:
/// it can be wrong inside heredocs and unusual one-liners, but it never fails.
pub(crate) fn extract_ruby_symbols(lines: &[&str]) -> Vec<ParsedSymbol> {
    let mut symbols: Vec<ParsedSymbol> = Vec::new();
    // One entry per open block: the index of its symbol, or None for blocks that are not symbols.
    let mut stack: Vec<Option<usize>> = Vec::new();
    let mut in_block_comment = false;
    for (index, raw) in lines.iter().enumerate() {
        // `=begin` starts a comment only at column zero. A trailing comment or a string must not
        // open or close a block: `do |item| # note` still needs its `end`.
        if raw.starts_with("=begin") {
            in_block_comment = true;
        }
        if in_block_comment {
            in_block_comment = !raw.starts_with("=end");
            continue;
        }
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let code_owned = strip_ruby_strings(line);
        let code = code_owned.trim_end();
        if code.is_empty() {
            continue;
        }
        let line_no = index as i64 + 1;
        if is_ruby_end(code) {
            if let Some(Some(symbol)) = stack.pop() {
                symbols[symbol].end_line = line_no;
            }
            continue;
        }
        if let Some((name, kind, endless)) = ruby_symbol(code) {
            // A `def` inside a class or module is a method; a top-level `def` is a function.
            let inside_type = stack
                .iter()
                .flatten()
                .any(|open| matches!(symbols[*open].kind.as_str(), "class" | "module"));
            let kind = if kind == "function" && inside_type {
                "method".to_string()
            } else {
                kind
            };
            symbols.push(ParsedSymbol {
                is_exported: !name.starts_with('_'),
                name,
                kind,
                start_line: line_no,
                end_line: line_no,
                signature: Some(line.chars().take(240).collect()),
            });
            if !endless && !closes_on_same_line(code) {
                stack.push(Some(symbols.len() - 1));
            }
        } else if opens_ruby_block(code) && !closes_on_same_line(code) {
            stack.push(None);
        }
    }
    // A block still open at the end of the file runs to the last line.
    let last = lines.len() as i64;
    for symbol in stack.into_iter().flatten() {
        symbols[symbol].end_line = last;
    }
    symbols
}

/// For each symbol `(start line, end line, name)`, in one pass over the symbols in file order: the line ranges of its direct
/// children and the names of the symbols around it. Checking every pair of symbols instead would
/// take quadratic time on a file with thousands of methods.
pub(crate) fn ruby_symbol_scopes(symbols: &[(i64, i64, &str)]) -> Vec<(Vec<(i64, i64)>, Vec<String>)> {
    let mut parents: Vec<Option<usize>> = Vec::with_capacity(symbols.len());
    let mut children: Vec<Vec<(i64, i64)>> = vec![Vec::new(); symbols.len()];
    let mut open: Vec<usize> = Vec::new();
    for (index, &(start, end, _)) in symbols.iter().enumerate() {
        while let Some(&top) = open.last() {
            if symbols[top].0 < start && symbols[top].1 >= end {
                break;
            }
            open.pop();
        }
        let parent = open.last().copied();
        if let Some(parent) = parent {
            children[parent].push((start, end));
        }
        parents.push(parent);
        open.push(index);
    }
    children
        .into_iter()
        .enumerate()
        .map(|(index, nested)| {
            let mut enclosing = Vec::new();
            let mut up = parents[index];
            while let Some(parent) = up {
                enclosing.push(symbols[parent].2.to_string());
                up = parents[parent];
            }
            (nested, enclosing)
        })
        .collect()
}

fn is_ruby_end(line: &str) -> bool {
    line.strip_prefix("end")
        .is_some_and(|rest| !rest.starts_with(|ch: char| ch.is_alphanumeric() || ch == '_'))
}

fn closes_on_same_line(line: &str) -> bool {
    line.ends_with(" end") || line.ends_with(";end") || line.ends_with("; end")
}

/// Whether the line starts a block that needs an `end`, other than a class, module or method.
fn opens_ruby_block(line: &str) -> bool {
    let first = line
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .next()
        .unwrap_or_default();
    // A keyword at the start of a line is never a trailing modifier (`return x if y`).
    if matches!(first, "if" | "unless" | "while" | "until" | "case" | "begin" | "for") || line.starts_with("class <<") {
        return true;
    }
    // `value = if ...` and friends.
    if ["= if ", "= unless ", "= case ", "= while ", "= until "]
        .iter()
        .any(|pattern| line.contains(pattern))
        || line.ends_with("= begin")
        || line.ends_with("= case")
    {
        return true;
    }
    line.ends_with(" do") || (line.ends_with('|') && line.contains(" do |"))
}

/// Name, kind and whether the method is an endless `def name = expression`. A class or module
/// declared as `class Admin::Users` is named `Users`, because constants are looked up by their
/// last segment (`Admin::Users.new` refers to `Users`).
fn ruby_symbol(line: &str) -> Option<(String, String, bool)> {
    let (name, kind, endless) = ruby_declaration(line)?;
    if kind == "function" {
        return Some((name, kind, endless));
    }
    let last = name.rsplit("::").find(|part| !part.is_empty())?.to_string();
    Some((last, kind, endless))
}

/// Like `ruby_symbol`, but a class or module keeps its full qualified name.
fn ruby_declaration(line: &str) -> Option<(String, String, bool)> {
    let mut line = line;
    for modifier in [
        "private ",
        "protected ",
        "public ",
        "module_function ",
        "private_class_method ",
    ] {
        if let Some(rest) = line.strip_prefix(modifier) {
            line = rest.trim_start();
            break;
        }
    }
    for (keyword, kind) in [("class ", "class"), ("module ", "module")] {
        if let Some(after) = line.strip_prefix(keyword) {
            if after.starts_with('<') {
                return None;
            }
            let name: String = after
                .chars()
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == ':')
                .collect();
            return (!name.is_empty()).then(|| (name, kind.to_string(), false));
        }
    }
    let after = line.strip_prefix("def ")?.trim_start();
    // `def self.call` and `def Config.load` name the method, not its receiver.
    let after = match after.split_once('.') {
        Some((receiver, rest)) if receiver.chars().all(|ch| ch.is_alphanumeric() || ch == '_') => rest,
        _ => after,
    };
    let name: String = after
        .chars()
        .take_while(|ch| !ch.is_whitespace() && *ch != '(' && *ch != ';')
        .collect();
    if name.is_empty() {
        return None;
    }
    let mut rest = after[name.len()..].trim_start();
    if rest.starts_with('(') {
        let mut depth = 0;
        for (offset, ch) in rest.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        rest = rest[offset + 1..].trim_start();
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    let endless = rest.starts_with('=') && !rest.starts_with("==");
    Some((name, "function".to_string(), endless))
}

/// `require`, `require_relative` and `load` with a literal path. `require_relative` paths are
/// written with a leading `./` so that they resolve against the importing file.
pub(crate) fn extract_ruby_imports(content: &str) -> Vec<(String, String)> {
    let mut imports = Vec::new();
    let mut in_block_comment = false;
    for raw in content.lines() {
        if raw.starts_with("=begin") {
            in_block_comment = true;
        }
        if in_block_comment {
            in_block_comment = !raw.starts_with("=end");
            continue;
        }
        let line = raw.trim();
        if line.starts_with('#') {
            continue;
        }
        for keyword in ["require_relative", "require", "load"] {
            let Some(rest) = line.strip_prefix(keyword) else {
                continue;
            };
            if !rest.starts_with([' ', '(']) {
                continue;
            }
            let rest = rest.trim_start_matches(['(', ' ']);
            let Some(quote) = rest.chars().next().filter(|ch| *ch == '\'' || *ch == '"') else {
                break;
            };
            let Some(spec) = rest[1..].split(quote).next().filter(|spec| !spec.is_empty()) else {
                break;
            };
            if keyword == "require_relative" && !spec.starts_with('.') {
                imports.push((format!("./{spec}"), keyword.to_string()));
            } else {
                imports.push((spec.to_string(), keyword.to_string()));
            }
            break;
        }
    }
    imports
}

/// Resolves a load-path or relative `require` to a tracked `.rb` file.
pub(crate) fn resolve_ruby_spec(from_path: &str, spec: &str, files: &HashSet<String>) -> Option<String> {
    if spec.starts_with('.') {
        let base = normalize_path(&format!("{}/{}", parent_dir(from_path), spec));
        let with_rb = if base.ends_with(".rb") {
            base
        } else {
            format!("{base}.rb")
        };
        return files.contains(&with_rb).then_some(with_rb);
    }
    if spec.starts_with('/') {
        return None;
    }
    let file = if spec.ends_with(".rb") {
        spec.to_string()
    } else {
        format!("{spec}.rb")
    };
    ["lib", "app", "", "spec", "test"].iter().find_map(|root| {
        let candidate = if root.is_empty() {
            file.clone()
        } else {
            format!("{root}/{file}")
        };
        files.contains(&candidate).then_some(candidate)
    })
}

/// Names a Ruby symbol's body may call or refer to: methods called on a receiver (`order.total`),
/// calls with parentheses (`build(item)`) and constants (`Billing::Invoice.new`). Bare lowercase
/// words are skipped because Ruby cannot tell a local variable from a method call without them
/// being resolved; the symbol edge builder drops names that match no or too many definitions.
///
/// A class or module only owns the lines that no nested symbol owns, and a symbol never refers to
/// the namespaces it sits in (`nested` and `enclosing` carry that). `nested` holds the line ranges
/// of the symbol's direct children, in file order and without overlap (`ruby_symbol_scopes`).
pub(crate) fn ruby_symbol_calls(
    lines: &[&str],
    start_line: i64,
    end_line: i64,
    self_name: &str,
    nested: &[(i64, i64)],
    enclosing: &[&str],
) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    if lines.is_empty() {
        return names;
    }
    let start = start_line.max(1) as usize;
    let end = (end_line.max(start_line) as usize).min(lines.len());
    let mut nested_next = 0;
    let mut in_block_comment = false;
    for (offset, raw) in lines[start - 1..end].iter().enumerate() {
        let line_no = (start + offset) as i64;
        // `nested` is sorted and does not overlap, so one moving index replaces a scan per line.
        while nested_next < nested.len() && nested[nested_next].1 < line_no {
            nested_next += 1;
        }
        if nested.get(nested_next).is_some_and(|(from, _)| line_no >= *from) {
            continue;
        }
        // `=begin` starts a comment only at column zero, matching extract_ruby_symbols.
        if raw.starts_with("=begin") {
            in_block_comment = true;
        }
        if in_block_comment {
            in_block_comment = !raw.starts_with("=end");
            continue;
        }
        let line = raw.trim();
        if line.starts_with('#') {
            continue;
        }
        // Comments and quoted text are not calls. `Skipped.call` in either must not link.
        let code_owned = strip_ruby_strings(line);
        let line = code_owned.trim_end();
        if line.is_empty() {
            continue;
        }
        // The first line of a symbol holds its own name, which is not a call.
        let own_name = if offset == 0 {
            ruby_declaration(line).map(|(name, _, _)| name)
        } else {
            None
        };
        let bytes = line.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if !(is_ident_byte(bytes[index]) && (index == 0 || !is_ident_byte(bytes[index - 1]))) {
                index += 1;
                continue;
            }
            let mut stop = index;
            while stop < bytes.len() && is_ident_byte(bytes[stop]) {
                stop += 1;
            }
            if stop < bytes.len() && (bytes[stop] == b'?' || bytes[stop] == b'!') {
                stop += 1;
            }
            let word = &line[index..stop];
            let after_dot = line[..index].ends_with('.') && !line[..index].ends_with("..");
            let called = bytes.get(stop) == Some(&b'(');
            let constant = word.chars().next().is_some_and(|ch| ch.is_ascii_uppercase());
            let is_call = after_dot || called || constant;
            let is_own = own_name
                .as_deref()
                .is_some_and(|name| name.split("::").any(|part| part == word));
            if is_call
                && !is_own
                && word != self_name
                && !enclosing.contains(&word)
                && !is_ruby_call_noise(word)
                && seen.insert(word.to_string())
            {
                names.push(word.to_string());
            }
            index = stop;
        }
    }
    names
}

/// Methods that nearly every Ruby program calls on core types or on procs. Linking them by name
/// would tie unrelated code to the one or two repository methods that share the name.
fn is_ruby_call_noise(word: &str) -> bool {
    matches!(
        word,
        "new"
            | "initialize"
            | "puts"
            | "print"
            | "p"
            | "require"
            | "require_relative"
            | "raise"
            | "super"
            | "lambda"
            | "proc"
            | "call"
            | "each"
            | "map"
            | "select"
            | "reject"
            | "find"
            | "first"
            | "last"
            | "count"
            | "size"
            | "length"
            | "name"
            | "id"
            | "save"
            | "to_s"
            | "to_a"
            | "to_h"
            | "to_i"
            | "to_sym"
            | "fetch"
            | "join"
            | "merge"
            | "dup"
            | "freeze"
            | "tap"
            | "then"
            | "send"
            | "inspect"
            | "empty?"
            | "nil?"
            | "present?"
            | "blank?"
            | "include?"
            | "key?"
            | "is_a?"
            | "respond_to?"
    ) || word.starts_with(|ch: char| ch.is_ascii_digit())
}

/// The first quoted string on a line: `gem "rails", "~> 7"` gives `rails`.
pub(crate) fn first_quoted(text: &str) -> Option<String> {
    let start = text.find(['\'', '"'])?;
    let quote = text[start..].chars().next()?;
    let rest = &text[start + 1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

/// A `Gemfile`: every `gem` line is a dependency, and the ones in a development or test group
/// are dev dependencies. A Gemfile declares no name, so the caller names the package.
pub(crate) fn parse_gemfile(content: &[u8], name: String) -> ParsedManifest {
    let text = String::from_utf8_lossy(content);
    let mut deps = Vec::new();
    // One flag per open `do` block: whether it is a development or test group.
    let mut blocks: Vec<bool> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('#') {
            continue;
        }
        // A comment after `do` (`group :development do # tools`) is still a block, and a comment
        // that mentions `:test` does not make the gem a dev dependency. Quoted names stay, so
        // `gem "puma"` is still a gem line.
        let structural = without_ruby_comment(line).trim();
        if structural.is_empty() {
            continue;
        }
        let dev_words = structural.contains(":development") || structural.contains(":test");
        if structural.ends_with(" do") || (structural.ends_with('|') && structural.contains(" do |")) {
            blocks.push(structural.starts_with("group") && dev_words);
        } else if is_ruby_end(structural) {
            blocks.pop();
        } else if opens_ruby_block(structural) && !closes_on_same_line(structural) {
            // An `if`, `unless` or `case` has its own `end`, which must not close a group.
            blocks.push(false);
        } else if let Some(rest) = structural.strip_prefix("gem ") {
            if let Some(dep) = first_quoted(rest) {
                let dev = blocks.iter().any(|dev| *dev) || (dev_words && rest.contains("group"));
                deps.push((dep, dev));
            }
        }
    }
    ParsedManifest { name, deps }
}

/// A `*.gemspec`: the gem's `name` and its runtime and development dependencies.
pub(crate) fn parse_gemspec(content: &[u8]) -> Option<ParsedManifest> {
    let text = String::from_utf8_lossy(content);
    let mut name = None;
    let mut deps = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some((_, after)) = line.split_once(".name") {
            let after = after.trim_start();
            if after.starts_with('=') && !after.starts_with("==") && name.is_none() {
                name = first_quoted(after);
            }
        }
        // `Gem::Specification.new "tool", VERSION do |spec|` gives the name as an argument.
        if name.is_none() {
            if let Some((_, after)) = line.split_once("Specification.new") {
                let after = after.trim_start_matches(['(', ' ']);
                if after.starts_with(['\'', '"']) {
                    name = first_quoted(after);
                }
            }
        }
        for (method, dev) in [
            ("add_dependency", false),
            ("add_runtime_dependency", false),
            ("add_development_dependency", true),
        ] {
            if let Some((_, after)) = line.split_once(method) {
                if let Some(dep) = first_quoted(after) {
                    deps.push((dep, dev));
                }
            }
        }
    }
    Some(ParsedManifest { name: name?, deps })
}

/// Constants a Ruby file mentions, with string contents and comments left out.
pub(crate) fn ruby_constant_refs(content: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut in_block_comment = false;
    for raw in content.lines() {
        if raw.starts_with("=begin") {
            in_block_comment = true;
        }
        if in_block_comment {
            in_block_comment = !raw.starts_with("=end");
            continue;
        }
        let code = strip_ruby_strings(raw);
        let bytes = code.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if !(is_ident_byte(bytes[index]) && (index == 0 || !is_ident_byte(bytes[index - 1]))) {
                index += 1;
                continue;
            }
            let mut stop = index;
            while stop < bytes.len() && is_ident_byte(bytes[stop]) {
                stop += 1;
            }
            if bytes[index].is_ascii_uppercase() && seen.insert(code[index..stop].to_string()) {
                names.push(code[index..stop].to_string());
            }
            index = stop;
        }
    }
    names
}

/// The line up to a comment. A `#` inside quotes is part of the string.
fn without_ruby_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if let Some(open) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == open {
                quote = None;
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch == '#' {
            return &line[..index];
        }
    }
    line
}

/// The code of a line: quoted text is blanked and a trailing comment is cut off.
fn strip_ruby_strings(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in line.chars() {
        match quote {
            Some(open) => {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == open {
                    quote = None;
                }
                out.push(' ');
            }
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
                out.push(' ');
            }
            None if ch == '#' => break,
            None => out.push(ch),
        }
    }
    out
}

/// File-to-file links for Ruby code that reaches another file by constant name instead of
/// `require` (Rails and Zeitwerk load files that way). A constant links only when exactly one
/// other file defines a class or module of that name, so shared namespaces link nothing.
pub(crate) fn ruby_constant_edges(
    symbols: &[RepoIndexSymbolInput],
    refs: &[(String, Vec<String>)],
    existing: &[RepoIndexEdgeInput],
) -> Vec<RepoIndexEdgeInput> {
    let mut definers: HashMap<&str, HashSet<&str>> = HashMap::new();
    for symbol in symbols {
        if symbol.language == "ruby" && matches!(symbol.kind.as_str(), "class" | "module") {
            definers
                .entry(symbol.name.as_str())
                .or_default()
                .insert(symbol.path.as_str());
        }
    }
    let mut linked: HashSet<(&str, &str)> = existing
        .iter()
        .filter_map(|edge| Some((edge.from_path.as_str(), edge.to_path.as_deref()?)))
        .collect();
    let mut edges = Vec::new();
    for (path, names) in refs {
        for name in names {
            let Some(paths) = definers.get(name.as_str()) else {
                continue;
            };
            let mut paths = paths.iter();
            let (Some(target), None) = (paths.next(), paths.next()) else {
                continue;
            };
            if *target != path.as_str() && linked.insert((path.as_str(), target)) {
                edges.push(RepoIndexEdgeInput {
                    from_path: path.clone(),
                    to_ref: name.clone(),
                    kind: "constant".to_string(),
                    resolved: true,
                    to_path: Some((*target).to_string()),
                });
            }
        }
    }
    edges
}
