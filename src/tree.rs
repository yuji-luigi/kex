//! Folders as a tree, with what kex knows about each git repo in it. Used by `kex browse`
//! (for GUIs that navigate on their own) and by the repo picker of `kex init` / `kex add`.
//!
//! The listing is breadth-first, so a cap on entries fills the shallow levels first. Repos
//! are leaves: kex never lists the inside of a git repo. The costly git fields (branch,
//! changed files, remote, last commit) are read only for the repos in the listing, a few
//! at a time in parallel.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::{config, git, util};

/// Folders that are never listed (or walked) unless `all` is set: version control, build
/// output and dependency caches.
pub const SKIP: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "build",
    "dist",
    ".dart_tool",
    "Pods",
    "vendor",
];

pub struct Opts {
    /// Levels below the root to list: 1 = only its direct subfolders.
    pub depth: u32,
    /// Stop after this many entries (the rest is marked `truncated`).
    pub max_entries: usize,
    /// Show dot-folders.
    pub hidden: bool,
    /// Don't skip [`SKIP`].
    pub all: bool,
    /// Follow symlinks to folders (each entry gets the real path; a link and its target are
    /// one entry). Off: a link is a `symlink` entry and isn't walked.
    pub follow_links: bool,
    /// Skip this many of the root's own subfolders (paging through a big folder).
    pub offset: usize,
    /// Read the git fields (branch, dirty, remote, last commit). Off: kinds only, fast.
    pub git: bool,
    /// Stop reading git fields after this long (the rest of the repos go without them and
    /// the tree says `git_partial`). `None`: no limit.
    pub git_budget: Option<std::time::Duration>,
}

impl Opts {
    /// The picker's options: links followed, git fields read.
    pub fn picker(depth: u32, max_entries: usize, offset: usize) -> Self {
        Opts {
            depth,
            max_entries,
            hidden: false,
            all: false,
            follow_links: true,
            offset,
            git: true,
            git_budget: Some(std::time::Duration::from_millis(DEFAULT_GIT_BUDGET_MS)),
        }
    }
}

impl Opts {
    /// Kinds only, no limits but the scan caps: for finding repos rather than showing them.
    pub fn scan() -> Self {
        Opts {
            depth: 1,
            max_entries: usize::MAX,
            hidden: false,
            all: false,
            follow_links: false,
            offset: 0,
            git: false,
            git_budget: None,
        }
    }
}

/// The git repos below `dir`, up to `depth` levels down (1 = its direct subfolders), with the
/// listing's skips. Doesn't go into repos.
pub fn git_dirs(dir: &Path, depth: u32) -> Vec<PathBuf> {
    repos_flat(dir, &Opts::scan(), Some(depth.max(1)))
        .entries
        .into_iter()
        .map(|e| e.path)
        .collect()
}

/// Default time for reading git fields in one listing (see `Opts::git_budget`).
pub const DEFAULT_GIT_BUDGET_MS: u64 = 5000;
/// Folders a listing may visit to count `repos_below` (all folders together, and at most
/// [`COUNT_CAP_PER_DIR`] for one folder). Past it the count is a minimum and says so.
const COUNT_CAP_TOTAL: usize = 20_000;
const COUNT_CAP_PER_DIR: usize = 5_000;
/// Folders `--repos-only` may visit.
const FLAT_SCAN_CAP: usize = 20_000;
/// Time the folder walks of one listing may take (`repos_below` counting, `--repos-only`);
/// past it they stop and say so, like the caps above.
pub const SCAN_BUDGET: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Dir,
    /// A git repo without a kex config.
    Repo,
    /// A git repo with a kex config (or a broken one: see `config_error`).
    KexRepo,
    /// A linked worktree: shares its main checkout's config.
    Worktree,
    /// Only when links aren't followed.
    Symlink,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Dir => "dir",
            Kind::Repo => "repo",
            Kind::KexRepo => "kex_repo",
            Kind::Worktree => "worktree",
            Kind::Symlink => "symlink",
        }
    }

    pub fn is_git(self) -> bool {
        matches!(self, Kind::Repo | Kind::KexRepo | Kind::Worktree)
    }
}

