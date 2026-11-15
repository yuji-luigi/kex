//! Picking git repos by browsing folders (`kex init` in a terminal), and `kex browse`, the
//! same listing (tree.rs) for programs that navigate on their own.
//!
//! The picker shows a tree a few levels deep below the folder you're in, as one widget: ↑/↓
//! move, Enter opens a folder (it becomes the top of the tree) or checks a repo, Space checks
//! a repo, ←/Backspace goes to `../`, Tab (or `d`) is done, Esc/Ctrl-C cancels. It follows
//! symlinks (one entry per real folder); `kex browse` doesn't (a link is a `symlink` entry).

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde_json::{Value, json};
use unicode_width::UnicodeWidthStr;

use crate::tree::{self, Entry, Kind, Tree};
use crate::ui::{self, Cancelled};
use crate::{git, util, view, workspace};

const TITLE: &str = "Init which repos?";

/// Entries in one picker view: keeps each prompt well under 1 MiB.
pub const PICKER_MAX_ENTRIES: usize = 200;

/// The most entries `kex browse` lists in one line (cli-events §9: 1,000 nodes per tree line).
pub const MAX_ENTRIES: usize = 1000;

#[derive(Clone, PartialEq, Debug)]
enum Action {
    Done,
    Up,
    Open(PathBuf),
    Toggle(PathBuf),
    /// Show another page of a folder with more subfolders than one view holds.
    Page(usize),
    /// A kex repo, worktree or unreadable folder: nothing to do.
    Nothing,
}

struct Item {
    action: Action,
    /// The entry without its children, and whether it is the last of its siblings at each
    /// level (for the tree lines).
    entry: Option<(Entry, Vec<bool>)>,
}

/// One picked repo.
pub struct Pick {
    pub path: PathBuf,
    /// The default repo name: from the folder name as shown (a symlink's own name, not its
    /// target's).
    pub name: String,
}

/// What the user picked, and the kex repos they saw on the way.
pub struct Picked {
    /// Sorted by path.
    pub selected: Vec<Pick>,
    pub kex_seen: Vec<PathBuf>,
}

struct Browser {
    cwd: PathBuf,
    /// Where the current page of `cwd`'s subfolders starts.
    offset: usize,
    depth: u32,
    max_entries: usize,
    selected: Vec<PathBuf>,
    cache: HashMap<(PathBuf, usize), Tree>,
    kex_seen: Vec<PathBuf>,
    /// The folder name each repo was shown under, so the label, `name`, `default_name` and
    /// the repo name kex writes all agree.
    shown_as: HashMap<PathBuf, String>,
}

impl Browser {
    fn new(start: &Path, depth: u32, selected: Vec<PathBuf>) -> Self {
        Browser {
            cwd: start.to_path_buf(),
            offset: 0,
            depth: depth.max(1),
            max_entries: PICKER_MAX_ENTRIES,
            selected,
            cache: HashMap::new(),
            kex_seen: Vec::new(),
            shown_as: HashMap::new(),
        }
    }

    fn tree(&mut self) -> &Tree {
        let (depth, max) = (self.depth, self.max_entries);
        self.cache
            .entry((self.cwd.clone(), self.offset))
            .or_insert_with_key(|(d, offset)| {
                tree::list(d, &tree::Opts::picker(depth, max, *offset))
            })
    }

    /// `done`, `..` (unless at the filesystem root), then the tree, parents before children.
    fn items(&mut self) -> Vec<Item> {
        let mut out = vec![Item {
            action: Action::Done,
            entry: None,
        }];
        if self.cwd.parent().is_some() {
            out.push(Item {
                action: Action::Up,
                entry: None,
            });
        }
        if self.offset > 0 {
            out.push(Item {
                action: Action::Page(0),
                entry: None,
            });
        }
        let next = self.tree().next_offset;
        let rows: Vec<(Entry, Vec<bool>)> = self
            .tree()
            .flat()
            .into_iter()
            .map(|(e, lasts)| {
                let mut e = e.clone();
                e.children = Vec::new();
                (e, lasts)
            })
            .collect();
        for (e, lasts) in rows {
            if e.kind == Kind::KexRepo && !self.kex_seen.contains(&e.path) {
                self.kex_seen.push(e.path.clone());
            }
            if e.kind == Kind::Repo {
                self.shown_as
                    .entry(e.path.clone())
                    .or_insert_with(|| e.name.clone());
            }
            let action = match e.kind {
                Kind::Dir if e.readable => Action::Open(e.path.clone()),
                Kind::Repo => Action::Toggle(e.path.clone()),
                _ => Action::Nothing,
            };
            out.push(Item {
                action,
                entry: Some((e, lasts)),
            });
        }
        if let Some(n) = next {
            out.push(Item {
                action: Action::Page(n),
                entry: None,
            });
        }
        out
    }

