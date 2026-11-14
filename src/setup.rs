//! Setting repos up: the one place that checks, plans and writes `kex.json` files
//! (BLUEPRINT §4.7). Every way into `kex init` ends here: the terminal browser, `--yes`,
//! `--repos` and `--workspace` build the same request a program sends with `--from`.
//!
//! Request (JSON): `[{path, name?, description?, repo_id?}]`, or
//! `{project?: "<id>" | {new: "<name>", id?}, repos: [...]}`.
//!
//! Everything is checked before anything is written, and every problem is reported at once
//! (one `error` event each, with `index` and `field`); then exit 2. With `--dry-run`, a
//! `plan` event says what would be written. Otherwise the configs are written all or
//! nothing: if one can't be, those already written are undone.

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};

use crate::config::{self, Config, Peer, PrimeBranch, Stage};
use crate::registry::Registry;
use crate::tree::{self, Kind};
use crate::{git, project, ui, util};

/// Project id → repo name → path.
pub type Projects = BTreeMap<String, BTreeMap<String, PathBuf>>;

/// One thing wrong with the input. `index` = which repo (in `repos`), `field` = which key.
struct Problem {
    index: Option<usize>,
    field: Option<String>,
    message: String,
}

#[derive(Default)]
struct Problems(Vec<Problem>);

impl Problems {
    fn add(&mut self, index: Option<usize>, field: Option<&str>, message: impl Into<String>) {
        self.0.push(Problem {
            index,
            field: field.map(String::from),
            message: message.into(),
        });
    }

    fn json(&self) -> Vec<Value> {
        self.0
            .iter()
            .map(|p| json!({ "message": p.message, "index": p.index, "field": p.field }))
            .collect()
    }

    /// Each problem as an `error` event (JSON) or a `kex: repos[i].field: …` line.
    fn report(&self) {
        for p in &self.0 {
            if ui::json() {
                ui::emit(
                    "error",
                    json!({ "message": p.message, "index": p.index, "field": p.field }),
                );
            } else {
                let at = match (p.index, p.field.as_deref()) {
                    (Some(i), Some(f)) => format!("repos[{i}].{f}: "),
                    (Some(i), None) => format!("repos[{i}]: "),
                    (None, Some(f)) => format!("{f}: "),
                    (None, None) => String::new(),
                };
                eprintln!("kex: {at}{}", p.message);
            }
        }
    }
}

struct Item {
    index: usize,
    /// The path as given, made absolute (not resolved: a symlink keeps its own name).
    given: PathBuf,
    /// Resolved (symlinks followed); `None` when it isn't a usable repo.
    path: Option<PathBuf>,
    name: Option<String>,
    description: Option<String>,
    /// A `repo_id` to use instead of a new one (from a dry run's `plan.request`).
    repo_id: Option<String>,
    /// Already a kex repo: its config.
    existing: Option<Config>,
}

/// The project the repos join.
struct Target {
    id: String,
    new: bool,
    name: Option<String>,
}

/// `--from <file|->`: read the request, then [`run`].
pub fn from_source(source: &str, project_flag: Option<String>, dry_run: bool) -> Result<u8> {
    let text = if source == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .context("reading the repos from stdin")?;
        s
    } else {
        let p = util::expand_tilde(source);
        std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => run(v, project_flag, dry_run, &[]),
        Err(e) => {
            let mut problems = Problems::default();
            problems.add(None, None, format!("not valid JSON: {e}"));
            problems.report();
            Ok(2)
        }
    }
}

