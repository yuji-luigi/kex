//! `kex doctor`, `kex scan`, `kex link` (BLUEPRINT §4.7).

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

use crate::project::{self, Repo};
use crate::registry::{self, GlobalConfig, Registry};
use crate::{config, gh, git, util};

pub fn doctor() -> Result<u8> {
    let p = project::resolve(&std::env::current_dir()?)?;
    p.print_header();
    for r in &p.repos {
        let remote = git::shared_remote(&r.path);
        println!(
            "  {} {:<8} {}",
            util::green("✓"),
            r.name,
            util::tilde(&r.path)
        );
        let wt = if r.worktree {
            "  (worktree: used for this command only)"
        } else {
            ""
        };
        println!("      branch    {}{wt}", git::current_branch(&r.path));
        let cfg = match &r.config_path {
            Some(c) => util::tilde(c),
            None => format!(
                "none on this branch, read from {}",
                r.prime().unwrap_or("?")
            ),
        };
        println!("      config    {cfg}");
        let root = match &r.root_commit {
            Some(c) => c.chars().take(10).collect(),
            None => "unknown (shallow clone?): identity falls back to repo_id + remote".to_string(),
        };
        println!("      identity  {} · first commit {root}", r.repo_id);
        let url = remote.as_deref().and_then(|n| git::remote_url(&r.path, n));
        println!("      remote    {}", url.unwrap_or_else(|| "none".into()));
        let hosting = match gh::provider(r) {
            gh::Provider::GitHub => "GitHub (via gh)",
            gh::Provider::GitLab => "GitLab (not supported yet, PR features off)",
            gh::Provider::Unknown => "unknown host (PR features off)",
        };
        println!("      hosting   {hosting}");
        if let Some(remote) = &remote {
            for b in &r.config.prime_branches {
                let exists = git::ok(
                    &r.path,
                    &[
                        "rev-parse",
                        "--verify",
                        "--quiet",
                        &format!("refs/remotes/{remote}/{}", b.branch),
                    ],
                );
                if !exists {
                    println!(
                        "      {} prime branch {} not found on {remote}",
                        util::yellow("⚠"),
                        b.branch
                    );
                }
            }
        }
    }
    let st = gh::state();
    let gh_line = if st == gh::State::Ready {
        util::green("ready")
    } else {
        util::yellow(gh::help(st))
    };
    println!("gh        {gh_line}");
    println!(
        "registry  {}",
        util::tilde(&registry::kex_dir().join("registry.json"))
    );
    Ok(0)
}

fn register(reg: &mut Registry, r: &Repo) {
    reg.register(
        &r.config.project,
        &r.repo_id,
        &r.name,
        r.root_commit.as_deref(),
        r.prime(),
        &r.path,
    );
}

/// Index every kex repo under `dir` and remember `dir` as a search root.
pub fn scan(dir: Option<PathBuf>) -> Result<u8> {
    let dir = util::canon(&match dir {
        Some(d) => d,
        None => std::env::current_dir()?,
    });
    let mut candidates = Vec::new();
    project::find_git_dirs(&dir, 4, &mut candidates);
    let mut reg = Registry::load()?;
    let mut n = 0;
    for path in candidates {
        if !matches!(config::config_path(&path), Ok(Some(_))) {
            continue;
        }
        if let Some(r) = project::open(&path, None).filter(|r| !r.worktree) {
            println!(
                "  {:<24} {:<8} {}",
                r.config.project,
                r.name,
                util::tilde(&r.path)
            );
            register(&mut reg, &r);
            n += 1;
        }
    }
    reg.save()?;
    let mut global = GlobalConfig::load()?;
    if !global.roots.contains(&dir) {
        global.roots.push(dir.clone());
        global.save()?;
    }
    println!(
        "found {n} kex repo{} under {} (now a search root)",
        if n == 1 { "" } else { "s" },
        util::tilde(&dir)
    );
    Ok(0)
}

pub fn link(path: &Path) -> Result<u8> {
    let r =
        project::open(path, None).ok_or_else(|| anyhow!("{} is not a kex repo", path.display()))?;
    let mut reg = Registry::load()?;
    register(&mut reg, &r);
    reg.save()?;
    println!(
        "linked {} ({}) → {}",
        r.name,
        r.config.project,
        util::tilde(&r.path)
    );
    Ok(0)
}
