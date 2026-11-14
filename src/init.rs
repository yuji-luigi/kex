//! `kex init` (alias `kex add`): set up this repo and the ones around it (BLUEPRINT §4.7).
//!
//! Each way of choosing repos ends in the same request that `--from` takes, handed to
//! [`setup::run`]: the terminal browser (default), `--yes` (this repo), `--repos` (these
//! folders), `--workspace` (a workspace file's repos) and `--from` (a program's request).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::setup::{self, folder_name, lossy};
use crate::{browse, config, git, ui, util, workspace};

pub struct Opts {
    pub dir: Option<PathBuf>,
    pub project: Option<String>,
    pub repo: Option<String>,
    pub yes: bool,
    pub reset: bool,
    pub repos: Option<Vec<String>>,
    pub workspace: Option<PathBuf>,
    pub from: Option<String>,
    pub dry_run: bool,
    pub depth: u32,
}

pub fn run(o: Opts) -> Result<u8> {
    if let Some(src) = &o.from {
        return setup::from_source(src, o.project, o.dry_run);
    }
    let dir = match &o.dir {
        Some(d) => util::canon(&util::expand_tilde(&d.to_string_lossy())),
        None => std::env::current_dir()?,
    };
    if !dir.is_dir() {
        bail!("not a folder: {}", dir.display());
    }
    let ask = util::interactive() && !ui::json() && !o.yes;
    let here = git::root(&dir).ok().map(|p| util::canon(&p));
    // Siblings are looked for in the folder around this repo (or in DIR).
    let base = here
        .as_deref()
        .and_then(Path::parent)
        .map_or_else(|| dir.clone(), Path::to_path_buf);
    let current = here.as_deref().and_then(setup::project_at);

    if o.reset
        && let Some(root) = &here
        && let Some(path) = config::config_path(root)?
    {
        return reset(&path, o.repo, ask);
    }

    // `--yes` / `--repo`, and plain `kex init` without a terminal (as in kex 0.1): this repo.
    let single = o.yes
        || o.repo.is_some()
        || (!ask && !ui::json() && o.repos.is_none() && o.workspace.is_none());
    // Kex repos whose projects can be joined: this one, and the ones seen on the way.
    let mut near: Vec<PathBuf> = here.iter().cloned().collect();
    let mut repos: Vec<Value> = if let Some(f) = &o.workspace {
        let w = workspace::load(&util::expand_tilde(&f.to_string_lossy()))?;
        match use_workspace(&w, ask, &mut near)? {
            Some(r) => r,
            None if ask => browse_repos(&base, here.as_deref(), &current, o.depth, &mut near)?,
            None => Vec::new(),
        }
    } else if let Some(list) = &o.repos {
        list.iter()
            .map(|w| w.trim())
            .filter(|w| !w.is_empty())
            .map(|w| json!({ "path": lossy(&base.join(util::expand_tilde(w))) }))
            .collect()
    } else if single {
        let Some(root) = &here else {
            bail!(
                "not in a git repo; run kex init in one, or pick repos with kex init <DIR>, --repos or --from"
            );
        };
        if let Some(path) = config::config_path(root)? {
            say!(
                "already a kex repo: {} (use --reset to give it new ids)",
                util::tilde(&path)
            );
            return Ok(0);
        }
        vec![json!({ "path": lossy(root), "name": o.repo })]
    } else if !ask {
        bail!(
            "kex init --json asks nothing: list folders with `kex browse --json`, then send the repos with `kex init --json --from -` (or pass --yes or --repos)"
        );
    } else {
        let mut dirs = workspace::search_dirs(&dir);
        if !dirs.contains(&base) {
            dirs.push(base.clone());
        }
        match offer_workspace(&dirs, &mut near)? {
            Some(r) => r,
            None => browse_repos(&base, here.as_deref(), &current, o.depth, &mut near)?,
        }
    };
    if repos.is_empty() {
        say!("nothing selected");
        return Ok(0);
    }

    let paths: Vec<PathBuf> = repos
        .iter()
        .filter_map(|r| r["path"].as_str().map(PathBuf::from))
        .collect();
    let project = match (o.project, current) {
        (Some(f), Some(c)) if f != c => {
            bail!("this repo is in {c}; run kex init outside it to add repos to {f}")
        }
        (_, Some(c)) => json!(c),
        // An id nobody uses yet makes a project with that id (as in kex 0.1).
        (Some(f), None) => {
            paths_and(&mut near, &paths);
            let cwd = std::env::current_dir()?;
            if setup::known_projects(&near, &cwd).contains_key(&f) {
                json!(f)
            } else {
                json!({ "new": f, "id": f })
            }
        }
        (None, None) => {
            paths_and(&mut near, &paths);
            let nearby = setup::nearby(&near);
            let default = folder_name(Some(&base)).unwrap_or_else(|| "project".into());
            if ask {
                ask_project(&nearby, &default)?
            } else if nearby.len() == 1 {
                json!(nearby.keys().next())
            } else {
                // A new project named after the folder around the first repo.
                Value::Null
            }
        }
    };
    if ask {
        let cwd = std::env::current_dir()?;
        let known = setup::known_projects(&near, &cwd);
        let taken: HashSet<&str> = project
            .as_str()
            .and_then(|id| known.get(id))
            .map(|m| m.keys().map(String::as_str).collect())
            .unwrap_or_default();
        ask_names(&mut repos, &taken)?;
    }
    setup::run(
        json!({ "project": project, "repos": repos }),
        None,
        o.dry_run,
        &near,
    )
}

