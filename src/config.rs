//! Finding, parsing and writing a repo's `kex.json` (BLUEPRINT §3).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::git;

pub const SUPPORTED_VERSION: u32 = 1;
pub const SCHEMA_URL: &str =
    "https://raw.githubusercontent.com/yuji-luigi/kex/main/schema/kex.schema.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub version: u32,
    pub project: String,
    pub repo: String,
    pub repo_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hosting: Option<Hosting>,
    #[serde(default)]
    pub prime_branches: Vec<PrimeBranch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protected: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore_branches: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peers: Vec<Peer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<RunSpec>,
}

/// `"github.com"` or `{ "domain": "git.acme.com", "provider": "gitlab" }` (§3.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Hosting {
    Domain(String),
    Full {
        domain: String,
        provider: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrimeBranch {
    pub branch: String,
    pub stage: Stage,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Dev,
    Staging,
    Production,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    pub repo: String,
    pub remote: String,
}

/// How to run a repo; every field overrides detection (§5.3.1).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<OsCommand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev: Option<OsCommand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<String>,
}

impl RunSpec {
    /// `over` wins field by field (used for `kex.local.json`).
    pub fn merge(self, over: RunSpec) -> RunSpec {
        RunSpec {
            preset: over.preset.or(self.preset),
            install: over.install.or(self.install),
            dev: over.dev.or(self.dev),
            port: over.port.or(self.port),
            ready: over.ready.or(self.ready),
        }
    }
}

/// A command, the same everywhere or per OS.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OsCommand {
    One(String),
    PerOs {
        #[serde(default)]
        default: Option<String>,
        #[serde(default)]
        macos: Option<String>,
        #[serde(default)]
        linux: Option<String>,
        #[serde(default)]
        windows: Option<String>,
    },
}

impl OsCommand {
    pub fn for_this_os(&self) -> Option<String> {
        match self {
            OsCommand::One(s) => Some(s.clone()),
            OsCommand::PerOs {
                default,
                macos,
                linux,
                windows,
            } => {
                let os = if cfg!(windows) {
                    windows
                } else if cfg!(target_os = "macos") {
                    macos
                } else {
                    linux
                };
                os.clone().or_else(|| default.clone())
            }
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct LocalConfig {
    #[serde(default)]
    run: Option<RunSpec>,
}

impl Config {
    /// The default prime branch: the first one with `stage: dev`.
    pub fn default_prime(&self) -> Option<&PrimeBranch> {
        self.prime_branches.iter().find(|b| b.stage == Stage::Dev)
    }
}

/// Config path in a repo root: `.config/kex.json`, else `kex.json`. Both at once is an error.
pub fn config_path(repo_root: &Path) -> Result<Option<PathBuf>> {
    let nested = repo_root.join(".config").join("kex.json");
    let flat = repo_root.join("kex.json");
    match (nested.is_file(), flat.is_file()) {
        (true, true) => bail!(
            "both {} and {} exist; keep only one",
            nested.display(),
            flat.display()
        ),
        (true, false) => Ok(Some(nested)),
        (false, true) => Ok(Some(flat)),
        (false, false) => Ok(None),
    }
}

pub fn parse(text: &str, origin: &str) -> Result<Config> {
    let config: Config = serde_json::from_str(text)
        .map_err(|e| anyhow!("{origin}:{}:{}: {e}", e.line(), e.column()))?;
    if config.version != SUPPORTED_VERSION {
        bail!(
            "{origin}: version {} is not supported (this kex reads version {SUPPORTED_VERSION})",
            config.version
        );
    }
    Ok(config)
}

pub fn load(path: &Path) -> Result<Config> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse(&text, &path.display().to_string())
}

/// The config as committed on `rev` (for checkouts of an old branch without it, §4.3).
pub fn load_at_rev(repo_root: &Path, rev: &str) -> Option<Config> {
    for r in [rev.to_string(), format!("origin/{rev}")] {
        for rel in [".config/kex.json", "kex.json"] {
            let spec = format!("{r}:{rel}");
            if let Ok(text) = git::git(repo_root, &["show", &spec]) {
                return parse(&text, &spec).ok();
            }
        }
    }
    None
}

/// Personal `run` overrides from `kex.local.json` next to the config.
pub fn load_local(config_path: &Path) -> Result<Option<RunSpec>> {
    let path = config_path.with_file_name("kex.local.json");
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let local: LocalConfig = serde_json::from_str(&text)
        .map_err(|e| anyhow!("{}:{}:{}: {e}", path.display(), e.line(), e.column()))?;
    Ok(local.run)
}

pub fn save(path: &Path, config: &Config) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(config)? + "\n")
        .with_context(|| format!("writing {}", path.display()))
}

/// 6 random lowercase letters/digits, for project and repo ids.
pub fn random_suffix() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    let mut n = h.finish();
    let alphabet = b"0123456789abcdefghijklmnopqrstuvwxyz";
    (0..6)
        .map(|_| {
            let c = alphabet[(n % 36) as usize] as char;
            n /= 36;
            c
        })
        .collect()
}

pub fn new_repo_id() -> String {
    format!("r_{}", random_suffix())
}
