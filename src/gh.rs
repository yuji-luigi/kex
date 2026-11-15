//! Everything that talks to GitHub goes through the `gh` CLI (BLUEPRINT §6).

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::config::Hosting;
use crate::project::Repo;
use crate::{git, util};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Missing,
    LoggedOut,
    Ready,
}

pub fn state() -> State {
    if util::which("gh").is_none() {
        return State::Missing;
    }
    let ok = crate::cancel::status(
        Command::new("gh")
            .args(["auth", "status"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    )
    .is_ok_and(|s| s.success());
    if ok { State::Ready } else { State::LoggedOut }
}

/// What to do when `gh` isn't usable. kex shows the commands and never runs them.
pub fn help(state: State) -> String {
    let install = if cfg!(target_os = "macos") {
        "brew install gh"
    } else if cfg!(windows) {
        "winget install --id GitHub.cli"
    } else {
        "see https://github.com/cli/cli#installation"
    };
    match state {
        State::Missing => format!(
            "this needs the GitHub CLI (gh), which isn't installed.\n\n  Install:  {install}\n  Then:     gh auth login\n  Docs:     https://cli.github.com/"
        ),
        State::LoggedOut => {
            "the GitHub CLI (gh) is installed but not logged in.\n\n  Run:  gh auth login".into()
        }
        State::Ready => String::new(),
    }
}

/// Run gh in `dir` and return its stdout.
pub fn gh(dir: &Path, args: &[&str]) -> Result<String> {
    let out = crate::cancel::output(
        Command::new("gh")
            .args(args)
            .current_dir(dir)
            .env("GH_PAGER", "cat"),
    )
    .context("failed to run gh")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("Could not resolve") || err.contains("HTTP 404") {
            bail!(
                "{} (is the active gh account allowed to see this repo? try `gh auth switch`)",
                err.trim()
            );
        }
        bail!("{}", err.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// Run gh with the terminal attached; `no_pager` prints straight to the screen.
pub fn gh_inherit(dir: &Path, args: &[&str], no_pager: bool) -> Result<bool> {
    let mut c = Command::new("gh");
    c.args(args).current_dir(dir);
    if no_pager || crate::ui::json() {
        c.env("GH_PAGER", "cat");
    }
    if crate::ui::json() {
        // Keep stdout for events: capture, then send the text as log lines.
        let out = crate::cancel::output(c.env("NO_COLOR", "1")).context("failed to run gh")?;
        let text = String::from_utf8_lossy(&out.stdout);
        if !text.trim().is_empty() {
            crate::ui::say(text.trim_end());
        }
        let err = String::from_utf8_lossy(&out.stderr);
        if !err.trim().is_empty() {
            crate::ui::warn(err.trim_end());
        }
        return Ok(out.status.success());
    }
    Ok(crate::cancel::status(&mut c)
        .context("failed to run gh")?
        .success())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    GitHub,
    GitLab,
    Unknown,
}

fn by_domain(d: &str) -> Provider {
    if d == "github.com" || d.starts_with("github.") {
        Provider::GitHub
    } else if d == "gitlab.com" {
        Provider::GitLab
    } else {
        Provider::Unknown
    }
}

/// §3.4: the `hosting` field, else the shared remote's domain, else GitHub when there's no remote.
pub fn provider(r: &Repo) -> Provider {
    match &r.config.hosting {
        Some(Hosting::Full {
            provider: Some(p), ..
        }) => match p.as_str() {
            "github" => Provider::GitHub,
            "gitlab" => Provider::GitLab,
            _ => Provider::Unknown,
        },
        Some(Hosting::Domain(d))
        | Some(Hosting::Full {
            domain: d,
            provider: None,
        }) => by_domain(d),
        None => match git::shared_remote(&r.path).and_then(|n| git::remote_url(&r.path, &n)) {
            Some(url) => git::remote_domain(&url).map_or(Provider::Unknown, |d| by_domain(&d)),
            None => Provider::GitHub,
        },
    }
}