/// Git details of a repo entry.
#[derive(Clone, Default, Debug)]
pub struct GitInfo {
    pub branch: String,
    /// Changed (or untracked) files.
    pub dirty: usize,
    pub remote: Option<String>,
    pub last_commit_at: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    /// 1 = directly in the root.
    pub depth: u32,
    pub hidden: bool,
    pub readable: bool,
    pub error: Option<String>,
    pub modified_at: Option<String>,
    /// Folders: git repos anywhere below it (the same skips as the listing; repos aren't
    /// looked into). A minimum when `repos_below_truncated`.
    pub repos_below: usize,
    pub repos_below_truncated: bool,
    /// Folders: its subfolders weren't (all) listed, because of the depth or the cap.
    pub truncated: bool,
    pub children: Vec<Entry>,
    pub git: Option<GitInfo>,
    pub stack: Vec<&'static str>,
    /// Kex repos.
    pub project: Option<String>,
    pub repo: Option<String>,
    pub config_error: Option<String>,
    /// Worktrees: the main checkout.
    pub main_repo: Option<PathBuf>,
    /// Symlinks: the resolved target and what it is (`missing` when it doesn't exist).
    pub target: Option<PathBuf>,
    pub target_kind: Option<&'static str>,
}

pub struct Tree {
    pub root: PathBuf,
    pub kind: Kind,
    pub readable: bool,
    pub error: Option<String>,
    pub entries: Vec<Entry>,
    /// Some folder's subfolders weren't listed because of `max_entries`.
    pub truncated: bool,
    pub count: usize,
    /// The root's own subfolders: how many there are, where this page started, and where the
    /// next page starts when the cap cut the root's list (`has_more`).
    pub children_total: usize,
    pub offset: usize,
    pub next_offset: Option<usize>,
    /// Git fields were asked for, and whether the time budget cut them short.
    pub git: bool,
    pub git_partial: bool,
    /// `--repos-only`: `entries` is a flat list of repos.
    pub flat: bool,
}

/// A folder in a listing, before it's classified.
struct Raw {
    name: String,
    path: PathBuf,
    link: bool,
    /// `follow_links` off: the link's target.
    target: Option<PathBuf>,
}

/// The subfolders of `dir` that the options let through, sorted by name.
fn read(dir: &Path, o: &Opts) -> Result<Vec<Raw>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| short_error(&e))?;
    let mut out: Vec<Raw> = Vec::new();
    for e in rd.flatten() {
        // Names that aren't valid UTF-8 can't be shown or sent as JSON.
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        if (name.starts_with('.') && !o.hidden) || (!o.all && SKIP.contains(&name.as_str())) {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        let path = e.path();
        if ft.is_symlink() {
            let target_is_dir = std::fs::metadata(&path).is_ok_and(|m| m.is_dir());
            if o.follow_links {
                if target_is_dir {
                    out.push(Raw {
                        name,
                        path: util::canon(&path),
                        link: true,
                        target: None,
                    });
                }
            } else {
                // A link to a file isn't a folder; a dangling one is shown (it may be a
                // folder that's gone).
                let dangling = std::fs::metadata(&path).is_err();
                if target_is_dir || dangling {
                    let t = std::fs::read_link(&path).ok().map(|t| dir.join(t));
                    out.push(Raw {
                        name,
                        target: t.map(|t| util::canon(&t)),
                        path,
                        link: true,
                    });
                }
            }
        } else if ft.is_dir() {
            out.push(Raw {
                name,
                path: if o.follow_links {
                    util::canon(&path)
                } else {
                    path
                },
                link: false,
                target: None,
            });
        }
    }
    // Following links: the real folder before links to it, one entry per real path.
    out.sort_by_key(|r| (r.link, r.name.to_lowercase()));
    if o.follow_links {
        let mut seen = HashSet::new();
        out.retain(|r| seen.insert(r.path.clone()));
    }
    out.sort_by_key(|r| r.name.to_lowercase());
    Ok(out)
}

