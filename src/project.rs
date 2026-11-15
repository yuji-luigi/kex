//! Turning "the repo I'm in" into "all the repos of this project" (BLUEPRINT §4).

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

use crate::config::{self, Config, Peer, Stage};
use crate::registry::{GlobalConfig, Registry};
use crate::{git, util, view};

const DAY: u64 = 86_400;

/// One checkout of a kex repo.
#[derive(Debug)]
pub struct Repo {
    pub name: String,
    pub repo_id: String,
    pub path: PathBuf,
    pub config: Config,
    /// `None` when the checked-out branch has no config and it was read from git.
    pub config_path: Option<PathBuf>,
    pub worktree: bool,
    pub root_commit: Option<String>,
}

impl Repo {
    fn new(path: PathBuf, config: Config, config_path: Option<PathBuf>) -> Self {
        Repo {
            name: config.repo.clone(),
            repo_id: config.repo_id.clone(),
            worktree: git::is_worktree(&path),
            root_commit: git::root_commit(&path),
            path,
            config,
            config_path,
        }
    }

    pub fn prime(&self) -> Option<&str> {
        self.config.default_prime().map(|b| b.branch.as_str())
    }

    /// `stage: production` branches and the `protected` patterns.
    pub fn is_protected(&self, branch: &str) -> bool {
        self.config
            .prime_branches
            .iter()
            .any(|b| b.stage == Stage::Production && b.branch == branch)
            || self
                .config
                .protected
                .iter()
                .any(|p| util::glob_match(p, branch))
    }
}

/// Open the kex repo at `path`. When the checked-out branch has no config, read it from
/// `fallback_prime`.
pub fn open(path: &Path, fallback_prime: Option<&str>) -> Option<Repo> {
    let root = util::canon(&git::root(path).ok()?);
    match config::config_path(&root) {
        Ok(Some(p)) => Some(Repo::new(root, config::load(&p).ok()?, Some(p))),
        Ok(None) => Some(Repo::new(
            root.clone(),
            config::load_at_rev(&root, fallback_prime?)?,
            None,
        )),
        Err(_) => None,
    }
}

pub struct Project {
    pub id: String,
    /// Every checkout found; `repos[0]` is the one kex runs in.
    pub repos: Vec<Repo>,
    pub missing: Vec<Peer>,
    pub warnings: Vec<String>,
}

