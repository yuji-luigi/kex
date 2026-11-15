//! cli-events/1 views and messages (spec/cli-events-1.md, docs/self-description.md).
//!
//! Every piece of UI text kex puts in a view or a prompt comes from [`MESSAGES`]: an id and
//! its English template with `{params}`. [`msg`] renders the template, so the text a client
//! shows untranslated is always the template filled in, and the manifest's `messages` is
//! this table. Data (repo names, paths, branch names, titles) is never a message.

use serde_json::{Map, Value, json};

/// id → English template. Ids are stable: change the wording, keep the id; a new meaning
/// gets a new id.
pub const MESSAGES: &[(&str, &str)] = &[
    // columns
    ("col.repo", "Repo"),
    ("col.branch", "Branch"),
    ("col.ahead", "↑"),
    ("col.behind", "↓"),
    ("col.changed", "Changed"),
    ("col.prime", "Prime"),
    ("col.author", "Author"),
    ("col.last_commit", "Last commit"),
    ("col.pr", "PR"),
    ("col.base", "Into"),
    ("col.head", "From"),
    ("col.opened", "Opened"),
    ("col.file", "File"),
    ("col.additions", "+"),
    ("col.deletions", "−"),
    ("col.check", "Check"),
    ("col.state", "State"),
    ("col.runtime", "Runs with"),
    ("col.port", "Port"),
    ("col.reason", "Why"),
    ("col.name", "Name"),
    ("col.path", "Folder"),
    ("col.folder", "Folder"),
    ("col.repo_id", "Id"),
    ("col.prime_branches", "Prime branches"),
    ("col.gitignore", ".gitignore"),
    ("col.stack", "Stack"),
    ("col.addable", "To set up"),
    ("col.already_kex", "Already kex"),
    ("col.missing", "Missing"),
    ("col.project", "Project"),
    ("col.repos", "Repos"),
    ("col.command", "Command"),
    ("col.description", "What it does"),
    // card fields
    ("field.repos", "Repos"),
    ("field.missing", "Not on this computer"),
    ("field.path", "Path"),
    ("field.author", "Author"),
    ("field.branches", "Branches"),
    ("field.url", "Link"),
    ("field.state", "State"),
    ("field.installed", "Installed"),
    ("field.latest", "Latest"),
    ("field.writes", "Writes"),
    ("field.config", "kex.json"),
    ("field.description", "Description"),
    // badges
    ("badge.production", "production"),
    ("badge.draft", "draft"),
    ("badge.worktree", "worktree"),
    ("badge.already_kex", "already in the project"),
    ("badge.changes", "changes"),
    ("badge.new_branch", "new branch"),
    ("badge.booted", "booted"),
    ("badge.needs_gh", "needs gh"),
    ("badge.kex_project", "kex: {project}"),
    ("badge.broken_config", "broken kex.json"),
    ("badge.failed", "failed"),
    // project
    ("project.title", "Project"),
    ("project.missing", "Not on this computer: {repos}"),
    // status
    ("status.title", "Status"),
    ("status.no_upstream", "no upstream"),
    ("status.missing", "Not on this computer: {repos}"),
    // pull
    ("pull.up_to_date", "✓ up to date"),
    ("pull.pulled", "↓ {n} commits pulled"),
    ("pull.pulled_one", "↓ {n} commit pulled"),
    ("pull.dirty", "⚠ uncommitted changes, pull skipped"),
    (
        "pull.diverged",
        "⚠ diverged ({ahead} ahead, {behind} behind), pull skipped",
    ),
    ("pull.no_upstream", "no upstream, pull skipped"),
    ("pull.fetch_failed", "✗ fetch failed: {error}"),
    (
        "pull.switch_failed",
        "✗ could not switch to {branch}: {error}",
    ),
    ("pull.failed", "✗ pull failed: {error}"),
    ("pull.unmerged", "Not merged yet"),
    ("pull.no_remote", "no remote"),
    ("pull.no_prime", "no prime branch found on {remote}"),
    // gh
    ("gh.missing", "The GitHub CLI (gh) isn't installed"),
    (
        "gh.logged_out",
        "The GitHub CLI (gh) is installed but not logged in",
    ),
    // prs and a PR
    ("prs.title", "Open PRs"),
    ("prs.empty", "No open PRs"),
    ("pr.pick", "Open PRs"),
    ("pr.quit", "Quit"),
    ("pr.action", "#{number} {title}"),
    ("pr.merge", "Merge"),
    ("pr.diff", "Show diff"),
    ("pr.back", "Back"),
    (
        "pr.confirm_merge",
        "Merge #{number} \"{title}\" ({repo}) into {base}?",
    ),
    ("pr.typed", "Type `{word}` to merge:"),
    ("pr.merged", "#{number} merged into {base}"),
    (
        "pr.merge_pending",
        "#{number} merge requested, but it is still {state}",
    ),
    ("pr.body", "Description"),
    ("pr.files", "Files changed"),
    ("pr.checks", "Checks"),
    // push
    ("push.pick", "Push the current branch of which repos?"),
    ("push.confirm", "Push these?"),
    (
        "push.typed",
        "{repo}: {branch} is protected. Type `{branch}` to push:",
    ),
    ("push.done", "Pushed {branch} ({repo})"),
    ("push.failed", "Push of {branch} failed ({repo})"),
    // run
    ("run.pick", "Run which repos?"),
    ("run.simulator", "iOS simulator"),
    ("run.hot_reload", "Hot reload"),
    ("run.hot_restart", "Hot restart"),
    ("run.quit_app", "Quit"),
    ("run.bad_line", "expected a JSON object"),
    ("run.input_needs_repo", "input needs \"repo\""),
    ("run.no_service", "no running service \"{repo}\""),
    (
        "run.unknown_message",
        "unknown message; expected type input or stop",
    ),
    ("clones.pick", "{repo} has {n} clones. Which one?"),
    ("clones.remember", "Remember this choice?"),
    // versions
    ("update.available", "kex {latest} is available"),
    ("update.latest", "kex {current} is the latest version"),
    ("update.install", "Install {version}"),
    ("update.install_confirm", "Install kex {version} now?"),
    ("update.confirm", "Install kex {version}?"),
    ("update.done", "Updated to kex {version}"),
    // menu
    ("menu.pick", "kex"),
    ("menu.pull", "update all repos + show unmerged work"),
    ("menu.run", "start services"),
    ("menu.pr", "review and merge PRs"),
    ("menu.status", "git status of every repo"),
    ("menu.push", "push current branch of selected repos"),
    ("menu.doctor", "what kex knows about this project"),
    ("menu.update", "install kex {version} (you have {current})"),
    // init
    ("init.project", "Project"),
    ("init.join", "Join {project} ({repos})"),
    ("init.new_project", "Create a new project"),
    ("init.new_project_name", "New project name:"),
    ("init.repo_name", "Repo name for {folder}:"),
    ("init.found_workspace", "Found {file} in {dir}. Use it?"),
    (
        "init.pick_workspace",
        "Found {n} workspace files. Use which one?",
    ),
    ("init.no_workspace", "Don't use a workspace file"),
    (
        "init.use_workspace_one",
        "Init this repo as a kex workspace? {names}",
    ),
    (
        "init.use_workspace",
        "Init these {n} repos as a kex workspace? {names}",
    ),
    ("init.use_names", "Use these repo names? {names}"),
    ("init.reset_name", "New repo name:"),
    // added / plan
    ("added.title", "Set up"),
    ("added.summary", "{n} repos set up in {project}"),
    ("added.summary_one", "{n} repo set up in {project}"),
    (
        "added.nothing",
        "Nothing new: every repo is already in {project}",
    ),
    ("added.failed", "Nothing was written (all or nothing)"),
    (
        "added.commit_hint",
        "Check the prime branches, then commit kex.json in each repo.",
    ),
    ("plan.title", "Will set up"),
    ("plan.new_project", "New project {id}"),
    ("plan.project", "Project {id}"),
    ("plan.invalid", "Can't set these up: see the errors"),
    ("plan.apply", "Write {n} configs"),
    ("plan.apply_one", "Write {n} config"),
    ("plan.apply_confirm", "Write {n} kex.json into {project}?"),
    // browse
    ("tree.up", "../"),
    ("tree.next", "More"),
    ("tree.repos_below", "{n} repos"),
    ("tree.repos_below_one", "{n} repo"),
    ("tree.link", "→ {target}"),
    ("tree.link_missing", "→ {target} (missing)"),
    (
        "tree.git_partial",
        "Some repos have no git details (time budget)",
    ),
    ("workspaces.title", "Workspace files"),
    ("workspaces.empty", "No workspace files here"),
    ("workspaces.use", "Set up its {n} repos"),
    (
        "workspaces.use_confirm",
        "Check the {n} repos of {file} (nothing is written yet)?",
    ),
    // --from problems
    ("from.not_json", "not valid JSON: {error}"),
    ("from.missing", "missing"),
    ("from.must_be_string", "must be a string"),
    ("from.path_empty", "must be a non-empty string"),
    ("from.no_path", "{path} doesn't exist"),
    ("from.not_folder", "{path} isn't a folder"),
    ("from.not_git_repo", "{path} isn't a git repo"),
    (
        "from.not_top",
        "{path} isn't the top folder of a git repo (that is {top})",
    ),
    ("from.same_repo", "the same repo as repos[{index}]"),
    (
        "from.worktree",
        "{path} is a linked worktree; add its main checkout instead{main}",
    ),
    ("from.too_big", "the request is over {limit} bytes"),
    (
        "from.unknown_field",
        "unknown field \"{field}\" (expected project, repos)",
    ),
    ("from.not_array", "must be an array"),
    (
        "from.bad_shape",
        "expected an array of repos or an object {project, repos}",
    ),
    ("from.no_repos", "no repos given"),
    (
        "from.other_project",
        "already a kex repo in project {project} (as \"{repo}\"); kex doesn't move repos between projects",
    ),
    (
        "from.not_object",
        "must be an object {path, name?, description?}",
    ),
    (
        "from.unknown_repo_field",
        "unknown field \"{field}\" (expected path, name, description, repo_id)",
    ),
    (
        "from.bad_repo_id",
        "\"{id}\" isn't a repo id: r_ and 1 to 32 lowercase letters or digits",
    ),
    ("from.repo_id_used", "{id} is already used by another repo"),
    (
        "from.repo_id_twice",
        "{id} is given twice (repos[{index}] too)",
    ),
    (
        "from.unknown_project",
        "unknown project \"{id}\" (kex knows: {known}); use {\"new\": \"<name>\"} to create one",
    ),
    (
        "from.project_twice",
        "\"{id}\" here but --project {flag}; pass one",
    ),
    (
        "from.unknown_project_field",
        "unknown field \"{field}\" (expected new, id)",
    ),
    ("from.id_not_string", "\"id\" must be a string"),
    ("from.new_needs_name", "{\"new\": …} needs a non-empty name"),
    (
        "from.bad_project_id",
        "\"{id}\" isn't a project id: lowercase letters, digits and -",
    ),
    (
        "from.bad_project",
        "must be a project id or {\"new\": \"<name>\"}",
    ),
];

