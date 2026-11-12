//! `kex init`: write this repo's `kex.json`, joining a project found next to it or creating
//! one (BLUEPRINT §4.7).

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::{Result, bail};

use crate::config::{self, Config, Peer, PrimeBranch, Stage};
use crate::project::{self, Repo};
use crate::registry::Registry;
use crate::{git, util};

pub struct Opts {
    pub project: Option<String>,
    pub repo: Option<String>,
    pub yes: bool,
    pub reset: bool,
}

pub fn run(o: Opts) -> Result<u8> {
    let root = util::canon(&git::root(&std::env::current_dir()?)?);
    let ask = util::interactive() && !o.yes;

    if let Some(path) = config::config_path(&root)? {
        if !o.reset {
            println!(
                "already a kex repo: {} (use --reset to give it new ids)",
                util::tilde(&path)
            );
            return Ok(0);
        }
        // A copied config (§4.4): new repo_id and a new name, so nothing clashes.
        let mut c = config::load(&path)?;
        let name = match o.repo {
            Some(n) => n,
            None if ask => inquire::Text::new("New repo name:")
                .with_default(&format!("{}-2", c.repo))
                .prompt()?,
            None => bail!("pass --repo <name> with --reset"),
        };
        c.repo_id = config::new_repo_id();
        c.repo = name;
        config::save(&path, &c)?;
        println!(
            "new repo_id {} and name \"{}\" written to {}. Review and commit it.",
            c.repo_id,
            c.repo,
            util::tilde(&path)
        );
        return Ok(0);
    }

    // Kex repos next to this one, grouped by project.
    let mut nearby: BTreeMap<String, Vec<Repo>> = BTreeMap::new();
    if let Some(g) = root.parent().and_then(Path::parent) {
        let mut candidates = Vec::new();
        project::find_git_dirs(g, 2, &mut candidates);
        for d in candidates {
            let d = util::canon(&d);
            if d == root || !matches!(config::config_path(&d), Ok(Some(_))) {
                continue;
            }
            if let Some(r) = project::open(&d, None).filter(|r| !r.worktree) {
                nearby.entry(r.config.project.clone()).or_default().push(r);
            }
        }
    }
    let new_project = |name: &str| format!("{}-{}", util::slug(name), config::random_suffix());
    let default_name = root
        .parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".into());
    let ids: Vec<&String> = nearby.keys().collect();
    let project = match o.project {
        Some(p) => p,
        None if ask => {
            let mut labels: Vec<String> = nearby
                .iter()
                .map(|(id, rs)| {
                    format!(
                        "Join {id} ({})",
                        rs.iter()
                            .map(|r| r.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
                .collect();
            labels.push("Create a new project".into());
            let i = if labels.len() == 1 {
                0
            } else {
                inquire::Select::new("Project", labels).raw_prompt()?.index
            };
            if i < ids.len() {
                ids[i].clone()
            } else {
                new_project(
                    &inquire::Text::new("New project name:")
                        .with_default(&util::slug(&default_name))
                        .prompt()?,
                )
            }
        }
        None if ids.len() == 1 => ids[0].clone(),
        None => new_project(&default_name),
    };

    let folder = root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let repo = match o.repo {
        Some(r) => r,
        None if ask => inquire::Text::new("Repo name (short, used in flags like --repos):")
            .with_default(&util::slug(&folder))
            .prompt()?,
        None => util::slug(&folder),
    };
    let members = nearby.get(&project).map(Vec::as_slice).unwrap_or_default();
    if members.iter().any(|r| r.name == repo) {
        bail!("a repo named \"{repo}\" is already in {project}; pick another with --repo");
    }
    let peers = members
        .iter()
        .filter_map(|r| {
            let remote = git::shared_remote(&r.path).and_then(|n| git::remote_url(&r.path, &n))?;
            Some(Peer {
                repo: r.name.clone(),
                remote,
            })
        })
        .collect();

    let config = Config {
        schema: Some(config::SCHEMA_URL.into()),
        version: config::SUPPORTED_VERSION,
        project: project.clone(),
        repo: repo.clone(),
        repo_id: config::new_repo_id(),
        hosting: None,
        prime_branches: detect_primes(&root),
        protected: Vec::new(),
        ignore_branches: Vec::new(),
        peers,
        run: None,
    };
    let path = if root.join(".config").is_dir() {
        root.join(".config").join("kex.json")
    } else {
        root.join("kex.json")
    };
    config::save(&path, &config)?;
    ignore_local(&root, &path)?;

    let mut reg = Registry::load()?;
    let prime = config.default_prime().map(|b| b.branch.as_str());
    reg.register(
        &project,
        &config.repo_id,
        &repo,
        git::root_commit(&root).as_deref(),
        prime,
        &root,
    );
    reg.save()?;

    println!("wrote {}", util::tilde(&path));
    println!("  project  {project}");
    println!("  repo     {repo} ({})", config.repo_id);
    for b in &config.prime_branches {
        println!("  prime    {} ({:?})", b.branch, b.stage);
    }
    println!(
        "Check the prime branches, then commit the file. Run kex init in the other repos to join {project}."
    );
    Ok(0)
}

/// dev/develop + main/master when present, else the current branch.
fn detect_primes(root: &Path) -> Vec<PrimeBranch> {
    let mut names: HashSet<String> = HashSet::new();
    for (prefix, strip) in [("refs/heads", "2"), ("refs/remotes", "3")] {
        let format = format!("--format=%(refname:lstrip={strip})");
        if let Ok(out) = git::git(root, &["for-each-ref", &format, prefix]) {
            names.extend(out.lines().map(String::from));
        }
    }
    let pick = |opts: &[&str]| {
        opts.iter()
            .find(|b| names.contains(**b))
            .map(|b| b.to_string())
    };
    let pb = |branch: String, stage, description: &str| PrimeBranch {
        branch,
        stage,
        description: description.into(),
    };
    match (
        pick(&["dev", "develop", "development"]),
        pick(&["main", "master"]),
    ) {
        (Some(d), Some(m)) => vec![
            pb(d, Stage::Dev, "Integration branch"),
            pb(m, Stage::Production, "Live"),
        ],
        (Some(d), None) => vec![pb(d, Stage::Dev, "Integration branch")],
        (None, Some(m)) => vec![pb(m, Stage::Dev, "Main branch")],
        (None, None) => vec![pb(git::current_branch(root), Stage::Dev, "Main branch")],
    }
}

/// `kex.local.json` holds personal overrides, so it must never be committed.
fn ignore_local(root: &Path, config_path: &Path) -> Result<()> {
    let rel = config_path
        .with_file_name("kex.local.json")
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| "kex.local.json".into());
    let gitignore = root.join(".gitignore");
    let text = std::fs::read_to_string(&gitignore).unwrap_or_default();
    if text.lines().any(|l| l.trim() == rel) {
        return Ok(());
    }
    let sep = if text.is_empty() || text.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    std::fs::write(&gitignore, format!("{text}{sep}{rel}\n"))?;
    Ok(())
}
