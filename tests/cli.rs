//! End-to-end tests: real git repos in a temp folder, the real `kex` binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

    fn kex(&self, dir: &Path, args: &[&str]) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_kex"));
        c.args(args)
            .current_dir(dir)
            .env("KEX_HOME", self.root().join("kexhome"))
            .env("NO_COLOR", "1");
        isolate_git(&mut c);
        c.output().unwrap()
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

fn status_json(env: &Env, dir: &Path) -> serde_json::Value {
    serde_json::from_str(&env.kex_ok(dir, &["status", "--json"])).unwrap()
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
