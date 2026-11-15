//! `kex pull` and `kex status` (BLUEPRINT §5.2, §5.4).

use std::collections::HashSet;

use anyhow::Result;

use serde_json::{Value, json};

use crate::project::{self, Repo};
use crate::{gh, git, ui, util, view};

pub struct Update {
    pub branch: String,
    /// As the terminal shows it (colored).
    pub status: String,
    /// The same as a message (`pull.*`), and the tone of the card.
    pub msg: Value,
    pub tone: &'static str,
    pub ok: bool,
}

fn first_line(e: &anyhow::Error) -> String {
    e.to_string().lines().next().unwrap_or("").to_string()
}

/// Fetch, then fast-forward the current branch. Never merges, never touches a dirty tree.
pub fn update(r: &Repo, to_prime: bool) -> Update {
    let dir = &r.path;
    let mut branch = git::current_branch(dir);
    // `tone`: ok / warn / muted / danger; danger = failed (exit code 1).
    let out = |branch: String, id: &str, params: Value, tone: &'static str| {
        let msg = view::msg(id, params);
        let text = msg["text"].as_str().unwrap_or_default().to_string();
        let status = match tone {
            "ok" => util::green(text),
            "warn" => util::yellow(text),
            "danger" => util::red(text),
            _ => util::dim(text),
        };
        Update {
            branch,
            status,
            msg,
            tone,
            ok: tone != "danger",
        }
    };

    if let Err(e) = git::git(dir, &["fetch", "--all", "--prune", "--quiet"]) {
        let error = first_line(&e);
        return out(
            branch,
            "pull.fetch_failed",
            json!({ "error": error }),
            "danger",
        );
    }
    if git::is_dirty(dir) {
        return out(branch, "pull.dirty", Value::Null, "warn");
    }
    if to_prime && let Some(p) = r.prime().filter(|p| *p != branch) {
        if let Err(e) = git::git(dir, &["switch", "--quiet", p]) {
            let params = json!({ "branch": p, "error": first_line(&e) });
            return out(branch, "pull.switch_failed", params, "danger");
        }
        branch = p.to_string();
    }
    if !git::has_upstream(dir) {
        return out(branch, "pull.no_upstream", Value::Null, "muted");
    }
    let behind = git::count(dir, &["HEAD..@{u}"]);
    let ahead = git::count(dir, &["@{u}..HEAD"]);
    if behind == 0 {
        return out(branch, "pull.up_to_date", Value::Null, "ok");
    }
    if ahead > 0 {
        let params = json!({ "ahead": ahead, "behind": behind });
        return out(branch, "pull.diverged", params, "warn");
    }
    match git::git(dir, &["merge", "--ff-only", "--quiet", "@{u}"]) {
        Ok(_) => {
            let id = if behind == 1 {
                "pull.pulled_one"
            } else {
                "pull.pulled"
            };
            out(branch, id, json!({ "n": behind }), "ok")
        }
        Err(e) => out(
            branch,
            "pull.failed",
            json!({ "error": first_line(&e) }),
            "danger",
        ),
    }
}

/// The key of a repo's pull card.
pub fn key(r: &Repo) -> String {
    format!("pull:{}", r.path.display())
}

/// A `pull` event: the old fields, plus its card. `found`: the unmerged branches (or why
/// there is no list); `None` when not reported (a second clone, `kex run`).
pub fn event(r: &Repo, u: &Update, found: Option<&Result<Vec<Branch>, Note>>) -> Value {
    let mut sections = Vec::new();
    let mut notes = Vec::new();
    match found {
        Some(Ok(bs)) if !bs.is_empty() => {
            let columns = json!([
                view::col("branch", "branch"),
                view::col("ahead", "count"),
                view::col("behind", "count"),
                view::col("prime", "branch"),
                view::col("author", "text"),
                view::col("last_commit", "date"),
            ]);
            let rows: Vec<Value> = bs
                .iter()
                .map(|b| {
                    json!({ "key": b.name, "cells": {
                        "branch": b.name, "ahead": b.ahead, "behind": b.behind,
                        "prime": b.prime, "author": b.author, "last_commit": b.iso_date,
                    }})
                })
                .collect();
            sections.push(view::view(
                "table",
                json!({ "title": view::m("pull.unmerged"), "columns": columns, "rows": rows }),
            ));
        }
        Some(Err(n)) => notes.push(view::note("info", n.msg.clone())),
        _ => {}
    }
    let mut v = json!({
        "repo": r.name, "path": r.path, "branch": u.branch, "ok": u.ok,
        "status": ui::strip_ansi(&u.status),
        "key": key(r),
        "view": view::view("card", json!({
            "title": r.name,
            "subtitle": u.branch,
            "tone": u.tone,
            "summary": u.msg,
            "fields": [view::field("path", json!(r.path), "path")],
            "sections": sections,
            "notes": notes,
        })),
    });
    if let Some(f) = found {
        v["unmerged"] = match f {
            Ok(bs) => json!(bs.iter().map(Branch::json).collect::<Vec<_>>()),
            Err(_) => Value::Null,
        };
        v["unmerged_note"] = match f {
            Err(n) => json!(n.text()),
            Ok(_) => Value::Null,
        };
    }
    v
}

/// Why there is no list of unmerged branches.
pub struct Note {
    msg: Value,
}