/// `paths` added to `near` (no duplicates).
fn paths_and(near: &mut Vec<PathBuf>, paths: &[PathBuf]) {
    for p in paths {
        if !near.contains(p) {
            near.push(p.clone());
        }
    }
}

/// `--reset`: a copied config (§4.4) gets a new repo_id and a new name, so nothing clashes.
fn reset(path: &Path, repo: Option<String>, ask: bool) -> Result<u8> {
    let mut c = config::load(path)?;
    let name = match repo {
        Some(n) => match util::repo_name_problem(&n) {
            Some(p) => bail!("--repo: {p}"),
            None => n,
        },
        None if ask => ui::text_checked(
            "New repo name:",
            &format!("{}-2", c.repo),
            util::repo_name_problem,
        )?,
        None => bail!("pass --repo <name> with --reset"),
    };
    c.repo_id = config::new_repo_id();
    c.repo = name;
    config::save(path, &c)?;
    say!(
        "new repo_id {} and name \"{}\" written to {}. Review and commit it.",
        c.repo_id,
        c.repo,
        util::tilde(path)
    );
    Ok(0)
}

/// The terminal browser, from `base`. The repo kex runs in starts checked when it isn't set
/// up yet. Kex repos seen on the way go into `near` (their projects can be joined).
fn browse_repos(
    base: &Path,
    here: Option<&Path>,
    current: &Option<String>,
    depth: u32,
    near: &mut Vec<PathBuf>,
) -> Result<Vec<Value>> {
    let pre: Vec<PathBuf> = here
        .filter(|_| current.is_none())
        .filter(|h| !git::is_worktree(h))
        .map(Path::to_path_buf)
        .into_iter()
        .collect();
    let picked = browse::pick(base, depth, pre)?;
    paths_and(near, &picked.kex_seen);
    Ok(picked
        .selected
        .into_iter()
        .map(|p| json!({ "path": lossy(&p.path), "name": p.name }))
        .collect())
}

/// Workspace files in `dirs`: none → `None`; one → "Use it?"; several → which one (or none).
fn offer_workspace(dirs: &[PathBuf], near: &mut Vec<PathBuf>) -> Result<Option<Vec<Value>>> {
    let mut loaded = Vec::new();
    for f in workspace::find(dirs) {
        match workspace::load(&f) {
            Ok(w) => loaded.push(w),
            Err(e) => warn!("{} skipping {}: {e:#}", util::yellow("⚠"), util::tilde(&f)),
        }
    }
    let file_name = |w: &workspace::Workspace| folder_name(Some(&w.file)).unwrap_or_default();
    let dir = |w: &workspace::Workspace| util::tilde(w.file.parent().unwrap_or(Path::new("/")));
    let w = match loaded.len() {
        0 => return Ok(None),
        1 => {
            let msg = format!(
                "Found {} in {}. Use it?",
                file_name(&loaded[0]),
                dir(&loaded[0])
            );
            if !ui::confirm(&msg, true)? {
                return Ok(None);
            }
            loaded.swap_remove(0)
        }
        n => {
            let mut opts: Vec<ui::Opt> = loaded
                .iter()
                .map(|w| {
                    let c = &w.json()["counts"];
                    ui::Opt::new(
                        lossy(&w.file),
                        format!(
                            "{}  {} · {} repos, {} already kex",
                            file_name(w),
                            dir(w),
                            c["repos"],
                            c["already_kex"]
                        ),
                    )
                })
                .collect();
            opts.push(ui::Opt::new("skip", "Don't use a workspace file"));
            let i = ui::select_one(&format!("Found {n} workspace files. Use which one?"), &opts)?;
            if i == n {
                return Ok(None);
            }
            loaded.swap_remove(i)
        }
    };
    use_workspace(&w, true, near)
}