fn short_error(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => "permission denied".into(),
        std::io::ErrorKind::NotFound => "not found".into(),
        _ => e.to_string(),
    }
}

/// What the folder at `path` is (cheap: no git commands).
pub fn classify(path: &Path) -> Kind {
    let dot_git = path.join(".git");
    if !dot_git.exists() {
        return Kind::Dir;
    }
    if worktree_main(&dot_git).is_some() {
        return Kind::Worktree;
    }
    match config::config_path(path) {
        Ok(None) => Kind::Repo,
        _ => Kind::KexRepo,
    }
}

/// A linked worktree's `.git` is a file `gitdir: <main>/.git/worktrees/<name>`; returns
/// `<main>`. (A submodule's `.git` file points into `.git/modules` instead.)
fn worktree_main(dot_git: &Path) -> Option<PathBuf> {
    if !dot_git.is_file() {
        return None;
    }
    let text = std::fs::read_to_string(dot_git).ok()?;
    let gitdir = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    let gitdir = dot_git.parent()?.join(gitdir);
    let worktrees = gitdir.parent()?;
    if worktrees.file_name()? != "worktrees" {
        return None;
    }
    let common = worktrees.parent()?;
    Some(util::canon(common.parent().filter(|_| {
        common.file_name().is_some_and(|n| n == ".git")
    })?))
}

/// An entry for the folder at `path` with its kind and the cheap details.
pub fn entry(name: String, path: PathBuf, depth: u32) -> Entry {
    let mut e = Entry {
        hidden: name.starts_with('.'),
        name,
        kind: Kind::Dir,
        depth,
        readable: true,
        error: None,
        modified_at: None,
        repos_below: 0,
        repos_below_truncated: false,
        truncated: false,
        children: Vec::new(),
        git: None,
        stack: Vec::new(),
        project: None,
        repo: None,
        config_error: None,
        main_repo: None,
        target: None,
        target_kind: None,
        path,
    };
    e.kind = classify(&e.path);
    match e.kind {
        Kind::Dir => {
            if let Err(err) = std::fs::read_dir(&e.path) {
                e.readable = false;
                e.error = Some(short_error(&err));
            }
            e.modified_at = std::fs::metadata(&e.path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| util::rfc3339(d.as_secs() as i64));
        }
        Kind::Worktree => {
            e.main_repo = worktree_main(&e.path.join(".git"));
            e.stack = stack(&e.path);
        }
        Kind::KexRepo => {
            match config::config_path(&e.path).and_then(|p| match p {
                Some(p) => config::load(&p),
                None => anyhow::bail!("no config"),
            }) {
                Ok(c) => {
                    e.project = Some(c.project);
                    e.repo = Some(c.repo);
                }
                Err(err) => e.config_error = Some(format!("{err:#}")),
            }
            e.stack = stack(&e.path);
        }
        Kind::Repo => e.stack = stack(&e.path),
        Kind::Symlink => {}
    }
    e
}

/// Marker files → what the repo is built with, for icons.
pub fn stack(dir: &Path) -> Vec<&'static str> {
    const MARKERS: &[(&str, &str)] = &[
        ("package.json", "node"),
        ("deno.json", "deno"),
        ("pubspec.yaml", "flutter"),
        ("Cargo.toml", "rust"),
        ("go.mod", "go"),
        ("pyproject.toml", "python"),
        ("requirements.txt", "python"),
        ("Gemfile", "ruby"),
        ("composer.json", "php"),
        ("pom.xml", "java"),
        ("build.gradle", "gradle"),
        ("build.gradle.kts", "gradle"),
        ("Package.swift", "swift"),
        ("mix.exs", "elixir"),
    ];
    let mut out: Vec<&'static str> = Vec::new();
    for (file, s) in MARKERS {
        if !out.contains(s) && dir.join(file).is_file() {
            out.push(s);
        }
    }
    out
}