    /// Apply an action; true when browsing is finished.
    fn apply(&mut self, a: &Action) -> bool {
        match a {
            Action::Done => return true,
            Action::Up => {
                if let Some(p) = self.cwd.parent() {
                    self.cwd = p.to_path_buf();
                    self.offset = 0;
                }
            }
            Action::Open(p) => {
                self.cwd = p.clone();
                self.offset = 0;
            }
            Action::Page(o) => self.offset = *o,
            Action::Toggle(p) => {
                if let Some(i) = self.selected.iter().position(|s| s == p) {
                    self.selected.remove(i);
                } else {
                    self.selected.push(p.clone());
                }
            }
            Action::Nothing => {}
        }
        false
    }

    /// The tree of the current view, if it was listed.
    fn cached(&self) -> Option<&Tree> {
        self.cache.get(&(self.cwd.clone(), self.offset))
    }

    fn is_selected(&self, p: &Path) -> bool {
        self.selected.iter().any(|s| s == p)
    }

    /// The folder name `p` was shown under (its own name when it was never listed, e.g. a
    /// pre-checked repo).
    fn shown_name(&self, p: &Path) -> String {
        self.shown_as
            .get(p)
            .cloned()
            .or_else(|| crate::setup::folder_name(Some(p)))
            .unwrap_or_default()
    }

    fn default_name(&self, p: &Path) -> String {
        util::slug(&self.shown_name(p))
    }

    fn selected_names(&self) -> Vec<String> {
        self.selected.iter().map(|p| self.default_name(p)).collect()
    }

    fn label(&self, it: &Item, width: usize) -> String {
        match (&it.action, &it.entry) {
            (Action::Done, _) => format!("✓ Done ({} selected)", self.selected.len()),
            (Action::Up, _) => "../".into(),
            (Action::Page(0), _) => "… back to the first entries".into(),
            (Action::Page(n), _) => {
                let total = self.cached().map_or(0, |t| t.children_total);
                format!("… {} more (next page)", total.saturating_sub(*n))
            }
            (_, Some((e, lasts))) => {
                let mark = match e.kind {
                    Kind::Repo if self.is_selected(&e.path) => "[x]",
                    Kind::Repo => "[ ]",
                    _ => "   ",
                };
                let name = match e.kind {
                    Kind::Dir => format!("{}{}/", tree::branches(lasts), e.name),
                    _ => format!("{}{}", tree::branches(lasts), e.name),
                };
                let info = match e.kind {
                    Kind::Dir if !e.readable => "can't read".to_string(),
                    Kind::Dir => match e.repos_below {
                        0 => String::new(),
                        1 => "1 repo".into(),
                        k => format!("{k} repos"),
                    },
                    Kind::Repo => {
                        let g = e.git.clone().unwrap_or_default();
                        let mut s = g.branch;
                        if g.dirty > 0 {
                            s.push_str(&format!(" · {} changed", g.dirty));
                        }
                        let n = self.default_name(&e.path);
                        if n != util::slug(&e.name) {
                            s.push_str(&format!(" · as {n}"));
                        }
                        s
                    }
                    Kind::KexRepo => format!(
                        "kex: {}",
                        e.project.as_deref().unwrap_or("unreadable config")
                    ),
                    Kind::Worktree => "worktree".into(),
                    Kind::Symlink => "link".into(),
                };
                let pad = " ".repeat(width.saturating_sub(name.width()));
                format!("{mark} {name}{pad}  {info}").trim_end().to_string()
            }
            _ => String::new(),
        }
    }
}