/// A workspace file's repos without a kex config, after saying what it lists (and, when
/// asking, "Init these N repos?"; `None` = no). Its kex repos go into `near`.
fn use_workspace(
    w: &workspace::Workspace,
    ask: bool,
    near: &mut Vec<PathBuf>,
) -> Result<Option<Vec<Value>>> {
    use crate::tree::Kind;
    say!(
        "checking the {} folders of {} for kex.json…",
        w.folders.len(),
        folder_name(Some(&w.file)).unwrap_or_default()
    );
    for f in &w.folders {
        let shown = f
            .resolved
            .as_deref()
            .map(util::tilde)
            .unwrap_or_else(|| f.written.clone());
        if let Some(i) = f.duplicate_of {
            say!(
                "  {} {shown}  listed again (as {}); counted once",
                util::dim("·"),
                w.folders[i].written
            );
            continue;
        }
        match (f.exists, f.kind) {
            (false, _) => warn!("  {} missing   {}", util::yellow("⚠"), f.written),
            (_, Some(Kind::Repo)) => say!(
                "  {} {:<12} {shown}  git repo, no kex.json yet",
                util::green("+"),
                f.default_name()
            ),
            (_, Some(Kind::KexRepo)) => say!(
                "  {} {:<12} {shown}  already kex ({})",
                util::dim("·"),
                f.repo.as_deref().unwrap_or("?"),
                f.project.as_deref().unwrap_or("unreadable config")
            ),
            (_, Some(Kind::Worktree)) => {
                say!("  {} {shown}  a worktree (skipped)", util::dim("·"))
            }
            _ => say!("  {} {shown}  not a git repo", util::dim("·")),
        }
    }
    let kex: Vec<PathBuf> = w
        .unique()
        .filter(|f| f.kind == Some(Kind::KexRepo))
        .filter_map(|f| f.resolved.clone())
        .collect();
    paths_and(near, &kex);
    let repos: Vec<Value> = w
        .addable()
        .filter_map(|f| {
            Some(json!({ "path": lossy(f.resolved.as_ref()?), "name": f.default_name() }))
        })
        .collect();
    if repos.is_empty() {
        say!("nothing to set up from it: no git repo there is without a kex.json");
        return Ok(Some(repos));
    }
    let names: Vec<&str> = repos.iter().filter_map(|r| r["name"].as_str()).collect();
    let msg = match names.len() {
        1 => format!("Init this repo as a kex workspace? {}", names[0]),
        n => format!(
            "Init these {n} repos as a kex workspace? {}",
            names.join(", ")
        ),
    };
    Ok((!ask || ui::confirm(&msg, true)?).then_some(repos))
}

/// Join a project next to the repos, or create one.
fn ask_project(nearby: &setup::Projects, default: &str) -> Result<Value> {
    let mut opts: Vec<ui::Opt> = nearby
        .iter()
        .map(|(id, rs)| {
            let names: Vec<&str> = rs.keys().map(String::as_str).collect();
            ui::Opt::new(id, format!("Join {id} ({})", names.join(", ")))
        })
        .collect();
    opts.push(ui::Opt::new("new", "Create a new project"));
    let i = if opts.len() == 1 {
        0
    } else {
        ui::select_one("Project", &opts)?
    };
    if let Some(id) = nearby.keys().nth(i) {
        return Ok(json!(id));
    }
    let name = ui::text_checked(
        "New project name:",
        &util::slug(default),
        util::project_name_problem,
    )?;
    Ok(json!({ "new": name }))
}

/// Repo names: one confirm when the defaults are fine, else one question per repo.
fn ask_names(repos: &mut [Value], taken: &HashSet<&str>) -> Result<()> {
    let path = |r: &Value| PathBuf::from(r["path"].as_str().unwrap_or_default());
    let defaults: Vec<String> = repos
        .iter()
        .map(|r| match r["name"].as_str() {
            Some(n) => n.to_string(),
            None => setup::default_name(&path(r)),
        })
        .collect();
    let unique: HashSet<&str> = defaults.iter().map(String::as_str).collect();
    let fine = unique.len() == defaults.len()
        && defaults
            .iter()
            .all(|n| setup::name_problem(n, taken).is_none());
    if fine
        && ui::confirm(
            &format!("Use these repo names? {}", defaults.join(", ")),
            true,
        )?
    {
        return Ok(());
    }
    let mut names: Vec<String> = Vec::new();
    for (r, default) in repos.iter_mut().zip(defaults) {
        let problem = |n: &str| {
            setup::name_problem(n, taken).or_else(|| {
                names
                    .iter()
                    .any(|x| x == n)
                    .then(|| format!("two repos would be named \"{n}\""))
            })
        };
        let suggest = (1..)
            .map(|i| match i {
                1 => default.clone(),
                _ => format!("{default}-{i}"),
            })
            .find(|n| problem(n).is_none())
            .unwrap_or_default();
        let folder = folder_name(Some(&path(r))).unwrap_or_default();
        let n = ui::text_checked(&format!("Repo name for {folder}:"), &suggest, problem)?;
        r["name"] = json!(n);
        names.push(n);
    }
    Ok(())
}