/// Run a read-only git command, giving up (and killing it) at `deadline`. `None` on
/// failure or timeout; `Err(())` only for the timeout.
fn git_until(dir: &Path, args: &[&str], deadline: Option<Instant>) -> Result<Option<String>, ()> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let Ok(mut child) = Command::new("git")
        .args(args)
        .current_dir(dir)
        // Don't take the index lock to refresh stat info: kex only reads.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Ok(None);
    };
    // Read stdout on the side, so a big output can't block the child.
    let mut out = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_string(&mut s);
        }
        s
    });
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                let s = reader.join().unwrap_or_default();
                return Ok(st.success().then(|| s.trim_end().to_string()));
            }
            Ok(None) if deadline.is_some_and(|d| Instant::now() >= d) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(2)),
            Err(_) => return Ok(None),
        }
    }
}

/// Branch, changed files, shared remote and last commit date: three git commands. `None`
/// when `deadline` passed first.
pub fn git_info(dir: &Path, deadline: Option<Instant>) -> Option<GitInfo> {
    if deadline.is_some_and(|d| Instant::now() >= d) {
        return None;
    }
    let mut info = GitInfo {
        branch: "(detached)".into(),
        ..GitInfo::default()
    };
    if let Some(s) = git_until(dir, &["status", "--porcelain=v2", "--branch"], deadline).ok()? {
        for l in s.lines() {
            match l.strip_prefix("# branch.head ") {
                Some(b) if b != "(detached)" => info.branch = b.to_string(),
                Some(_) => {}
                None if !l.starts_with('#') && !l.is_empty() => info.dirty += 1,
                None => {}
            }
        }
    }
    info.last_commit_at = git_until(dir, &["log", "-1", "--format=%cI"], deadline)
        .ok()?
        .filter(|s| !s.is_empty());
    let remotes = git_until(
        dir,
        &["config", "--get-regexp", r"^remote\..*\.url$"],
        deadline,
    )
    .ok()?;
    if let Some(s) = remotes {
        let urls: Vec<(&str, &str)> = s
            .lines()
            .filter_map(|l| {
                let (k, v) = l.split_once(' ')?;
                Some((k.strip_prefix("remote.")?.strip_suffix(".url")?, v))
            })
            .collect();
        info.remote = ["upstream", "origin"]
            .iter()
            .find_map(|n| urls.iter().find(|(k, _)| k == n))
            .or(urls.first())
            .map(|(_, v)| v.to_string());
    }
    Some(info)
}

/// Fill in the git fields of the repo entries in `arena`, 8 at a time, within the options'
/// budget. Returns whether the budget cut it short.
fn fill_git(arena: &mut [Entry], o: &Opts) -> bool {
    if !o.git {
        return false;
    }
    let deadline = o.git_budget.map(|b| Instant::now() + b);
    let repos: Vec<usize> = (0..arena.len())
        .filter(|&i| arena[i].kind.is_git())
        .collect();
    let paths: Vec<PathBuf> = repos.iter().map(|&i| arena[i].path.clone()).collect();
    let infos = parallel_n(&paths, 8, |p| git_info(p, deadline));
    let mut partial = false;
    for (i, info) in repos.into_iter().zip(infos) {
        partial |= info.is_none();
        arena[i].git = info;
    }
    partial
}