/// Browse from `start`, `depth` levels shown; `pre` start checked.
pub fn pick(start: &Path, depth: u32, pre: Vec<PathBuf>) -> Result<Picked> {
    let mut b = Browser::new(start, depth, pre);
    terminal(&mut b)?;
    let mut selected: Vec<Pick> = b
        .selected
        .iter()
        .map(|p| Pick {
            path: p.clone(),
            name: b.default_name(p),
        })
        .collect();
    selected.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Picked {
        selected,
        kex_seen: b.kex_seen,
    })
}

/// `3 selected: api, web, portal`, cut to `width` columns: as many names as fit, then
/// `+K more`.
pub fn status_line(names: &[String], width: usize) -> String {
    let n = names.len();
    let head = format!("{n} selected");
    if n == 0 {
        return head;
    }
    let full = format!("{head}: {}", names.join(", "));
    if full.width() <= width {
        return full;
    }
    for k in (1..n).rev() {
        let s = format!("{head}: {}, +{} more", names[..k].join(", "), n - k);
        if s.width() <= width {
            return s;
        }
    }
    // Not even one whole name fits: cut the first.
    let more = if n > 1 {
        format!(", +{} more", n - 1)
    } else {
        String::new()
    };
    let room = width.saturating_sub(head.width() + 2 + more.width());
    if room < 4 {
        return fit(&head, width);
    }
    format!("{head}: {}{more}", fit(&names[0], room))
}

fn name_width(items: &[Item]) -> usize {
    items
        .iter()
        .filter_map(|i| i.entry.as_ref())
        .map(|(e, lasts)| {
            tree::branches(lasts).width() + e.name.width() + usize::from(e.kind == Kind::Dir)
        })
        .max()
        .unwrap_or(0)
        .min(48)
}

// ── kex browse ───────────────────────────────────────────────────────────────

pub struct BrowseOpts {
    pub dir: Option<PathBuf>,
    /// Default 3 for the tree; no limit with `repos_only`.
    pub depth: Option<u32>,
    pub repos_only: bool,
    pub git: bool,
    pub git_budget_ms: u64,
    pub max_entries: usize,
    pub hidden: bool,
    pub all: bool,
    pub offset: usize,
}

/// The options of this `kex browse` call that aren't the defaults, for its `open` / `up` /
/// `next` actions (the same listing, of another folder or page).
fn same_options(o: &BrowseOpts) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    if let Some(d) = o.depth {
        m.insert("depth".into(), json!(d));
    }
    if o.max_entries != 500 {
        m.insert("max_entries".into(), json!(o.max_entries));
    }
    for (k, on) in [
        ("repos_only", o.repos_only),
        ("no_git", !o.git),
        ("hidden", o.hidden),
        ("all", o.all),
    ] {
        if on {
            m.insert(k.into(), json!(true));
        }
    }
    if o.git_budget_ms != 5000 {
        m.insert("git_budget_ms".into(), json!(o.git_budget_ms));
    }
    m
}

fn browse_action(label: Value, mut options: serde_json::Map<String, Value>, dir: &Value) -> Value {
    options.insert("dir".into(), dir.clone());
    json!({ "label": label, "run": { "command": "browse", "options": options } })
}