impl Note {
    fn new(id: &str, params: Value) -> Self {
        Note {
            msg: view::msg(id, params),
        }
    }
    pub fn text(&self) -> String {
        self.msg["text"].as_str().unwrap_or_default().to_string()
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

pub struct Branch {
    pub name: String,
    pub prime: String,
    pub ahead: u32,
    pub behind: u32,
    pub author: String,
    pub date: String,
    /// RFC 3339, for the view.
    pub iso_date: String,
}

impl Branch {
    fn line(&self) -> String {
        format!(
            "{:<26} → {:<8} +{} / -{}  {}  {}",
            self.name,
            self.prime,
            self.ahead,
            self.behind,
            self.author,
            util::dim(&self.date)
        )
    }
    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "branch": self.name, "prime": self.prime, "ahead": self.ahead,
            "behind": self.behind, "author": self.author, "date": self.date,
        })
    }
}

/// Branches on the shared remote not merged into any prime branch. `Err` = why there is no
/// list (no remote, no prime branch).
pub fn unmerged(r: &Repo, all: bool, merged: &HashSet<String>) -> Result<Vec<Branch>, Note> {
    let dir = &r.path;
    let Some(remote) = git::shared_remote(dir) else {
        return Err(Note::new("pull.no_remote", Value::Null));
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
        return Err(Note::new("pull.no_prime", json!({ "remote": remote })));
    }
    let format = "--format=%(refname:short)%09%(authorname)%09%(committerdate:unix)%09%(committerdate:format:%Y-%m-%d %H:%M)%09%(committerdate:iso-strict)";
    let Ok(refs) = git::git(
        dir,
        &["for-each-ref", &format!("refs/remotes/{remote}"), format],
    ) else {
        return Ok(vec![]);
    };
    let cutoff = util::now().saturating_sub(30 * 86_400);
    let prefix = format!("{remote}/");
    let mut out = Vec::new();
    for line in refs.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 5 {
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
        out.push(Branch {
            name: branch.to_string(),
            prime: prime.to_string(),
            ahead,
            behind,
            author: author.to_string(),
            date: date.to_string(),
            iso_date: f[4].to_string(),
        });
    }
    Ok(out)
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
        let found = report.then(|| {
            let merged = if gh_ready {
                merged_heads(r)
            } else {
                HashSet::new()
            };
            unmerged(r, all, &merged)
        });
        (u, found)
    });
    let mut code = 0;
    for ((r, _), (u, found)) in jobs.iter().zip(results) {
        let mut ev = event(r, &u, found.as_ref());
        if found.is_none() {
            ev["unmerged"] = Value::Null;
            ev["unmerged_note"] = Value::Null;
        }
        ui::event("pull", ev);
        let lines: Vec<String> = match &found {
            Some(Ok(bs)) => bs.iter().map(Branch::line).collect(),
            Some(Err(note)) => vec![util::dim(note.text())],
            None => vec![],
        };
        let key = key(r);
        let path = if p.checkouts(&r.name).len() > 1 {
            format!(" {}", util::dim(util::tilde(&r.path)))
        } else {
            String::new()
        };
        let line = format!(
            "{} ({}){path} {}",
            util::bold(format!("== {}", r.name)),
            u.branch,
            u.status
        );
        ui::say_about(&key, "info", &line);
        if !lines.is_empty() {
            ui::say_about(&key, "info", "   unmerged:");
            for l in lines {
                ui::say_about(&key, "info", &format!("   {l}"));
            }
        }
        if !u.ok {
            code = 1;
        }
    }
    if !gh_ready {
        say!(
            "{}",
            util::dim("(gh not available: branches merged by squash may still be listed)")
        );
    }
    Ok(code)
}

pub fn status() -> Result<u8> {
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
    if ui::json() {
        let list: Vec<serde_json::Value> = repos
            .iter()
            .zip(&rows)
            .map(|(r, (branch, up, ahead, behind, changed))| {
                serde_json::json!({
                    "repo": r.name, "path": r.path, "branch": branch, "upstream": up,
                    "ahead": ahead, "behind": behind, "changed": changed, "worktree": r.worktree,
                    "description": r.config.description,
                })
            })
            .collect();
        let missing: Vec<&str> = p.missing.iter().map(|m| m.repo.as_str()).collect();
        let rows: Vec<Value> = repos
            .iter()
            .zip(&rows)
            .map(|(r, (branch, up, ahead, behind, changed))| {
                let (ahead, behind) = if *up {
                    (
                        view::count(u64::from(*ahead), "info"),
                        view::count(u64::from(*behind), "warn"),
                    )
                } else {
                    let none = view::m("status.no_upstream");
                    (
                        view::cell(Value::Null, Some("muted"), Some(none)),
                        Value::Null,
                    )
                };
                let mut row = json!({
                    "key": r.path,
                    "cells": {
                        "repo": r.name, "branch": branch, "ahead": ahead, "behind": behind,
                        "changed": view::count(*changed as u64, "warn"),
                    },
                });
                if r.worktree {
                    row["badges"] = json!([view::badge("worktree", "muted")]);
                }
                if let Some(d) = &r.config.description {
                    row["detail"] = json!(d);
                }
                row
            })
            .collect();
        let mut notes = Vec::new();
        if !missing.is_empty() {
            let m = view::msg("status.missing", json!({ "repos": missing.join(", ") }));
            notes.push(view::note("warn", m));
        }
        ui::emit(
            "status",
            json!({
                "project": p.id, "repos": list, "missing": missing,
                "key": "status",
                "view": view::view("table", json!({
                    "title": view::m("status.title"),
                    "subtitle": p.id,
                    "columns": [
                        view::col("repo", "text"), view::col("branch", "branch"),
                        view::col("ahead", "count"), view::col("behind", "count"),
                        view::col("changed", "count"),
                    ],
                    "rows": rows,
                    "notes": notes,
                })),
            }),
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
        say!(
            "  {:<8} {:<24} {}  {}",
            r.name,
            branch,
            parts.join(" "),
            util::dim(util::tilde(&r.path))
        );
    }
    Ok(0)
}
