//! `kex run`: detect how to run each repo, start them together, stop them together
//! (BLUEPRINT §5.3).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::ChildStdin;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::config;
use crate::project::{self, Repo};
use crate::registry::{GlobalConfig, Registry};
use crate::{flutter, pull, ui, util};

pub struct Opts {
    pub repos: Option<Vec<String>>,
    pub no_update: bool,
    pub install: bool,
    pub sim: Option<String>,
    pub force: bool,
}

/// How one repo will be run, and why.
struct Plan<'a> {
    repo: &'a Repo,
    label: String,
    reason: String,
    install: Option<String>,
    dev: Option<String>,
    port: Option<u16>,
    ready: Option<String>,
    /// `flutter` or `fvm flutter` for the flutter-ios runtime.
    flutter: Option<String>,
    lockfile: Option<PathBuf>,
    deps_dir: Option<PathBuf>,
    unavailable: Option<String>,
}

/// §5.3.1: `kex.local.json`, then `kex.json`, then detection from the repo's files.
fn plan_for(r: &Repo) -> Plan<'_> {
    let mut spec = r.config.run.clone().unwrap_or_default();
    if let Some(local) = r
        .config_path
        .as_deref()
        .and_then(|p| config::load_local(p).ok().flatten())
    {
        spec = spec.merge(local);
    }
    let dir = &r.path;
    let mut plan = Plan {
        repo: r,
        label: String::new(),
        reason: String::new(),
        install: spec.install.as_ref().and_then(|c| c.for_this_os()),
        dev: spec.dev.as_ref().and_then(|c| c.for_this_os()),
        port: spec.port,
        ready: spec.ready.clone(),
        flutter: None,
        lockfile: None,
        deps_dir: None,
        unavailable: None,
    };
    let preset = spec.preset.as_deref();
    if preset.is_none() && plan.dev.is_some() {
        plan.label = plan.dev.clone().unwrap_or_default();
        plan.reason = "kex.json".into();
        return plan;
    }
    if preset == Some("flutter-ios") || (preset.is_none() && is_flutter(dir)) {
        let fvm = (dir.join(".fvmrc").exists() || dir.join(".fvm").exists())
            && util::which("fvm").is_some();
        let fl = if fvm { "fvm flutter" } else { "flutter" };
        plan.label = "flutter-ios".into();
        plan.reason = if preset.is_some() {
            "kex.json".into()
        } else {
            "pubspec.yaml".into()
        };
        plan.install.get_or_insert_with(|| format!("{fl} pub get"));
        plan.lockfile = Some(dir.join("pubspec.lock"));
        plan.deps_dir = Some(dir.join(".dart_tool"));
        plan.flutter = Some(fl.into());
        if !cfg!(target_os = "macos") {
            plan.unavailable = Some("Flutter: iOS runtime needs macOS".into());
        }
        return plan;
    }
    match preset {
        Some("node") | None if dir.join("package.json").is_file() => node(&mut plan, dir),
        Some(p) if p != "node" => plan.unavailable = Some(format!("unknown preset \"{p}\"")),
        _ => plan.unavailable = Some("don't know how to run it: add \"run\" to kex.json".into()),
    }
    plan
}

fn is_flutter(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("pubspec.yaml")).is_ok_and(|t| t.contains("sdk: flutter"))
}

