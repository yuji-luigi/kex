//! The `flutter-ios` runtime: pick a simulator, boot it, hot reload on save (BLUEPRINT §5.3).

use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::util;

pub struct Sim {
    pub name: String,
    pub runtime: String,
    pub udid: String,
    pub booted: bool,
}

/// Available iOS simulators, booted ones and iPhones first.
fn list() -> Result<Vec<Sim>> {
    let out = Command::new("xcrun")
        .args(["simctl", "list", "devices", "available", "-j"])
        .output()
        .context("running xcrun (is Xcode installed?)")?;
    if !out.status.success() {
        bail!(
            "xcrun simctl failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let mut sims = Vec::new();
    for (rt, devices) in v["devices"].as_object().into_iter().flatten() {
        // com.apple.CoreSimulator.SimRuntime.iOS-18-2 → iOS 18.2
        let Some(short) = rt
            .rsplit("SimRuntime.")
            .next()
            .filter(|s| s.starts_with("iOS"))
        else {
            continue;
        };
        let runtime = short.replacen('-', " ", 1).replace('-', ".");
        for d in devices.as_array().into_iter().flatten() {
            sims.push(Sim {
                name: d["name"].as_str().unwrap_or("?").into(),
                udid: d["udid"].as_str().unwrap_or_default().into(),
                booted: d["state"] == "Booted",
                runtime: runtime.clone(),
            });
        }
    }
    sims.sort_by(|a, b| {
        b.booted
            .cmp(&a.booted)
            .then(
                b.name
                    .starts_with("iPhone")
                    .cmp(&a.name.starts_with("iPhone")),
            )
            .then(b.runtime.cmp(&a.runtime))
            .then(a.name.cmp(&b.name))
    });
    Ok(sims)
}

/// `name` (from `--sim` or the global config), else ask, else the first one.
pub fn choose(name: Option<&str>) -> Result<Sim> {
    let mut sims = list()?;
    if sims.is_empty() {
        bail!("no iOS simulators found (add one in Xcode → Settings → Components)");
    }
    if let Some(n) = name {
        return match sims.iter().position(|s| s.name.eq_ignore_ascii_case(n)) {
            Some(i) => Ok(sims.swap_remove(i)),
            None => {
                let names: Vec<&str> = sims.iter().map(|s| s.name.as_str()).collect();
                bail!(
                    "no simulator named \"{n}\"; available: {}",
                    names.join(", ")
                )
            }
        };
    }
    if !util::interactive() {
        return Ok(sims.swap_remove(0));
    }
    let opts: Vec<crate::ui::Opt> = sims
        .iter()
        .map(|s| {
            let label = format!(
                "{} ({}){}",
                s.name,
                s.runtime,
                if s.booted { " · booted" } else { "" }
            );
            crate::ui::Opt::new(&s.udid, label).data(serde_json::json!({
                "name": s.name, "runtime": s.runtime, "booted": s.booted,
            }))
        })
        .collect();
    let i = crate::ui::select_one("iOS simulator", &opts)?;
    Ok(sims.swap_remove(i))
}

pub fn boot(s: &Sim) {
    let quiet = |cmd: &str, args: &[&str]| {
        Command::new(cmd)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|st| st.success())
    };
    // Already booted is fine.
    quiet("xcrun", &["simctl", "boot", &s.udid]);
    quiet("xcrun", &["simctl", "bootstatus", &s.udid]);
    // Newer Xcode versions replaced Simulator.app with DeviceHub.app.
    let opened = quiet(
        "open",
        &["-a", "Simulator", "--args", "-CurrentDeviceUDID", &s.udid],
    ) || quiet("open", &["-a", "DeviceHub"]);
    if !opened {
        warn!(
            "kex: simulator window app not found; {} is booted headless",
            s.name
        );
    }
}

/// Hot reload on save: `flutter run` reloads on SIGUSR1. The watcher lives as long as the
/// returned value.
#[cfg(unix)]
pub fn watch(lib: &Path, pidfile: &Path) -> Option<Box<dyn std::any::Any>> {
    use notify::{EventKind, RecursiveMode, Watcher};
    use std::time::{Duration, Instant};

    let pidfile = pidfile.to_path_buf();
    let last = std::sync::Mutex::new(Instant::now());
    let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else { return };
        let relevant = matches!(
            ev.kind,
            EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
        ) && ev
            .paths
            .iter()
            .any(|p| p.extension().is_some_and(|e| e == "dart"));
        if !relevant {
            return;
        }
        let Ok(mut last) = last.lock() else { return };
        if last.elapsed() < Duration::from_millis(300) {
            return;
        }
        *last = Instant::now();
        if let Some(pid) = std::fs::read_to_string(&pidfile)
            .ok()
            .and_then(|s| s.trim().parse::<i32>().ok())
        {
            // SAFETY: sending a signal to a pid has no memory-safety requirements.
            unsafe {
                libc::kill(pid, libc::SIGUSR1);
            }
        }
    })
    .ok()?;
    w.watch(lib, RecursiveMode::Recursive).ok()?;
    Some(Box::new(w))
}

#[cfg(not(unix))]
pub fn watch(_lib: &Path, _pidfile: &Path) -> Option<Box<dyn std::any::Any>> {
    None
}
