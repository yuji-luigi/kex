//! Editor workspace files that list a project's folders, so `kex init` / `kex add` can
//! offer "these are your repos" instead of making you browse.
//!
//! Supported formats (see [`FORMATS`]): VS Code `*.code-workspace` (also written by Cursor,
//! VSCodium and Windsurf). Adding a format = one more [`Format`] in the list.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::tree::{self, Kind};
use crate::util;

/// One workspace file format.
pub trait Format: Sync {
    /// Short id sent in JSON, e.g. `vscode`.
    fn id(&self) -> &'static str;
    /// Whether a file with this name is in this format.
    fn matches(&self, file_name: &str) -> bool;
    /// The folders the file lists: as written, and resolved to an absolute path (`None`
    /// when it can't be, e.g. a remote URI).
    fn folders(&self, file: &Path, text: &str) -> Result<Vec<Listed>>;
}

pub struct Listed {
    pub written: String,
    pub resolved: Option<PathBuf>,
    pub name: Option<String>,
}

pub const FORMATS: &[&dyn Format] = &[&VsCode];

/// `*.code-workspace`: JSON with comments and trailing commas, `folders: [{path, name?}]`
/// (paths relative to the file) or `{uri}` with a `file://` URI.
pub struct VsCode;

impl Format for VsCode {
    fn id(&self) -> &'static str {
        "vscode"
    }

    fn matches(&self, file_name: &str) -> bool {
        file_name.ends_with(".code-workspace") && file_name.len() > ".code-workspace".len()
    }

    fn folders(&self, file: &Path, text: &str) -> Result<Vec<Listed>> {
        let v: Value = serde_json::from_str(&jsonc(text))
            .with_context(|| format!("{} isn't valid JSON", file.display()))?;
        let Some(folders) = v.get("folders").and_then(Value::as_array) else {
            bail!("{} has no \"folders\" list", file.display());
        };
        let dir = file.parent().unwrap_or(Path::new("."));
        Ok(folders
            .iter()
            .filter_map(|f| {
                let name = f.get("name").and_then(Value::as_str).map(String::from);
                if let Some(p) = f.get("path").and_then(Value::as_str) {
                    let p2 = util::expand_tilde(p);
                    return Some(Listed {
                        written: p.to_string(),
                        resolved: Some(dir.join(p2)),
                        name,
                    });
                }
                let uri = f.get("uri").and_then(Value::as_str)?;
                Some(Listed {
                    written: uri.to_string(),
                    resolved: uri
                        .strip_prefix("file://")
                        .map(|p| PathBuf::from(percent_decode(p))),
                    name,
                })
            })
            .collect())
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(x) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(x);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// JSON with `//` and `/* */` comments and trailing commas → plain JSON.
pub fn jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            match c {
                '\\' => {
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                }
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_str = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut prev = ' ';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
                out.push(' ');
            }
            (']' | '}', _) => {
                // Drop a comma before the closing bracket (and the space between).
                let trimmed = out.trim_end().len();
                if out[..trimmed].ends_with(',') {
                    out.truncate(trimmed - 1);
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// A workspace file and its folders, classified.
pub struct Workspace {
    pub file: PathBuf,
    pub format: &'static str,
    pub folders: Vec<Folder>,
}

pub struct Folder {
    pub written: String,
    /// Absolute and resolved (symlinks too) when it exists.
    pub resolved: Option<PathBuf>,
    /// The name the workspace gives it.
    pub name: Option<String>,
    pub exists: bool,
    pub kind: Option<Kind>,
    pub project: Option<String>,
    pub repo: Option<String>,
    /// The file lists the same folder (same resolved path) earlier, at this index. A
    /// duplicate is shown but counted and set up once.
    pub duplicate_of: Option<usize>,
}

impl Folder {
    /// The repo name kex would use: the workspace's name for it, else its folder name.
    pub fn default_name(&self) -> String {
        let folder = self
            .resolved
            .as_deref()
            .and_then(Path::file_name)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        util::slug(self.name.as_deref().unwrap_or(&folder))
    }
}

/// Workspace files directly in `dirs` (no duplicates), sorted.
pub fn find(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        let mut here: Vec<PathBuf> = rd
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file() || t.is_symlink()))
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| FORMATS.iter().any(|f| f.matches(n)))
            })
            .map(|e| util::canon(&e.path()))
            .collect();
        here.sort();
        for f in here {
            if !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out
}

