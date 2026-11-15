//! `kex update` and the "new version available" notice (BLUEPRINT §8.5).
//!
//! Releases are GitHub releases of yuji-luigi/kex with one archive per target
//! (`kex-<target>.tar.gz`, `.zip` on Windows) and a `.sha256` next to each. Downloads use
//! `curl` and unpack with `tar`, both shipped with macOS, Linux and Windows 10+.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::registry::{self, kex_dir};
use crate::{ui, util};

const REPO: &str = "yuji-luigi/kex";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const DAY: u64 = 86_400;

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    checked_at: u64,
    latest: Option<String>,
}

fn cache_path() -> PathBuf {
    kex_dir().join("update.json")
}

/// `1.10.0` > `1.9.3`; a pre-release suffix is ignored.
pub fn newer(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v')
            .split(['-', '+'])
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    parse(a) > parse(b)
}

fn curl(args: &[&str]) -> Result<Vec<u8>> {
    let out = crate::cancel::output(
        Command::new("curl")
            .args(["-fsSL", "--proto", "=https", "--retry", "2"])
            .args(args),
    )
    .context("curl not found")?;
    if !out.status.success() {
        bail!(
            "download failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

/// Latest release version from GitHub, e.g. `0.2.0`.
fn fetch_latest(timeout_secs: u32) -> Result<String> {
    let t = timeout_secs.to_string();
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = curl(&[
        "--max-time",
        &t,
        "-H",
        "Accept: application/vnd.github+json",
        &url,
    ])?;
    let v: serde_json::Value = serde_json::from_slice(&body)?;
    let tag = v["tag_name"].as_str().context("no tag_name in release")?;
    // release-please tags look like `v0.2.0` (or `kex-v0.2.0`).
    Ok(tag.rsplit('v').next().unwrap_or(tag).to_string())
}

/// The release archive name for this machine.
fn target() -> Option<&'static str> {
    use std::env::consts::{ARCH, OS};
    Some(match (OS, ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        ("linux", "aarch64") => "aarch64-unknown-linux-musl",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        _ => return None,
    })
}

fn check_disabled() -> bool {
    std::env::var_os("KEX_NO_UPDATE_CHECK").is_some() || std::env::var_os("CI").is_some()
}

/// The newer version, if the last check (at most a day old) found one. Never blocks:
/// a stale cache is refreshed in the background for next time.
pub fn available() -> Option<String> {
    if check_disabled() {
        return None;
    }
    let cache: Cache = registry::read_json(&cache_path()).unwrap_or_default();
    if util::now().saturating_sub(cache.checked_at) > DAY {
        refresh_in_background();
    }
    cache.latest.filter(|l| newer(l, CURRENT))
}

/// A detached `kex update --check-only --quiet`, so the current command never waits on
/// the network.
fn refresh_in_background() {
    // Mark the check as done now, so parallel kex runs don't all start one.
    let mut cache: Cache = registry::read_json(&cache_path()).unwrap_or_default();
    cache.checked_at = util::now();
    let _ = registry::write_json(&cache_path(), &cache);
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe)
            .args(["update", "--check", "--quiet"])
            .env("KEX_NO_UPDATE_CHECK", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

/// Printed after a command finishes (stderr, so pipes stay clean).
pub fn notice() {
    if !util::interactive() && !ui::json() {
        return;
    }
    if let Some(v) = available() {
        ui::event(
            "update_available",
            serde_json::json!({
                "current": CURRENT, "latest": v,
                "key": "update_available",
                "view": crate::view::view("card", serde_json::json!({
                    "title": crate::view::msg("update.available", serde_json::json!({ "latest": v })),
                    "tone": "info",
                    "actions": [install_action(&v)],
                })),
            }),
        );
        if !ui::json() {
            eprintln!(
                "\n{} kex {v} is available (you have {CURRENT}). Run `kex update`.",
                util::yellow("↑")
            );
        }
    }
}

fn package_manager_hint(exe: &Path) -> Option<&'static str> {
    let p = exe.to_string_lossy();
    if p.contains("/Cellar/") || p.contains("/homebrew/") {
        Some("brew upgrade kex")
    } else if p.contains("\\scoop\\") {
        Some("scoop update kex")
    } else if p.contains("\\WinGet\\") {
        Some("winget upgrade kex")
    } else {
        None
    }
}

/// The "Install" button: runs `kex update --yes`, after the client's confirm.
fn install_action(version: &str) -> serde_json::Value {
    let p = serde_json::json!({ "version": version });
    serde_json::json!({
        "label": crate::view::msg("update.install", p.clone()),
        "effect": "install",
        "confirm": crate::view::msg("update.install_confirm", p),
        "run": { "command": "update", "options": { "yes": true } },
    })
}

pub fn run(check_only: bool, quiet: bool, yes: bool) -> Result<u8> {
    let latest = fetch_latest(if quiet { 5 } else { 15 });
    let mut cache: Cache = registry::read_json(&cache_path()).unwrap_or_default();
    cache.checked_at = util::now();
    if let Ok(l) = &latest {
        cache.latest = Some(l.clone());
    }
    let _ = registry::write_json(&cache_path(), &cache);
    if quiet {
        return Ok(0);
    }
    let latest = latest.context("could not check for a new version")?;
    let has_new = newer(&latest, CURRENT);
    let (title, tone, actions) = if has_new {
        let t = crate::view::msg("update.available", serde_json::json!({ "latest": latest }));
        (t, "info", vec![install_action(&latest)])
    } else {
        let t = crate::view::msg("update.latest", serde_json::json!({ "current": CURRENT }));
        (t, "ok", vec![])
    };
    ui::event(
        "version",
        serde_json::json!({
            "current": CURRENT, "latest": latest, "update": has_new,
            "key": "version",
            "view": crate::view::view("card", serde_json::json!({
                "title": title, "tone": tone,
                "fields": [
                    crate::view::field("installed", serde_json::json!(CURRENT), "code"),
                    crate::view::field("latest", serde_json::json!(latest), "code"),
                ],
                "actions": actions,
            })),
        }),
    );
    let about = |text: String| ui::say_about("version", "info", &text);
    if !has_new {
        let about = |text: String| ui::say_about("version", "win", &text);
        about(format!(
            "{} kex {CURRENT} is the latest version",
            util::green("✓")
        ));
        return Ok(0);
    }
    about(format!("kex {latest} is available (you have {CURRENT})"));
    about(format!(
        "  https://github.com/{REPO}/releases/tag/v{latest}"
    ));
    if check_only {
        return Ok(0);
    }
    let exe = util::canon(&std::env::current_exe()?);
    if let Some(cmd) = package_manager_hint(&exe) {
        say!("kex was installed with a package manager; update with `{cmd}`");
        return Ok(0);
    }
    let q = ui::Q::new("update.confirm", serde_json::json!({ "version": latest }));
    if !yes && util::interactive() && !ui::confirm(q, true)? {
        return Ok(0);
    }
    install(&latest, &exe)?;
    ui::event(
        "updated",
        serde_json::json!({
            "version": latest,
            "key": "version",
            "view": crate::view::view("card", serde_json::json!({
                "title": crate::view::msg("update.done", serde_json::json!({ "version": latest })),
                "tone": "ok",
            })),
        }),
    );
    ui::say_about(
        "version",
        "win",
        &format!(
            "{} kex {latest} installed to {}",
            util::green("✓"),
            util::tilde(&exe)
        ),
    );
    Ok(0)
}

fn install(version: &str, exe: &Path) -> Result<()> {
    let target = target().context("no prebuilt kex for this platform; build it from source")?;
    let ext = if cfg!(windows) { "zip" } else { "tar.gz" };
    let name = format!("kex-{target}.{ext}");
    let base = format!("https://github.com/{REPO}/releases/download/v{version}");
    say!("downloading {name}");
    let archive = curl(&[&format!("{base}/{name}")])?;
    let sums = String::from_utf8(curl(&[&format!("{base}/{name}.sha256")])?)?;
    let expected = sums.split_whitespace().next().unwrap_or("").to_lowercase();
    let actual: String = Sha256::digest(&archive)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if expected != actual {
        bail!("checksum mismatch for {name}; not installing");
    }

    let dir = exe.parent().context("where is kex installed?")?;
    let work = dir.join(format!(".kex-update-{}", std::process::id()));
    std::fs::create_dir_all(&work)
        .with_context(|| format!("can't write to {} (try sudo?)", dir.display()))?;
    let result = (|| -> Result<()> {
        let file = work.join(&name);
        std::fs::write(&file, &archive)?;
        let ok = crate::cancel::status(
            Command::new("tar")
                .arg("-xf")
                .arg(&file)
                .arg("-C")
                .arg(&work),
        )
        .context("tar not found")?
        .success();
        if !ok {
            bail!("could not unpack {name}");
        }
        let bin = work.join(if cfg!(windows) { "kex.exe" } else { "kex" });
        if !bin.is_file() {
            bail!("{name} has no kex binary");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))?;
        }
        // A running exe can't be overwritten on Windows, but it can be renamed away.
        if cfg!(windows) {
            let old = exe.with_extension("old.exe");
            let _ = std::fs::remove_file(&old);
            std::fs::rename(exe, &old)?;
        }
        std::fs::rename(&bin, exe).with_context(|| format!("replacing {}", exe.display()))
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(newer("0.2.0", "0.1.9"));
        assert!(newer("1.10.0", "1.9.3"));
        assert!(newer("v1.0.0", "0.9.0"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1.0-rc.1", "0.1.0"));
    }
}