/// Git repos anywhere below `dir`, with the listing's skips; repos aren't looked into and
/// links aren't followed. Visits at most `budget` folders (shared by the whole listing) and
/// [`COUNT_CAP_PER_DIR`] for this one; `true` = it stopped early.
fn count_below(dir: &Path, o: &Opts, budget: &mut usize, until: Instant) -> (usize, bool) {
    let mut cap = COUNT_CAP_PER_DIR.min(*budget);
    let mut stack = vec![dir.to_path_buf()];
    let mut found = 0;
    while let Some(d) = stack.pop() {
        if cap == 0 || Instant::now() >= until {
            return (found, true);
        }
        cap -= 1;
        *budget -= 1;
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            if !e.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let name = e.file_name();
            let name = name.to_string_lossy();
            if (name.starts_with('.') && !o.hidden) || (!o.all && SKIP.contains(&name.as_ref())) {
                continue;
            }
            let p = e.path();
            if p.join(".git").exists() {
                found += 1;
            } else {
                stack.push(p);
            }
        }
    }
    (found, false)
}

/// `git@github.com:Acme/api.git` → `github.com/Acme/api`: host (lowercase) and path (as
/// written).
pub fn remote_display(url: &str) -> String {
    let n = git::normalize_remote(url);
    let Some((host, path)) = n.split_once('/') else {
        return n;
    };
    let k = path.split('/').count();
    let trimmed = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let segs: Vec<&str> = trimmed.split(['/', ':']).collect();
    if segs.len() < k {
        return n;
    }
    format!("{host}/{}", segs[segs.len() - k..].join("/"))
}

/// Run `f` on every item, `n` at a time, keeping the order.
fn parallel_n<T: Sync, R: Send>(items: &[T], n: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let mut out = Vec::with_capacity(items.len());
    for chunk in items.chunks(n.max(1)) {
        out.extend(util::parallel(chunk, &f));
    }
    out
}