/// §4.3: find the current repo, then every other repo of its project.
pub fn resolve(cwd: &Path) -> Result<Project> {
    let mut reg = Registry::load()?;
    let root = util::canon(&git::root(cwd)?);
    let current = match config::config_path(&root)? {
        Some(p) => Repo::new(root.clone(), config::load(&p)?, Some(p)),
        None => {
            let prime = reg.entry_for_path(&root).and_then(|e| e.prime.clone());
            let c = prime
                .as_deref()
                .and_then(|b| config::load_at_rev(&root, b))
                .ok_or_else(|| {
                    anyhow!(
                        "not a kex repo: no kex.json in {} (run `kex init`)",
                        root.display()
                    )
                })?;
            Repo::new(root.clone(), c, None)
        }
    };
    let id = current.config.project.clone();
    let mut seen = HashSet::from([current.path.clone()]);
    let mut found = Vec::new();

    // Known paths first; drop the ones that moved or now belong to something else.
    if let Some(entries) = reg.projects.get(&id).cloned() {
        for (repo_id, e) in entries {
            for p in e.paths {
                if seen.contains(&p) {
                    continue;
                }
                match open(&p, e.prime.as_deref()) {
                    Some(r)
                        if r.config.project == id
                            && r.repo_id == repo_id
                            && !r.worktree
                            && r.path == p =>
                    {
                        seen.insert(p);
                        found.push(r);
                    }
                    _ => reg.forget_path(&id, &repo_id, &p),
                }
            }
        }
    }

    // Siblings and the parent's siblings: always, because it's cheap.
    let base = if current.worktree {
        git::main_checkout(&current.path)
            .map(|p| util::canon(&p))
            .unwrap_or_else(|| current.path.clone())
    } else {
        current.path.clone()
    };
    let candidates = base
        .parent()
        .and_then(Path::parent)
        .map(|g| crate::tree::git_dirs(g, 2))
        .unwrap_or_default();
    add_candidates(&id, candidates, &mut seen, &mut found);

    // Deep scan of the roots only when a peer is missing, at most once a day.
    let mut missing = missing_peers(&current, &found);
    if !missing.is_empty() {
        let roots = GlobalConfig::load()?.roots;
        let due = reg
            .not_found
            .get(&id)
            .is_none_or(|t| util::now().saturating_sub(*t) > DAY);
        if !roots.is_empty() && due {
            let c: Vec<PathBuf> = roots
                .iter()
                .flat_map(|r| crate::tree::git_dirs(r, 4))
                .collect();
            add_candidates(&id, c, &mut seen, &mut found);
            missing = missing_peers(&current, &found);
            if missing.is_empty() {
                reg.not_found.remove(&id);
            } else {
                reg.not_found.insert(id.clone(), util::now());
            }
        }
    }

    // Inside a worktree, the worktree replaces its main checkout for this command (§4.5).
    if current.worktree {
        found.retain(|r| r.repo_id != current.repo_id);
    }
    found.sort_by(|a, b| a.name.cmp(&b.name).then(a.path.cmp(&b.path)));
    let mut repos = vec![current];
    repos.extend(found);

    for r in repos.iter().filter(|r| !r.worktree) {
        reg.register(
            &id,
            &r.repo_id,
            &r.name,
            r.root_commit.as_deref(),
            r.prime(),
            &r.path,
        );
    }
    let warnings = duplicate_warnings(&repos);
    if let Err(e) = reg.save() {
        warn!("kex: could not save the registry: {e:#}");
    }
    Ok(Project {
        id,
        repos,
        missing,
        warnings,
    })
}

fn add_candidates(
    id: &str,
    candidates: Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
    found: &mut Vec<Repo>,
) {
    for c in candidates {
        let c = util::canon(&c);
        if seen.contains(&c) || !matches!(config::config_path(&c), Ok(Some(_))) {
            continue;
        }
        if let Some(r) = open(&c, None)
            && r.config.project == id
            && !r.worktree
        {
            seen.insert(c);
            found.push(r);
        }
    }
}

/// Union of `peers` over the found repos, minus the ones found (§4.6).
fn missing_peers(current: &Repo, found: &[Repo]) -> Vec<Peer> {
    let all = || std::iter::once(current).chain(found);
    let names: HashSet<&str> = all().map(|r| r.name.as_str()).collect();
    let mut out: Vec<Peer> = Vec::new();
    for p in all().flat_map(|r| &r.config.peers) {
        if !names.contains(p.repo.as_str()) && !out.iter().any(|o| o.repo == p.repo) {
            out.push(p.clone());
        }
    }
    out
}

/// §4.4: same repo_id with different first commits = a copied config.
fn duplicate_warnings(repos: &[Repo]) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut by_id: BTreeMap<&str, Vec<&Repo>> = BTreeMap::new();
    let mut by_name: BTreeMap<&str, HashSet<&str>> = BTreeMap::new();
    for r in repos {
        by_id.entry(&r.repo_id).or_default().push(r);
        by_name.entry(&r.name).or_default().insert(&r.repo_id);
    }
    for (repo_id, rs) in &by_id {
        let roots: HashSet<&str> = rs.iter().filter_map(|r| r.root_commit.as_deref()).collect();
        if roots.len() > 1 {
            let paths: Vec<String> = rs
                .iter()
                .map(|r| format!("    {}", util::tilde(&r.path)))
                .collect();
            warnings.push(format!(
                "copied kex config: repo_id {repo_id} (\"{}\") is used by unrelated repos:\n{}\n  In the copy, run `kex init --reset` to give it new ids.",
                rs[0].name,
                paths.join("\n")
            ));
        }
    }
    for (name, ids) in by_name {
        if ids.len() > 1 {
            warnings.push(format!(
                "two different repos are named \"{name}\" in this project; rename one with `kex init --reset`"
            ));
        }
    }
    warnings
}