/// Check, plan and (unless `dry_run`) write a request. `near`: kex repos the caller saw
/// (e.g. while browsing), whose projects can be joined.
pub fn run(
    input: Value,
    project_flag: Option<String>,
    dry_run: bool,
    near: &[PathBuf],
) -> Result<u8> {
    let cwd = std::env::current_dir()?;
    let mut problems = Problems::default();

    let (project_value, raw_items) = match input {
        Value::Array(a) => (None, a),
        Value::Object(o) => {
            for k in o.keys() {
                if k != "project" && k != "repos" {
                    problems.add(
                        None,
                        Some(k),
                        format!("unknown field \"{k}\" (expected project, repos)"),
                    );
                }
            }
            let repos = match o.get("repos") {
                Some(Value::Array(a)) => a.clone(),
                Some(_) => {
                    problems.add(None, Some("repos"), "must be an array");
                    Vec::new()
                }
                None => {
                    problems.add(None, Some("repos"), "missing");
                    Vec::new()
                }
            };
            (o.get("project").cloned(), repos)
        }
        _ => {
            problems.add(
                None,
                None,
                "expected an array of repos or an object {project, repos}",
            );
            (None, Vec::new())
        }
    };
    if raw_items.is_empty() && problems.0.is_empty() {
        problems.add(None, Some("repos"), "no repos given");
    }

    let mut items: Vec<Item> = Vec::new();
    for (i, raw) in raw_items.iter().enumerate() {
        if let Some(it) = parse_item(i, raw, &cwd, &items, &mut problems) {
            items.push(it);
        }
    }

    let mut paths: Vec<PathBuf> = items.iter().filter_map(|it| it.path.clone()).collect();
    paths.extend(near.iter().cloned());
    let known = known_projects(&paths, &cwd);
    let target = resolve_project(
        project_value,
        project_flag,
        &items,
        &known,
        &cwd,
        &mut problems,
    );
    let members = target
        .as_ref()
        .filter(|t| !t.new)
        .and_then(|t| known.get(&t.id).cloned())
        .unwrap_or_default();

    // Already kex repos: fine in this project (skipped), an error in another one.
    let mut skipped: Vec<Value> = Vec::new();
    let mut todo: Vec<&Item> = Vec::new();
    for it in &items {
        let Some(path) = &it.path else { continue };
        match (&it.existing, &target) {
            (Some(c), Some(t)) if c.project == t.id => skipped.push(skipped_json(it, path, c)),
            (Some(c), _) => problems.add(
                Some(it.index),
                Some("path"),
                format!(
                    "already a kex repo in project {} (as \"{}\"); kex doesn't move repos between projects",
                    c.project, c.repo
                ),
            ),
            (None, _) => todo.push(it),
        }
    }

    // Names: valid, not taken in the project, not twice in this batch.
    let taken: HashSet<&str> = members.keys().map(String::as_str).collect();
    let mut names: Vec<String> = Vec::new();
    for (k, it) in todo.iter().enumerate() {
        let name = it.name.clone().unwrap_or_else(|| default_name(&it.given));
        if let Some(p) = name_problem(&name, &taken) {
            problems.add(Some(it.index), Some("name"), p);
        } else if let Some(j) = names[..k].iter().position(|n| *n == name) {
            problems.add(
                Some(it.index),
                Some("name"),
                format!(
                    "two repos would be named \"{name}\" (repos[{}] too)",
                    todo[j].index
                ),
            );
        }
        names.push(name);
    }

    // Pinned repo ids: not twice, not already used by any repo kex knows.
    let used: HashSet<String> = Registry::load()
        .map(|r| {
            r.projects
                .values()
                .flat_map(|p| p.keys().cloned())
                .collect()
        })
        .unwrap_or_default();
    for (k, it) in todo.iter().enumerate() {
        let Some(id) = &it.repo_id else { continue };
        if used.contains(id) {
            problems.add(
                Some(it.index),
                Some("repo_id"),
                format!("{id} is already used by another repo"),
            );
        } else if let Some(j) = todo[..k]
            .iter()
            .find(|o| o.repo_id.as_deref() == Some(id.as_str()))
        {
            problems.add(
                Some(it.index),
                Some("repo_id"),
                format!("{id} is given twice (repos[{}] too)", j.index),
            );
        }
    }

    // What each repo gets written (reading .gitignore can fail too: still before writing).
    // Every new config lists the members with a remote and the other repos of this batch.
    let mut plans = Vec::new();
    if let Some(t) = &target
        && problems.0.is_empty()
    {
        let member_peers: Vec<Peer> = members.iter().filter_map(|(n, p)| peer(n, p)).collect();
        let batch_peers: Vec<Option<Peer>> = todo
            .iter()
            .zip(&names)
            .map(|(it, n)| peer(n, it.path.as_deref()?))
            .collect();
        for (k, it) in todo.iter().enumerate() {
            let mut peers = member_peers.clone();
            peers.extend(
                batch_peers
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != k)
                    .filter_map(|(_, p)| p.clone()),
            );
            let path = it.path.as_deref().unwrap_or(Path::new("."));
            let config = Config {
                schema: Some(config::SCHEMA_URL.into()),
                version: config::SUPPORTED_VERSION,
                project: t.id.clone(),
                repo: names[k].clone(),
                repo_id: it.repo_id.clone().unwrap_or_else(config::new_repo_id),
                description: it.description.clone(),
                hosting: None,
                prime_branches: detect_primes(path),
                protected: Vec::new(),
                ignore_branches: Vec::new(),
                peers,
                run: None,
            };
            match plan(path, config) {
                Ok(p) => plans.push((it.index, p)),
                Err(e) => problems.add(Some(it.index), Some("path"), format!("{e:#}")),
            }
        }
    }

    let project_json = target
        .as_ref()
        .map(|t| json!({ "id": t.id, "new": t.new, "name": t.name }));
    let Some(target) = target.filter(|_| problems.0.is_empty()) else {
        problems.report();
        if dry_run {
            ui::event(
                "plan",
                json!({ "valid": false, "project": project_json, "repos": [], "skipped": skipped, "errors": problems.json() }),
            );
        }
        return Ok(2);
    };
    say!(
        "{} {}{}",
        util::bold("project"),
        target.id,
        if target.new { " (new)" } else { "" }
    );

    if dry_run {
        let repos: Vec<Value> = plans
            .iter()
            .map(|(i, p)| {
                json!({
                    "index": i,
                    "path": lossy(&p.root),
                    "repo": p.config.repo,
                    "repo_id": p.config.repo_id,
                    "config_path": lossy(&p.path),
                    "config": p.config,
                    "gitignore": { "path": lossy(&p.gitignore), "add": p.gitignore_line },
                })
            })
            .collect();
        for (_, p) in &plans {
            say!("would write {}:", util::tilde(&p.path));
            say!("{}", serde_json::to_string_pretty(&p.config)?);
            if let Some(l) = &p.gitignore_line {
                say!("and add {l} to {}", util::tilde(&p.gitignore));
            }
        }
        // The same input with every id pinned: sent back as it is, the real run writes
        // exactly the configs above.
        let mut entries: Vec<(usize, Value)> = plans
            .iter()
            .map(|(i, p)| {
                let mut v = json!({
                    "path": lossy(&p.root),
                    "name": p.config.repo,
                    "repo_id": p.config.repo_id,
                });
                if let Some(d) = &p.config.description {
                    v["description"] = json!(d);
                }
                (*i, v)
            })
            .collect();
        for sk in &skipped {
            let i = sk["index"].as_u64().unwrap_or(0) as usize;
            entries.push((i, json!({ "path": sk["path"] })));
        }
        entries.sort_by_key(|(i, _)| *i);
        let request = json!({
            "project": if target.new {
                json!({ "new": target.name, "id": target.id })
            } else {
                json!(target.id)
            },
            "repos": entries.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
        });
        ui::event(
            "plan",
            json!({ "valid": true, "project": project_json, "repos": repos, "skipped": skipped, "errors": [], "request": request }),
        );
        return Ok(0);
    }

    let plans: Vec<Plan> = plans.into_iter().map(|(_, p)| p).collect();
    let failed = apply(&plans)?;
    if !failed.is_empty() {
        // Writes stop at the first failure: the plans before it were written, then undone.
        let rolled_back = failed[0].0 > 0;
        let failed: Vec<Value> = failed
            .iter()
            .map(|(k, e)| {
                let p = &plans[*k];
                warn!("{} {}: {e}", util::red("✗"), p.config.repo);
                json!({ "repo": p.config.repo, "path": lossy(&p.root), "message": e })
            })
            .collect();
        warn!("nothing was written (all or nothing)");
        ui::event(
            "added",
            json!({ "project": target.id, "repos": [], "skipped": skipped, "failed": failed, "rolled_back": rolled_back }),
        );
        return Ok(1);
    }
    let mut reg = Registry::load()?;
    for p in &plans {
        register(&mut reg, p);
        let primes: Vec<&str> = p
            .config
            .prime_branches
            .iter()
            .map(|b| b.branch.as_str())
            .collect();
        say!(
            "{} {:<12} {} ({}, prime {})",
            util::green("✓"),
            p.config.repo,
            util::tilde(&p.path),
            p.config.repo_id,
            primes.join(", ")
        );
    }
    reg.save()?;
    for sk in &skipped {
        say!(
            "  {} {:<12} already in {}",
            util::dim("·"),
            sk["repo"].as_str().unwrap_or_default(),
            target.id
        );
    }
    ui::event(
        "added",
        json!({
            "project": target.id,
            "repos": plans.iter().map(added_json).collect::<Vec<_>>(),
            "skipped": skipped,
            "failed": [],
        }),
    );
    if !plans.is_empty() {
        say!("Check the prime branches, then commit kex.json in each repo.");
    }
    Ok(0)
}

