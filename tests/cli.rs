//! End-to-end tests: real git repos in a temp folder, the real `kex` binary.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

struct Env {
    tmp: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Env {
            tmp: tempfile::tempdir().unwrap(),
        }
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn cmd(&self, dir: &Path, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_kex"));
        c.args(args)
            .current_dir(dir)
            .env("KEX_HOME", self.root().join("kexhome"))
            .env("KEX_NO_UPDATE_CHECK", "1")
            .env("NO_COLOR", "1");
        isolate_git(&mut c);
        c
    }

    fn kex(&self, dir: &Path, args: &[&str]) -> Output {
        let out = self.cmd(dir, args).output().unwrap();
        if args.contains(&"--json") {
            conform(&stdout(&out));
        }
        out
    }

    fn kex_ok(&self, dir: &Path, args: &[&str]) -> String {
        let out = self.kex(dir, args);
        assert!(
            out.status.success(),
            "kex {args:?} failed:\n{}{}",
            stdout(&out),
            stderr(&out)
        );
        stdout(&out)
    }

    /// A repo with one commit on `main`.
    fn repo(&self, rel: &str) -> PathBuf {
        let d = self.root().join(rel);
        fs::create_dir_all(&d).unwrap();
        git(&d, &["init", "-q", "-b", "main"]);
        commit(&d, "README.md", rel);
        d
    }
}

/// An empty global git config, so the developer's own settings don't leak into tests.
fn empty_gitconfig() -> &'static Path {
    static PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let p = std::env::temp_dir().join("kex-tests-empty-gitconfig");
        fs::write(&p, "").unwrap();
        p
    })
}

fn isolate_git(c: &mut Command) {
    c.env("GIT_CONFIG_GLOBAL", empty_gitconfig())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Jane Doe")
        .env("GIT_AUTHOR_EMAIL", "jane@example.com")
        .env("GIT_COMMITTER_NAME", "Jane Doe")
        .env("GIT_COMMITTER_EMAIL", "jane@example.com");
}

fn git(dir: &Path, args: &[&str]) {
    let mut c = Command::new("git");
    c.args(args).current_dir(dir);
    isolate_git(&mut c);
    let out = c.output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", stderr(&out));
}

fn commit(dir: &Path, file: &str, content: &str) {
    fs::write(dir.join(file), content).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", &format!("update {file}")]);
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// `--json` prints one event per line; every line must be JSON, and the stream must conform
/// to cli-events/1 ([`conform`]).
fn events(text: &str) -> Vec<serde_json::Value> {
    conform(text)
}

// ── cli-events/1 conformance (spec/cli-events-1.md §13) ─────────────────────────────────

/// The schema, compiled once.
fn validator() -> &'static jsonschema::Validator {
    static V: std::sync::OnceLock<jsonschema::Validator> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../spec/cli-events-1.schema.json")).unwrap();
        jsonschema::validator_for(&schema).unwrap()
    })
}

/// `kex manifest --json`, run once: (hello, manifest).
fn manifest() -> &'static (serde_json::Value, serde_json::Value) {
    static M: std::sync::OnceLock<(serde_json::Value, serde_json::Value)> =
        std::sync::OnceLock::new();
    M.get_or_init(|| {
        let tmp = tempfile::tempdir().unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_kex"))
            .args(["manifest", "--json"])
            .current_dir(tmp.path())
            .env("KEX_HOME", tmp.path().join("kexhome"))
            .env("KEX_NO_UPDATE_CHECK", "1")
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", stderr(&out));
        let lines: Vec<serde_json::Value> = stdout(&out)
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        for l in &lines {
            assert_valid(l);
        }
        let m = lines
            .iter()
            .find(|l| l["type"] == "manifest")
            .expect("manifest line");
        (lines[0].clone(), m["manifest"].clone())
    })
}

fn assert_valid(line: &serde_json::Value) {
    let errors: Vec<String> = validator()
        .iter_errors(line)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "line breaks cli-events/1: {errors:#?}\n{line}"
    );
}

/// RFC 8785 canonical JSON (keys sorted by UTF-16 units), as the manifest hash needs.
fn canonical(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            let parts: Vec<String> = keys
                .iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap(),
                        canonical(&o[k.as_str()])
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        serde_json::Value::Array(a) => {
            format!(
                "[{}]",
                a.iter().map(canonical).collect::<Vec<_>>().join(",")
            )
        }
        other => other.to_string(),
    }
}

fn manifest_hash(m: &serde_json::Value) -> String {
    use sha2::Digest;
    let d = sha2::Sha256::digest(canonical(m).as_bytes());
    d.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// `{name}` placeholders of a template.
fn placeholders(t: &str) -> Vec<String> {
    t.split('{')
        .skip(1)
        .filter_map(|s| s.split_once('}').map(|(n, _)| n.to_string()))
        .collect()
}

fn render(t: &str, params: &serde_json::Value) -> String {
    let mut out = t.to_string();
    // A `{…}` that names no param is literal text (as kex renders it).
    for name in placeholders(t) {
        if params.get(&name).is_none() {
            continue;
        }
        let v = match &params[&name] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Null => String::new(),
            other => other.to_string(),
        };
        out = out.replacen(&format!("{{{name}}}"), &v, 1);
    }
    out
}

/// Every message id is in the manifest; its params are placeholders of the template; and
/// the English next to it is the template filled in.
fn check_messages(v: &serde_json::Value, messages: &serde_json::Value) {
    match v {
        serde_json::Value::Object(o) => {
            if let Some(id) = o.get("msg").and_then(|m| m.as_str()) {
                let t = messages[id]
                    .as_str()
                    .unwrap_or_else(|| panic!("msg {id} isn't in the manifest's messages"));
                let names = placeholders(t);
                let params = o.get("params").cloned().unwrap_or(serde_json::Value::Null);
                if let Some(p) = params.as_object() {
                    for k in p.keys() {
                        assert!(names.contains(k), "{id}: param {k} isn't in \"{t}\"");
                    }
                }
                let main = ["text", "message", "label"]
                    .iter()
                    .find_map(|k| o.get(*k).and_then(|x| x.as_str()));
                if let Some(main) = main {
                    assert_eq!(main, render(t, &params), "{id}: text isn't its template");
                }
            }
            o.values().for_each(|x| check_messages(x, messages));
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| check_messages(x, messages)),
        _ => {}
    }
}

