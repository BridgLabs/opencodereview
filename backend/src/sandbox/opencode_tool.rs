use serde::Deserialize;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};

const DENIED: &str = "OpenCode tool request denied";
const MAX_INPUT: u64 = 512 * 1024;
const MAX_FILE: u64 = 256 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    tool: String,
    input: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadInput {
    path: String,
    #[serde(default = "first_line")]
    offset: usize,
    #[serde(default = "line_limit")]
    limit: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchInput {
    path: String,
    pattern: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteInput {
    path: String,
    content: String,
}

fn first_line() -> usize {
    1
}

fn line_limit() -> usize {
    200
}

/// One request per process. The plugin bounds process lifetime and stdout independently.
pub fn serve(input: impl Read) -> i32 {
    let (status, response) = match execute(input) {
        Ok(content) => (0, serde_json::json!({"v":1,"status":"ok","content":content})),
        Err(Failure::Rejected) => (2, serde_json::json!({"v":1,"status":"rejected","code":"request"})),
        Err(Failure::Fault) => (70, serde_json::json!({"v":1,"status":"fault","code":"internal"})),
    };
    if writeln!(std::io::stdout().lock(), "{response}").is_err() {
        return 70;
    }
    status
}

#[derive(Debug, PartialEq)]
enum Failure {
    Rejected,
    Fault,
}

fn execute(input: impl Read) -> Result<String, Failure> {
    let mut bytes = Vec::new();
    input
        .take(MAX_INPUT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::Fault)?;
    if bytes.len() as u64 > MAX_INPUT {
        return Err(Failure::Rejected);
    }
    let request: Request = serde_json::from_slice(&bytes).map_err(|_| Failure::Rejected)?;
    operate(request).map_err(|_| Failure::Rejected)
}

fn operate(request: Request) -> Result<String, String> {
    if request.tool == "ocr_write" {
        let input: WriteInput = serde_json::from_value(request.input).map_err(|_| DENIED)?;
        if input.content.len() > 64 * 1024 {
            return Err(DENIED.into());
        }
        guard(
            "Write",
            serde_json::json!({"file_path": input.path, "content": input.content}),
        )?;
        return write(&input);
    }
    if request.tool == "ocr_bash" {
        guard("Bash", request.input.clone())?;
        let command = request.input["command"].as_str().ok_or(DENIED)?;
        let operation =
            super::runner::read_commands::parse(command, &[Path::new("/work"), Path::new("/evidence/snapshot")])
                .map_err(|_| DENIED)?;
        // Interpret the shared validated plan. No shell or external process receives model text.
        return match operation {
            super::runner::read_commands::SnapshotCommand::Read { path, offset, limit } => Ok(text(&path)?
                .split_inclusive('\n')
                .skip(offset - 1)
                .take(limit)
                .collect()),
            super::runner::read_commands::SnapshotCommand::Search { path, pattern, limit } => {
                search(&path, &pattern, limit)
            }
            super::runner::read_commands::SnapshotCommand::List { path } => {
                let directory = open(&path)?;
                if !directory.metadata().map_err(|_| DENIED)?.is_dir() {
                    return Err(DENIED.into());
                }
                let mut names = Vec::new();
                for entry in
                    std::fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd())).map_err(|_| DENIED)?
                {
                    if names.len() >= 4096 {
                        return Err(DENIED.into());
                    }
                    let name = entry
                        .map_err(|_| DENIED)?
                        .file_name()
                        .into_string()
                        .map_err(|_| DENIED)?;
                    if name.chars().any(char::is_control) {
                        return Err(DENIED.into());
                    }
                    names.push(name);
                }
                names.sort();
                let output = names.into_iter().map(|name| name + "\n").collect::<String>();
                if output.len() > MAX_FILE as usize {
                    return Err(DENIED.into());
                }
                Ok(output)
            }
        };
    }
    if matches!(request.tool.as_str(), "ocr_grep" | "ocr_glob") {
        let input: SearchInput = serde_json::from_value(request.input).map_err(|_| DENIED)?;
        if input.pattern.is_empty() || input.pattern.len() > 256 || input.pattern.chars().any(char::is_control) {
            return Err(DENIED.into());
        }
        if request.tool == "ocr_grep" {
            guard("Grep", serde_json::json!({"path": input.path}))?;
            return search(Path::new(&input.path), &input.pattern, 80);
        }
        guard("Glob", serde_json::json!({"path": input.path}))?;
        return glob(Path::new(&input.path), &input.pattern);
    }
    if request.tool != "ocr_read" {
        return Err(DENIED.into());
    }
    let input: ReadInput = serde_json::from_value(request.input).map_err(|_| DENIED)?;
    if input.path.len() > 4096
        || input.path.chars().any(char::is_control)
        || input.path.contains('\\')
        || input.offset == 0
        || input.offset > 1_000_000
        || !(1..=200).contains(&input.limit)
    {
        return Err(DENIED.into());
    }
    let event = serde_json::to_vec(&serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Read",
        "tool_input": {"file_path": input.path},
    }))
    .map_err(|_| DENIED)?;
    super::guard_tool(event.as_slice()).map_err(|_| DENIED)?;
    // The archive opener walks every component with openat(O_NOFOLLOW), including parents.
    // Keep this descriptor: canonicalize followed by open would reintroduce a symlink race.
    let file = crate::review_archive::open_archive(Path::new(&input.path), false).map_err(|_| DENIED)?;
    let metadata = file.metadata().map_err(|_| DENIED)?;
    if !metadata.is_file() || metadata.len() > MAX_FILE {
        return Err(DENIED.into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes).map_err(|_| DENIED)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(DENIED.into());
    }
    let text = String::from_utf8(bytes).map_err(|_| DENIED)?;
    Ok(text
        .split_inclusive('\n')
        .skip(input.offset - 1)
        .take(input.limit)
        .collect())
}