/// An already set-up repo of the target project in `skipped`, with what the input asks for
/// but kex leaves alone (it never edits a kex.json).
fn skipped_json(it: &Item, path: &Path, c: &Config) -> Value {
    let mut v = json!({
        "index": it.index,
        "path": lossy(path),
        "folder": folder_name(Some(path)),
        "reason": "already_kex",
        "in_project": true,
        "repo": c.repo,
        "project": c.project,
    });
    let mut differs = Vec::new();
    if it.name.as_ref().is_some_and(|n| *n != c.repo) {
        differs.push(format!("name (it is \"{}\")", c.repo));
    }
    if it.description.is_some() && it.description != c.description {
        differs.push("description".to_string());
    }
    if it.repo_id.as_ref().is_some_and(|r| *r != c.repo_id) {
        differs.push(format!("repo_id (it is {})", c.repo_id));
    }
    if !differs.is_empty() {
        v["differs"] = json!(
            differs
                .iter()
                .map(|d| d.split(' ').next().unwrap_or_default())
                .collect::<Vec<_>>()
        );
        v["note"] = json!(format!(
            "already set up; not changed: {} (edit its kex.json to change it)",
            differs.join(", ")
        ));
    }
    v
}

/// What's wrong with a repo name: the schema's rule, or taken in the project.
pub fn name_problem(n: &str, taken: &HashSet<&str>) -> Option<String> {
    util::repo_name_problem(n).or_else(|| {
        taken
            .contains(n)
            .then(|| format!("a repo named \"{n}\" is already in this project"))
    })
}