/// One `BrowseEntry` (raw JSON) as a tree node.
fn node(e: &Value, here: Option<&Path>) -> Value {
    let s = |k: &str| e[k].as_str().unwrap_or_default();
    let (path, name, kind) = (s("path"), s("name"), s("kind"));
    let mut n = json!({ "key": path, "label": name, "value": path, "kind": kind });
    let mut badges = Vec::new();
    let mut cells = serde_json::Map::new();
    if let Some(b) = e["branch"].as_str() {
        cells.insert("branch".into(), json!(b));
    }
    if let Some(d) = e["dirty"].as_u64() {
        cells.insert("changed".into(), view::count(d, "warn"));
    }
    if let Some(st) = e["stack"].as_array().filter(|a| !a.is_empty()) {
        cells.insert("stack".into(), json!(st));
    }
    let item = || json!({ "path": path, "name": util::slug(name) });
    match kind {
        "repo" => {
            n["selectable"] = json!(true);
            n["item"] = item();
            if here.is_some_and(|h| h == util::canon(Path::new(path))) {
                n["checked"] = json!(true);
            }
        }
        "kex_repo" => {
            n["selectable"] = json!(false);
            badges.push(match e["project"].as_str() {
                Some(p) => view::badge_p("kex_project", json!({ "project": p }), "muted"),
                None => view::badge("broken_config", "danger"),
            });
        }
        "worktree" => {
            n["selectable"] = json!(false);
            badges.push(view::badge("worktree", "muted"));
        }
        "symlink" => {
            let target = s("target");
            let to_repo = s("target_kind") == "repo";
            n["selectable"] = json!(to_repo);
            if to_repo {
                n["item"] = item();
            } else {
                n["tone"] = json!("muted");
            }
            n["detail"] = if s("target_kind") == "missing" {
                view::msg("tree.link_missing", json!({ "target": target }))
            } else {
                view::msg("tree.link", json!({ "target": target }))
            };
        }
        _ => {
            let below = e["repos_below"].as_u64().unwrap_or(0);
            if below > 0 {
                let id = if below == 1 {
                    "tree.repos_below_one"
                } else {
                    "tree.repos_below"
                };
                n["detail"] = view::msg(id, json!({ "n": below }));
            }
            if e["truncated"] == true {
                n["more"] = json!(true);
            }
        }
    }
    if e["readable"] == false {
        n["tone"] = json!("danger");
        n["detail"] = json!(s("error"));
    }
    if !badges.is_empty() {
        n["badges"] = json!(badges);
    }
    if !cells.is_empty() {
        n["cells"] = Value::Object(cells);
    }
    if let Some(ch) = e["children"].as_array() {
        n["children"] = json!(ch.iter().map(|c| node(c, here)).collect::<Vec<_>>());
    }
    n
}

/// The `tree` event's view: the listing as nodes, with `open`, `up` and `next`.
fn tree_view(t: &Value, o: &BrowseOpts, here: Option<&Path>) -> Value {
    let same = same_options(o);
    let nodes: Vec<Value> = t["tree"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| node(e, here))
        .collect();
    let root = t["root"].as_str().unwrap_or_default();
    let mut v = view::view(
        "tree",
        json!({
            "root": { "label": util::tilde(Path::new(root)), "value": root },
            "columns": [
                view::col("branch", "branch"), view::col("changed", "count"),
                view::col("stack", "badge"),
            ],
            "nodes": nodes,
            "open": { "run": { "command": "browse", "options": same }, "option": "dir" },
        }),
    );
    if !t["parent"].is_null() {
        v["up"] = browse_action(view::m("tree.up"), same.clone(), &t["parent"]);
    }
    if let Some(n) = t["next_offset"].as_u64() {
        let mut opts = same.clone();
        opts.insert("offset".into(), json!(n));
        v["next"] = browse_action(view::m("tree.next"), opts, &t["root"]);
    }
    if t["git_partial"] == true {
        v["notes"] = json!([view::note("warn", view::m("tree.git_partial"))]);
    }
    v
}

/// The `workspaces` event's view: one card per file, with a dry-run action.
fn workspaces_view(ws: &[Value]) -> Value {
    let rows: Vec<Value> = ws
        .iter()
        .map(|w| {
            let path = w["path"].as_str().unwrap_or_default();
            if let Some(e) = w["error"].as_str() {
                return json!({
                    "key": path, "cells": { "file": path }, "tone": "danger", "detail": e,
                });
            }
            let c = &w["counts"];
            let addable = c["addable"].as_u64().unwrap_or(0);
            let mut row = json!({
                "key": path,
                "cells": {
                    "name": w["name"], "file": path, "addable": addable,
                    "already_kex": c["already_kex"], "missing": c["missing"],
                },
            });
            if addable > 0 {
                let file = Path::new(path)
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default();
                row["actions"] = json!([{
                    "label": view::msg("workspaces.use", json!({ "n": addable })),
                    "confirm": view::msg(
                        "workspaces.use_confirm",
                        json!({ "n": addable, "file": file }),
                    ),
                    "run": {
                        "command": "init",
                        "options": { "workspace": path, "dry_run": true },
                    },
                }]);
            }
            row
        })
        .collect();
    view::view(
        "cards",
        json!({
            "title": view::m("workspaces.title"),
            "columns": [
                view::col("name", "text"), view::col("file", "path"),
                view::col("addable", "count"), view::col("already_kex", "count"),
                view::col("missing", "count"),
            ],
            "rows": rows,
            "empty": view::m("workspaces.empty"),
        }),
    )
}

