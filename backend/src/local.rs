use std::path::Path;
use std::process::Command;

pub fn default_branch(path: &Path) -> Option<String> {
    if let Ok(value) = git(path, &["symbolic-ref", "refs/remotes/origin/HEAD", "--short"]) {
        return Some(value.strip_prefix("origin/").unwrap_or(&value).to_string());
    }
    let branches = git(path, &["branch", "-r"]).ok()?;
    if branches.lines().any(|line| line.trim() == "origin/main") {
        return Some("main".to_string());
    }
    if branches.lines().any(|line| line.trim() == "origin/master") {
        return Some("master".to_string());
    }
    None
}

fn git(path: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