fn guard(tool: &str, input: serde_json::Value) -> Result<(), String> {
    let event = serde_json::to_vec(&serde_json::json!({
        "hook_event_name": "PreToolUse", "tool_name": tool, "tool_input": input,
    }))
    .map_err(|_| DENIED)?;
    super::guard_tool(event.as_slice()).map_err(|_| DENIED.into())
}

fn open(path: &Path) -> Result<std::fs::File, String> {
    let text = path.to_str().ok_or(DENIED)?;
    if text.len() > 4096 || text.chars().any(char::is_control) || text.contains('\\') {
        return Err(DENIED.into());
    }
    crate::review_archive::open_archive(path, false).map_err(|_| DENIED.into())
}

fn text(path: &Path) -> Result<String, String> {
    let file = open(path)?;
    let metadata = file.metadata().map_err(|_| DENIED)?;
    if !metadata.is_file() || metadata.len() > MAX_FILE {
        return Err(DENIED.into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes).map_err(|_| DENIED)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(DENIED.into());
    }
    String::from_utf8(bytes).map_err(|_| DENIED.into())
}

// Traverse only descriptor-opened directories. Each eventual file open walks from / with
// O_NOFOLLOW again, so replacing a checked entry with a symlink cannot escape the roots.
fn files(path: &Path, depth: usize, entries: &mut usize, result: &mut Vec<PathBuf>) -> Result<(), String> {
    if depth > 32 {
        return Err(DENIED.into());
    }
    let file = open(path)?;
    let metadata = file.metadata().map_err(|_| DENIED)?;
    if metadata.is_file() {
        result.push(path.to_owned());
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(DENIED.into());
    }
    let mut children = Vec::new();
    for child in std::fs::read_dir(format!("/proc/self/fd/{}", file.as_raw_fd())).map_err(|_| DENIED)? {
        *entries += 1;
        if *entries > 4096 {
            return Err(DENIED.into());
        }
        let child = child.map_err(|_| DENIED)?;
        let kind = child.file_type().map_err(|_| DENIED)?;
        if kind.is_dir() || kind.is_file() {
            children.push(path.join(child.file_name()));
        }
    }
    children.sort();
    for child in children {
        files(&child, depth + 1, entries, result)?;
    }
    Ok(())
}

fn search(path: &Path, pattern: &str, limit: usize) -> Result<String, String> {
    let mut paths = Vec::new();
    files(path, 0, &mut 0, &mut paths)?;
    paths.sort();
    let mut output = String::new();
    let mut count = 0;
    let mut bytes = 0;
    for path in paths {
        let text = text(&path)?;
        bytes += text.len();
        if bytes > 4 * 1024 * 1024 {
            return Err(DENIED.into());
        }
        for (index, line) in text.split_inclusive('\n').enumerate() {
            let line = line.strip_suffix('\n').unwrap_or(line);
            if line.contains(pattern) {
                output.push_str(&format!("{}:{}:{line}\n", path.display(), index + 1));
                if output.len() > MAX_FILE as usize {
                    return Err(DENIED.into());
                }
                count += 1;
                if count == limit {
                    return Ok(output);
                }
            }
        }
    }
    Ok(output)
}