impl Project {
    /// Repo names, the current one first.
    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        for r in &self.repos {
            if !v.contains(&r.name) {
                v.push(r.name.clone());
            }
        }
        v
    }

    pub fn checkouts(&self, name: &str) -> Vec<&Repo> {
        self.repos.iter().filter(|r| r.name == name).collect()
    }

    /// One checkout of `name`; asks when there are several clones (§4.5).
    pub fn pick(&self, name: &str) -> Result<&Repo> {
        let cs = self.checkouts(name);
        if cs.len() == 1 || std::ptr::eq(cs[0], &self.repos[0]) {
            return Ok(cs[0]);
        }
        let key = format!("{}/{name}", self.id);
        let mut reg = Registry::load()?;
        if let Some(r) = reg
            .preferred
            .get(&key)
            .and_then(|p| cs.iter().find(|r| &r.path == p))
        {
            return Ok(r);
        }
        if !util::interactive() {
            warn!(
                "kex: {name} has {} clones, using {}",
                cs.len(),
                util::tilde(&cs[0].path)
            );
            return Ok(cs[0]);
        }
        let opts: Vec<crate::ui::Opt> = cs
            .iter()
            .map(|r| {
                let label = format!(
                    "{} ({})",
                    util::tilde(&r.path),
                    git::current_branch(&r.path)
                );
                crate::ui::Opt::new(r.path.display().to_string(), label).row(serde_json::json!({
                    "cells": { "path": r.path, "branch": git::current_branch(&r.path) },
                }))
            })
            .collect();
        let q = crate::ui::Q::new(
            "clones.pick",
            serde_json::json!({ "repo": name, "n": cs.len() }),
        )
        .with(
            "columns",
            serde_json::json!([view::col("path", "path"), view::col("branch", "branch")]),
        );
        let i = crate::ui::select_one(q, &opts)?;
        let remember = crate::ui::Q::new("clones.remember", serde_json::Value::Null);
        if crate::ui::confirm(remember, true)? {
            reg.preferred.insert(key, cs[i].path.clone());
            reg.save()?;
        }
        Ok(cs[i])
    }

    pub fn print_header(&self) {
        let names: Vec<&str> = self.repos.iter().map(|r| r.name.as_str()).collect();
        let mut seen = std::collections::HashSet::new();
        let names: Vec<&str> = names.into_iter().filter(|n| seen.insert(*n)).collect();
        let missing: Vec<&str> = self.missing.iter().map(|m| m.repo.as_str()).collect();
        let mut fields = vec![view::field("repos", serde_json::json!(names), "text")];
        if !missing.is_empty() {
            let mut f = view::field("missing", serde_json::json!(missing), "text");
            f["tone"] = serde_json::json!("warn");
            fields.push(f);
        }
        let notes: Vec<serde_json::Value> = self
            .warnings
            .iter()
            .map(|w| view::note("warn", serde_json::json!(w)))
            .collect();
        crate::ui::event(
            "project",
            serde_json::json!({
                "key": "project",
                "view": view::view("card", serde_json::json!({
                    "title": view::m("project.title"),
                    "subtitle": self.id,
                    "fields": fields,
                    "notes": notes,
                })),
                "id": self.id,
                "repos": self.repos.iter().map(|r| serde_json::json!({
                    "repo": r.name, "path": r.path, "worktree": r.worktree,
                })).collect::<Vec<_>>(),
                "missing": self.missing.iter().map(|m| serde_json::json!({
                    "repo": m.repo, "remote": m.remote,
                })).collect::<Vec<_>>(),
                "warnings": self.warnings,
            }),
        );
        let about = |level: &str, text: String| crate::ui::say_about("project", level, &text);
        about("info", format!("{} {}", util::bold("project"), self.id));
        for w in &self.warnings {
            crate::ui::warn_about("project", &format!("{} {w}", util::yellow("⚠")));
        }
        for m in &self.missing {
            about(
                "info",
                format!(
                    "  {} {:<8} not on this machine ({})",
                    util::red("✗"),
                    m.repo,
                    m.remote
                ),
            );
        }
    }
}