/// Read and classify a workspace file.
pub fn load(file: &Path) -> Result<Workspace> {
    let file = util::canon(file);
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    let Some(format) = FORMATS.iter().find(|f| f.matches(&name)) else {
        bail!(
            "{} isn't a workspace file kex knows (supported: *.code-workspace)",
            file.display()
        );
    };
    let text =
        std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    let mut folders: Vec<Folder> = Vec::new();
    for l in format.folders(&file, &text)? {
        let exists = l.resolved.as_deref().is_some_and(Path::is_dir);
        // Existing folders: real path (symlinks too); missing ones: `.` and `..` removed.
        let resolved = l.resolved.map(|p| {
            if exists {
                util::canon(&p)
            } else {
                util::normalize(&p)
            }
        });
        let duplicate_of = resolved
            .as_ref()
            .and_then(|r| folders.iter().position(|f| f.resolved.as_ref() == Some(r)));
        folders.push({
            let mut f = Folder {
                written: l.written,
                name: l.name,
                exists,
                kind: None,
                project: None,
                repo: None,
                resolved,
                duplicate_of,
            };
            if let (true, Some(p)) = (exists, &f.resolved) {
                let e = tree::entry(String::new(), p.clone(), 1);
                f.kind = Some(e.kind);
                f.project = e.project;
                f.repo = e.repo;
            }
            f
        });
    }
    Ok(Workspace {
        file,
        format: format.id(),
        folders,
    })
}

impl Workspace {
    /// `shop` for `shop.code-workspace`.
    pub fn name(&self) -> String {
        let n = self
            .file
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        n.split('.').next().unwrap_or(&n).to_string()
    }

    /// Git repos without a kex config: the ones to set up.
    pub fn addable(&self) -> impl Iterator<Item = &Folder> {
        self.unique().filter(|f| f.kind == Some(Kind::Repo))
    }

    /// The folders, each listed once.
    pub fn unique(&self) -> impl Iterator<Item = &Folder> {
        self.folders.iter().filter(|f| f.duplicate_of.is_none())
    }

    pub fn json(&self) -> Value {
        let folders: Vec<Value> = self
            .folders
            .iter()
            .map(|f| {
                json!({
                    "path": f.written,
                    "resolved": f.resolved.as_deref().map(|p| p.to_string_lossy().into_owned()),
                    "name": f.name,
                    "exists": f.exists,
                    "kind": f.kind.map(Kind::as_str),
                    "project": f.project,
                    "repo": f.repo,
                    "default_name": f.default_name(),
                    "duplicate_of": f.duplicate_of,
                })
            })
            .collect();
        let count = |k: Kind| self.unique().filter(|f| f.kind == Some(k)).count();
        json!({
            "path": self.file.to_string_lossy(),
            "name": self.name(),
            "format": self.format,
            "folders": folders,
            "counts": {
                "folders": self.unique().count(),
                "repos": count(Kind::Repo) + count(Kind::KexRepo),
                "addable": count(Kind::Repo),
                "already_kex": count(Kind::KexRepo),
                "worktrees": count(Kind::Worktree),
                "missing": self.unique().filter(|f| !f.exists).count(),
                "duplicates": self.folders.len() - self.unique().count(),
            },
        })
    }
}

/// Where to look for workspace files around `dir`: the folder, and when it's inside a git
/// repo, that repo's top folder and the folder around it.
pub fn search_dirs(dir: &Path) -> Vec<PathBuf> {
    let dir = util::canon(dir);
    let mut out = vec![dir.clone()];
    if let Ok(root) = crate::git::root(&dir) {
        let root = util::canon(&root);
        if let Some(p) = root.parent() {
            out.push(root.clone());
            out.push(p.to_path_buf());
        }
    }
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_json_with_comments_and_trailing_commas() {
        let text = r#"{
  // the repos
  "folders": [
    { "path": "api", "name": "API // server" }, /* web next */
    { "path": "../web", },
    { "uri": "file:///tmp/a%20b" },
    { "uri": "vscode-remote://ssh/x" },
  ],
  "settings": { "a": "x/*y*/", },
}"#;
        let v: Value = serde_json::from_str(&jsonc(text)).unwrap();
        assert_eq!(v["folders"][0]["name"], "API // server");
        assert_eq!(v["settings"]["a"], "x/*y*/");
        let f = VsCode
            .folders(Path::new("/w/shop/shop.code-workspace"), text)
            .unwrap();
        assert_eq!(f.len(), 4);
        assert_eq!(f[0].resolved.as_deref(), Some(Path::new("/w/shop/api")));
        assert_eq!(f[1].resolved.as_deref(), Some(Path::new("/w/shop/../web")));
        assert_eq!(f[1].written, "../web");
        assert_eq!(f[2].resolved.as_deref(), Some(Path::new("/tmp/a b")));
        assert!(f[3].resolved.is_none());
        assert!(VsCode.matches("x.code-workspace") && !VsCode.matches(".code-workspace"));
    }
}