/// `kex browse`: list a folder as a tree, and the workspace files around it. Asks nothing.
pub fn command(o: BrowseOpts) -> Result<u8> {
    let dir = match &o.dir {
        Some(d) => util::expand_tilde(&d.to_string_lossy()),
        None => std::env::current_dir()?,
    };
    if !dir.is_dir() {
        bail!("not a folder: {}", dir.display());
    }
    if o.max_entries > MAX_ENTRIES {
        bail!("--max-entries can be at most {MAX_ENTRIES}; page with --offset");
    }
    let opts = tree::Opts {
        depth: o.depth.unwrap_or(3).max(1),
        max_entries: o.max_entries.max(1),
        hidden: o.hidden,
        all: o.all,
        follow_links: false,
        offset: o.offset,
        git: o.git,
        git_budget: (o.git_budget_ms > 0)
            .then(|| std::time::Duration::from_millis(o.git_budget_ms)),
    };
    let t = if o.repos_only {
        tree::repos_flat(&dir, &opts, o.depth.map(|d| d.max(1)))
    } else {
        tree::list(&dir, &opts)
    };
    let workspaces: Vec<Value> = workspace::find(&workspace::search_dirs(&t.root))
        .iter()
        .map(|f| match workspace::load(f) {
            Ok(w) => w.json(),
            Err(e) => json!({ "path": crate::setup::lossy(f), "error": format!("{e:#}") }),
        })
        .collect();
    if ui::json() {
        let depth = if o.repos_only {
            o.depth
        } else {
            Some(opts.depth)
        };
        let mut tree = t.json(depth, opts.max_entries);
        let here = std::env::current_dir()
            .ok()
            .and_then(|d| git::root(&d).ok())
            .map(|p| util::canon(&p));
        tree["key"] = json!("tree");
        tree["view"] = tree_view(&tree, &o, here.as_deref());
        ui::emit("tree", tree);
        ui::emit(
            "workspaces",
            json!({
                "workspaces": workspaces,
                "key": "workspaces",
                "view": workspaces_view(&workspaces),
            }),
        );
        return Ok(0);
    }
    say!("{}", util::bold(util::tilde(&t.root)));
    if let Some(e) = &t.error {
        warn!("  can't read: {e}");
    }
    if t.flat {
        for e in &t.entries {
            let rel = e.path.strip_prefix(&t.root).unwrap_or(&e.path);
            let info = match e.kind {
                Kind::KexRepo => format!("kex: {}", e.project.as_deref().unwrap_or("?")),
                Kind::Worktree => "worktree".into(),
                _ => e.git.as_ref().map(|g| g.branch.clone()).unwrap_or_default(),
            };
            say!("  {}  {}", rel.display(), util::dim(info));
        }
        if t.truncated {
            say!("{}", util::dim("(stopped early; see --max-entries)"));
        }
        return Ok(0);
    }
    for (e, lasts) in t.flat() {
        let info = match e.kind {
            Kind::Dir if !e.readable => {
                format!("can't read ({})", e.error.as_deref().unwrap_or("?"))
            }
            Kind::Dir if e.repos_below == 1 => "1 repo below".into(),
            Kind::Dir if e.repos_below > 0 => format!("{} repos below", e.repos_below),
            Kind::Dir => String::new(),
            Kind::Repo => e.git.as_ref().map(|g| g.branch.clone()).unwrap_or_default(),
            Kind::KexRepo => format!(
                "kex: {}",
                e.project.as_deref().unwrap_or("unreadable config")
            ),
            Kind::Worktree => "worktree".into(),
            Kind::Symlink => format!(
                "→ {}",
                e.target
                    .as_deref()
                    .map(util::tilde)
                    .unwrap_or_else(|| "?".into())
            ),
        };
        let slash = if e.kind == Kind::Dir { "/" } else { "" };
        let more = if e.kind == Kind::Dir && e.truncated {
            " …"
        } else {
            ""
        };
        say!(
            "{}{}{slash}{more}  {}",
            tree::branches(&lasts),
            e.name,
            util::dim(info)
        );
    }
    if let Some(n) = t.next_offset {
        say!(
            "{}",
            util::dim(format!(
                "… {} more here (--offset {n} for the next page)",
                t.children_total - n
            ))
        );
    } else if t.truncated {
        say!(
            "{}",
            util::dim(format!(
                "(stopped after {} entries; see --max-entries)",
                t.count
            ))
        );
    }
    for w in &workspaces {
        match w.get("error") {
            Some(e) => warn!(
                "workspace file {}: {}",
                w["path"].as_str().unwrap_or("?"),
                e.as_str().unwrap_or("")
            ),
            None => say!(
                "workspace file {} · {} repos · {} already in kex",
                w["path"].as_str().unwrap_or("?"),
                w["counts"]["repos"],
                w["counts"]["already_kex"]
            ),
        }
    }
    Ok(0)
}

