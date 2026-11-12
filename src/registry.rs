//! Machine-local state: where each repo is (`registry.json`) and personal settings
//! (`config.json`). Never committed (BLUEPRINT §4.1).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::util;

/// `~/.config/kex` (or `%APPDATA%\kex`). `KEX_HOME` overrides it, e.g. in tests.
pub fn kex_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("KEX_HOME") {
        return PathBuf::from(d);
    }
    if cfg!(windows)
        && let Some(a) = std::env::var_os("APPDATA")
    {
        return PathBuf::from(a).join("kex");
    }
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(x).join("kex");
    }
    util::home()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("kex")
}

fn read_json<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("{} is not valid JSON; fix or delete it", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Write to a temp file, then rename, so parallel kex processes never see half a file.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(value)? + "\n")?;
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Registry {
    /// project id → repo_id → entry
    #[serde(default)]
    pub projects: BTreeMap<String, BTreeMap<String, Entry>>,
    /// project id → when the last deep scan missed a peer (§4.3 step 5)
    #[serde(default)]
    pub not_found: BTreeMap<String, u64>,
    /// repo path → lockfile hash at the last successful install (§5.3)
    #[serde(default)]
    pub install_hashes: BTreeMap<String, String>,
    /// `project/repo` → the clone to use when there are several (§4.5)
    #[serde(default)]
    pub preferred: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub repo: String,
    #[serde(default)]
    pub root_commit: Option<String>,
    /// Default prime branch, to read the config from when a branch has none.
    #[serde(default)]
    pub prime: Option<String>,
    #[serde(default)]
    pub paths: Vec<PathBuf>,
}

impl Registry {
    fn path() -> PathBuf {
        kex_dir().join("registry.json")
    }

    pub fn load() -> Result<Self> {
        read_json(&Self::path())
    }

    pub fn save(&self) -> Result<()> {
        write_json(&Self::path(), self)
    }

    pub fn register(
        &mut self,
        project: &str,
        repo_id: &str,
        repo: &str,
        root_commit: Option<&str>,
        prime: Option<&str>,
        path: &Path,
    ) {
        let e = self
            .projects
            .entry(project.into())
            .or_default()
            .entry(repo_id.into())
            .or_default();
        e.repo = repo.into();
        if root_commit.is_some() {
            e.root_commit = root_commit.map(Into::into);
        }
        if prime.is_some() {
            e.prime = prime.map(Into::into);
        }
        if !e.paths.iter().any(|p| p == path) {
            e.paths.push(path.to_path_buf());
        }
    }

    pub fn forget_path(&mut self, project: &str, repo_id: &str, path: &Path) {
        if let Some(e) = self
            .projects
            .get_mut(project)
            .and_then(|p| p.get_mut(repo_id))
        {
            e.paths.retain(|p| p != path);
        }
    }

    pub fn entry_for_path(&self, path: &Path) -> Option<&Entry> {
        self.projects
            .values()
            .flat_map(|p| p.values())
            .find(|e| e.paths.iter().any(|p| p == path))
    }
}

/// Personal settings (§3.2).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct GlobalConfig {
    /// Folders searched deeply when a peer repo is missing.
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    /// Default iOS simulator name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sim: Option<String>,
    /// `pwsh`, `powershell` or `cmd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_shell: Option<String>,
}

impl GlobalConfig {
    fn path() -> PathBuf {
        kex_dir().join("config.json")
    }

    pub fn load() -> Result<Self> {
        read_json(&Self::path())
    }

    pub fn save(&self) -> Result<()> {
        write_json(&Self::path(), self)
    }
}
