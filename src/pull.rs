//! `kex pull` and `kex status` (BLUEPRINT §5.2, §5.4).

use std::collections::HashSet;

use anyhow::Result;

use crate::project::{self, Repo};
use crate::{gh, git, util};

pub struct Update {
    pub branch: String,
    pub status: String,
    pub ok: bool,
}

fn first_line(e: &anyhow::Error) -> String {
    e.to_string().lines().next().unwrap_or("").to_string()
}

/// Fetch, then fast-forward the current branch. Never merges, never touches a dirty tree.
pub fn update(r: &Repo, to_prime: bool) -> Update {
    let dir = &r.path;
    let mut branch = git::current_branch(dir);
    let fail = |branch: String, msg: String| Update {
        branch,
        status: util::red(format!("✗ {msg}")),
        ok: false,
    };
    let done = |branch: String, status: String| Update {
        branch,
        status,
        ok: true,
    };

    if let Err(e) = git::git(dir, &["fetch", "--all", "--prune", "--quiet"]) {
        return fail(branch, format!("fetch failed: {}", first_line(&e)));
    }
    if git::is_dirty(dir) {
        return done(branch, util::yellow("⚠ uncommitted changes, pull skipped"));
    }
    if to_prime && let Some(p) = r.prime().filter(|p| *p != branch) {
        if let Err(e) = git::git(dir, &["switch", "--quiet", p]) {
            return fail(
                branch,
                format!("could not switch to {p}: {}", first_line(&e)),
            );
        }
        branch = p.to_string();
    }
    if !git::has_upstream(dir) {
        return done(branch, util::dim("no upstream, pull skipped"));
    }
    let behind = git::count(dir, &["HEAD..@{u}"]);
    let ahead = git::count(dir, &["@{u}..HEAD"]);
    if behind == 0 {
        return done(branch, util::green("✓ up to date"));
    }
    if ahead > 0 {
        return done(
            branch,
            util::yellow(format!(
                "⚠ diverged ({ahead} ahead, {behind} behind), pull skipped"
            )),
        );
    }
    match git::git(dir, &["merge", "--ff-only", "--quiet", "@{u}"]) {
        Ok(_) => done(
            branch,
            util::green(format!(
                "↓ {behind} commit{} pulled",
                if behind == 1 { "" } else { "s" }
            )),
        ),
        Err(e) => fail(branch, format!("pull failed: {}", first_line(&e))),
    }
}

/// Head branches of merged PRs, so squash-merged branches don't show as unmerged.
fn merged_heads(r: &Repo) -> HashSet<String> {
    if gh::provider(r) != gh::Provider::GitHub {
        return HashSet::new();
    }
    let args = [
        "pr",
        "list",
        "--state",
        "merged",
        "--limit",
        "200",
        "--json",
        "headRefName",
        "--jq",
        ".[].headRefName",
    ];
    gh::gh(&r.path, &args)
        .map(|s| s.lines().map(String::from).collect())
        .unwrap_or_default()
}

/// Branches on the shared remote not merged into any prime branch, one line each.
pub fn unmerged(r: &Repo, all: bool, merged: &HashSet<String>) -> Vec<String> {
    let dir = &r.path;
    let Some(remote) = git::shared_remote(dir) else {
        return vec![util::dim("no remote")];
    };
    let primes: Vec<&str> = r
        .config
        .prime_branches
        .iter()
        .map(|b| b.branch.as_str())
        .filter(|b| {
            git::ok(
                dir,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("refs/remotes/{remote}/{b}"),
                ],
            )
        })
        .collect();
    if primes.is_empty() {
        return vec![util::dim(format!("no prime branch found on {remote}"))];
    }
    let format = "--format=%(refname:short)%09%(authorname)%09%(committerdate:unix)%09%(committerdate:format:%Y-%m-%d %H:%M)";
    let Ok(refs) = git::git(
        dir,
        &["for-each-ref", &format!("refs/remotes/{remote}"), format],
    ) else {
        return vec![];
    };
    let cutoff = util::now().saturating_sub(30 * 86_400);
    let prefix = format!("{remote}/");
    let mut out = Vec::new();
    for line in refs.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 4 {
            continue;
        }
        let (tip, author, date) = (f[0], f[1], f[3]);
        let Some(branch) = tip.strip_prefix(&prefix) else {
            continue;
        };
        let skip = branch == "HEAD"
            || r.config.prime_branches.iter().any(|b| b.branch == branch)
            || r.is_protected(branch)
            || r.config
                .ignore_branches
                .iter()
                .any(|p| util::glob_match(p, branch))
            || merged.contains(branch)
            || (!all && f[2].parse::<u64>().unwrap_or(0) < cutoff);
        if skip {
            continue;
        }
        let merged_into_prime = primes.iter().any(|p| {
            git::ok(
                dir,
                &["merge-base", "--is-ancestor", tip, &format!("{remote}/{p}")],
            )
        });
        if merged_into_prime {
            continue;
        }
        // Closest prime = the one this branch is the fewest commits ahead of.
        let (prime, behind, ahead) = primes
            .iter()
            .filter_map(|p| {
                git::left_right(dir, &format!("{remote}/{p}...{tip}")).map(|(b, a)| (*p, b, a))
            })
            .min_by_key(|x| x.2)
            .unwrap_or((primes[0], 0, 0));
        out.push(format!(
            "{branch:<26} → {prime:<8} +{ahead} / -{behind}  {author}  {}",
            util::dim(date)
        ));
    }
    out
}