// ── Terminal widget ──────────────────────────────────────────────────────────

/// Raw mode and a hidden cursor for as long as it lives.
struct Raw;

impl Raw {
    fn on() -> Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        let _ = crossterm::execute!(std::io::stderr(), crossterm::cursor::Hide);
        Ok(Raw)
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        let _ = crossterm::execute!(std::io::stderr(), crossterm::cursor::Show);
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn terminal(b: &mut Browser) -> Result<()> {
    let mut drawn = 0u16;
    let result = widget(b, &mut drawn);
    // Erase the widget whatever happened (done, cancel or an error).
    if drawn > 0 {
        let _ = crossterm::execute!(
            std::io::stderr(),
            crossterm::cursor::MoveUp(drawn),
            crossterm::cursor::MoveToColumn(0),
            crossterm::terminal::Clear(crossterm::terminal::ClearType::FromCursorDown)
        );
    }
    result?;
    let names = b.selected_names();
    let shown = if names.is_empty() {
        util::dim("none")
    } else {
        names.join(", ")
    };
    eprintln!("{} {shown}", util::bold(format!("? {TITLE}")));
    Ok(())
}

/// The widget loop. `drawn` = lines on screen, so the caller can erase them.
fn widget(b: &mut Browser, drawn: &mut u16) -> Result<()> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