/// The repo name kex suggests for the folder at `path`: the slug of its own name.
pub fn default_name(path: &Path) -> String {
    util::slug(&folder_name(Some(path)).unwrap_or_default())
}

fn optional_string(
    o: &Map<String, Value>,
    key: &'static str,
    i: usize,
    problems: &mut Problems,
) -> Option<String> {
    match o.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.trim().is_empty() => None,
        Some(Value::String(s)) => Some(s.trim().to_string()),
        Some(_) => {
            problems.add(Some(i), Some(key), "must be a string");
            None
        }
    }
}

fn parse_item(
    i: usize,
    raw: &Value,
    cwd: &Path,
    earlier: &[Item],
    problems: &mut Problems,
) -> Option<Item> {
    let Value::Object(o) = raw else {
        problems.add(
            Some(i),
            None,
            "must be an object {path, name?, description?}",
        );
        return None;
    };
    for k in o.keys() {
        if !["path", "name", "description", "repo_id"].contains(&k.as_str()) {
            problems.add(
                Some(i),
                Some(k),
                format!("unknown field \"{k}\" (expected path, name, description, repo_id)"),
            );
        }
    }
    let name = optional_string(o, "name", i, problems);
    let description = optional_string(o, "description", i, problems);
    if let Some(p) = description.as_deref().and_then(util::description_problem) {
        problems.add(Some(i), Some("description"), p);
    }
    let repo_id = optional_string(o, "repo_id", i, problems);
    if let Some(id) = &repo_id
        && !valid_repo_id(id)
    {
        problems.add(
            Some(i),
            Some("repo_id"),
            format!("\"{id}\" isn't a repo id: r_ and 1 to 32 lowercase letters or digits"),
        );
    }
    let raw_path = match o.get("path") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim(),
        Some(_) => {
            problems.add(Some(i), Some("path"), "must be a non-empty string");
            return None;
        }
        None => {
            problems.add(Some(i), Some("path"), "missing");
            return None;
        }
    };
    let given = cwd.join(util::expand_tilde(raw_path));
    let mut it = Item {
        index: i,
        path: None,
        name,
        description,
        repo_id,
        existing: None,
        given,
    };
    let mut bad = |m: String| problems.add(Some(i), Some("path"), m);
    if !it.given.exists() {
        bad(format!("{} doesn't exist", it.given.display()));
        return Some(it);
    }
    if !it.given.is_dir() {
        bad(format!("{} isn't a folder", it.given.display()));
        return Some(it);
    }
    let real = util::canon(&it.given);
    if !real.join(".git").exists() {
        bad(match git::root(&real) {
            Ok(r) => format!(
                "{} isn't the top folder of a git repo (that is {})",
                real.display(),
                util::canon(&r).display()
            ),
            Err(_) => format!("{} isn't a git repo", real.display()),
        });
        return Some(it);
    }
    if let Some(j) = earlier.iter().find(|e| e.path.as_deref() == Some(&real)) {
        bad(format!("the same repo as repos[{}]", j.index));
        return Some(it);
    }
    match tree::classify(&real) {
        Kind::Worktree => {
            let main = git::main_checkout(&real)
                .map(|m| format!(" ({})", m.display()))
                .unwrap_or_default();
            bad(format!(
                "{} is a linked worktree; add its main checkout instead{main}",
                real.display()
            ));
            return Some(it);
        }
        Kind::KexRepo => match config::config_path(&real).and_then(|p| match p {
            Some(p) => config::load(&p),
            None => bail!("no config"),
        }) {
            Ok(c) => it.existing = Some(c),
            Err(e) => {
                bad(format!("has a kex config that can't be used: {e:#}"));
                return Some(it);
            }
        },
        _ => {}
    }
    it.path = Some(real);
    Some(it)
}