pub fn run(all: bool, to_prime: bool) -> Result<u8> {
    let p = project::resolve(&std::env::current_dir()?)?;
    p.print_header();
    let gh_ready = gh::state() == gh::State::Ready;
    let mut seen = HashSet::new();
    // Clones share branches: report unmerged work once per repo.
    let jobs: Vec<(&Repo, bool)> = p
        .repos
        .iter()
        .map(|r| (r, seen.insert(r.repo_id.clone())))
        .collect();
    let results = util::parallel(&jobs, |(r, report)| {
        let u = update(r, to_prime);
        let lines = if *report {
            let merged = if gh_ready {
                merged_heads(r)
            } else {
                HashSet::new()
            };
            unmerged(r, all, &merged)
        } else {
            Vec::new()
        };
        (u, lines)
    });
    let mut code = 0;
    for ((r, _), (u, lines)) in jobs.iter().zip(results) {
        let path = if p.checkouts(&r.name).len() > 1 {
            format!(" {}", util::dim(util::tilde(&r.path)))
        } else {
            String::new()
        };
        println!(
            "{} ({}){path} {}",
            util::bold(format!("== {}", r.name)),
            u.branch,
            u.status
        );
        if !lines.is_empty() {
            println!("   unmerged:");
            for l in lines {
                println!("   {l}");
            }
        }
        if !u.ok {
            code = 1;
        }
    }
    if !gh_ready {
        println!(
            "{}",
            util::dim("(gh not available: branches merged by squash may still be listed)")
        );
    }
    Ok(code)
}

pub fn status(json: bool) -> Result<u8> {
    let p = project::resolve(&std::env::current_dir()?)?;
    let repos: Vec<&Repo> = p.repos.iter().collect();
    let rows = util::parallel(&repos, |r| {
        let up = git::has_upstream(&r.path);
        let (behind, ahead) = if up {
            git::left_right(&r.path, "@{u}...HEAD").unwrap_or((0, 0))
        } else {
            (0, 0)
        };
        (
            git::current_branch(&r.path),
            up,
            ahead,
            behind,
            git::changed_files(&r.path),
        )
    });
    if json {
        let list: Vec<serde_json::Value> = repos
            .iter()
            .zip(&rows)
            .map(|(r, (branch, up, ahead, behind, changed))| {
                serde_json::json!({
                    "repo": r.name, "path": r.path, "branch": branch, "upstream": up,
                    "ahead": ahead, "behind": behind, "changed": changed, "worktree": r.worktree,
                })
            })
            .collect();
        let missing: Vec<&str> = p.missing.iter().map(|m| m.repo.as_str()).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(
                &serde_json::json!({ "project": p.id, "repos": list, "missing": missing })
            )?
        );
        return Ok(0);
    }
    p.print_header();
    for (r, (branch, up, ahead, behind, changed)) in repos.iter().zip(rows) {
        let mut parts = Vec::new();
        if !up {
            parts.push(util::dim("no upstream"));
        }
        if ahead > 0 {
            parts.push(format!("↑{ahead}"));
        }
        if behind > 0 {
            parts.push(format!("↓{behind}"));
        }
        parts.push(if changed > 0 {
            util::yellow(format!("{changed} changed"))
        } else {
            util::green("clean")
        });
        println!(
            "  {:<8} {:<24} {}  {}",
            r.name,
            branch,
            parts.join(" "),
            util::dim(util::tilde(&r.path))
        );
    }
    Ok(0)
}