    let _raw = Raw::on()?;
    let mut err = std::io::stderr();
    let mut items = b.items();
    let first_entry = |items: &[Item]| items.iter().position(|i| i.entry.is_some()).unwrap_or(0);
    let mut cursor = first_entry(&items);
    let mut top = 0usize;
    let mut note: Option<String> = None;
    loop {
        let (cols, rows) = match crossterm::terminal::size() {
            Ok((c, r)) if c >= 20 && r >= 7 => (c, r),
            _ => (80, 24),
        };
        let cols = usize::from(cols);
        let page = usize::from(rows.saturating_sub(5)).clamp(3, 20);
        if cursor < top {
            top = cursor;
        } else if cursor >= top + page {
            top = cursor + 1 - page;
        }
        let width = name_width(&items);
        // Every line must fit: a wrapped line would break the redraw.
        let room = cols.saturating_sub(TITLE.width() + 4);
        let mut lines = vec![format!(
            "{} {}",
            util::bold(format!("? {TITLE}")),
            fit_left(&util::tilde(&b.cwd), room)
        )];
        for (i, it) in items.iter().enumerate().skip(top).take(page) {
            let text = fit(&b.label(it, width), cols.saturating_sub(3));
            let styled = match &it.entry {
                _ if i == cursor => util::paint("1;36", &text),
                Some((e, _)) if matches!(e.kind, Kind::KexRepo | Kind::Worktree) => {
                    util::dim(&text)
                }
                Some((e, _)) if b.is_selected(&e.path) => util::green(&text),
                _ => text,
            };
            lines.push(format!("{} {styled}", if i == cursor { ">" } else { " " }));
        }
        lines.push(util::green(status_line(
            &b.selected_names(),
            cols.saturating_sub(1),
        )));
        let more = if items.len() > page {
            format!(" · {}/{}", cursor + 1, items.len())
        } else {
            String::new()
        };
        let hint = note.take().unwrap_or_else(|| {
            format!(
                "↑↓ move · enter open/check · space check · ← ../ · tab done · esc cancel{more}"
            )
        });
        lines.push(util::dim(fit(&hint, cols.saturating_sub(1))));
        if *drawn > 0 {
            crossterm::queue!(err, crossterm::cursor::MoveUp(*drawn))?;
        }
        crossterm::queue!(
            err,
            crossterm::cursor::MoveToColumn(0),
            crossterm::terminal::Clear(crossterm::terminal::ClearType::FromCursorDown)
        )?;
        for l in &lines {
            write!(err, "{l}\r\n")?;
        }
        err.flush()?;
        *drawn = lines.len() as u16;

        let Event::Key(k) = event::read()? else {
            continue;
        };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let before = b.cwd.clone();
        let before_offset = b.offset;
        let current = &items[cursor];
        let action = match k.code {
            KeyCode::Char('c') if ctrl => return Err(Cancelled.into()),
            KeyCode::Esc => return Err(Cancelled.into()),
            KeyCode::Up | KeyCode::Char('k') => {
                cursor = cursor.checked_sub(1).unwrap_or(items.len() - 1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                cursor = (cursor + 1) % items.len();
                None
            }
            KeyCode::PageUp => {
                cursor = cursor.saturating_sub(page);
                None
            }
            KeyCode::PageDown => {
                cursor = (cursor + page).min(items.len() - 1);
                None
            }
            KeyCode::Home => {
                cursor = 0;
                None
            }
            KeyCode::End => {
                cursor = items.len() - 1;
                None
            }
            KeyCode::Tab | KeyCode::Char('d') => Some(Action::Done),
            KeyCode::Enter => Some(current.action.clone()),
            KeyCode::Right => match &current.action {
                a @ Action::Open(_) => Some(a.clone()),
                _ => None,
            },
            KeyCode::Char(' ') => match &current.action {
                a @ Action::Toggle(_) => Some(a.clone()),
                _ => {
                    note = Some(why_not(current));
                    None
                }
            },
            KeyCode::Left | KeyCode::Backspace => Some(Action::Up),
            _ => None,
        };
        let Some(action) = action else { continue };
        if action == Action::Nothing {
            note = Some(why_not(&items[cursor]));
            continue;
        }
        if b.apply(&action) {
            break;
        }
        if b.cwd != before || b.offset != before_offset {
            items = b.items();
            top = 0;
            // Going up: land on the folder we came from.
            cursor = items
                .iter()
                .position(|i| i.entry.as_ref().is_some_and(|(e, _)| e.path == before))
                .unwrap_or_else(|| first_entry(&items));
        }
    }
    Ok(())
}

/// Why the row under the cursor can't be checked.
fn why_not(it: &Item) -> String {
    match &it.entry {
        Some((e, _)) => match e.kind {
            Kind::Dir if !e.readable => "can't read this folder".into(),
            Kind::Dir => "only git repos can be checked; enter opens the folder".into(),
            Kind::KexRepo => "already a kex repo".into(),
            Kind::Worktree => "a worktree shares its main checkout's kex.json".into(),
            _ => "can't be checked".into(),
        },
        None => "enter: choose this row".into(),
    }
}

/// Keep the end of `s` (the folder you're in) when it's wider than `width` columns.
fn fit_left(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut tail: Vec<char> = Vec::new();
    let mut w = 1; // the "…"
    for c in s.chars().rev() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > width {
            break;
        }
        w += cw;
        tail.push(c);
    }
    format!("…{}", tail.into_iter().rev().collect::<String>())
}