/// The kex repos next to `paths` (their siblings and their parents' siblings, like
/// `kex status` looks), and the kex repos in `paths` themselves, by project.
pub fn nearby(paths: &[PathBuf]) -> Projects {
    let mut out = Projects::new();
    let mut dirs: Vec<PathBuf> = paths.to_vec();
    let mut scanned = HashSet::new();
    for p in paths {
        if let Some(g) = p.parent().and_then(Path::parent)
            && scanned.insert(g.to_path_buf())
        {
            dirs.extend(tree::git_dirs(g, 2));
        }
    }
    for d in dirs {
        add_kex_repo(&mut out, &d);
    }
    out
}

/// Projects kex can see from here: [`nearby`] `paths`, the registry and the repo kex runs
/// in.
pub fn known_projects(paths: &[PathBuf], cwd: &Path) -> Projects {
    let mut out = nearby(paths);
    if let Ok(reg) = Registry::load() {
        for entries in reg.projects.values() {
            for e in entries.values() {
                for p in e.paths.iter().filter(|p| p.is_dir()) {
                    add_kex_repo(&mut out, p);
                }
            }
        }
    }
    if let Ok(root) = git::root(cwd) {
        add_kex_repo(&mut out, &root);
    }
    out
}

/// A kex repo (not a worktree) at `path` into `out`. Registry entries can be stale; only the
/// configs on disk count.
fn add_kex_repo(out: &mut Projects, path: &Path) {
    if !matches!(config::config_path(path), Ok(Some(_))) {
        return;
    }
    if let Some(r) = project::open(path, None).filter(|r| !r.worktree) {
        out.entry(r.config.project.clone())
            .or_default()
            .entry(r.name.clone())
            .or_insert(r.path);
    }
}

