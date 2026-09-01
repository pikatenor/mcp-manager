use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn main() {
    if let Some(rev) = git(&["rev-parse", "--short", "HEAD"]) {
        println!("cargo:rustc-env=MCP_MANAGER_GIT_REVISION={rev}");
    }
    if let Some(tag) = git(&["describe", "--exact-match", "--tags", "HEAD"]) {
        println!("cargo:rustc-env=MCP_MANAGER_GIT_HEAD_TAG={tag}");
    }
    rerun_if_git_changes();
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn rerun_if_git_changes() {
    let Some(git_dir) = git(&["rev-parse", "--absolute-git-dir"]) else {
        return;
    };
    let git_dir = PathBuf::from(git_dir);
    println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
    if let Ok(head) = std::fs::read_to_string(git_dir.join("HEAD")) {
        if let Some(rest) = head.strip_prefix("ref: ") {
            let ref_path = git_dir.join(Path::new(rest.trim()));
            println!("cargo:rerun-if-changed={}", ref_path.display());
        }
    }
}
