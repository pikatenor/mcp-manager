//! Resolve the user's login-shell `PATH` so children spawned from a GUI
//! context (launchd's minimal `PATH`) still find commands like `npx`.

use std::env;
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Bound on `$SHELL -ilc env`: a broken rc file must not hang app startup.
const SHELL_ENV_TIMEOUT: Duration = Duration::from_secs(3);

/// Merge `login` PATH entries in front of `base`, dropping duplicates and
/// empty entries (empty entries mean "current directory" and are never wanted).
fn merge_paths(base: &str, login: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    login
        .split(':')
        .chain(base.split(':'))
        .filter(|entry| !entry.is_empty() && seen.insert(entry.to_string()))
        .collect::<Vec<_>>()
        .join(":")
}

/// Extract the last `PATH=` assignment from `env`-style output; later
/// assignments win, matching how the shell itself would evaluate it.
fn parse_path_from_env_output(output: &str) -> Option<String> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("PATH="))
        .next_back()
        .map(str::to_string)
}

/// Run `shell -ilc env` and return its `PATH` (login + interactive rc files
/// applied). `None` on any failure or timeout.
fn login_shell_path(shell: &str) -> Option<String> {
    let mut child = Command::new(shell)
        .args(["-ilc", "env"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + SHELL_ENV_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(_) => return None,
        }
    }
    let mut output = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut output);
    }
    parse_path_from_env_output(&output)
}

/// One-shot startup hook: merge the user's login-shell `PATH` into this
/// process's `PATH` so stdio MCP children resolve commands when the app is
/// launched from Finder/dmg. No-op if the login shell cannot be queried.
pub fn fix_path_for_children() {
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    if let Some(login) = login_shell_path(&shell) {
        let current = env::var("PATH").unwrap_or_default();
        let merged = merge_paths(&current, &login);
        if !merged.is_empty() {
            env::set_var("PATH", merged);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_path_finds_assignment_among_env_noise() {
        let output = "HOME=/Users/test\nPATH=/a:/b\nSHELL=/bin/zsh\n";
        assert_eq!(
            parse_path_from_env_output(output),
            Some("/a:/b".to_string())
        );
    }

    #[test]
    fn parse_path_takes_last_assignment() {
        let output = "PATH=/old\nPATH=/new";
        assert_eq!(parse_path_from_env_output(output), Some("/new".to_string()));
    }

    #[test]
    fn parse_path_missing_returns_none() {
        assert_eq!(parse_path_from_env_output("HOME=/Users/test\n"), None);
        assert_eq!(parse_path_from_env_output(""), None);
    }

    #[test]
    fn merge_prepends_login_entries_and_dedupes() {
        let merged = merge_paths(
            "/usr/bin:/bin:/usr/sbin:/sbin",
            "/Users/test/.asdf/shims:/opt/homebrew/bin:/usr/bin:/bin",
        );
        assert_eq!(
            merged,
            "/Users/test/.asdf/shims:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
        );
    }

    #[test]
    fn merge_skips_empty_entries() {
        let merged = merge_paths("/usr/bin::/bin", ":/opt/homebrew/bin:");
        assert_eq!(merged, "/opt/homebrew/bin:/usr/bin:/bin");
    }

    #[test]
    fn merge_handles_empty_side() {
        assert_eq!(merge_paths("", "/a:/b"), "/a:/b");
        assert_eq!(merge_paths("/a:/b", ""), "/a:/b");
    }

    #[test]
    fn login_shell_path_resolves_system_shell() {
        let path = login_shell_path("/bin/sh").expect("login shell PATH");
        assert!(path.contains("bin"), "unexpected PATH: {path}");
    }
}