fn node(plan: &mut Plan, dir: &Path) {
    let pkg: serde_json::Value = std::fs::read_to_string(dir.join("package.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    const LOCKS: [(&str, &str); 5] = [
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lock", "bun"),
        ("bun.lockb", "bun"),
        ("package-lock.json", "npm"),
    ];
    let lock = LOCKS.iter().find(|(f, _)| dir.join(f).is_file());
    let declared = pkg["packageManager"]
        .as_str()
        .and_then(|s| s.split('@').next())
        .filter(|s| !s.is_empty());
    let (pm, why) = match (declared, lock) {
        (Some(pm), _) => (pm.to_string(), "packageManager".to_string()),
        (None, Some((f, pm))) => (pm.to_string(), f.to_string()),
        (None, None) => ("npm".into(), "package.json".into()),
    };
    plan.lockfile = lock.map(|(f, _)| dir.join(f));
    plan.deps_dir = Some(dir.join("node_modules"));
    plan.install.get_or_insert_with(|| format!("{pm} install"));
    plan.reason = why;
    if plan.dev.is_none() {
        match ["dev", "start", "serve"]
            .into_iter()
            .find(|s| pkg["scripts"][*s].is_string())
        {
            Some(s) => {
                plan.dev = Some(format!("{pm} run {s}"));
                plan.reason = format!("{}, script \"{s}\"", plan.reason);
            }
            None => {
                plan.unavailable = Some("package.json has no dev, start or serve script".into())
            }
        }
    }
    if plan.port.is_none() {
        plan.port = env_port(dir);
    }
    plan.label = format!("node · {}", plan.dev.clone().unwrap_or_default());
}

/// `PORT=` from `.env`, like the old Makefiles did.
fn env_port(dir: &Path) -> Option<u16> {
    let text = std::fs::read_to_string(dir.join(".env")).ok()?;
    text.lines()
        .find_map(|l| l.trim().strip_prefix("PORT="))
        .and_then(|v| v.trim().trim_matches(['"', '\'']).parse().ok())
}

/// The user's shell: `sh -c` on macOS/Linux, PowerShell on Windows (§8.3).
pub fn shell(cmd: &str) -> Command {
    if cfg!(windows) {
        let configured = GlobalConfig::load().ok().and_then(|g| g.windows_shell);
        let sh = configured.unwrap_or_else(|| {
            if util::which("pwsh").is_some() {
                "pwsh".into()
            } else {
                "powershell".into()
            }
        });
        let mut c = Command::new(&sh);
        if sh.eq_ignore_ascii_case("cmd") {
            c.args(["/C", cmd]);
        } else {
            c.args(["-NoProfile", "-Command", cmd]);
        }
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", cmd]);
        c
    }
}

pub fn run(o: Opts) -> Result<u8> {
    let p = project::resolve(&std::env::current_dir()?)?;
    p.print_header();
    let mut plans = Vec::new();
    for name in p.names() {
        plans.push(plan_for(p.pick(&name)?));
    }
    for pl in plans.iter().filter(|x| x.unavailable.is_some()) {
        say!(
            "  {} {:<8} {}",
            util::dim("-"),
            pl.repo.name,
            util::dim(pl.unavailable.as_deref().unwrap_or(""))
        );
    }
    let avail: Vec<usize> = (0..plans.len())
        .filter(|i| plans[*i].unavailable.is_none())
        .collect();

    let chosen: Vec<usize> = if let Some(names) = &o.repos {
        let mut v = Vec::new();
        for n in names {
            match plans.iter().position(|x| &x.repo.name == n) {
                Some(i) if plans[i].unavailable.is_none() => v.push(i),
                Some(i) => bail!(
                    "{n} can't run here: {}",
                    plans[i].unavailable.as_deref().unwrap_or("")
                ),
                None => bail!(
                    "no repo named \"{n}\" in this project (have: {})",
                    p.names().join(", ")
                ),
            }
        }
        v
    } else if util::interactive() && avail.len() > 1 {
        let opts: Vec<ui::Opt> = avail
            .iter()
            .map(|i| {
                let x = &plans[*i];
                let label = format!("{:<8} {:<28} ({})", x.repo.name, x.label, x.reason);
                ui::Opt::new(&x.repo.name, label).data(serde_json::json!({
                    "runtime": x.label, "reason": x.reason, "port": x.port,
                    "path": x.repo.path,
                }))
            })
            .collect();
        let all: Vec<usize> = (0..opts.len()).collect();
        ui::multi_select("Run which repos?", &opts, &all)?
            .into_iter()
            .map(|c| avail[c])
            .collect()
    } else {
        for i in &avail {
            let x = &plans[*i];
            say!(
                "  {} {:<8} {} ({})",
                util::green("✓"),
                x.repo.name,
                x.label,
                util::dim(&x.reason)
            );
        }
        avail.clone()
    };
    if chosen.is_empty() {
        say!("nothing to run");
        return Ok(0);
    }
    let chosen: Vec<&Plan> = chosen.iter().map(|i| &plans[*i]).collect();
    if !o.no_update {
        update_and_install(&chosen, o.install)?;
    }

    let global = GlobalConfig::load()?;
    let mut services = Vec::new();
    for pl in &chosen {
        if let Some(port) = pl.port.filter(|port| port_in_use(*port))
            && !o.force
        {
            say!(
                "{} {}: port {port} is already in use (maybe a worktree runs it?), skipped. Use --force to run anyway.",
                util::yellow("⚠"),
                pl.repo.name
            );
            continue;
        }
        let (cmd, pidfile) = match &pl.flutter {
            Some(fl) => {
                let sim = flutter::choose(o.sim.as_deref().or(global.sim.as_deref()))?;
                flutter::boot(&sim);
                let pidfile = std::env::temp_dir().join(format!("kex-flutter-{}.pid", sim.udid));
                let _ = std::fs::remove_file(&pidfile);
                (
                    format!(
                        "{fl} run -d {} --pid-file \"{}\"",
                        sim.udid,
                        pidfile.display()
                    ),
                    Some(pidfile),
                )
            }
            None => (pl.dev.clone().unwrap_or_default(), None),
        };
        services.push(Service {
            name: pl.repo.name.clone(),
            dir: pl.repo.path.clone(),
            cmd,
            pidfile,
            ready: pl.ready.clone(),
        });
    }
    supervise(services)
}

/// Pull, then install only when the lockfile changed or dependencies are missing.
fn update_and_install(plans: &[&Plan], force: bool) -> Result<()> {
    let updates = util::parallel(plans, |pl| pull::update(pl.repo, false));
    for (pl, u) in plans.iter().zip(&updates) {
        ui::event(
            "pull",
            serde_json::json!({
                "repo": pl.repo.name, "path": pl.repo.path, "branch": u.branch,
                "ok": u.ok, "status": ui::strip_ansi(&u.status),
            }),
        );
        say!(
            "{} ({}) {}",
            util::bold(format!("== {}", pl.repo.name)),
            u.branch,
            u.status
        );
    }
    let mut reg = Registry::load()?;
    for pl in plans {
        let Some(cmd) = &pl.install else { continue };
        let key = pl.repo.path.display().to_string();
        let hash = pl
            .lockfile
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .map(|b| util::fnv1a(&b));
        let deps_missing = pl.deps_dir.as_ref().is_some_and(|d| !d.exists());
        let up_to_date = !deps_missing
            && match &hash {
                Some(h) => reg.install_hashes.get(&key) == Some(h),
                None => pl.deps_dir.is_some(),
            };
        if up_to_date && !force {
            continue;
        }
        say!("{} {}: {cmd}", util::bold("==>"), pl.repo.name);
        let st = shell(cmd)
            .current_dir(&pl.repo.path)
            .stdout(ui::child_stdout())
            .status()
            .with_context(|| format!("running {cmd}"))?;
        if !st.success() {
            bail!("{}: `{cmd}` failed", pl.repo.name);
        }
        if let Some(h) = hash {
            reg.install_hashes.insert(key, h);
        }
    }
    reg.save()
}

struct Service {
    name: String,
    dir: PathBuf,
    cmd: String,
    pidfile: Option<PathBuf>,
    ready: Option<String>,
}

const COLORS: [&str; 6] = ["36", "35", "33", "32", "34", "31"];

/// Start everything with prefixed output; Ctrl-C stops everything.
fn supervise(services: Vec<Service>) -> Result<u8> {
    if services.is_empty() {
        return Ok(0);
    }
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        // Children share the terminal, so they get Ctrl-C too; kex only waits, then cleans up.
        let _ = ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst));
    }
    let width = services.iter().map(|s| s.name.len()).max().unwrap_or(0);
    let json = ui::json();
    // JSON mode: stdin carries `input` / `stop` messages, forwarded to the services.
    let inputs: Arc<Mutex<HashMap<String, ChildStdin>>> = Arc::default();
    let default_input = services
        .iter()
        .find(|s| s.pidfile.is_some())
        .map(|s| s.name.clone());
    let mut children: Vec<(String, String, Child)> = Vec::new();
    let mut watchers = Vec::new();
    let mut stdin_taken = false;
    for (i, s) in services.into_iter().enumerate() {
        let prefix = util::paint(COLORS[i % COLORS.len()], format!("[{:<width$}]", s.name));
        let mut c = shell(&s.cmd);
        c.current_dir(&s.dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if !json {
            c.env("FORCE_COLOR", "1");
        }
        // flutter reads r / R / q from the keyboard; only one service can own it.
        if json {
            c.stdin(Stdio::piped());
        } else if s.pidfile.is_some() && !stdin_taken {
            c.stdin(Stdio::inherit());
            stdin_taken = true;
        } else {
            c.stdin(Stdio::null());
        }
        say!("{prefix} {}", util::dim(format!("$ {}", s.cmd)));
        let mut child = match c.spawn() {
            Ok(ch) => ch,
            Err(e) => {
                say!("{prefix} {}", util::red(format!("failed to start: {e}")));
                continue;
            }
        };
        ui::event(
            "started",
            serde_json::json!({ "repo": s.name, "cmd": s.cmd, "pid": child.id(), "path": s.dir }),
        );
        if let Some(stdin) = child.stdin.take() {
            inputs.lock().unwrap().insert(s.name.clone(), stdin);
        }
        let streams: [Option<(&'static str, Box<dyn Read + Send>)>; 2] = [
            child
                .stdout
                .take()
                .map(|s| ("stdout", Box::new(s) as Box<dyn Read + Send>)),
            child
                .stderr
                .take()
                .map(|s| ("stderr", Box::new(s) as Box<dyn Read + Send>)),
        ];
        for (kind, stream) in streams.into_iter().flatten() {
            let (prefix, name) = (prefix.clone(), s.name.clone());
            std::thread::spawn(move || pipe(stream, &prefix, &name, kind));
        }
        if let Some(url) = s.ready.clone() {
            let (prefix, stop, name) = (prefix.clone(), stop.clone(), s.name.clone());
            std::thread::spawn(move || wait_ready(&url, &prefix, &name, &stop));
        }
        if let Some(pf) = &s.pidfile {
            match flutter::watch(&s.dir.join("lib"), pf) {
                Some(w) => {
                    say!(
                        "{prefix} {}",
                        util::dim("hot reload on save: watching lib/ (r / R / q also work)")
                    );
                    watchers.push(w);
                }
                None => say!("{prefix} {}", util::dim("press r to hot reload")),
            }
        }
        children.push((prefix, s.name.clone(), child));
    }
    if json {
        let (inputs, stop) = (inputs.clone(), stop.clone());
        std::thread::spawn(move || read_commands(&inputs, default_input.as_deref(), &stop));
    }

    let mut done = vec![false; children.len()];
    let mut stop_at: Option<Instant> = None;
    let mut code = 0;
    loop {
        for (i, (prefix, name, ch)) in children.iter_mut().enumerate() {
            if !done[i]
                && let Ok(Some(st)) = ch.try_wait()
            {
                done[i] = true;
                if !st.success() && !stop.load(Ordering::SeqCst) {
                    code = 1;
                }
                ui::event(
                    "exited",
                    serde_json::json!({ "repo": name, "code": st.code(), "success": st.success() }),
                );
                say!("{prefix} {}", util::dim(format!("exited ({st})")));
            }
        }
        if done.iter().all(|d| *d) {
            break;
        }
        if stop.load(Ordering::SeqCst) {
            if stop_at.is_none() && json {
                // No shared terminal sent Ctrl-C: interrupt the services ourselves.
                inputs.lock().unwrap().clear();
                for (i, (_, _, ch)) in children.iter_mut().enumerate() {
                    if !done[i] {
                        interrupt(ch);
                    }
                }
            }
            let since = *stop_at.get_or_insert_with(Instant::now);
            if since.elapsed() > Duration::from_secs(5) {
                for (i, (_, _, ch)) in children.iter_mut().enumerate() {
                    if !done[i] {
                        kill_tree(ch);
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    drop(watchers);
    Ok(code)
}

/// JSON mode: `{"type":"input","repo":"app","text":"r"}` types into a service (default: the
/// flutter one), `{"type":"stop"}` stops everything.
fn read_commands(
    inputs: &Mutex<HashMap<String, ChildStdin>>,
    default: Option<&str>,
    stop: &AtomicBool,
) {
    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        line.clear();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            return; // closed: keep running, Ctrl-C / SIGTERM still work
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            ui::emit(
                "error",
                serde_json::json!({ "message": "expected a JSON object" }),
            );
            continue;
        };
        match v["type"].as_str() {
            Some("stop") => {
                stop.store(true, Ordering::SeqCst);
                return;
            }
            Some("input") => {
                let Some(repo) = v["repo"].as_str().or(default) else {
                    ui::emit(
                        "error",
                        serde_json::json!({ "message": "input needs \"repo\"" }),
                    );
                    continue;
                };
                let text = v["text"].as_str().unwrap_or("");
                let mut map = inputs.lock().unwrap();
                match map.get_mut(repo) {
                    Some(w) => {
                        let _ = w.write_all(text.as_bytes()).and_then(|_| w.flush());
                    }
                    None => ui::emit(
                        "error",
                        serde_json::json!({ "message": format!("no running service \"{repo}\"") }),
                    ),
                }
            }
            _ => ui::emit(
                "error",
                serde_json::json!({ "message": "unknown message; expected type input or stop" }),
            ),
        }
    }
}

fn interrupt(ch: &mut Child) {
    #[cfg(unix)]
    // SAFETY: plain kill(2) on our own child's pid.
    unsafe {
        libc::kill(ch.id() as libc::pid_t, libc::SIGINT);
    }
    #[cfg(windows)]
    kill_tree(ch);
}

fn pipe(stream: Box<dyn Read + Send>, prefix: &str, name: &str, kind: &str) {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf);
                if ui::json() {
                    let text = ui::strip_ansi(line.trim_end_matches(['\n', '\r']));
                    ui::emit(
                        "output",
                        serde_json::json!({ "repo": name, "stream": kind, "line": text }),
                    );
                    continue;
                }
                let mut out = std::io::stdout().lock();
                let _ = writeln!(out, "{prefix} {}", line.trim_end_matches(['\n', '\r']));
            }
        }
    }
}

