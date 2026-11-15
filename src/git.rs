//! Thin wrappers around the `git` command line (BLUEPRINT §8: shell out, don't reimplement).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::util;

/// Run git in `dir` and return its trimmed stdout.
pub fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = crate::cancel::output(Command::new("git").args(args).current_dir(dir))
        .context("failed to run git (is it installed?)")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// Run git in `dir`, only caring whether it succeeds.
pub fn ok(dir: &Path, args: &[&str]) -> bool {
    crate::cancel::status(
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    )
    .is_ok_and(|s| s.success())
}

/// Top folder of the checkout containing `dir`.
pub fn root(dir: &Path) -> Result<PathBuf> {
    match git(dir, &["rev-parse", "--show-toplevel"]) {
        Ok(s) => Ok(PathBuf::from(s)),
        Err(_) => bail!("not inside a git repository: {}", dir.display()),
    }
}

fn abs_dir(dir: &Path, which: &str) -> Option<PathBuf> {
    git(dir, &["rev-parse", "--path-format=absolute", which])
        .ok()
        .map(|s| util::canon(Path::new(&s)))
}

/// A linked worktree has its own git dir inside the main checkout's common dir (§4.5).
pub fn is_worktree(dir: &Path) -> bool {
    match (abs_dir(dir, "--git-dir"), abs_dir(dir, "--git-common-dir")) {
        (Some(g), Some(c)) => g != c,
        _ => false,
    }
}

/// The main checkout of a worktree (the folder holding the shared `.git`).
pub fn main_checkout(dir: &Path) -> Option<PathBuf> {
    let common = abs_dir(dir, "--git-common-dir")?;
    (common.file_name()? == ".git").then(|| common.parent().map(Path::to_path_buf))?
}

/// First commit of the history; `None` in shallow clones, where it isn't the real one (§4.4).
pub fn root_commit(dir: &Path) -> Option<String> {
    if git(dir, &["rev-parse", "--is-shallow-repository"]).ok()? == "true" {
        return None;
    }
    git(dir, &["rev-list", "--max-parents=0", "HEAD"])
        .ok()?
        .lines()
        .min()
        .map(str::to_string)
}

pub fn current_branch(dir: &Path) -> String {
    git(dir, &["branch", "--show-current"])
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(detached)".into())
}

pub fn changed_files(dir: &Path) -> usize {
    git(dir, &["status", "--porcelain"])
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

/// Uncommitted changes to tracked files. Untracked files don't block a fast-forward.
pub fn is_dirty(dir: &Path) -> bool {
    git(dir, &["status", "--porcelain", "--untracked-files=no"]).is_ok_and(|s| !s.is_empty())
}

pub fn has_upstream(dir: &Path) -> bool {
    ok(
        dir,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    )
}

/// `git rev-list --count <args>`, 0 on error.
pub fn count(dir: &Path, args: &[&str]) -> u32 {
    let mut full = vec!["rev-list", "--count"];
    full.extend_from_slice(args);
    git(dir, &full)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// `(left, right)` commit counts of a `a...b` range.
pub fn left_right(dir: &Path, range: &str) -> Option<(u32, u32)> {
    let s = git(dir, &["rev-list", "--left-right", "--count", range]).ok()?;
    let mut it = s.split_whitespace().map(|n| n.parse().ok());
    Some((it.next()??, it.next()??))
}

pub fn remote_url(dir: &Path, name: &str) -> Option<String> {
    git(dir, &["remote", "get-url", name]).ok()
}

/// Where the team's branches live: `upstream` in a fork setup, else `origin` (§5.2).
pub fn shared_remote(dir: &Path) -> Option<String> {
    let remotes = git(dir, &["remote"]).ok()?;
    let list: Vec<&str> = remotes.lines().collect();
    ["upstream", "origin"]
        .into_iter()
        .find(|n| list.contains(n))
        .or(list.first().copied())
        .map(str::to_string)
}

/// `git@github.com:acme/x.git`, `https://github.com/acme/x` → `github.com/acme/x`.
pub fn normalize_remote(url: &str) -> String {
    let mut s = url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_string();
    if let Some(i) = s.find("://") {
        s = s[i + 3..].to_string();
        if let Some(at) = s.find('@').filter(|at| !s[..*at].contains('/')) {
            s = s[at + 1..].to_string();
        }
        if let (Some(colon), Some(slash)) = (s.find(':'), s.find('/'))
            && colon < slash
        {
            s = format!("{}{}", &s[..colon], &s[slash..]);
        }
    } else if let Some(at) = s.find('@') {
        s = s[at + 1..].replacen(':', "/", 1);
    }
    s.to_lowercase()
}

/// Host of a remote URL; `None` for local paths.
pub fn remote_domain(url: &str) -> Option<String> {
    let u = url.trim();
    let windows_drive = u.len() > 1 && u.as_bytes()[1] == b':';
    if u.starts_with('/') || u.starts_with('.') || u.starts_with("file://") || windows_drive {
        return None;
    }
    let n = normalize_remote(u);
    let host = n.split('/').next()?;
    (host.contains('.') || host == "localhost").then(|| host.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remotes() {
        for url in [
            "git@github.com:Acme/shop-api.git",
            "https://github.com/acme/shop-api",
            "ssh://git@github.com:22/acme/shop-api.git",
        ] {
            assert_eq!(normalize_remote(url), "github.com/acme/shop-api", "{url}");
        }
        assert_eq!(
            remote_domain("git@github.com:a/b.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(remote_domain("/tmp/origin.git"), None);
    }
}