pub fn template(id: &str) -> Option<&'static str> {
    MESSAGES.iter().find(|(k, _)| *k == id).map(|(_, t)| *t)
}

/// Fill `{name}` placeholders, in one pass (a value containing `{x}` stays as it is).
pub fn render(template: &str, params: &Value) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(j) = after.find('}') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let name = &after[..j];
        match params.get(name) {
            Some(Value::String(s)) => out.push_str(s),
            Some(Value::Null) => {}
            Some(v) => out.push_str(&v.to_string()),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    out
}

/// UI text: `{"text", "msg", "params"?}`. An unknown id is a bug (tests catch it); it
/// still renders, as the id.
pub fn msg(id: &str, params: Value) -> Value {
    debug_assert!(template(id).is_some(), "unknown message id {id}");
    let text = render(template(id).unwrap_or(id), &params);
    let mut v = json!({ "text": text, "msg": id });
    if params.as_object().is_some_and(|p| !p.is_empty()) {
        v["params"] = params;
    }
    v
}

/// A message without params.
pub fn m(id: &str) -> Value {
    msg(id, Value::Null)
}

/// A column: its label is `col.<key>`.
pub fn col(key: &str, ty: &str) -> Value {
    let mut c = json!({ "key": key, "label": m(&format!("col.{key}")) });
    if ty != "text" {
        c["type"] = json!(ty);
    }
    c
}