fn kill_tree(ch: &mut Child) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &ch.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = ch.kill();
}

fn host_port(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let hp = rest.split('/').next()?;
    Some(if hp.contains(':') {
        hp.to_string()
    } else if url.starts_with("https") {
        format!("{hp}:443")
    } else {
        format!("{hp}:80")
    })
}

fn reachable(addr: &str, timeout: Duration) -> bool {
    addr.to_socket_addrs()
        .map(|mut a| a.any(|a| TcpStream::connect_timeout(&a, timeout).is_ok()))
        .unwrap_or(false)
}

fn wait_ready(url: &str, prefix: &str, name: &str, stop: &AtomicBool) {
    let Some(addr) = host_port(url) else { return };
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(180) && !stop.load(Ordering::SeqCst) {
        if reachable(&addr, Duration::from_millis(300)) {
            ui::event("ready", serde_json::json!({ "repo": name, "url": url }));
            say!("{prefix} {}", util::green(format!("ready: {url}")));
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn port_in_use(port: u16) -> bool {
    reachable(&format!("localhost:{port}"), Duration::from_millis(200))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_urls() {
        assert_eq!(
            host_port("http://localhost:3000/health").as_deref(),
            Some("localhost:3000")
        );
        assert_eq!(
            host_port("https://example.com").as_deref(),
            Some("example.com:443")
        );
    }
}