/// List `root`. Paths in the result are absolute; `root` itself is resolved.
pub fn list(root: &Path, o: &Opts) -> Tree {
    let root = util::canon(root);
    let mut tree = Tree {
        kind: classify(&root),
        readable: true,
        error: None,
        entries: Vec::new(),
        truncated: false,
        count: 0,
        children_total: 0,
        offset: o.offset,
        next_offset: None,
        git: o.git,
        git_partial: false,
        flat: false,
        root: root.clone(),
    };
    // Arena of entries; `kids[i]` are the children of entry i, `top` the root's.
    let mut arena: Vec<Entry> = Vec::new();
    let mut kids: Vec<Vec<usize>> = Vec::new();
    let mut top: Vec<usize> = Vec::new();
    let mut visited: HashSet<PathBuf> = HashSet::from([root.clone()]);
    // Folders to list at the next level: None = the root.
    let mut queue: Vec<Option<usize>> = vec![None];
    for level in 1..=o.depth.max(1) {
        let mut next = Vec::new();
        for parent in queue {
            let dir = parent.map_or(root.clone(), |i| arena[i].path.clone());
            if tree.count >= o.max_entries {
                tree.truncated = true;
                if let Some(i) = parent {
                    arena[i].truncated = true;
                }
                continue;
            }
            let raws = match read(&dir, o) {
                Ok(r) if parent.is_none() => {
                    tree.children_total = r.len();
                    r.into_iter().skip(o.offset).collect()
                }
                Ok(r) => r,
                Err(err) => {
                    match parent {
                        Some(i) => {
                            arena[i].readable = false;
                            arena[i].error = Some(err);
                        }
                        None => {
                            tree.readable = false;
                            tree.error = Some(err);
                        }
                    }
                    continue;
                }
            };
            for (n, r) in raws.into_iter().enumerate() {
                if tree.count >= o.max_entries {
                    tree.truncated = true;
                    match parent {
                        Some(i) => arena[i].truncated = true,
                        None => tree.next_offset = Some(o.offset + n),
                    }
                    break;
                }
                if o.follow_links && !visited.insert(r.path.clone()) {
                    continue; // already in this listing
                }
                let mut e = if r.link && !o.follow_links {
                    let mut e = entry(r.name, r.path, level);
                    e.kind = Kind::Symlink;
                    e.stack.clear();
                    e.project = None;
                    e.repo = None;
                    e.main_repo = None;
                    e.target_kind = Some(match &r.target {
                        Some(t) if t.exists() => classify(t).as_str(),
                        _ => "missing",
                    });
                    e.target = r.target;
                    e.readable = true;
                    e.error = None;
                    e
                } else {
                    entry(r.name, r.path, level)
                };
                // A link (followed) to a folder above the root: shown, not walked.
                let loops = o.follow_links && root.starts_with(&e.path);
                if e.kind == Kind::Dir && loops {
                    e.truncated = true;
                }
                let i = arena.len();
                let walk = e.kind == Kind::Dir && e.readable && !loops;
                arena.push(e);
                kids.push(Vec::new());
                match parent {
                    Some(p) => kids[p].push(i),
                    None => top.push(i),
                }
                tree.count += 1;
                if walk {
                    next.push(Some(i));
                }
            }
        }
        queue = next;
    }
    // Folders at the last level: truncated when they have subfolders to show.
    for i in queue.into_iter().flatten() {
        arena[i].truncated = read(&arena[i].path, o).is_ok_and(|r| !r.is_empty());
    }

    tree.git_partial = fill_git(&mut arena, o);

    // Repo counts, children before parents (higher indexes are deeper or later). Where the
    // listing stopped, the rest of the subtree is walked (within the caps).
    let mut budget = COUNT_CAP_TOTAL;
    let until = Instant::now() + SCAN_BUDGET;
    for i in (0..arena.len()).rev() {
        if arena[i].kind != Kind::Dir {
            continue;
        }
        let loops = o.follow_links && root.starts_with(&arena[i].path);
        let (n, cut) = if loops {
            (0, true)
        } else if arena[i].truncated && arena[i].readable {
            count_below(&arena[i].path, o, &mut budget, until)
        } else {
            kids[i]
                .iter()
                .fold((0, false), |(n, cut), &k| match arena[k].kind {
                    Kind::Dir => (
                        n + arena[k].repos_below,
                        cut || arena[k].repos_below_truncated,
                    ),
                    k if k.is_git() => (n + 1, cut),
                    _ => (n, cut),
                })
        };
        arena[i].repos_below = n;
        arena[i].repos_below_truncated = cut;
    }

    // Nest: take entries out of the arena, deepest first.
    let mut slots: Vec<Option<Entry>> = arena.into_iter().map(Some).collect();
    for i in (0..slots.len()).rev() {
        let children: Vec<Entry> = kids[i].iter().filter_map(|&k| slots[k].take()).collect();
        if let Some(e) = slots[i].as_mut() {
            e.children = children;
        }
    }
    tree.entries = top.into_iter().filter_map(|i| slots[i].take()).collect();
    tree
}

/// `--repos-only`: every git repo below `root` (any depth, or `max_depth` levels), as a
/// flat list sorted by path. Stops at `o.max_entries` repos or after visiting
/// [`FLAT_SCAN_CAP`] folders (`truncated`). Links aren't followed.
pub fn repos_flat(root: &Path, o: &Opts, max_depth: Option<u32>) -> Tree {
    let root = util::canon(root);
    let mut tree = Tree {
        kind: classify(&root),
        readable: std::fs::read_dir(&root).is_ok(),
        error: std::fs::read_dir(&root).err().map(|e| short_error(&e)),
        entries: Vec::new(),
        truncated: false,
        count: 0,
        children_total: 0,
        offset: 0,
        next_offset: None,
        git: o.git,
        git_partial: false,
        flat: true,
        root: root.clone(),
    };
    let mut found: Vec<Entry> = Vec::new();
    let mut queue = std::collections::VecDeque::from([(root.clone(), 0u32)]);
    let mut visited = 0usize;
    let until = Instant::now() + SCAN_BUDGET;
    'scan: while let Some((dir, depth)) = queue.pop_front() {
        visited += 1;
        if visited > FLAT_SCAN_CAP || Instant::now() >= until {
            tree.truncated = true;
            break;
        }
        let Ok(raws) = read(
            &dir,
            &Opts {
                follow_links: false,
                ..Opts::picker(1, 0, 0)
            }
            .with(o),
        ) else {
            continue;
        };
        for r in raws {
            if r.link {
                continue;
            }
            let level = depth + 1;
            if r.path.join(".git").exists() {
                if found.len() >= o.max_entries {
                    tree.truncated = true;
                    break 'scan;
                }
                found.push(entry(r.name, r.path, level));
            } else if max_depth.is_none_or(|m| level < m) {
                queue.push_back((r.path, level));
            }
        }
    }
    tree.git_partial = fill_git(&mut found, o);
    found.sort_by(|a, b| a.path.cmp(&b.path));
    tree.count = found.len();
    tree.entries = found;
    tree
}