pub fn badge(name: &str, tone: &str) -> Value {
    json!({ "text": m(&format!("badge.{name}")), "tone": tone })
}

pub fn badge_p(name: &str, params: Value, tone: &str) -> Value {
    json!({ "text": msg(&format!("badge.{name}"), params), "tone": tone })
}

/// A cell with a tone, or with a text shown instead of its value.
pub fn cell(value: Value, tone: Option<&str>, text: Option<Value>) -> Value {
    let mut c = json!({ "value": value });
    if let Some(t) = tone {
        c["tone"] = json!(t);
    }
    if let Some(t) = text {
        c["text"] = t;
    }
    c
}

/// A card field; its label is `field.<key>` unless `label` is given.
pub fn field(key: &str, value: Value, ty: &str) -> Value {
    let mut f = json!({ "key": key, "label": m(&format!("field.{key}")), "value": value });
    if ty != "text" {
        f["type"] = json!(ty);
    }
    f
}

pub fn note(level: &str, text: Value) -> Value {
    json!({ "level": level, "text": text })
}

/// `{"kind": kind, ...fields}`.
pub fn view(kind: &str, fields: Value) -> Value {
    let mut v = Map::new();
    v.insert("kind".into(), json!(kind));
    if let Value::Object(f) = fields {
        v.extend(f);
    }
    Value::Object(v)
}

