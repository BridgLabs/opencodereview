use std::path::Path;

const ERROR: &str =
    "Frozen reviews permit only bounded snapshot reads, fixed-text searches and directory listings through exec";

pub(super) const INSTRUCTIONS: &str = "\nSnapshot access through the Bash tool is limited to these exact command forms:\n/usr/bin/sed -n '1,120p' -- '/work/context/PR.md'\n/usr/bin/grep -r -n -F -- 'literal text' '/evidence/snapshot/trees' | /usr/bin/head -n 80\n/usr/bin/ls -1A -- '/work/context'\nUse absolute paths within /work or /evidence/snapshot. For sed, choose positive start/end line numbers spanning at most 200 lines. Quote arguments with single quotes. Use one foreground Bash call per command, without background execution or sandbox overrides. These snapshot-reading commands are permitted; project commands, other shell operations, network and writes remain forbidden. Read assigned files before adding them to reviewedFiles.\n";

pub(super) fn validate_input(input: &serde_json::Value, roots: &[&Path]) -> Result<(), String> {
    let input = input.as_object().ok_or(ERROR)?;
    let disabled = |key: &str| {
        input
            .get(key)
            .is_some_and(|value| !value.is_null() && value.as_bool() != Some(false))
    };
    if input.keys().any(|key| {
        ![
            "command",
            "description",
            "timeout",
            "run_in_background",
            "dangerouslyDisableSandbox",
        ]
        .contains(&key.as_str())
    }) || disabled("run_in_background")
        || disabled("dangerouslyDisableSandbox")
        || input.get("timeout").is_some_and(|value| {
            !value.is_null() && !value.as_u64().is_some_and(|timeout| (1..=30_000).contains(&timeout))
        })
    {
        return Err(ERROR.into());
    }
    validate(
        input.get("command").and_then(|value| value.as_str()).ok_or(ERROR)?,
        roots,
    )
}

// Accept only single-quoted arguments and the standard escaped apostrophe.
fn quoted(input: &str) -> Result<(String, &str), String> {
    let mut remaining = input.strip_prefix('\'').ok_or(ERROR)?;
    let mut value = String::new();
    loop {
        let end = remaining.find('\'').ok_or(ERROR)?;
        value.push_str(&remaining[..end]);
        remaining = &remaining[end + 1..];
        if let Some(next) = remaining.strip_prefix("\\''") {
            value.push('\'');
            remaining = next;
        } else {
            return Ok((value, remaining));
        }
    }
}

pub(super) fn validate(command: &str, roots: &[&Path]) -> Result<(), String> {
    if command.len() > 8192 || command.chars().any(char::is_control) {
        return Err(ERROR.into());
    }
    let (path, tail) = if let Some(arguments) = command.strip_prefix("/usr/bin/sed -n ") {
        let (range, tail) = quoted(arguments)?;
        let (start, end) = range
            .strip_suffix('p')
            .and_then(|range| range.split_once(','))
            .ok_or(ERROR)?;
        if !start.bytes().chain(end.bytes()).all(|byte| byte.is_ascii_digit()) {
            return Err(ERROR.into());
        }
        let start = start.parse::<u32>().map_err(|_| ERROR)?;
        let end = end.parse::<u32>().map_err(|_| ERROR)?;
        if start == 0 || end < start || end - start >= 200 || end > 1_000_000 {
            return Err(ERROR.into());
        }
        quoted(tail.strip_prefix(" -- ").ok_or(ERROR)?)?
    } else if let Some(arguments) = command.strip_prefix("/usr/bin/grep -r -n -F -- ") {
        let (query, tail) = quoted(arguments)?;
        if query.is_empty() || query.len() > 256 {
            return Err(ERROR.into());
        }
        let (path, tail) = quoted(tail.strip_prefix(' ').ok_or(ERROR)?)?;
        (path, tail.strip_suffix(" | /usr/bin/head -n 80").ok_or(ERROR)?)
    } else if let Some(arguments) = command.strip_prefix("/usr/bin/ls -1A -- ") {
        quoted(arguments)?
    } else {
        return Err(ERROR.into());
    };
    if !tail.is_empty()
        || path.is_empty()
        || !Path::new(&path).is_absolute()
        || Path::new(&path)
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(ERROR.into());
    }
    let path = Path::new(&path).canonicalize().map_err(|_| ERROR)?;
    if !roots
        .iter()
        .any(|root| root.canonicalize().is_ok_and(|root| path.starts_with(root)))
    {
        return Err(ERROR.into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "read_commands_tests.rs"]
mod tests;
