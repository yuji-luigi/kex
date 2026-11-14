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
        self.cmd(dir, args).output().unwrap()
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

/// `--json` prints one event per line; every line must be JSON.
fn events(text: &str) -> Vec<serde_json::Value> {
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON ({e}): {l}")))
        .collect()
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
fn init_json_asks_nothing() {
    let env = Env::new();
    let api = env.repo("shop/api");
    for cmd in ["init", "add"] {
        let out = env
            .cmd(&api, &[cmd, "--json"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
        let ev = events(&stdout(&out));
        assert!(!ev.iter().any(|e| e["type"] == "prompt"));
        let e = find_event(&ev, "error");
        let m = e["message"].as_str().unwrap();
        assert!(
            m.contains("kex browse --json") && m.contains("--from -"),
            "{m}"
        );
        assert_eq!(ev.last().unwrap()["code"], 2);
    }
    assert!(!api.join("kex.json").exists());

    // Without a terminal (and not JSON), plain `kex init` sets up this repo, as in 0.1.
    env.kex_ok(&api, &["init"]);
    assert_eq!(read_config(&api)["repo"], "api");
    // Outside a repo there's nothing to set up without picking.
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
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
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
        fs::canonicalize(&shop).unwrap().to_str().unwrap()
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
        fs::canonicalize(&ext).unwrap().to_str().unwrap()
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
        gone["resolved"].as_str().unwrap().ends_with("shop/gone"),
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
        fs::canonicalize(&deep).unwrap().to_str().unwrap()
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