/// The project of the kex repo at or above `dir`, if any.
pub fn project_at(dir: &Path) -> Option<String> {
    let root = git::root(dir).ok()?;
    let path = config::config_path(&root).ok()??;
    config::load(&path).ok().map(|c| c.project)
}

fn resolve_project(
    value: Option<Value>,
    flag: Option<String>,
    items: &[Item],
    known: &Projects,
    cwd: &Path,
    problems: &mut Problems,
) -> Option<Target> {
    let joined = |id: &str| Target {
        id: id.to_string(),
        new: false,
        name: None,
    };
    let existing = |id: &str, problems: &mut Problems| -> Option<Target> {
        if known.contains_key(id) {
            return Some(joined(id));
        }
        let list: Vec<&str> = known.keys().map(String::as_str).collect();
        problems.add(
            None,
            Some("project"),
            format!(
                "unknown project \"{id}\" (kex knows: {}); use {{\"new\": \"<name>\"}} to create one",
                if list.is_empty() { "none".to_string() } else { list.join(", ") }
            ),
        );
        None
    };
    let new = |name: &str, id: Option<String>| Target {
        id: id.unwrap_or_else(|| format!("{}-{}", util::slug(name), config::random_suffix())),
        new: true,
        name: Some(name.to_string()),
    };
    match value {
        None | Some(Value::Null) => {
            if let Some(f) = flag {
                return existing(&f, problems);
            }
            // The project of the kex repo kex runs in, else a new one named after the
            // folder around the first repo.
            if let Some(id) = project_at(cwd) {
                return Some(joined(&id));
            }
            let name = items
                .iter()
                .find_map(|it| it.path.as_deref().and_then(Path::parent))
                .and_then(|p| folder_name(Some(p)))
                .unwrap_or_else(|| "project".into());
            Some(new(&name, None))
        }
        Some(Value::String(id)) => {
            if let Some(f) = flag.filter(|f| *f != id) {
                problems.add(
                    None,
                    Some("project"),
                    format!("\"{id}\" here but --project {f}; pass one"),
                );
                return None;
            }
            existing(&id, problems)
        }
        Some(Value::Object(o)) => {
            for k in o.keys().filter(|k| *k != "new" && *k != "id") {
                problems.add(
                    None,
                    Some("project"),
                    format!("unknown field \"{k}\" (expected new, id)"),
                );
            }
            // `{"new": "Shop", "id": "shop-k2m9qa"}`: the id a dry run planned.
            let pinned = match o.get("id") {
                None | Some(Value::Null) => None,
                Some(Value::String(id)) => Some(id.trim().to_string()),
                Some(_) => {
                    problems.add(None, Some("project"), "\"id\" must be a string");
                    return None;
                }
            };
            let name = match o.get("new") {
                Some(Value::String(n)) => n.trim(),
                _ => {
                    problems.add(None, Some("project"), "{\"new\": …} needs a non-empty name");
                    return None;
                }
            };
            if let Some(p) = util::project_name_problem(name) {
                problems.add(None, Some("project"), p);
                return None;
            }
            match pinned {
                // `<slug of at most 40>-xxxxxx`, as kex makes them.
                Some(id) if !util::valid_repo_name(&id) || id.len() > util::NAME_MAX + 7 => {
                    problems.add(
                        None,
                        Some("project"),
                        format!("\"{id}\" isn't a project id: lowercase letters, digits and -"),
                    );
                    None
                }
                // Already there (e.g. the same request was sent before): join it, so
                // sending a plan's request twice is harmless.
                Some(id) if known.contains_key(&id) => Some(joined(&id)),
                pinned => Some(new(name, pinned)),
            }
        }
        Some(_) => {
            problems.add(
                None,
                Some("project"),
                "must be a project id or {\"new\": \"<name>\"}",
            );
            None
        }
    }
}

/// `r_9xk2mq`: what `kex init` makes (`r_` + 6), with some room.
fn valid_repo_id(id: &str) -> bool {
    id.strip_prefix("r_").is_some_and(|rest| {
        (1..=32).contains(&rest.len())
            && rest
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    })
}

// ── Writing ──────────────────────────────────────────────────────────────────