fn glob(path: &Path, pattern: &str) -> Result<String, String> {
    let patterns = pattern.split('/').collect::<Vec<_>>();
    if patterns.len() > 32 || patterns.iter().any(|part| matches!(*part, "" | "." | "..")) {
        return Err(DENIED.into());
    }
    let mut paths = Vec::new();
    files(path, 0, &mut 0, &mut paths)?;
    paths.sort();
    let directory = open(path)?.metadata().map_err(|_| DENIED)?.is_dir();
    let mut output = String::new();
    for file in paths {
        let relative = if directory {
            file.strip_prefix(path).map_err(|_| DENIED)?
        } else {
            Path::new(file.file_name().ok_or(DENIED)?)
        };
        let relative = relative.to_str().ok_or(DENIED)?;
        let names = relative.split('/').collect::<Vec<_>>();
        let mut previous = vec![false; names.len() + 1];
        previous[0] = true;
        for pattern in &patterns {
            let mut next = vec![false; names.len() + 1];
            next[0] = *pattern == "**" && previous[0];
            for (index, name) in names.iter().enumerate() {
                next[index + 1] = if *pattern == "**" {
                    previous[index + 1] || next[index]
                } else {
                    let pattern = std::ffi::CString::new(*pattern).map_err(|_| DENIED)?;
                    let name = std::ffi::CString::new(*name).map_err(|_| DENIED)?;
                    previous[index] && unsafe { libc::fnmatch(pattern.as_ptr(), name.as_ptr(), 0) == 0 }
                };
            }
            previous = next;
        }
        if previous[names.len()] {
            output.push_str(&format!("{}\n", file.display()));
            if output.len() > MAX_FILE as usize {
                return Err(DENIED.into());
            }
        }
    }
    Ok(output)
}

fn write(input: &WriteInput) -> Result<String, String> {
    let directory = open(Path::new("/output"))?;
    if !directory.metadata().map_err(|_| DENIED)?.is_dir() {
        return Err(DENIED.into());
    }
    let name = Path::new(&input.path)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(DENIED)?;
    let name = std::ffi::CString::new(name).map_err(|_| DENIED)?;
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    let status = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if status == 0 {
        let metadata = unsafe { metadata.assume_init() };
        if metadata.st_mode & libc::S_IFMT != libc::S_IFREG || metadata.st_nlink != 1 {
            return Err(DENIED.into());
        }
    } else if std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOENT) {
        return Err(DENIED.into());
    }
    let temporary = std::ffi::CString::new(format!(".ocr-{}", super::secret()?)).map_err(|_| DENIED)?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            temporary.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(DENIED.into());
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    let result = (|| {
        file.write_all(input.content.as_bytes()).map_err(|_| DENIED)?;
        file.sync_all().map_err(|_| DENIED)?;
        // Rename changes only this directory entry. Even a concurrent symlink swap cannot
        // redirect writes into another inode: model content goes only to our new file.
        let status = unsafe {
            libc::renameat(
                directory.as_raw_fd(),
                temporary.as_ptr(),
                directory.as_raw_fd(),
                name.as_ptr(),
            )
        };
        if status != 0 {
            return Err(DENIED);
        }
        directory.sync_all().map_err(|_| DENIED)?;
        Ok(())
    })();
    if result.is_err() {
        unsafe {
            libc::unlinkat(directory.as_raw_fd(), temporary.as_ptr(), 0);
        }
    }
    result.map_err(|_| DENIED)?;
    Ok(format!("Wrote {}\n", input.path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_tool_rejects_untrusted_requests() {
        for request in [
            serde_json::json!({"tool":"read","input":{"path":"/work/input.txt"}}),
            serde_json::json!({"tool":"ocr_read","input":{"path":"/proc/self/environ"}}),
            serde_json::json!({"tool":"ocr_read","input":{"path":"/work/../etc/passwd"}}),
            serde_json::json!({"tool":"ocr_read","input":{"path":"/work/input.txt","limit":201}}),
            serde_json::json!({"tool":"ocr_read","input":{"path":"/work/input.txt","command":"id"}}),
            serde_json::json!({"tool":"ocr_bash","input":{"command":"curl https://example.com"}}),
        ] {
            assert_eq!(execute(request.to_string().as_bytes()), Err(Failure::Rejected));
        }
        assert_eq!(execute(&b"not json"[..]), Err(Failure::Rejected));
        assert_eq!(
            execute(vec![b' '; MAX_INPUT as usize + 1].as_slice()),
            Err(Failure::Rejected)
        );
    }
}