/// cli-events §5.4: a RunAction must name a command of the manifest that isn't
/// needs_terminal, with valid options, stdin only for its JSON input (valid against its
/// schema), no cwd, and a confirm unless it's read-only navigation.
fn check_run(run: &serde_json::Value, action: Option<&serde_json::Value>, navigation: bool) {
    let m = &manifest().1;
    let id = run["command"].as_str().unwrap();
    let cmd = m["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("action runs unknown command {id}"));
    assert_ne!(
        cmd["needs_terminal"], true,
        "action runs a terminal command"
    );
    assert!(run.get("cwd").is_none(), "kex never sets cwd: {run}");
    let opts = cmd["options"].as_array().unwrap();
    for (k, v) in run["options"].as_object().into_iter().flatten() {
        let o = opts
            .iter()
            .find(|o| o["id"] == k.as_str())
            .unwrap_or_else(|| panic!("{id} has no option {k}"));
        let ok = match o["type"].as_str().unwrap() {
            "boolean" => v.is_boolean(),
            "integer" => v
                .as_u64()
                .is_some_and(|n| o["max"].as_u64().is_none_or(|max| n <= max)),
            "list" => v.is_array() || v.is_string(),
            _ => v.is_string(),
        };
        assert!(ok, "{id} --{k}: {v} isn't a {}", o["type"]);
    }
    if let Some(stdin) = run.get("stdin") {
        let ji = &cmd["json_input"];
        assert!(ji.is_object(), "{id} takes no stdin");
        let opt = ji["option"].as_str().unwrap();
        let stdin_value = opts.iter().find(|o| o["id"] == opt).unwrap()["stdin_value"].clone();
        assert_eq!(run["options"][opt], stdin_value);
        let v = jsonschema::validator_for(&ji["schema"]).unwrap();
        let errors: Vec<String> = v.iter_errors(stdin).map(|e| e.to_string()).collect();
        assert!(
            errors.is_empty(),
            "stdin breaks the input schema: {errors:?}\n{stdin}"
        );
        assert!(stdin.to_string().len() <= 1024 * 1024);
    }
    let read_only = cmd["read_only"] == true;
    if !(navigation && read_only) {
        let a = action.expect("a non-navigation RunAction is an action");
        assert!(a.get("confirm").is_some(), "RunAction without confirm: {a}");
    }
}

/// Every cell is under a column: a table's / cards' rows, a tree's nodes, a prompt's
/// options (cli-events §5.1, §6).
fn check_cells(v: &serde_json::Value) {
    fn keys(columns: &serde_json::Value) -> Vec<String> {
        columns
            .as_array()
            .map(|c| {
                c.iter()
                    .map(|c| c["key"].as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }
    fn cells_ok(cells: &serde_json::Value, cols: &[String], what: &str) {
        for k in cells.as_object().into_iter().flatten().map(|(k, _)| k) {
            assert!(
                cols.contains(k),
                "{what}: cell {k} has no column (columns: {cols:?})"
            );
        }
    }
    fn nodes(ns: &serde_json::Value, cols: &[String]) {
        for n in ns.as_array().into_iter().flatten() {
            if let Some(c) = n.get("cells") {
                cells_ok(c, cols, "tree node");
            }
            nodes(&n["children"], cols);
        }
    }
    match v {
        serde_json::Value::Object(o) => {
            let cols = keys(&v["columns"]);
            let appending = v["mode"] == "append" && v.get("columns").is_none();
            if !appending {
                for r in v["rows"].as_array().into_iter().flatten() {
                    cells_ok(&r["cells"], &cols, "row");
                }
            }
            nodes(&v["nodes"], &cols);
            if v["type"] == "prompt" {
                for opt in v["options"].as_array().into_iter().flatten() {
                    if let Some(c) = opt.get("view").map(|w| &w["cells"]) {
                        assert!(
                            !cols.is_empty(),
                            "an option with a view in a prompt without columns"
                        );
                        cells_ok(c, &cols, "prompt option");
                    }
                }
            }
            o.values().for_each(check_cells);
        }
        serde_json::Value::Array(a) => a.iter().for_each(check_cells),
        _ => {}
    }
}

fn check_actions(v: &serde_json::Value) {
    match v {
        serde_json::Value::Object(o) => {
            for (k, x) in o {
                match k.as_str() {
                    "actions" => {
                        for a in x.as_array().unwrap() {
                            if let Some(run) = a.get("run") {
                                check_run(run, Some(a), false);
                            }
                        }
                    }
                    "up" | "next" if x.get("run").is_some() => check_run(&x["run"], Some(x), true),
                    "open" if x.get("option").is_some() => check_run(&x["run"], None, true),
                    _ => {}
                }
                check_actions(x);
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(check_actions),
        _ => {}
    }
}

/// Parse a `--json` stdout and check it against cli-events/1.
fn conform(text: &str) -> Vec<serde_json::Value> {
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| {
            assert!(l.len() <= 1024 * 1024, "line over 1 MiB");
            serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON ({e}): {l}"))
        })
        .collect();
    if lines.is_empty() {
        return lines;
    }
    let (hello, m) = manifest();
    let standard = [
        "hello", "manifest", "prompt", "log", "error", "result", "exit", "answer",
    ];
    let types: Vec<&str> = m["event_types"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["type"].as_str().unwrap())
        .collect();
    assert_eq!(lines[0]["type"], "hello", "hello first");
    assert_eq!(lines[0]["manifest_hash"], hello["manifest_hash"]);
    assert_eq!(lines[0]["standard"], "cli-events/1");
    for (i, l) in lines.iter().enumerate() {
        assert_valid(l);
        let t = l["type"].as_str().unwrap();
        assert!(
            standard.contains(&t) || types.contains(&t),
            "{t} isn't in event_types"
        );
        assert!(i == 0 || t != "hello", "hello twice");
        if t == "exit" {
            assert_eq!(i, lines.len() - 1, "exit last");
            let code = l["code"].as_u64().unwrap();
            if let Some(e) = m["exit_codes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["code"] == code)
            {
                assert_eq!(l["tone"], e["tone"]);
            }
        }
        check_messages(l, &m["messages"]);
        check_actions(l);
        check_cells(l);
    }
    assert_eq!(lines.last().unwrap()["type"], "exit", "exit last");
    lines
}

fn status_json(env: &Env, dir: &Path) -> serde_json::Value {
    events(&env.kex_ok(dir, &["status", "--json"]))
        .into_iter()
        .find(|e| e["type"] == "status")
        .expect("a status event")
}

fn repo_names(v: &serde_json::Value) -> Vec<String> {
    v["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["repo"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn not_a_kex_repo() {
    let env = Env::new();
    let api = env.repo("shop/api");
    let out = env.kex(&api, &["doctor"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("not a kex repo"), "{}", stderr(&out));
}

#[test]
fn both_config_files_is_an_error() {
    let env = Env::new();
    let api = env.repo("shop/api");
    env.kex_ok(&api, &["init", "--yes", "--repo", "api"]);
    fs::create_dir_all(api.join(".config")).unwrap();
    fs::copy(api.join("kex.json"), api.join(".config/kex.json")).unwrap();
    let out = env.kex(&api, &["doctor"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("keep only one"));
}

#[test]
fn init_joins_the_project_next_to_it() {
    let env = Env::new();
    let api = env.repo("shop/api");
    let web = env.repo("shop/web");
    env.kex_ok(&api, &["init", "--yes", "--repo", "api"]);
    env.kex_ok(&web, &["init", "--yes", "--repo", "web"]);

    let read = |d: &Path| -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(d.join("kex.json")).unwrap()).unwrap()
    };
    let (a, w) = (read(&api), read(&web));
    assert_eq!(a["project"], w["project"]);
    assert_ne!(a["repo_id"], w["repo_id"]);
    assert_eq!(a["version"], 1);
    assert!(
        fs::read_to_string(api.join(".gitignore"))
            .unwrap()
            .contains("kex.local.json")
    );

    // Found from either side, on any layout.
    assert_eq!(repo_names(&status_json(&env, &web)), ["web", "api"]);
    assert_eq!(repo_names(&status_json(&env, &api)), ["api", "web"]);
}

#[test]
fn finds_repos_in_different_folders_through_the_registry() {
    let env = Env::new();
    let api = env.repo("work/clients/acme/api");
    let web = env.repo("elsewhere/web");
    env.kex_ok(&api, &["init", "--yes", "--repo", "api"]);
    let project = serde_json::from_str::<serde_json::Value>(
        &fs::read_to_string(api.join("kex.json")).unwrap(),
    )
    .unwrap()["project"]
        .as_str()
        .unwrap()
        .to_string();
    env.kex_ok(
        &web,
        &["init", "--yes", "--repo", "web", "--project", &project],
    );
    assert_eq!(repo_names(&status_json(&env, &web)), ["web", "api"]);
}

#[test]
fn worktrees_are_ignored_except_the_current_one() {
    let env = Env::new();
    let api = env.repo("shop/api");
    let web = env.repo("shop/web");
    env.kex_ok(&api, &["init", "--yes", "--repo", "api"]);
    env.kex_ok(&web, &["init", "--yes", "--repo", "web"]);
    git(&api, &["add", "."]);
    git(&api, &["commit", "-qm", "add kex"]);
    let wt = env.root().join("shop/api-agent");
    git(
        &api,
        &["worktree", "add", "-q", "-b", "agent", wt.to_str().unwrap()],
    );

    // From web: the worktree next to api isn't a second api.
    let v = status_json(&env, &web);
    assert_eq!(repo_names(&v), ["web", "api"]);

    // From inside the worktree: it replaces api for this command.
    let v = status_json(&env, &wt);
    assert_eq!(repo_names(&v), ["api", "web"]);
    assert_eq!(v["repos"][0]["worktree"], true);
    assert_eq!(v["repos"][0]["branch"], "agent");
}

#[test]
fn copied_config_is_reported() {
    let env = Env::new();
    let api = env.repo("shop/api");
    env.kex_ok(&api, &["init", "--yes", "--repo", "api"]);
    let copy = env.repo("shop/new-api");
    fs::copy(api.join("kex.json"), copy.join("kex.json")).unwrap();
    let out = env.kex(&api, &["doctor"]);
    assert!(
        stderr(&out).contains("copied kex config"),
        "{}",
        stderr(&out)
    );

    // `init --reset` gives the copy its own identity.
    env.kex_ok(&copy, &["init", "--reset", "--repo", "new-api"]);
    let out = env.kex(&api, &["doctor"]);
    assert!(
        !stderr(&out).contains("copied kex config"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn pull_fast_forwards_and_reports_unmerged_branches() {
    let env = Env::new();
    let origin = env.root().join("origin.git");
    fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "-q", "--bare", "-b", "main"]);

    let api = env.repo("shop/api");
    git(&api, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&api, &["push", "-q", "-u", "origin", "main"]);
    git(&api, &["switch", "-q", "-c", "dev"]);
    git(&api, &["push", "-q", "-u", "origin", "dev"]);
    env.kex_ok(&api, &["init", "--yes", "--repo", "api"]);

    // A teammate pushes to dev and opens a feature branch.
    let mate = env.root().join("mate");
    git(
        env.root(),
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            mate.to_str().unwrap(),
        ],
    );
    git(&mate, &["switch", "-q", "dev"]);
    commit(&mate, "dev.txt", "dev work");
    git(&mate, &["push", "-q", "origin", "dev"]);
    git(&mate, &["switch", "-q", "-c", "feature/login"]);
    commit(&mate, "login.txt", "login");
    git(&mate, &["push", "-q", "origin", "feature/login"]);

    let out = env.kex_ok(&api, &["pull"]);
    assert!(out.contains("↓ 1 commit pulled"), "{out}");
    assert!(out.contains("feature/login"), "{out}");
    assert!(out.contains("→ dev"), "{out}");
    assert!(api.join("dev.txt").exists());

    // Once merged into dev, it's no longer listed.
    git(&mate, &["switch", "-q", "dev"]);
    git(&mate, &["merge", "-q", "--ff-only", "feature/login"]);
    git(&mate, &["push", "-q", "origin", "dev"]);
    let out = env.kex_ok(&api, &["pull"]);
    assert!(!out.contains("feature/login"), "{out}");
}

#[test]
fn pull_skips_dirty_repos() {
    let env = Env::new();
    let api = env.repo("shop/api");
    env.kex_ok(&api, &["init", "--yes", "--repo", "api"]);
    fs::write(api.join("README.md"), "wip").unwrap();
    let out = env.kex_ok(&api, &["pull"]);
    assert!(out.contains("uncommitted changes, pull skipped"), "{out}");
}

#[test]
fn run_detects_node_and_runs_custom_commands() {
    let env = Env::new();
    let web = env.repo("shop/web");
    let pkg = r#"{"name":"web","scripts":{"start":"echo started"}}"#;
    fs::write(web.join("package.json"), pkg).unwrap();
    fs::write(web.join("package-lock.json"), "{}").unwrap();
    env.kex_ok(&web, &["init", "--yes", "--repo", "web"]);

    let api = env.repo("shop/api");
    env.kex_ok(&api, &["init", "--yes", "--repo", "api"]);
    let mut c: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(api.join("kex.json")).unwrap()).unwrap();
    c["run"] = serde_json::json!({ "dev": "echo hello-from-api" });
    fs::write(api.join("kex.json"), c.to_string()).unwrap();

    let tools = env.repo("shop/tools");
    env.kex_ok(&tools, &["init", "--yes", "--repo", "tools"]);

    let out = env.kex(&api, &["run", "--no-update", "--repos", "api"]);
    let text = stdout(&out);
    assert!(out.status.success(), "{text}{}", stderr(&out));
    assert!(text.contains("hello-from-api"), "{text}");

    // Without --repos (and no terminal), everything runnable starts, with what was detected.
    let text = stdout(&env.kex(&api, &["run", "--no-update"]));
    assert!(text.contains("node · npm run start"), "{text}");
    assert!(
        text.contains("package-lock.json, script \"start\""),
        "{text}"
    );
    assert!(text.contains("don't know how to run it"), "{text}");
}

#[test]
#[cfg(unix)]
fn pr_check_without_gh_explains_how_to_install() {
    let env = Env::new();
    let api = env.repo("shop/api");
    env.kex_ok(&api, &["init", "--yes", "--repo", "api"]);
    let mut c = Command::new(env!("CARGO_BIN_EXE_kex"));
    c.args(["pr", "check"])
        .current_dir(&api)
        .env("KEX_HOME", env.root().join("kexhome"))
        .env("PATH", "/usr/bin:/bin");
    let out = c.output().unwrap();
    if Path::new("/usr/bin/gh").exists() {
        return; // gh installed system-wide; nothing to test here.
    }
    assert_eq!(out.status.code(), Some(3));
    assert!(stderr(&out).contains("gh auth login"), "{}", stderr(&out));
    assert!(stderr(&out).contains("https://cli.github.com/"));
}

#[test]
fn json_mode_streams_events_and_takes_answers_on_stdin() {
    let env = Env::new();
    let api = env.repo("p/api");
    let web = env.repo("p/web");
    env.kex_ok(&api, &["init", "--yes", "--repo", "api"]);
    env.kex_ok(&web, &["init", "--yes", "--repo", "web"]);
    for (dir, word) in [(&api, "hello-api"), (&web, "hello-web")] {
        let mut c: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("kex.json")).unwrap()).unwrap();
        c["run"] = serde_json::json!({ "dev": format!("echo {word}") });
        fs::write(dir.join("kex.json"), c.to_string()).unwrap();
    }

    // Pull: structured per-repo events, nothing but JSON on stdout.
    let ev = events(&env.kex_ok(&api, &["pull", "--json"]));
    assert_eq!(ev[0]["type"], "hello");
    assert!(ev.iter().any(|e| e["type"] == "project"));
    assert_eq!(ev.iter().filter(|e| e["type"] == "pull").count(), 2);
    assert_eq!(ev.last().unwrap()["type"], "exit");

    // Run: the repo question is a prompt; answer it by value.
    let mut c = env.cmd(&api, &["run", "--json", "--no-update"]);
    c.stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut child = c.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"answer\":[\"web\"]}\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let ev = events(&stdout(&out));
    let prompt = ev.iter().find(|e| e["type"] == "prompt").expect("prompt");
    assert_eq!(prompt["kind"], "multiselect");
    assert_eq!(prompt["options"].as_array().unwrap().len(), 2);
    let lines: Vec<&serde_json::Value> = ev.iter().filter(|e| e["type"] == "output").collect();
    assert!(
        lines
            .iter()
            .any(|e| e["repo"] == "web" && e["line"] == "hello-web")
    );
    assert!(!lines.iter().any(|e| e["repo"] == "api"));
    assert!(
        ev.iter()
            .any(|e| e["type"] == "exited" && e["repo"] == "web")
    );

    // Cancelling a prompt exits with 130.
    let mut c = env.cmd(&api, &["run", "--json", "--no-update"]);
    c.stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut child = c.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"cancel\":true}\n")
        .unwrap();
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(130));
}

fn read_config(dir: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(dir.join("kex.json")).unwrap()).unwrap()
}

fn peer_names(c: &serde_json::Value) -> Vec<String> {
    c["peers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["repo"].as_str().unwrap().to_string())
        .collect()
}

/// A repo with an `origin` remote, so it can be listed in `peers`.
fn repo_with_remote(env: &Env, rel: &str) -> PathBuf {
    let d = env.repo(rel);
    let name = Path::new(rel).file_name().unwrap().to_str().unwrap();
    let url = format!("https://example.com/acme/{name}.git");
    git(&d, &["remote", "add", "origin", &url]);
    d
}

fn project_of(dir: &Path) -> String {
    read_config(dir)["project"].as_str().unwrap().to_string()
}

fn last_name(v: &serde_json::Value) -> String {
    Path::new(v.as_str().unwrap())
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn added_names(ev: &[serde_json::Value]) -> Vec<String> {
    find_event(ev, "added")["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["repo"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn init_repos_adds_folders_to_this_repos_project() {
    let env = Env::new();
    let web = repo_with_remote(&env, "shop/web");
    let api = repo_with_remote(&env, "shop/api");
    let app = repo_with_remote(&env, "shop/app");
    fs::create_dir_all(app.join(".config")).unwrap();
    env.kex_ok(&web, &["init", "--yes", "--repo", "web"]);

    // From inside a kex repo: its project; `kex add` is the same command.
    env.kex_ok(&web, &["add", "--repos", "api,app"]);
    let (w, a, p) = (
        read_config(&web),
        read_config(&api),
        read_config(&app.join(".config")),
    );
    assert!(!app.join("kex.json").exists());
    assert_eq!(a["project"], w["project"]);
    assert_eq!(p["project"], w["project"]);
    assert_eq!(
        (a["repo"].as_str(), p["repo"].as_str()),
        (Some("api"), Some("app"))
    );
    assert_ne!(a["repo_id"], p["repo_id"]);
    // Each new config lists the existing repo and the other new one.
    assert_eq!(peer_names(&a), ["web", "app"]);
    assert_eq!(peer_names(&p), ["web", "api"]);
    assert!(
        fs::read_to_string(app.join(".gitignore"))
            .unwrap()
            .contains(".config/kex.local.json")
    );
    assert_eq!(repo_names(&status_json(&env, &web)), ["web", "api", "app"]);

    // Again: everything is already there.
    let ev = events(&env.kex_ok(&api, &["init", "--repos", "api,app,web", "--json"]));
    let added = find_event(&ev, "added");
    assert_eq!(added["repos"].as_array().unwrap().len(), 0);
    let skipped = added["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 3);
    assert!(
        skipped
            .iter()
            .all(|s| s["reason"] == "already_kex" && s["in_project"] == true)
    );

    // --project that isn't this repo's project.
    let out = env.kex(
        &web,
        &["init", "--repos", "api", "--project", "other-123456"],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("this repo is in"), "{}", stderr(&out));
}

#[test]
fn init_json_asks_the_project_and_the_name_of_this_repo() {
    let env = Env::new();
    let api = env.repo("shop/api");
    let web = env.repo("shop/web");

    // No project around: straight to a new project's name, then the repo name. An answer
    // breaking the rules is an error for that prompt, which stays open.
    let input = "{\"type\":\"answer\",\"id\":\"p1\",\"answer\":\"Shop\"}\n\
                 {\"type\":\"answer\",\"id\":\"p2\",\"answer\":\"Bad Name\"}\n\
                 {\"type\":\"answer\",\"id\":\"p2\",\"answer\":\"backend\"}\n";
    let (ev, code) = kex_stdin(&env, &api, &["init", "--json"], input);
    assert_eq!(code, Some(0), "{ev:?}");
    let prompts: Vec<&serde_json::Value> = ev.iter().filter(|e| e["type"] == "prompt").collect();
    assert_eq!(prompts.len(), 2, "{prompts:?}");
    assert_eq!(prompts[0]["msg"], "init.new_project_name");
    assert_eq!(prompts[1]["msg"], "init.repo_name");
    assert_eq!(prompts[1]["message"], "Repo name for api:");
    assert_eq!(prompts[1]["default"], "api");
    assert_eq!(prompts[1]["pattern"], "^[a-z0-9][a-z0-9-]*$");
    let rejected = ev
        .iter()
        .find(|e| e["type"] == "error" && e["id"] == "p2")
        .expect("the bad name is an error for p2");
    assert!(rejected["message"].as_str().unwrap().contains("repo name"));
    let c = read_config(&api);
    assert_eq!(c["repo"], "backend");
    assert!(c["project"].as_str().unwrap().starts_with("shop-"));
    let added = find_event(&ev, "added");
    assert_eq!(added["view"]["kind"], "table");

    // With a project next to it: "Project" (join it, or new), then the name (null = default).
    let input = "{\"type\":\"answer\",\"id\":\"p1\",\"answer\":0}\n{\"type\":\"answer\",\"id\":\"p2\",\"answer\":null}\n";
    let (ev, code) = kex_stdin(&env, &web, &["add", "--json"], input);
    assert_eq!(code, Some(0), "{ev:?}");
    let p1 = find_event(&ev, "prompt");
    assert_eq!(p1["kind"], "select");
    assert_eq!(p1["msg"], "init.project");
    assert_eq!(p1["options"][0]["value"], c["project"]);
    assert_eq!(
        p1["options"][0]["data"]["repos"],
        serde_json::json!(["backend"])
    );
    assert_eq!(p1["options"][1]["value"], "new");
    assert!(p1["columns"].is_array());
    assert_eq!(read_config(&web)["project"], c["project"]);
    assert_eq!(read_config(&web)["repo"], "web");

    // Already set up: says so, exit 0; outside a repo: an error.
    let (ev, code) = kex_stdin(&env, &web, &["init", "--json"], "");
    assert_eq!(code, Some(0));
    assert!(!ev.iter().any(|e| e["type"] == "prompt"));
    let (_, code) = kex_stdin(&env, env.root(), &["init", "--json"], "");
    assert_eq!(code, Some(2));

    // Without a terminal (and not JSON), plain `kex init` sets up this repo, as in 0.1.
    let solo = env.repo("solo/one");
    env.kex_ok(&solo, &["init"]);
    assert_eq!(read_config(&solo)["repo"], "one");
    let out = env.kex(env.root(), &["init"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("not in a git repo"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn init_repos_checks_every_folder_before_writing() {
    let env = Env::new();
    let one = env.repo("shop/a/api");
    let two = env.repo("shop/b/api");
    let web = env.repo("shop/web");
    let shop = env.root().join("shop");

    // Two folders would both be named "api": nothing written.
    let out = env.kex(&shop, &["init", "--repos", "a/api,b/api"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("two repos would be named \"api\""),
        "{}",
        stderr(&out)
    );
    assert!(!one.join("kex.json").exists() && !two.join("kex.json").exists());

    // The same in JSON mode: error events with index and field, then exit 2.
    let ev = events(&stdout(
        &env.kex(&shop, &["init", "--repos", "a/api,nope", "--json"]),
    ));
    let e = find_event(&ev, "error");
    assert_eq!(
        (e["index"].as_u64(), e["field"].as_str()),
        (Some(1), Some("path"))
    );
    assert!(
        e["message"].as_str().unwrap().contains("doesn't exist"),
        "{e}"
    );
    assert_eq!(ev.last().unwrap()["code"], 2);

    // Relative to DIR or absolute; one project for all.
    let one_path = one.display().to_string();
    env.kex_ok(&shop, &["init", "--repos", &format!("web,{one_path}")]);
    assert_eq!(project_of(&web), project_of(&one));
    assert!(!two.join("kex.json").exists());

    // The second api can't take a name that's already in the project.
    let out = env.kex(&web, &["init", "--repos", two.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("already in"), "{}", stderr(&out));
}

#[test]
fn init_yes_with_a_dir_sets_up_that_repo_in_the_project_next_to_it() {
    let env = Env::new();
    let admin = env.repo("shop/admin");
    env.kex_ok(&admin, &["init", "--yes", "--repo", "admin"]);
    let api = env.repo("shop/api");
    let web = env.repo("shop/web");
    let ev = events(&env.kex_ok(env.root(), &["init", "--yes", "--json", "shop/web"]));
    assert_eq!(added_names(&ev), ["web"]);
    assert_eq!(project_of(&web), project_of(&admin));
    assert!(!api.join("kex.json").exists() && !api.join(".gitignore").exists());

    // A folder that isn't a repo: --yes has nothing to set up.
    fs::create_dir_all(env.root().join("plain")).unwrap();
    let out = env.kex(env.root(), &["init", "--yes", "plain"]);
    assert_eq!(out.status.code(), Some(2));

    // A --project nobody uses yet is a new project with that id (as in 0.1).
    env.kex_ok(&api, &["init", "--yes", "--project", "billing-123456"]);
    assert_eq!(project_of(&api), "billing-123456");
}

#[test]
#[cfg(unix)]
fn a_symlinked_repo_is_named_after_the_link() {
    let env = Env::new();
    let ext = env.repo("elsewhere/ext");
    let shop = env.root().join("shop");
    fs::create_dir_all(&shop).unwrap();
    std::os::unix::fs::symlink(&ext, shop.join("linked")).unwrap();
    let ev = events(&env.kex_ok(&shop, &["init", "--repos", "linked", "--json"]));
    assert_eq!(added_names(&ev), ["linked"]);
    assert_eq!(
        last_name(&find_event(&ev, "added")["repos"][0]["path"]),
        "ext"
    ); // the real path
    assert_eq!(read_config(&ext)["repo"], "linked");
}

#[test]
#[cfg(unix)]
fn failed_setup_leaves_the_gitignore_as_it_was() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    let api = env.repo("shop/api"); // no .gitignore at all
    let web = env.repo("shop/web");
    fs::write(web.join(".gitignore"), "node_modules\n").unwrap();
    fs::create_dir_all(api.join(".config")).unwrap();
    fs::create_dir_all(web.join(".config")).unwrap();
    fs::set_permissions(web.join(".config"), fs::Permissions::from_mode(0o555)).unwrap();
    let shop = env.root().join("shop");
    let out = env.kex(&shop, &["init", "--repos", "api,web", "--json"]);
    fs::set_permissions(web.join(".config"), fs::Permissions::from_mode(0o755)).unwrap();
    if web.join(".config/kex.json").exists() {
        return; // running as root: permissions aren't enforced, nothing to test
    }
    // api was written first, then undone when web failed: all or nothing.
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    let ev = events(&stdout(&out));
    let added = find_event(&ev, "added");
    assert_eq!(added["repos"].as_array().unwrap().len(), 0);
    assert_eq!(added["rolled_back"], true);
    assert_eq!(added["failed"][0]["repo"], "web");
    assert!(
        added["failed"][0]["message"]
            .as_str()
            .unwrap()
            .contains("kex.json")
    );
    assert_eq!(
        fs::read_to_string(web.join(".gitignore")).unwrap(),
        "node_modules\n"
    );
    assert!(!api.join(".config/kex.json").exists() && !api.join(".gitignore").exists());

    // `kex init --yes` goes the same way.
    fs::set_permissions(web.join(".config"), fs::Permissions::from_mode(0o555)).unwrap();
    let out = env.kex(&web, &["init", "--yes", "--repo", "web"]);
    fs::set_permissions(web.join(".config"), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        fs::read_to_string(web.join(".gitignore")).unwrap(),
        "node_modules\n"
    );

    // Once fixed, a second run sets up both.
    env.kex_ok(&shop, &["init", "--repos", "api,web"]);
    assert!(api.join(".config/kex.json").exists() && web.join(".config/kex.json").exists());
}

/// Run kex with `--json`, feeding `input` on stdin; returns the events and the exit code.
fn kex_stdin(
    env: &Env,
    dir: &Path,
    args: &[&str],
    input: &str,
) -> (Vec<serde_json::Value>, Option<i32>) {
    let mut c = env.cmd(dir, args);
    c.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    // kex may stop reading early (a request over the limit): a broken pipe is fine.
    let _ = child.stdin.take().unwrap().write_all(input.as_bytes());
    let out = child.wait_with_output().unwrap();
    (events(&stdout(&out)), out.status.code())
}

fn find_event<'a>(ev: &'a [serde_json::Value], kind: &str) -> &'a serde_json::Value {
    ev.iter()
        .find(|e| e["type"] == kind)
        .unwrap_or_else(|| panic!("no {kind} event in {ev:?}"))
}

/// The entry named `name` anywhere in a nested tree.
fn tree_entry<'a>(tree: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    fn walk<'a>(es: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
        for e in es.as_array()? {
            if e["name"] == name {
                return Some(e);
            }
            if let Some(f) = walk(&e["children"], name) {
                return Some(f);
            }
        }
        None
    }
    walk(tree, name).unwrap_or_else(|| panic!("no {name} in {tree}"))
}

#[test]
fn browse_lists_a_tree_for_programs() {
    let env = Env::new();
    let api = repo_with_remote(&env, "shop/api");
    fs::write(api.join("package.json"), "{}").unwrap();
    fs::write(api.join("new.txt"), "x").unwrap();
    let admin = env.repo("shop/admin");
    env.kex_ok(&admin, &["init", "--yes", "--repo", "admin"]);
    env.repo("shop/clients/acme/portal");
    let wt = env.root().join("shop/api-wt");
    git(
        &api,
        &["worktree", "add", "-q", "-b", "wt", wt.to_str().unwrap()],
    );
    for d in ["shop/node_modules/x", "shop/.cache/y", "shop/empty"] {
        fs::create_dir_all(env.root().join(d)).unwrap();
    }
    let shop = env.root().join("shop");

    let ev = events(&env.kex_ok(env.root(), &["browse", "shop", "--depth", "2", "--json"]));
    assert_eq!(ev[0]["type"], "hello");
    let t = find_event(&ev, "tree");
    // The request is echoed: resolved root and depth.
    assert_eq!(
        t["root"],
        dunce::canonicalize(&shop).unwrap().to_str().unwrap()
    );
    assert_eq!(t["depth"], 2);
    assert_eq!(t["truncated"], false);
    let names: Vec<&str> = t["tree"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["admin", "api", "api-wt", "clients", "empty"]); // no node_modules, .cache
    let a = tree_entry(&t["tree"], "api");
    assert_eq!(a["kind"], "repo");
    assert_eq!(a["branch"], "main");
    assert_eq!(a["dirty"], 2); // package.json and new.txt
    assert_eq!(a["remote_display"], "example.com/acme/api");
    assert_eq!(a["stack"], serde_json::json!(["node"]));
    assert!(a["last_commit_at"].as_str().unwrap().starts_with("20"));
    let k = tree_entry(&t["tree"], "admin");
    assert_eq!(
        (k["kind"].as_str(), k["repo"].as_str()),
        (Some("kex_repo"), Some("admin"))
    );
    let w = tree_entry(&t["tree"], "api-wt");
    assert_eq!(w["kind"], "worktree");
    assert_eq!(last_name(&w["main_repo"]), "api");
    // clients/acme is at the last level: its portal isn't listed, but counted.
    let acme = tree_entry(&t["tree"], "acme");
    assert_eq!(acme["truncated"], true);
    assert_eq!(acme["children"], serde_json::json!([]));
    assert_eq!(acme["repos_below"], 1);
    assert_eq!(tree_entry(&t["tree"], "clients")["repos_below"], 1);
    assert_eq!(tree_entry(&t["tree"], "empty")["truncated"], false);
    assert_eq!(
        find_event(&ev, "workspaces")["workspaces"],
        serde_json::json!([])
    );

    // --hidden, --all, and the entry cap.
    let ev = events(&env.kex_ok(
        &shop,
        &["browse", "--depth", "1", "--hidden", "--all", "--json"],
    ));
    let t = find_event(&ev, "tree");
    assert!(tree_entry(&t["tree"], ".cache")["hidden"] == true);
    tree_entry(&t["tree"], "node_modules");
    let ev = events(&env.kex_ok(&shop, &["browse", "--max-entries", "3", "--json"]));
    let t = find_event(&ev, "tree");
    assert_eq!(t["truncated"], true);
    assert_eq!(t["count"], 3);

    // Text output, and an unknown folder.
    let out = env.kex_ok(&shop, &["browse", "--depth", "3"]);
    assert!(out.contains("└─ portal"), "{out}");
    assert_eq!(env.kex(&shop, &["browse", "nope"]).status.code(), Some(2));
}

#[test]
#[cfg(unix)]
fn browse_shows_symlinks_without_following_them() {
    let env = Env::new();
    let ext = env.repo("elsewhere/ext");
    let shop = env.root().join("shop");
    fs::create_dir_all(&shop).unwrap();
    std::os::unix::fs::symlink(&ext, shop.join("linked")).unwrap();
    std::os::unix::fs::symlink(shop.join("gone"), shop.join("dangling")).unwrap();
    let ev = events(&env.kex_ok(&shop, &["browse", "--json"]));
    let t = find_event(&ev, "tree");
    let l = tree_entry(&t["tree"], "linked");
    assert_eq!(l["kind"], "symlink");
    assert_eq!(l["target_kind"], "repo");
    assert_eq!(
        l["target"],
        dunce::canonicalize(&ext).unwrap().to_str().unwrap()
    );
    assert_eq!(tree_entry(&t["tree"], "dangling")["target_kind"], "missing");

    // --from takes the link and resolves it; the repo is named after the link.
    let req = serde_json::json!([{ "path": shop.join("linked") }]).to_string();
    let (ev, code) = kex_stdin(&env, &shop, &["init", "--from", "-", "--json"], &req);
    assert_eq!(code, Some(0), "{ev:?}");
    assert_eq!(find_event(&ev, "added")["repos"][0]["repo"], "linked");
    assert_eq!(read_config(&ext)["repo"], "linked");
}

const WORKSPACE: &str = r#"{
  // VS Code writes comments and trailing commas
  "folders": [
    { "path": "api", "name": "API Server" }, /* named */
    { "path": "../shop/web" },
    { "path": "admin" },
    { "path": "gone", },
  ],
  "settings": {},
}"#;

#[test]
fn init_sets_up_the_repos_of_a_workspace_file() {
    let env = Env::new();
    let api = env.repo("shop/api");
    let web = env.repo("shop/web");
    let admin = env.repo("shop/admin");
    env.kex_ok(&admin, &["init", "--yes", "--repo", "admin"]);
    let other = env.repo("shop/other");
    let file = env.root().join("shop/shop.code-workspace");
    fs::write(&file, WORKSPACE).unwrap();

    // kex browse reports the file with its folders classified.
    let ev = events(&env.kex_ok(&web, &["browse", "--json"]));
    let ws = &find_event(&ev, "workspaces")["workspaces"][0];
    assert_eq!(ws["name"], "shop");
    assert_eq!(ws["format"], "vscode");
    assert_eq!(
        ws["counts"],
        serde_json::json!({ "folders": 4, "repos": 3, "addable": 2, "already_kex": 1, "worktrees": 0, "missing": 1, "duplicates": 0 })
    );
    assert_eq!(ws["folders"][1]["path"], "../shop/web"); // as written
    assert_eq!(last_name(&ws["folders"][1]["resolved"]), "web");
    assert_eq!(ws["folders"][0]["default_name"], "api-server");
    assert_eq!(ws["folders"][2]["project"], project_of(&admin));

    // --workspace with --yes: no questions; the names it gives; admin's project, next to them.
    let ev = events(&env.kex_ok(
        &web,
        &[
            "init",
            "--yes",
            "--json",
            "--workspace",
            file.to_str().unwrap(),
        ],
    ));
    assert!(!ev.iter().any(|e| e["type"] == "prompt"));
    assert!(
        ev.iter()
            .any(|e| e["type"] == "log" && e["text"].as_str().unwrap().contains("missing"))
    );
    assert_eq!(added_names(&ev), ["api-server", "web"]);
    assert_eq!(project_of(&web), project_of(&admin));
    assert!(!other.join("kex.json").exists()); // not in the workspace

    // --yes never picks up a workspace file on its own: init --yes is the one repo, as before.
    env.kex_ok(&other, &["init", "--yes", "--repo", "other"]);
    assert_eq!(read_config(&other)["repo"], "other");
    assert_eq!(read_config(&api)["repo"], "api-server");
}

#[test]
fn init_from_validates_everything_before_writing() {
    let env = Env::new();
    let api = env.repo("shop/api");
    let web = env.repo("shop/web");
    let admin = env.repo("shop/admin");
    env.kex_ok(&admin, &["init", "--yes", "--repo", "admin"]);
    let project = project_of(&admin);
    let other = env.repo("else/other");
    env.kex_ok(
        &other,
        &[
            "init",
            "--yes",
            "--repo",
            "other",
            "--project",
            "other-abc123",
        ],
    );
    fs::create_dir_all(env.root().join("shop/plain")).unwrap();
    let shop = env.root().join("shop");

    // Every problem at once, with index and field; nothing written.
    let bad = serde_json::json!({
        "project": "nope-123456",
        "repos": [
            { "path": "api", "name": "API" },
            { "path": "web", "colour": "red" },
            { "path": "plain" },
            { "path": "missing" },
            { "path": "./web" },
            { "path": other },
            { "name": "x" },
            "api",
        ],
    })
    .to_string();
    let (ev, code) = kex_stdin(&env, &shop, &["init", "--from", "-", "--json"], &bad);
    assert_eq!(code, Some(2));
    let errors: Vec<(serde_json::Value, serde_json::Value)> = ev
        .iter()
        .filter(|e| e["type"] == "error")
        .map(|e| (e["index"].clone(), e["field"].clone()))
        .collect();
    let j = |i: Option<u64>, f: Option<&str>| (serde_json::json!(i), serde_json::json!(f));
    for want in [
        j(Some(0), Some("name")),
        j(Some(1), Some("colour")),
        j(Some(2), Some("path")),
        j(Some(3), Some("path")),
        j(Some(4), Some("path")),
        j(Some(6), Some("path")),
        j(Some(7), None),
        j(None, Some("project")),
    ] {
        assert!(errors.contains(&want), "missing {want:?} in {errors:?}");
    }
    assert!(!api.join("kex.json").exists() && !web.join("kex.json").exists());
    assert!(!api.join(".gitignore").exists());

    // A repo of another project is an error once the project is known.
    let req = serde_json::json!({ "project": project, "repos": [{ "path": other }] }).to_string();
    let (ev, _) = kex_stdin(&env, &shop, &["add", "--from", "-", "--json"], &req);
    let e = find_event(&ev, "error");
    assert_eq!(
        (e["index"].as_u64(), e["field"].as_str()),
        (Some(0), Some("path"))
    );
    assert!(
        e["message"]
            .as_str()
            .unwrap()
            .contains("already a kex repo in project"),
        "{e}"
    );

    // --dry-run: the plan, nothing written.
    let req = serde_json::json!({
        "project": project,
        "repos": [
            { "path": "api", "description": "The API" },
            { "path": "admin" },
            { "path": web, "name": "site" },
        ],
    })
    .to_string();
    let (ev, code) = kex_stdin(
        &env,
        &shop,
        &["init", "--from", "-", "--dry-run", "--json"],
        &req,
    );
    assert_eq!(code, Some(0), "{ev:?}");
    let plan = find_event(&ev, "plan");
    assert_eq!(plan["valid"], true);
    assert_eq!(
        plan["project"],
        serde_json::json!({ "id": project, "new": false, "name": null })
    );
    assert_eq!(plan["repos"][0]["config"]["description"], "The API");
    assert_eq!(plan["repos"][1]["index"], 2);
    assert_eq!(plan["repos"][1]["config"]["repo"], "site");
    assert_eq!(plan["repos"][0]["gitignore"]["add"], "kex.local.json");
    assert_eq!(plan["skipped"][0]["index"], 1);
    assert_eq!(plan["skipped"][0]["reason"], "already_kex");
    assert!(!api.join("kex.json").exists());

    // For real: the same request writes both, and a second run skips them.
    let (ev, code) = kex_stdin(&env, &shop, &["init", "--from", "-", "--json"], &req);
    assert_eq!(code, Some(0), "{ev:?}");
    let added = find_event(&ev, "added");
    assert_eq!(added["repos"].as_array().unwrap().len(), 2);
    assert_eq!(read_config(&api)["description"], "The API");
    assert_eq!(read_config(&web)["repo"], "site");
    assert_eq!(project_of(&api), project);
    let (ev, code) = kex_stdin(&env, &shop, &["init", "--from", "-", "--json"], &req);
    assert_eq!(code, Some(0), "{ev:?}");
    let added = find_event(&ev, "added");
    assert_eq!(added["repos"], serde_json::json!([]));
    assert_eq!(added["skipped"].as_array().unwrap().len(), 3);
    // The description shows up in status --json.
    let st = status_json(&env, &api);
    let r = st["repos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["repo"] == "api")
        .unwrap();
    assert_eq!(r["description"], "The API");
}

#[test]
fn from_accepts_a_plain_array_and_a_new_project() {
    let env = Env::new();
    let api = env.repo("shop/api");
    let web = env.repo("shop/web");
    let file = env.root().join("req.json");
    fs::write(
        &file,
        serde_json::json!([{ "path": api }, { "path": web }]).to_string(),
    )
    .unwrap();
    let ev = events(&env.kex_ok(
        env.root(),
        &["add", "--from", file.to_str().unwrap(), "--json"],
    ));
    let added = find_event(&ev, "added");
    assert!(
        added["project"].as_str().unwrap().starts_with("shop-"),
        "{added}"
    );
    assert!(read_config(&api).get("peers").is_none()); // no remotes, so no peers
    assert_eq!(project_of(&api), project_of(&web));

    let env = Env::new();
    let api = env.repo("shop/api");
    let req = serde_json::json!({ "project": { "new": "Big Shop" }, "repos": [{ "path": api }] });
    let (ev, code) = kex_stdin(
        &env,
        env.root(),
        &["init", "--from", "-", "--json"],
        &req.to_string(),
    );
    assert_eq!(code, Some(0), "{ev:?}");
    assert!(project_of(&api).starts_with("big-shop-"));
    // Not JSON at all: one error, exit 2.
    let (ev, code) = kex_stdin(&env, env.root(), &["init", "--from", "-", "--json"], "nope");
    assert_eq!(code, Some(2));
    assert!(
        find_event(&ev, "error")["message"]
            .as_str()
            .unwrap()
            .contains("not valid JSON")
    );
}

#[test]
#[cfg(unix)]
fn from_writes_all_or_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    let one = env.repo("b/one");
    let two = env.repo("b/two");
    fs::create_dir_all(two.join(".config")).unwrap();
    fs::set_permissions(two.join(".config"), fs::Permissions::from_mode(0o555)).unwrap();
    let req = serde_json::json!([{ "path": one }, { "path": two }]).to_string();
    let (ev, code) = kex_stdin(&env, env.root(), &["init", "--from", "-", "--json"], &req);
    fs::set_permissions(two.join(".config"), fs::Permissions::from_mode(0o755)).unwrap();
    if two.join(".config/kex.json").exists() {
        return; // running as root
    }
    assert_eq!(code, Some(1), "{ev:?}");
    let added = find_event(&ev, "added");
    assert_eq!(added["repos"], serde_json::json!([]));
    assert_eq!(added["rolled_back"], true);
    assert_eq!(added["failed"][0]["repo"], "two");
    assert!(!one.join("kex.json").exists());
    assert!(!one.join(".gitignore").exists());
    let leftovers: Vec<_> = fs::read_dir(&one)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains("kex-tmp"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
#[cfg(unix)]
fn sigint_while_writing_undoes_everything() {
    let env = Env::new();
    let one = env.repo("b/one");
    let two = env.repo("b/two");
    let req = serde_json::json!([{ "path": one }, { "path": two }]).to_string();
    let mut c = env.cmd(env.root(), &["init", "--from", "-", "--json"]);
    c.env("KEX_TEST_WRITE_PAUSE_MS", "3000")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let mut child = c.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(req.as_bytes())
        .unwrap();
    let t0 = std::time::Instant::now();
    while !one.join("kex.json").exists() {
        assert!(t0.elapsed().as_secs() < 20, "kex.json never appeared");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let ok = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(ok.success());
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(130));
    assert!(!one.join("kex.json").exists());
    assert!(!one.join(".gitignore").exists());
    assert!(!two.join("kex.json").exists());
}

#[test]
fn a_workspace_file_listing_a_repo_twice_sets_it_up_once() {
    let env = Env::new();
    let api = env.repo("shop/api");
    let web = env.repo("shop/web");
    let wt = env.root().join("shop/api-wt");
    git(
        &api,
        &["worktree", "add", "-q", "-b", "wt", wt.to_str().unwrap()],
    );
    #[cfg(unix)]
    std::os::unix::fs::symlink(&web, env.root().join("shop/web-link")).unwrap();
    let file = env.root().join("shop/shop.code-workspace");
    let folders = if cfg!(unix) {
        r#"[{"path":"api"},{"path":"web"},{"path":"./web"},{"path":"web-link"},{"path":"api-wt"},{"path":"x/../gone"}]"#
    } else {
        r#"[{"path":"api"},{"path":"web"},{"path":"./web"},{"path":"api-wt"},{"path":"x/../gone"}]"#
    };
    fs::write(&file, format!(r#"{{"folders":{folders}}}"#)).unwrap();

    let ev = events(&env.kex_ok(&web, &["browse", "--json"]));
    let ws = &find_event(&ev, "workspaces")["workspaces"][0];
    let c = &ws["counts"];
    assert_eq!(
        (c["repos"].as_u64(), c["addable"].as_u64()),
        (Some(2), Some(2))
    );
    assert_eq!(c["worktrees"], 1);
    assert_eq!(c["duplicates"], if cfg!(unix) { 2 } else { 1 });
    assert_eq!(ws["folders"][2]["duplicate_of"], 1);
    // A missing folder's path is still normalized.
    let gone = ws["folders"].as_array().unwrap().last().unwrap();
    assert!(
        gone["resolved"]
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with("shop/gone"),
        "{gone}"
    );

    let ev = events(&env.kex_ok(
        env.root(),
        &[
            "add",
            "--yes",
            "--json",
            "--workspace",
            file.to_str().unwrap(),
        ],
    ));
    let added = find_event(&ev, "added");
    let names: Vec<&str> = added["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["repo"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["api", "web"]);
    assert_eq!(read_config(&web)["repo"], "web");
    assert_eq!(peer_names_or_none(&read_config(&api)), Vec::<String>::new());
}

fn peer_names_or_none(c: &serde_json::Value) -> Vec<String> {
    if c.get("peers").is_none() {
        return Vec::new();
    }
    peer_names(c)
}

#[test]
fn browse_pages_through_a_big_folder() {
    let env = Env::new();
    let big = env.root().join("big");
    for i in 0..8 {
        fs::create_dir_all(big.join(format!("d{i}"))).unwrap();
    }
    env.repo("big/zrepo");
    let ev = events(&env.kex_ok(&big, &["browse", "--max-entries", "5", "--json"]));
    let t = find_event(&ev, "tree");
    assert_eq!(t["children_total"], 9);
    assert_eq!(
        (t["has_more"].as_bool(), t["next_offset"].as_u64()),
        (Some(true), Some(5))
    );
    assert_eq!(t["tree"].as_array().unwrap().len(), 5);
    let ev = events(&env.kex_ok(
        &big,
        &["browse", "--max-entries", "5", "--offset", "5", "--json"],
    ));
    let t = find_event(&ev, "tree");
    assert_eq!(t["offset"], 5);
    assert_eq!(t["has_more"], false);
    let names: Vec<&str> = t["tree"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["d5", "d6", "d7", "zrepo"]);
}

#[test]
fn repo_names_must_follow_the_schema_everywhere() {
    let env = Env::new();
    let web = env.repo("shop/web");
    let out = env.kex(&web, &["init", "--yes", "--repo", "Bad Name"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("isn't a repo name"),
        "{}",
        stderr(&out)
    );
    assert!(!web.join("kex.json").exists());
}

#[test]
fn a_dry_runs_request_writes_exactly_the_plan() {
    let env = Env::new();
    let api = env.repo("shop/api");
    let web = env.repo("shop/web");
    let req = serde_json::json!({
        "project": { "new": "Shop" },
        "repos": [{ "path": api, "description": "The API" }, { "path": web }],
    })
    .to_string();
    let (ev, code) = kex_stdin(
        &env,
        env.root(),
        &["init", "--from", "-", "--dry-run", "--json"],
        &req,
    );
    assert_eq!(code, Some(0), "{ev:?}");
    let plan = find_event(&ev, "plan").clone();
    let request = plan["request"].to_string();
    assert_eq!(plan["request"]["project"]["id"], plan["project"]["id"]);
    assert_eq!(
        plan["request"]["repos"][0]["repo_id"],
        plan["repos"][0]["repo_id"]
    );

    let (ev, code) = kex_stdin(
        &env,
        env.root(),
        &["init", "--from", "-", "--json"],
        &request,
    );
    assert_eq!(code, Some(0), "{ev:?}");
    for (i, dir) in [&api, &web].into_iter().enumerate() {
        assert_eq!(read_config(dir), plan["repos"][i]["config"], "repos[{i}]");
    }
    // Sending it again changes nothing.
    let (ev, code) = kex_stdin(
        &env,
        env.root(),
        &["init", "--from", "-", "--json"],
        &request,
    );
    assert_eq!(code, Some(0), "{ev:?}");
    assert_eq!(
        find_event(&ev, "added")["skipped"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // A changed name for a repo already set up: skipped, with a note.
    let changed = serde_json::json!({
        "project": plan["project"]["id"],
        "repos": [{ "path": web, "name": "site", "description": "new" }],
    })
    .to_string();
    let (ev, _) = kex_stdin(
        &env,
        env.root(),
        &["init", "--from", "-", "--json"],
        &changed,
    );
    let sk = &find_event(&ev, "added")["skipped"][0];
    assert_eq!(sk["differs"], serde_json::json!(["name", "description"]));
    assert!(
        sk["note"].as_str().unwrap().contains("it is \"web\""),
        "{sk}"
    );
    assert_eq!(read_config(&web)["repo"], "web");

    // Pinned ids are checked: format, twice, already used.
    let one = env.repo("shop/one");
    let two = env.repo("shop/two");
    let used = read_config(&api)["repo_id"].clone();
    let bad = serde_json::json!([
        { "path": one, "repo_id": "R-1" },
        { "path": two, "repo_id": used },
    ])
    .to_string();
    let (ev, code) = kex_stdin(&env, env.root(), &["add", "--from", "-", "--json"], &bad);
    assert_eq!(code, Some(2));
    let fields: Vec<(u64, &str)> = ev
        .iter()
        .filter(|e| e["type"] == "error")
        .map(|e| (e["index"].as_u64().unwrap(), e["field"].as_str().unwrap()))
        .collect();
    assert_eq!(fields, [(0, "repo_id"), (1, "repo_id")]);
}

#[test]
fn hello_lists_features() {
    let env = Env::new();
    let ev = events(&env.kex_ok(env.root(), &["browse", "--json", "--no-git"]));
    let f: Vec<&str> = ev[0]["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for want in [
        "browse",
        "from",
        "dry_run",
        "workspaces",
        "pin_ids",
        "paging",
    ] {
        assert!(f.contains(&want), "{want} in {f:?}");
    }
}

#[test]
fn browse_counts_the_whole_subtree_and_lists_repos_flat() {
    let env = Env::new();
    let deep = env.repo("w/a/b/c/d/deep");
    env.repo("w/a/near");
    let top = env.repo("w/top");
    fs::create_dir_all(env.root().join("w/node_modules/x/.git")).unwrap(); // skipped
    let w = env.root().join("w");

    let ev = events(&env.kex_ok(&w, &["browse", "--depth", "1", "--json"]));
    let t = find_event(&ev, "tree");
    let a = tree_entry(&t["tree"], "a");
    assert_eq!(a["repos_below"], 2); // near and the deep one, 5 levels down
    assert_eq!(a["repos_below_truncated"], false);

    let ev = events(&env.kex_ok(&w, &["browse", "--repos-only", "--json"]));
    let t = find_event(&ev, "tree");
    assert_eq!(t["flat"], true);
    assert_eq!(t["depth"], serde_json::Value::Null);
    let paths: Vec<String> = t["tree"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| last_name(&e["path"]))
        .collect();
    assert_eq!(paths, ["deep", "near", "top"]);
    let d = &t["tree"][0];
    assert_eq!(
        (d["depth"].as_u64(), d["branch"].as_str()),
        (Some(5), Some("main"))
    );
    assert_eq!(
        d["path"],
        dunce::canonicalize(&deep).unwrap().to_str().unwrap()
    );
    // --depth limits it; --max-entries caps it; --flat is the same.
    let ev = events(&env.kex_ok(&w, &["browse", "--flat", "--depth", "2", "--json"]));
    assert_eq!(find_event(&ev, "tree")["count"], 2);
    let ev = events(&env.kex_ok(
        &w,
        &["browse", "--repos-only", "--max-entries", "1", "--json"],
    ));
    let t = find_event(&ev, "tree");
    assert_eq!(
        (t["count"].as_u64(), t["truncated"].as_bool()),
        (Some(1), Some(true))
    );

    // --no-git: kinds only, no git fields. A tiny budget: some or all left out.
    let ev = events(&env.kex_ok(&w, &["browse", "--no-git", "--json"]));
    let t = find_event(&ev, "tree");
    assert_eq!(
        (t["git"].as_bool(), t["git_partial"].as_bool()),
        (Some(false), Some(false))
    );
    let tp = tree_entry(&t["tree"], "top");
    assert_eq!(tp["kind"], "repo");
    assert!(tp.get("branch").is_none() && tp.get("dirty").is_none());
    let ev = events(&env.kex_ok(
        &w,
        &["browse", "--repos-only", "--git-budget-ms", "1", "--json"],
    ));
    let t = find_event(&ev, "tree");
    assert_eq!(t["git_partial"], true);
    assert!(
        t["tree"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e.get("branch").is_none())
    );
    let _ = top;
}

#[test]
fn names_and_descriptions_have_length_limits() {
    let env = Env::new();
    let api = env.repo("shop/api");
    let long = "a".repeat(41);
    let req = serde_json::json!({
        "project": { "new": "p".repeat(41) },
        "repos": [{ "path": api, "name": long, "description": "two\nlines" }],
    })
    .to_string();
    let (ev, code) = kex_stdin(&env, env.root(), &["init", "--from", "-", "--json"], &req);
    assert_eq!(code, Some(2));
    let fields: Vec<(serde_json::Value, &str)> = ev
        .iter()
        .filter(|e| e["type"] == "error")
        .map(|e| (e["index"].clone(), e["field"].as_str().unwrap()))
        .collect();
    for want in [
        (serde_json::json!(0), "name"),
        (serde_json::json!(0), "description"),
        (serde_json::Value::Null, "project"),
    ] {
        assert!(fields.contains(&want), "{want:?} in {fields:?}");
    }
    let req = serde_json::json!([{ "path": api, "description": "x".repeat(201) }]).to_string();
    let (ev, code) = kex_stdin(&env, env.root(), &["init", "--from", "-", "--json"], &req);
    assert_eq!(code, Some(2));
    assert!(
        find_event(&ev, "error")["message"]
            .as_str()
            .unwrap()
            .contains("200")
    );

    let out = env.kex(&api, &["init", "--yes", "--repo", &long]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("at most 40"), "{}", stderr(&out));
    // A long folder name gives a default name of 40 characters.
    let longdir = env.repo(&format!("shop/{}", "b".repeat(50)));
    env.kex_ok(&longdir, &["init", "--yes"]);
    assert_eq!(read_config(&longdir)["repo"], "b".repeat(40));
    assert!(!api.join("kex.json").exists());
}

// ── cli-events/1: docs, manifest, limits, the generic flow ──────────────────────────────

/// Every example line in the standard and in kex's profile is valid cli-events/1.
#[test]
fn every_example_line_in_the_docs_validates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut n = 0;
    for doc in ["spec/cli-events-1.md", "docs/self-description.md"] {
        let text = fs::read_to_string(root.join(doc))
            .unwrap()
            .replace("\r\n", "\n");
        for block in text.split("```ndjson\n").skip(1) {
            let block = block.split("```").next().unwrap();
            for line in block.lines() {
                let line = line.trim().trim_start_matches(['←', '→']).trim();
                if line.is_empty() || line.contains('…') {
                    continue;
                }
                let v: serde_json::Value = serde_json::from_str(line)
                    .unwrap_or_else(|e| panic!("{doc}: not JSON ({e}): {line}"));
                assert_valid(&v);
                // kex's own examples also use kex's real messages and valid actions.
                if doc.starts_with("docs/") && v["type"] != "manifest" {
                    check_messages(&v, &manifest().1["messages"]);
                    check_actions(&v);
                }
                check_cells(&v);
                n += 1;
            }
        }
    }
    assert!(n >= 40, "only {n} example lines");
}

#[test]
fn hello_carries_the_hash_of_the_manifest() {
    let env = Env::new();
    let api = env.repo("shop/api");
    env.kex_ok(&api, &["init", "--yes"]);
    let (hello, m) = manifest();
    let hash = hello["manifest_hash"].as_str().unwrap();
    assert_eq!(hash.len(), 16);
    assert_eq!(hash, manifest_hash(m), "hash of the canonical manifest");
    // Stable across runs and the same in every command's hello.
    let again = events(&env.kex_ok(&api, &["manifest", "--json"]));
    assert_eq!(again[0]["manifest_hash"], hash);
    assert_eq!(
        manifest_hash(&find_event(&again, "manifest")["manifest"]),
        hash
    );
    let st = events(&env.kex_ok(&api, &["status", "--json"]));
    assert_eq!(st[0]["manifest_hash"], hash);
    assert!(
        st[0]["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "cli_events")
    );
    // Commands and their annotations.
    let cmd = |id: &str| -> serde_json::Value {
        m["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("no command {id}"))
    };
    assert_eq!(cmd("init")["aliases"], serde_json::json!([["add"]]));
    assert_eq!(cmd("init")["json_input"]["option"], "from");
    assert_eq!(cmd("browse")["read_only"], true);
    assert_eq!(cmd("run")["stop"], serde_json::json!({"type": "stop"}));
    assert_eq!(cmd("completions")["needs_terminal"], true);
    assert_eq!(cmd("pr.check")["words"], serde_json::json!(["pr", "check"]));
    assert!(
        m["commands"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["id"] != "help")
    );
    let update = cmd("update");
    assert!(
        !update["options"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["id"] == "quiet"),
        "hidden options are left out"
    );
    // The manifest line is well under 512 KiB.
    assert!(canonical(m).len() < 512 * 1024);
    // The text form.
    let text = env.kex_ok(&api, &["manifest"]);
    assert!(text.contains(hash) && text.contains("init (add)"), "{text}");
}

#[test]
fn completions_are_terminal_only() {
    let env = Env::new();
    // A usage error with --json is still a conforming stream.
    let out = env.kex(env.root(), &["status", "--no-such-flag", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(find_event(&events(&stdout(&out)), "exit")["tone"], "error");
    let out = env.kex(env.root(), &["completions", "zsh", "--json"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        env.kex_ok(env.root(), &["completions", "zsh"])
            .contains("kex")
    );
}

#[test]
fn limits_keep_lines_small() {
    let env = Env::new();
    // 1,000 folders: one tree line, under 1 MiB (conform checks every line).
    let big = env.root().join("big");
    for i in 0..1000 {
        fs::create_dir_all(big.join(format!("folder-with-a-longish-name-{i:04}"))).unwrap();
    }
    let out = env.kex(
        &big,
        &["browse", "--json", "--max-entries", "1000", "--depth", "1"],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let tree = find_event(&events(&stdout(&out)), "tree").clone();
    assert_eq!(tree["view"]["nodes"].as_array().unwrap().len(), 1000);
    let out = env.kex(&big, &["browse", "--json", "--max-entries", "1001"]);
    assert_eq!(out.status.code(), Some(2));

    // A --from request over 1 MiB is refused before it's read as JSON.
    let huge = format!("[{}]", vec!["{\"path\":\"x\"}"; 90_000].join(","));
    assert!(huge.len() > 1024 * 1024);
    let (ev, code) = kex_stdin(&env, env.root(), &["init", "--from", "-", "--json"], &huge);
    assert_eq!(code, Some(2));
    assert_eq!(find_event(&ev, "error")["msg"], "from.too_big");
}

/// argv for a RunAction, built only from the manifest (cli-events §7).
fn argv_for(run: &serde_json::Value) -> Vec<String> {
    let m = &manifest().1;
    let cmd = m["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == run["command"])
        .unwrap();
    let mut argv: Vec<String> = cmd["words"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap().to_string())
        .collect();
    let mut positionals = Vec::new();
    for o in cmd["options"].as_array().unwrap() {
        let Some(v) = run["options"].get(o["id"].as_str().unwrap()) else {
            continue;
        };
        let text = match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(a) => a
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect::<Vec<_>>()
                .join(o["separator"].as_str().unwrap_or(",")),
            other => other.to_string(),
        };
        match (o["flag"].as_str(), o["type"].as_str()) {
            (Some(f), Some("boolean")) => {
                if v == true {
                    argv.push(f.to_string());
                }
            }
            (Some(f), _) => argv.push(format!("{f}={text}")),
            (None, _) => positionals.push(text),
        }
    }
    argv.push("--json".into());
    if !positionals.is_empty() {
        argv.push("--".into());
        argv.extend(positionals);
    }
    argv
}

fn run_action(
    env: &Env,
    dir: &Path,
    run: &serde_json::Value,
) -> (Vec<serde_json::Value>, Option<i32>) {
    let argv = argv_for(run);
    let args: Vec<&str> = argv.iter().map(String::as_str).collect();
    let stdin = run
        .get("stdin")
        .map(|s| format!("{s}\n"))
        .unwrap_or_default();
    kex_stdin(env, dir, &args, &stdin)
}

/// The generic flow of cli-events §8, with nothing kex-specific: browse → pick the items of
/// selectable nodes → dry run → run the plan's action exactly as given → added.
#[test]
fn the_generic_setup_flow_works_from_the_views_alone() {
    let env = Env::new();
    let api = env.repo("shop/api");
    env.repo("shop/web");
    fs::create_dir_all(env.root().join("shop/docs")).unwrap();
    let ji = &manifest().1["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "init")
        .unwrap()["json_input"];
    let picker = &ji["picker"];

    // 1. Pick: the picker command, then the `open` of the view on the shop folder.
    let mut run = serde_json::json!({ "command": picker["command"], "options": picker["options"] });
    run["options"]["dir"] = serde_json::json!(lossy(&env.root().join("shop")));
    let (ev, code) = run_action(&env, &api, &run);
    assert_eq!(code, Some(0));
    let view = &find_event(&ev, "tree")["view"];
    assert_eq!(view["kind"], "tree");
    let kinds: Vec<&str> = picker["select_kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap())
        .collect();
    let items: Vec<serde_json::Value> = view["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["selectable"] == true && kinds.contains(&n["kind"].as_str().unwrap()))
        .map(|n| n["item"].clone())
        .collect();
    assert_eq!(items.len(), 2, "api and web: {view}");
    // The repo kex runs in starts checked.
    assert!(
        view["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["label"] == "api" && n["checked"] == true)
    );

    // 2. The document, into `picker.into`.
    let mut doc = serde_json::json!({ "project": { "new": "Shop" } });
    doc["repos"] = serde_json::json!(items);
    assert_eq!(picker["into"], "/repos");

    // 3. Review: a dry run.
    let opt = ji["option"].as_str().unwrap();
    let mut review = serde_json::json!({ "command": "init", "options": {}, "stdin": doc });
    review["options"][opt] = serde_json::json!("-");
    review["options"][ji["dry_run"].as_str().unwrap()] = serde_json::json!(true);
    let (ev, code) = run_action(&env, env.root(), &review);
    assert_eq!(code, Some(0), "{ev:?}");
    let plan = &find_event(&ev, "plan")["view"];
    assert_eq!(plan["rows"].as_array().unwrap().len(), 2);
    assert!(!api.join("kex.json").exists(), "a dry run writes nothing");
    let action = &plan["actions"][0];
    assert_eq!(action["effect"], "write");

    // 4. Run the action as given.
    let (ev, code) = run_action(&env, env.root(), &action["run"]);
    assert_eq!(code, Some(0), "{ev:?}");
    let added = &find_event(&ev, "added")["view"];
    assert_eq!(added["rows"].as_array().unwrap().len(), 2);
    let planned_id = plan["rows"][0]["cells"]["repo_id"].clone();
    assert!(
        added["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["cells"]["repo_id"] == planned_id),
        "the ids of the plan were written"
    );
}

fn lossy(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn push_and_a_known_update_come_as_views() {
    let env = Env::new();
    let remote = env.root().join("remote/api.git");
    fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "-q", "--bare"]);
    let api = env.repo("shop/api");
    git(&api, &["remote", "add", "origin", &lossy(&remote)]);
    env.kex_ok(&api, &["init", "--yes"]);
    commit(&api, "a.txt", "a");
    let ev = events(&env.kex_ok(&api, &["push", "--repos", "api", "--yes", "--json"]));
    let pushed = find_event(&ev, "pushed");
    assert_eq!(pushed["ok"], true);
    assert_eq!(pushed["key"], "push:api");
    assert_eq!(pushed["view"]["lines"][0]["level"], "win");
    assert_eq!(pushed["view"]["lines"][0]["text"]["msg"], "push.done");

    // A newer release in the update cache: an `update_available` card with an Install
    // action (checked against the manifest by `conform`), and the menu offers it.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let home = env.root().join("kexhome");
    fs::create_dir_all(&home).unwrap();
    fs::write(
        home.join("update.json"),
        format!("{{\"checked_at\":{now},\"latest\":\"99.0.0\"}}"),
    )
    .unwrap();
    let mut c = env.cmd(&api, &["status", "--json"]);
    c.env_remove("KEX_NO_UPDATE_CHECK").env_remove("CI");
    let out = c.output().unwrap();
    let ev = events(&stdout(&out));
    let up = find_event(&ev, "update_available");
    let action = &up["view"]["actions"][0];
    assert_eq!(action["effect"], "install");
    assert_eq!(
        action["run"],
        serde_json::json!({"command": "update", "options": {"yes": true}})
    );

    let mut c = env.cmd(&api, &["--json"]);
    c.env_remove("KEX_NO_UPDATE_CHECK")
        .env_remove("CI")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let mut child = c.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"type\":\"answer\",\"id\":\"p1\",\"cancel\":true}\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(130));
    let ev = events(&stdout(&out));
    let menu = find_event(&ev, "prompt");
    assert_eq!(menu["msg"], "menu.pick");
    assert!(menu["columns"].is_array());
    let last = menu["options"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["value"], "update");
    assert_eq!(
        last["view"]["cells"]["description"]["text"]["msg"],
        "menu.update"
    );
    assert_eq!(ev.last().unwrap()["tone"], "cancelled");
}

#[test]
#[should_panic(expected = "hello first")]
fn conformance_checks_catch_a_stream_without_hello() {
    conform("{\"type\":\"exit\",\"code\":0,\"tone\":\"ok\"}\n");
}

#[test]
#[should_panic(expected = "breaks cli-events/1")]
fn conformance_checks_catch_a_tool_line_without_a_view() {
    let hash = manifest().0["manifest_hash"].as_str().unwrap().to_string();
    conform(&format!(
        "{{\"type\":\"hello\",\"standard\":\"cli-events/1\",\"tool\":{{\"name\":\"kex\",\"version\":\"0\"}},\"manifest_version\":1,\"manifest_hash\":\"{hash}\"}}\n{{\"type\":\"pull\",\"repo\":\"api\"}}\n{{\"type\":\"exit\",\"code\":0,\"tone\":\"ok\"}}\n"
    ));
}

// ── Review 4: signals, stop, gh, manifest, reset, big trees ─────────────────────────────

/// A `kex … --json` child whose stdout is read line by line.
struct Live {
    child: std::process::Child,
    lines: std::io::Lines<std::io::BufReader<std::process::ChildStdout>>,
    seen: Vec<String>,
}

impl Live {
    fn start(env: &Env, dir: &Path, args: &[&str]) -> Live {
        let mut c = env.cmd(dir, args);
        c.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = c.spawn().unwrap();
        let out = child.stdout.take().unwrap();
        use std::io::BufRead;
        Live {
            child,
            lines: std::io::BufReader::new(out).lines(),
            seen: Vec::new(),
        }
    }

    /// Read until a line matches.
    fn until(&mut self, f: impl Fn(&serde_json::Value) -> bool) -> serde_json::Value {
        for l in self.lines.by_ref() {
            let l = l.unwrap();
            self.seen.push(l.clone());
            let v: serde_json::Value = serde_json::from_str(&l).unwrap();
            if f(&v) {
                return v;
            }
        }
        panic!("stream ended before the line: {:#?}", self.seen);
    }

    fn send(&mut self, line: &str) {
        let s = self.child.stdin.as_mut().unwrap();
        writeln!(s, "{line}").unwrap();
        s.flush().unwrap();
    }

    /// The rest of the stream (checked by `conform`) and the exit code.
    fn finish(mut self) -> (Vec<serde_json::Value>, Option<i32>) {
        for l in self.lines.by_ref() {
            self.seen.push(l.unwrap());
        }
        let code = self.child.wait().unwrap().code();
        let text = self.seen.join("\n") + "\n";
        (conform(&text), code)
    }
}

#[cfg(unix)]
fn sigint(child: &std::process::Child) {
    // SAFETY: kill(2) on our own child.
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGINT);
    }
}

/// A project with one repo whose service runs until stopped.
fn sleeping_service(env: &Env) -> PathBuf {
    let api = env.repo("p/api");
    env.kex_ok(&api, &["init", "--yes", "--repo", "api"]);
    let mut c = read_config(&api);
    c["run"] = serde_json::json!({ "dev": "sleep 30" });
    fs::write(api.join("kex.json"), c.to_string()).unwrap();
    api
}

#[cfg(unix)]
#[test]
fn sigint_with_a_prompt_open_ends_cancelled() {
    let env = Env::new();
    let api = env.repo("shop/api");
    env.kex_ok(&api, &["init", "--yes"]);
    let mut live = Live::start(&env, &api, &["push", "--json"]);
    live.until(|v| v["type"] == "prompt");
    sigint(&live.child);
    let (ev, code) = live.finish();
    assert_eq!(code, Some(130));
    assert_eq!(ev.last().unwrap()["tone"], "cancelled");
}

#[cfg(unix)]
#[test]
fn sigint_during_run_stops_the_services_and_ends_cancelled() {
    let env = Env::new();
    let api = sleeping_service(&env);
    let mut live = Live::start(&env, &api, &["run", "--json", "--no-update"]);
    live.until(|v| v["type"] == "started");
    sigint(&live.child);
    let (ev, code) = live.finish();
    assert_eq!(code, Some(130));
    let stopping = find_event(&ev, "stopping");
    assert_eq!(stopping["view"]["state"], "stopping");
    let exited = find_event(&ev, "exited");
    assert_eq!(exited["view"]["state"], "exited");
    assert_eq!(exited["view"]["tone"], "muted");
    assert_eq!(ev.last().unwrap()["tone"], "cancelled");
}

#[test]
fn the_stop_line_ends_run_ok_and_bad_lines_are_logged() {
    let env = Env::new();
    let api = sleeping_service(&env);
    let mut live = Live::start(&env, &api, &["run", "--json", "--no-update"]);
    let started = live.until(|v| v["type"] == "started");
    assert_eq!(started["view"]["state"], "running", "no URL to wait for");
    live.send("{\"type\":\"bogus\"}");
    let bad = live.until(|v| v["type"] == "log" && v["level"] == "error");
    assert_eq!(bad["msg"], "run.unknown_message");
    live.send("{\"type\":\"stop\"}");
    let (ev, code) = live.finish();
    assert_eq!(code, Some(0));
    assert!(
        !ev.iter().any(|e| e["type"] == "error"),
        "no fatal-looking error"
    );
    let exited = find_event(&ev, "exited");
    assert_eq!(exited["view"]["state"], "exited");
    assert_eq!(exited["view"]["tone"], "muted");
    assert!(exited["success"].is_boolean(), "kex-json field kept");
    assert_eq!(ev.last().unwrap()["tone"], "ok");
}

#[cfg(unix)]
#[test]
fn pr_check_without_gh_is_an_error_then_exit_3() {
    let env = Env::new();
    let api = repo_with_remote(&env, "shop/api");
    env.kex_ok(&api, &["init", "--yes"]);
    // A PATH with git and nothing else.
    let bin = env.root().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let git_path = String::from_utf8(
        Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    std::os::unix::fs::symlink(git_path.trim(), bin.join("git")).unwrap();
    let out = env
        .cmd(&api, &["pr", "check", "--json"])
        .env("PATH", &bin)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let ev = events(&stdout(&out));
    let e = find_event(&ev, "error");
    assert_eq!(e["msg"], "gh.missing");
    assert!(e.get("id").is_none());
    assert_eq!(ev.last().unwrap()["tone"], "missing_tool");
}

#[test]
fn manifest_sends_exactly_three_lines_even_with_an_update_known() {
    let env = Env::new();
    let home = env.root().join("kexhome");
    fs::create_dir_all(&home).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    fs::write(
        home.join("update.json"),
        format!("{{\"checked_at\":{now},\"latest\":\"99.0.0\"}}"),
    )
    .unwrap();
    let mut c = env.cmd(env.root(), &["manifest", "--json"]);
    c.env_remove("KEX_NO_UPDATE_CHECK").env_remove("CI");
    let ev = events(&stdout(&c.output().unwrap()));
    let types: Vec<&str> = ev.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(types, ["hello", "manifest", "exit"]);
}

#[test]
fn init_reset_asks_the_new_name_in_json() {
    let env = Env::new();
    let api = env.repo("shop/api");
    env.kex_ok(&api, &["init", "--yes"]);
    let before = read_config(&api);
    let input = "{\"type\":\"answer\",\"id\":\"p1\",\"answer\":\"Bad Name\"}\n{\"type\":\"answer\",\"id\":\"p1\",\"answer\":\"api-copy\"}\n";
    let (ev, code) = kex_stdin(&env, &api, &["init", "--reset", "--json"], input);
    assert_eq!(code, Some(0), "{ev:?}");
    let p = find_event(&ev, "prompt");
    assert_eq!(p["msg"], "init.reset_name");
    assert_eq!(p["pattern"], "^[a-z0-9][a-z0-9-]*$");
    assert!(ev.iter().any(|e| e["type"] == "error" && e["id"] == "p1"));
    let after = read_config(&api);
    assert_eq!(after["repo"], "api-copy");
    assert_ne!(after["repo_id"], before["repo_id"]);
}

#[test]
fn browse_returns_promptly_on_a_big_deep_tree() {
    let env = Env::new();
    let big = env.root().join("big");
    // 6,000 folders below one: more than one folder's walk may visit (5,000).
    for i in 0..60 {
        for j in 0..100 {
            fs::create_dir_all(big.join(format!("deep/a{i}/b{j}"))).unwrap();
        }
    }
    let start = std::time::Instant::now();
    let out = env.kex(&big, &["browse", "--json", "--depth", "1", "--no-git"]);
    assert!(out.status.success());
    assert!(start.elapsed() < std::time::Duration::from_secs(15));
    let tree = find_event(&events(&stdout(&out)), "tree").clone();
    let deep = &tree["tree"][0];
    assert_eq!(deep["name"], "deep");
    assert_eq!(deep["repos_below_truncated"], true, "{deep}");
}