/// Last component of `p`, e.g. the folder name of a repo.
pub fn folder_name(p: Option<&Path>) -> Option<String> {
    p.and_then(Path::file_name)
        .map(|s| s.to_string_lossy().to_string())
}

/// Paths in events as strings; never fails on non-UTF-8 (unlike serializing a `Path`).
pub fn lossy(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// `repo` as a peer entry: its name and the URL of its shared remote. `None` without a remote.
fn peer(repo: &str, path: &Path) -> Option<Peer> {
    let remote = git::shared_remote(path).and_then(|n| git::remote_url(path, &n))?;
    Some(Peer {
        repo: repo.into(),
        remote,
    })
}

/// What setting up one repo writes: its `kex.json`, and the `.gitignore` line for
/// `kex.local.json` when it's missing.
struct Plan {
    root: PathBuf,
    /// The `kex.json` to write (`.config/kex.json` when `.config/` exists).
    path: PathBuf,
    config: Config,
    gitignore: PathBuf,
    /// The line added to `.gitignore`; `None` when it's already there.
    gitignore_line: Option<String>,
    gitignore_before: Option<Vec<u8>>,
    gitignore_after: Option<String>,
}

/// Work out what writing `config` into `root` takes, reading what's there.
fn plan(root: &Path, config: Config) -> Result<Plan> {
    let path = if root.join(".config").is_dir() {
        root.join(".config").join("kex.json")
    } else {
        root.join("kex.json")
    };
    let rel = path
        .with_file_name("kex.local.json")
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| "kex.local.json".into());
    let gitignore = root.join(".gitignore");
    let before = match std::fs::read(&gitignore) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("reading {}", gitignore.display())),
    };
    let text = String::from_utf8_lossy(before.as_deref().unwrap_or_default()).into_owned();
    let (line, after) = if text.lines().any(|l| l.trim() == rel) {
        (None, None)
    } else {
        let sep = if text.is_empty() || text.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        (Some(rel.clone()), Some(format!("{text}{sep}{rel}\n")))
    };
    Ok(Plan {
        root: root.to_path_buf(),
        path,
        config,
        gitignore,
        gitignore_line: line,
        gitignore_before: before,
        gitignore_after: after,
    })
}

/// One entry of `added.repos`.
fn added_json(p: &Plan) -> Value {
    let cfg = &p.config;
    json!({
        "repo": cfg.repo,
        "repo_id": cfg.repo_id,
        "path": lossy(&p.root),
        "config": lossy(&p.path),
        "prime_branches": cfg.prime_branches,
        "peers": cfg.peers.iter().map(|p| &p.repo).collect::<Vec<_>>(),
        "description": cfg.description,
    })
}

/// Record a written repo in the registry.
fn register(reg: &mut Registry, p: &Plan) {
    reg.register(
        &p.config.project,
        &p.config.repo_id,
        &p.config.repo,
        git::root_commit(&p.root).as_deref(),
        p.config.default_prime().map(|b| b.branch.as_str()),
        &p.root,
    );
}