/// A count cell: muted at 0, `tone` otherwise.
pub fn count(n: u64, tone: &str) -> Value {
    if n == 0 {
        json!(0)
    } else {
        cell(json!(n), Some(tone), None)
    }
}

/// The placeholders of a template, in order.
#[cfg(test)]
pub fn placeholders(template: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        let after = &rest[i + 1..];
        match after.find('}') {
            Some(j) => {
                out.push(&after[..j]);
                rest = &after[j + 1..];
            }
            None => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_ids_follow_the_standard_and_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (id, t) in MESSAGES {
            assert!(seen.insert(*id), "{id} twice");
            let ok = id.split('.').enumerate().all(|(i, seg)| {
                !seg.is_empty()
                    && seg
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                    && (i > 0 || seg.starts_with(|c: char| c.is_ascii_lowercase()))
            });
            assert!(ok, "bad id {id}");
            assert!(!t.is_empty());
        }
    }

    #[test]
    fn renders_params() {
        assert_eq!(
            msg("pull.pulled", json!({"n": 3})),
            json!({"text": "↓ 3 commits pulled", "msg": "pull.pulled", "params": {"n": 3}})
        );
        assert_eq!(m("pr.quit"), json!({"text": "Quit", "msg": "pr.quit"}));
        assert_eq!(
            placeholders("{repo}: {branch} x `{branch}`"),
            ["repo", "branch", "branch"]
        );
    }
}