impl Opts {
    /// These options' skips (hidden, all) on top of `self`.
    fn with(mut self, o: &Opts) -> Self {
        self.hidden = o.hidden;
        self.all = o.all;
        self
    }
}

fn lossy(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

impl Entry {
    /// The entry as JSON; `children` only for folders and when `nested`.
    pub fn json(&self, nested: bool) -> Value {
        let mut v = json!({
            "name": self.name,
            "path": lossy(&self.path),
            "kind": self.kind.as_str(),
            "depth": self.depth,
            "hidden": self.hidden,
            "readable": self.readable,
        });
        if let Some(e) = &self.error {
            v["error"] = json!(e);
        }
        match self.kind {
            Kind::Dir => {
                v["repos_below"] = json!(self.repos_below);
                v["repos_below_truncated"] = json!(self.repos_below_truncated);
                v["truncated"] = json!(self.truncated);
                v["modified_at"] = json!(self.modified_at);
                if nested {
                    v["children"] =
                        Value::Array(self.children.iter().map(|c| c.json(true)).collect());
                }
            }
            Kind::Symlink => {
                v["target"] = json!(self.target.as_deref().map(lossy));
                v["target_kind"] = json!(self.target_kind);
            }
            _ => {
                // Absent with --no-git, or when the time budget ran out first.
                if let Some(g) = &self.git {
                    v["branch"] = json!(g.branch);
                    v["dirty"] = json!(g.dirty);
                    v["remote"] = json!(g.remote);
                    v["remote_display"] = json!(g.remote.as_deref().map(remote_display));
                    v["last_commit_at"] = json!(g.last_commit_at);
                }
                v["stack"] = json!(self.stack);
                if self.kind == Kind::KexRepo {
                    v["project"] = json!(self.project);
                    v["repo"] = json!(self.repo);
                    if let Some(e) = &self.config_error {
                        v["config_error"] = json!(e);
                    }
                }
                if self.kind == Kind::Worktree {
                    v["main_repo"] = json!(self.main_repo.as_deref().map(lossy));
                }
            }
        }
        v
    }
}

impl Tree {
    /// `depth`: as requested (`None` = no limit, `--repos-only` without `--depth`).
    pub fn json(&self, depth: Option<u32>, max_entries: usize) -> Value {
        json!({
            "root": lossy(&self.root),
            "parent": self.root.parent().map(lossy),
            "kind": self.kind.as_str(),
            "readable": self.readable,
            "error": self.error,
            "depth": depth,
            "max_entries": max_entries,
            "count": self.count,
            "truncated": self.truncated,
            "offset": self.offset,
            "children_total": self.children_total,
            "has_more": self.next_offset.is_some(),
            "next_offset": self.next_offset,
            "git": self.git,
            "git_partial": self.git_partial,
            "flat": self.flat,
            "tree": self.entries.iter().map(|e| e.json(true)).collect::<Vec<_>>(),
        })
    }

    /// Every entry, depth first (parents before their children), with whether it's the last
    /// of its siblings at each level (for drawing `├─` / `└─`).
    pub fn flat(&self) -> Vec<(&Entry, Vec<bool>)> {
        fn walk<'a>(es: &'a [Entry], lasts: &mut Vec<bool>, out: &mut Vec<(&'a Entry, Vec<bool>)>) {
            for (i, e) in es.iter().enumerate() {
                lasts.push(i + 1 == es.len());
                out.push((e, lasts.clone()));
                walk(&e.children, lasts, out);
                lasts.pop();
            }
        }
        let mut out = Vec::new();
        walk(&self.entries, &mut Vec::new(), &mut out);
        out
    }
}

/// `├─ ` / `└─ ` with `│  ` for the levels above.
pub fn branches(lasts: &[bool]) -> String {
    let mut s = String::new();
    for (i, last) in lasts.iter().enumerate() {
        let end = i + 1 == lasts.len();
        s.push_str(match (end, last) {
            (true, true) => "└─ ",
            (true, false) => "├─ ",
            (false, true) => "   ",
            (false, false) => "│  ",
        });
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(depth: u32, max: usize) -> Opts {
        Opts {
            depth,
            max_entries: max,
            hidden: false,
            all: false,
            follow_links: false,
            offset: 0,
            git: true,
            git_budget: None,
        }
    }

    fn names(es: &[Entry]) -> Vec<&str> {
        es.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn lists_levels_breadth_first_and_marks_truncation() {
        let t = tempfile::tempdir().unwrap();
        for d in ["a/b/c/d", "a/x", "e", "node_modules/z", ".hidden/y"] {
            std::fs::create_dir_all(t.path().join(d)).unwrap();
        }
        let tree = list(t.path(), &opts(2, 100));
        assert_eq!(names(&tree.entries), ["a", "e"]);
        let a = &tree.entries[0];
        assert_eq!(names(&a.children), ["b", "x"]);
        assert_eq!(a.children[0].depth, 2);
        assert!(a.children[0].truncated); // b has c below, not listed
        assert!(!a.children[1].truncated); // x is empty
        assert!(!a.truncated && !tree.truncated);
        assert_eq!(tree.count, 4);

        // The cap stops the listing; the shallow level comes first.
        let tree = list(t.path(), &opts(3, 3));
        assert_eq!(tree.count, 3);
        assert!(tree.truncated);
        assert_eq!(names(&tree.entries), ["a", "e"]);
        assert_eq!(names(&tree.entries[0].children), ["b"]);
        assert!(tree.entries[0].truncated);
        assert_eq!(tree.next_offset, None); // the root's own list is complete

        // The root's own list cut by the cap: page through it.
        let mut o = opts(1, 1);
        let tree = list(t.path(), &o);
        assert_eq!((names(&tree.entries), tree.children_total), (vec!["a"], 2));
        assert_eq!(tree.next_offset, Some(1));
        o.offset = 1;
        let tree = list(t.path(), &o);
        assert_eq!(names(&tree.entries), ["e"]);
        assert_eq!(tree.next_offset, None);

        // --hidden and --all.
        let mut o = opts(1, 100);
        o.hidden = true;
        o.all = true;
        assert_eq!(
            names(&list(t.path(), &o).entries),
            [".hidden", "a", "e", "node_modules"]
        );
        assert!(list(t.path(), &o).entries[0].hidden);
    }

    #[test]
    fn draws_branches() {
        assert_eq!(branches(&[false]), "├─ ");
        assert_eq!(branches(&[false, true]), "│  └─ ");
        assert_eq!(branches(&[true, false]), "   ├─ ");
    }

    #[test]
    fn shows_remotes_as_host_owner_name() {
        assert_eq!(
            remote_display("git@github.com:Acme/Shop-API.git"),
            "github.com/Acme/Shop-API"
        );
        assert_eq!(remote_display("https://gitlab.com/a/b"), "gitlab.com/a/b");
    }
}