/// Cut `s` to `width` terminal columns (wide characters count double), so it never wraps.
fn fit(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    let mut t = String::new();
    let mut w = 1; // the "…"
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if w + cw > width {
            break;
        }
        w += cw;
        t.push(c);
    }
    t.push('…');
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_lines() {
        assert_eq!(fit("abcdef", 4), "abc…");
        assert_eq!(fit("abc", 4), "abc");
        assert_eq!(fit_left("~/work/shop", 6), "…/shop");
        assert_eq!(fit_left("~/a", 6), "~/a");
    }

    #[test]
    fn fits_wide_characters_by_columns() {
        assert_eq!(fit("日本語のフォルダ", 7), "日本語…");
        assert!(fit("日本語のフォルダ", 7).width() <= 7);
        assert_eq!(fit_left("~/作業/店", 6), "…業/店"); // 1 + 2 + 1 + 2 columns
    }

    #[test]
    fn status_line_fits_names_then_counts_the_rest() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(status_line(&[], 40), "0 selected");
        assert_eq!(status_line(&n(&["a", "b"]), 40), "2 selected: a, b");
        let names = n(&["folder_x", "folder_abcd", "folder_looooong_name"]);
        assert_eq!(
            status_line(&names, 42),
            "3 selected: folder_x, folder_abcd, +1 more"
        );
        assert_eq!(status_line(&names, 30), "3 selected: folder_x, +2 more");
        let s = status_line(&n(&["folder_looooong_name"]), 20);
        assert_eq!(s, "1 selected: folder_…");
        assert!(s.width() <= 20);
    }

    #[test]
    fn apply_moves_and_toggles() {
        let root = std::env::temp_dir();
        let mut b = Browser::new(&root, 3, Vec::new());
        let api = root.join("api");
        assert!(!b.apply(&Action::Toggle(api.clone())));
        assert_eq!(b.selected, std::slice::from_ref(&api));
        assert!(!b.apply(&Action::Open(root.join("sub"))));
        assert_eq!(b.cwd, root.join("sub"));
        // The selection stays while moving; toggling again unchecks.
        assert!(!b.apply(&Action::Up));
        assert_eq!(b.cwd, root);
        assert!(!b.apply(&Action::Toggle(api)));
        assert!(b.selected.is_empty());
        assert!(!b.apply(&Action::Nothing));
        assert!(b.apply(&Action::Done));
    }

    #[test]
    fn pages_through_a_folder_bigger_than_one_view() {
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..PICKER_MAX_ENTRIES + 5 {
            std::fs::create_dir_all(tmp.path().join(format!("d{i:03}"))).unwrap();
        }
        std::fs::create_dir_all(tmp.path().join("zrepo/.git")).unwrap();
        let mut b = Browser::new(tmp.path(), 1, Vec::new());
        let items = b.items();
        let Some(Action::Page(next)) = items.last().map(|i| i.action.clone()) else {
            panic!("no next-page row");
        };
        assert_eq!(next, PICKER_MAX_ENTRIES);
        assert!(b.label(items.last().unwrap(), 10).contains("6 more"));
        assert!(!items.iter().any(|i| i.action == Action::Page(0)));
        // The next page has the rest, zrepo among them, and a way back.
        b.apply(&Action::Page(next));
        let items = b.items();
        assert!(items.iter().any(|i| i.action == Action::Page(0)));
        let z = items
            .iter()
            .find(|i| i.entry.as_ref().is_some_and(|(e, _)| e.name == "zrepo"))
            .expect("zrepo on page 2");
        b.apply(&z.action.clone());
        assert_eq!(b.selected_names(), ["zrepo"]);
    }

    #[test]
    fn up_stops_at_the_filesystem_root() {
        let top = std::env::temp_dir()
            .ancestors()
            .last()
            .unwrap()
            .to_path_buf();
        let mut b = Browser::new(&top, 3, Vec::new());
        let empty = tree::list(
            &std::env::temp_dir().join("kex-no-such-dir"),
            &tree::Opts::picker(1, 1, 0),
        );
        b.cache.insert((top.clone(), 0), empty); // don't list the real root
        b.apply(&Action::Up);
        assert_eq!(b.cwd, top);
        let items = b.items();
        assert!(matches!(items[0].action, Action::Done));
        assert!(!items.iter().any(|i| i.action == Action::Up));
    }
}