/// Write the plans, all or nothing: each file goes to a temporary file next to it and is
/// renamed into place, so none is ever half-written, and the first failure undoes the plans
/// already written. Returns the failure (plan index, message), or nothing. SIGINT / SIGTERM
/// during the writes undo everything written so far and return [`crate::ui::Cancelled`].
fn apply(plans: &[Plan]) -> Result<Vec<(usize, String)>> {
    let guard = signals::Guard::install();
    let pause = std::env::var("KEX_TEST_WRITE_PAUSE_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok());
    let mut written = 0;
    let mut failed = Vec::new();
    for (i, p) in plans.iter().enumerate() {
        if guard.hit() {
            break;
        }
        if let Err(e) = write_one(p) {
            failed.push((i, format!("{e:#}")));
            break;
        }
        written += 1;
        if let Some(ms) = pause {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
    }
    if guard.hit() || !failed.is_empty() {
        for p in plans[..written].iter().rev() {
            undo(p);
        }
    }
    if guard.hit() {
        warn!("interrupted: nothing was written");
        return Err(ui::Cancelled.into());
    }
    Ok(failed)
}

fn tmp_for(p: &Path) -> PathBuf {
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    p.with_file_name(format!(".{name}.kex-tmp-{}", std::process::id()))
}

fn write_one(p: &Plan) -> Result<()> {
    let text = serde_json::to_string_pretty(&p.config)? + "\n";
    let tmp_cfg = tmp_for(&p.path);
    std::fs::write(&tmp_cfg, text).with_context(|| format!("writing {}", p.path.display()))?;
    let fail = |e: std::io::Error, what: &Path, tmps: &[&Path]| {
        for t in tmps {
            let _ = std::fs::remove_file(t);
        }
        anyhow::Error::new(e).context(format!("writing {}", what.display()))
    };
    if let Some(after) = &p.gitignore_after {
        // A symlinked .gitignore is written through (a rename would replace the link).
        if p.gitignore.is_symlink() {
            std::fs::write(&p.gitignore, after).map_err(|e| fail(e, &p.gitignore, &[&tmp_cfg]))?;
        } else {
            let tmp_gi = tmp_for(&p.gitignore);
            std::fs::write(&tmp_gi, after)
                .map_err(|e| fail(e, &p.gitignore, &[&tmp_cfg, &tmp_gi]))?;
            std::fs::rename(&tmp_gi, &p.gitignore)
                .map_err(|e| fail(e, &p.gitignore, &[&tmp_cfg, &tmp_gi]))?;
        }
    }
    if let Err(e) = std::fs::rename(&tmp_cfg, &p.path) {
        restore_gitignore(p);
        return Err(fail(e, &p.path, &[&tmp_cfg]));
    }
    Ok(())
}

fn restore_gitignore(p: &Plan) {
    if p.gitignore_after.is_none() {
        return;
    }
    let _ = match &p.gitignore_before {
        Some(b) => std::fs::write(&p.gitignore, b),
        None => std::fs::remove_file(&p.gitignore),
    };
}

fn undo(p: &Plan) {
    let _ = std::fs::remove_file(&p.path);
    restore_gitignore(p);
}

/// SIGINT / SIGTERM become a flag while the guard lives, so a write isn't cut in half.
mod signals {
    #[cfg(unix)]
    mod imp {
        use std::sync::atomic::{AtomicBool, Ordering};

        static HIT: AtomicBool = AtomicBool::new(false);

        extern "C" fn on_signal(_: libc::c_int) {
            HIT.store(true, Ordering::SeqCst);
        }

        pub struct Guard {
            prev: Vec<(libc::c_int, libc::sighandler_t)>,
        }

        impl Guard {
            pub fn install() -> Self {
                HIT.store(false, Ordering::SeqCst);
                let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
                let prev = [libc::SIGINT, libc::SIGTERM]
                    .into_iter()
                    // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
                    .map(|s| (s, unsafe { libc::signal(s, handler) }))
                    .collect();
                Guard { prev }
            }

            pub fn hit(&self) -> bool {
                HIT.load(Ordering::SeqCst)
            }
        }

        impl Drop for Guard {
            fn drop(&mut self) {
                for (s, h) in &self.prev {
                    // SAFETY: puts back the handler that was there before.
                    unsafe { libc::signal(*s, *h) };
                }
            }
        }
    }

    #[cfg(not(unix))]
    mod imp {
        pub struct Guard;
        impl Guard {
            pub fn install() -> Self {
                Guard
            }
            pub fn hit(&self) -> bool {
                false
            }
        }
    }

    pub use imp::Guard;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_ids() {
        assert!(valid_repo_id("r_9xk2mq"));
        assert!(!valid_repo_id("r_") && !valid_repo_id("9xk2mq") && !valid_repo_id("r_ABC"));
    }

    #[test]
    fn names_follow_the_schema_and_the_project() {
        let taken: HashSet<&str> = ["web"].into();
        assert_eq!(name_problem("api", &taken), None);
        assert!(name_problem("web", &taken).unwrap().contains("already in"));
        assert!(name_problem("Bad Name", &taken).is_some());
        assert!(name_problem("-a", &taken).is_some() && name_problem("a_b", &taken).is_some());
    }
}
