//! The cli-events/1 manifest (docs/self-description.md §2): generated from clap plus one
//! small annotation table, so it can't drift from the real commands (a unit test checks
//! that every visible command and option has an annotation, and every annotation names
//! one).

use std::any::TypeId;
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::Result;
use clap::{ArgAction, CommandFactory};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::{ui, view};

pub const STANDARD: &str = "cli-events/1";
pub const MANIFEST_VERSION: u32 = 1;

/// One command's annotations: what clap can't know.
struct Cmd {
    id: &'static str,
    label: &'static str,
    group: &'static str,
    icon: &'static str,
    interactive: bool,
    long_running: bool,
    read_only: bool,
    needs_terminal: bool,
    stop: bool,
    json_input: bool,
    opts: &'static [Opt],
}

const C: Cmd = Cmd {
    id: "",
    label: "",
    group: "",
    icon: "",
    interactive: false,
    long_running: false,
    read_only: false,
    needs_terminal: false,
    stop: false,
    json_input: false,
    opts: &[],
};

/// One option's annotations.
struct Opt {
    id: &'static str,
    label: &'static str,
    ty: Option<&'static str>,
    placeholder: Option<&'static str>,
    pattern: Option<&'static str>,
    min_length: Option<u32>,
    max_length: Option<u32>,
    max: Option<u64>,
    role: Option<&'static str>,
    stdin_value: Option<&'static str>,
}

const O: Opt = Opt {
    id: "",
    label: "",
    ty: None,
    placeholder: None,
    pattern: None,
    min_length: None,
    max_length: None,
    max: None,
    role: None,
    stdin_value: None,
};

const NAME: &str = "^[a-z0-9][a-z0-9-]*$";

const COMMANDS: &[Cmd] = &[
    Cmd {
        id: "init",
        label: "Set up repos",
        group: "setup",
        icon: "init",
        interactive: true,
        json_input: true,
        opts: &[
            Opt {
                id: "dir",
                label: "Folder",
                ..O
            },
            Opt {
                id: "project",
                label: "Project",
                pattern: Some(NAME),
                max_length: Some(47),
                ..O
            },
            Opt {
                id: "repo",
                label: "Repo name",
                pattern: Some(NAME),
                min_length: Some(1),
                max_length: Some(40),
                ..O
            },
            Opt {
                id: "yes",
                label: "Don't ask",
                ..O
            },
            Opt {
                id: "reset",
                label: "New ids for a copied config",
                ..O
            },
            Opt {
                id: "repos",
                label: "Folders",
                placeholder: Some("api,web"),
                ..O
            },
            Opt {
                id: "workspace",
                label: "Workspace file",
                placeholder: Some("shop.code-workspace"),
                ..O
            },
            Opt {
                id: "from",
                label: "Repos (JSON)",
                ty: Some("json"),
                stdin_value: Some("-"),
                ..O
            },
            Opt {
                id: "dry_run",
                label: "Only show what would be written",
                role: Some("dry_run"),
                ..O
            },
            Opt {
                id: "depth",
                label: "Browser depth",
                ..O
            },
        ],
        ..C
    },
    Cmd {
        id: "browse",
        label: "Browse folders",
        group: "setup",
        icon: "folder",
        read_only: true,
        opts: &[
            Opt {
                id: "dir",
                label: "Folder",
                ..O
            },
            Opt {
                id: "depth",
                label: "Depth",
                ..O
            },
            Opt {
                id: "max_entries",
                label: "At most",
                max: Some(crate::browse::MAX_ENTRIES as u64),
                ..O
            },
            Opt {
                id: "repos_only",
                label: "Only repos, flat",
                ..O
            },
            Opt {
                id: "no_git",
                label: "Skip git details (fast)",
                ..O
            },
            Opt {
                id: "git_budget_ms",
                label: "Git time budget (ms)",
                ..O
            },
            Opt {
                id: "hidden",
                label: "Show dot-folders",
                ..O
            },
            Opt {
                id: "all",
                label: "Also node_modules, target, …",
                ..O
            },
            Opt {
                id: "offset",
                label: "Skip",
                ..O
            },
        ],
        ..C
    },
    Cmd {
        id: "pull",
        label: "Pull",
        group: "sync",
        icon: "pull",
        opts: &[
            Opt {
                id: "all",
                label: "Also old branches",
                ..O
            },
            Opt {
                id: "prime",
                label: "Switch to the prime branch first",
                ..O
            },
        ],
        ..C
    },
    Cmd {
        id: "status",
        label: "Status",
        group: "sync",
        icon: "status",
        ..C
    },
    Cmd {
        id: "run",
        label: "Run",
        group: "run",
        icon: "run",
        interactive: true,
        long_running: true,
        stop: true,
        opts: &[
            Opt {
                id: "repos",
                label: "Repos",
                pattern: Some(NAME),
                ..O
            },
            Opt {
                id: "no_update",
                label: "Skip pull + install",
                ..O
            },
            Opt {
                id: "install",
                label: "Always install",
                ..O
            },
            Opt {
                id: "sim",
                label: "iOS simulator",
                placeholder: Some("iPhone 17 Pro"),
                ..O
            },
            Opt {
                id: "force",
                label: "Start even if the port is taken",
                ..O
            },
        ],
        ..C
    },
    Cmd {
        id: "push",
        label: "Push",
        group: "sync",
        icon: "push",
        interactive: true,
        opts: &[
            Opt {
                id: "repos",
                label: "Repos",
                pattern: Some(NAME),
                ..O
            },
            Opt {
                id: "yes",
                label: "Don't ask",
                ..O
            },
        ],
        ..C
    },
    Cmd {
        id: "pr.check",
        label: "Review PRs",
        group: "review",
        icon: "pr",
        interactive: true,
        opts: &[Opt {
            id: "list",
            label: "Only list",
            ..O
        }],
        ..C
    },
    Cmd {
        id: "doctor",
        label: "Doctor",
        group: "info",
        icon: "doctor",
        ..C
    },
    Cmd {
        id: "scan",
        label: "Scan for repos",
        group: "setup",
        icon: "scan",
        opts: &[Opt {
            id: "dir",
            label: "Folder",
            ..O
        }],
        ..C
    },
    Cmd {
        id: "link",
        label: "Link a repo",
        group: "setup",
        icon: "link",
        opts: &[Opt {
            id: "path",
            label: "Repo",
            ..O
        }],
        ..C
    },
    Cmd {
        id: "completions",
        label: "Shell completions",
        group: "meta",
        icon: "terminal",
        needs_terminal: true,
        opts: &[Opt {
            id: "shell",
            label: "Shell",
            ..O
        }],
        ..C
    },
    Cmd {
        id: "update",
        label: "Update kex",
        group: "meta",
        icon: "update",
        interactive: true,
        opts: &[
            Opt {
                id: "check",
                label: "Only check",
                ..O
            },
            Opt {
                id: "yes",
                label: "Don't ask",
                ..O
            },
        ],
        ..C
    },
    Cmd {
        id: "manifest",
        label: "Manifest",
        group: "meta",
        icon: "manifest",
        read_only: true,
        ..C
    },
];

const EXIT_CODES: &[(u8, &str, &str, &str)] = &[
    (0, "ok", "Done", "Everything worked."),
    (
        1,
        "partial",
        "Some repos failed",
        "The others worked: see their results.",
    ),
    (
        2,
        "error",
        "Error",
        "A config, usage or network problem: see the error.",
    ),
    (
        3,
        "missing_tool",
        "A tool is missing",
        "kex needs a tool that isn't installed (e.g. gh): see the error.",
    ),
    (130, "cancelled", "Cancelled", "Stopped before the end."),
];

/// The tone of an exit code (`exit.tone`).
pub fn exit_tone(code: u8) -> &'static str {
    EXIT_CODES
        .iter()
        .find(|e| e.0 == code)
        .map_or("error", |e| e.1)
}

const EVENT_TYPES: &[(&str, &str, &str)] = &[
    (
        "project",
        "card",
        "The project kex works on: its repos, the missing ones.",
    ),
    (
        "status",
        "table",
        "kex status: branch, ahead/behind and changes of every repo.",
    ),
    (
        "pull",
        "card",
        "kex pull (and run's update step): one repo, with its unmerged branches.",
    ),
    ("prs", "table", "kex pr check: the open PRs of every repo."),
    ("pr", "card", "kex pr check: one PR's detail."),
    (
        "merged",
        "log",
        "kex pr check: a PR was merged (or the merge is pending).",
    ),
    ("pushed", "log", "kex push: one repo pushed, or not."),
    ("started", "service", "kex run: a service started."),
    (
        "output",
        "service",
        "kex run: one line of a service's output (append).",
    ),
    (
        "ready",
        "service",
        "kex run: a service answers on its URL (append).",
    ),
    (
        "stopping",
        "service",
        "kex run: a service is being stopped (append).",
    ),
    ("exited", "service", "kex run: a service exited (append)."),
    (
        "version",
        "card",
        "kex update: the installed and the latest version.",
    ),
    ("update_available", "card", "A newer kex is out."),
    ("updated", "card", "kex update: installed."),
    ("added", "table", "kex init: the repos set up."),
    (
        "plan",
        "table",
        "kex init --dry-run: what would be written.",
    ),
    ("tree", "tree", "kex browse: a folder as a tree."),
    (
        "workspaces",
        "cards",
        "kex browse: the workspace files around the folder.",
    ),
];

const VIEW_KINDS: &[&str] = &["card", "table", "cards", "tree", "service", "log"];
const VALUE_TYPES: &[&str] = &[
    "text", "number", "count", "branch", "path", "url", "date", "badge", "code",
];

/// The `--from` request (docs/json-protocol.md), object form.
fn from_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["repos"],
        "additionalProperties": false,
        "properties": {
            "project": {"oneOf": [
                {"type": "string", "title": "Join this project", "pattern": NAME, "maxLength": 47},
                {"type": "object", "title": "A new project", "required": ["new"],
                 "additionalProperties": false,
                 "properties": {
                    "new": {"type": "string", "title": "Name", "minLength": 1, "maxLength": 40},
                    "id": {"type": "string", "pattern": NAME, "maxLength": 47}}}
            ]},
            "repos": {"type": "array", "minItems": 1, "items": {
                "type": "object", "required": ["path"], "additionalProperties": false,
                "properties": {
                    "path": {"type": "string", "title": "Folder", "minLength": 1},
                    "name": {"type": "string", "title": "Repo name", "pattern": NAME,
                             "minLength": 1, "maxLength": 40},
                    "description": {"type": "string", "title": "Description", "maxLength": 200,
                                    "pattern": "^[^\\u0000-\\u001f\\u007f]*$"},
                    "repo_id": {"type": "string", "pattern": "^r_[a-z0-9]{1,32}$"}}}}
        }
    })
}

fn first_line(s: Option<&clap::builder::StyledStr>) -> String {
    s.map(|s| s.to_string())
        .unwrap_or_default()
        .lines()
        .next()
        .map(first_clause)
        .unwrap_or_default()
}

/// A help line up to its first "; " outside parentheses: "Set up repos …; --yes sets up only
/// this repo" → "Set up repos …", but "… (default 3; with --repos-only: no limit)" stays whole.
fn first_clause(line: &str) -> String {
    let mut depth = 0i32;
    for (i, c) in line.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ';' if depth == 0 && line[i + 1..].starts_with(' ') => return line[..i].trim().into(),
            _ => {}
        }
    }
    line.trim().to_string()
}

/// The leaf commands of the CLI, with their words: `(["pr", "check"], cmd)`.
fn leaves(cmd: &clap::Command) -> Vec<(Vec<String>, &clap::Command)> {
    fn walk<'a>(
        c: &'a clap::Command,
        words: Vec<String>,
        out: &mut Vec<(Vec<String>, &'a clap::Command)>,
    ) {
        for s in c
            .get_subcommands()
            .filter(|s| !s.is_hide_set() && s.get_name() != "help")
        {
            let mut w = words.clone();
            w.push(s.get_name().to_string());
            if s.has_subcommands() {
                walk(s, w, out);
            } else {
                out.push((w, s));
            }
        }
    }
    let mut out = Vec::new();
    walk(cmd, Vec::new(), &mut out);
    out
}

fn visible_args(c: &clap::Command) -> impl Iterator<Item = &clap::Arg> {
    c.get_arguments().filter(|a| {
        !a.is_hide_set() && a.get_id() != "help" && a.get_id() != "version" && !a.is_global_set()
    })
}

fn option_json(c: &clap::Command, a: &clap::Arg, ann: &Opt, positional: Option<usize>) -> Value {
    let mut o = Map::new();
    o.insert("id".into(), json!(a.get_id().as_str()));
    match positional {
        Some(i) => {
            o.insert("positional".into(), json!(i));
        }
        None => {
            if let Some(l) = a.get_long() {
                o.insert("flag".into(), json!(format!("--{l}")));
            }
            if let Some(s) = a.get_short() {
                o.insert("short".into(), json!(format!("-{s}")));
            }
        }
    }
    let tid = a.get_value_parser().type_id();
    let integer = [
        TypeId::of::<u8>(),
        TypeId::of::<u16>(),
        TypeId::of::<u32>(),
        TypeId::of::<u64>(),
        TypeId::of::<usize>(),
    ]
    .iter()
    .any(|t| tid == *t);
    let choices = a.get_possible_values();
    let ty = if let Some(t) = ann.ty {
        t
    } else if matches!(a.get_action(), ArgAction::SetTrue | ArgAction::SetFalse) {
        "boolean"
    } else if a.get_value_delimiter().is_some() {
        "list"
    } else if !choices.is_empty() {
        "choice"
    } else if tid == TypeId::of::<PathBuf>() {
        "path"
    } else if integer {
        "integer"
    } else {
        "string"
    };
    o.insert("type".into(), json!(ty));
    o.insert("label".into(), json!(ann.label));
    let help = first_line(a.get_help());
    if !help.is_empty() {
        o.insert("description".into(), json!(help));
    }
    if let Some(p) = ann.placeholder {
        o.insert("placeholder".into(), json!(p));
    }
    if a.is_required_set() {
        o.insert("required".into(), json!(true));
    }
    if ty != "boolean"
        && let Some(d) = a.get_default_values().first()
    {
        let d = d.to_string_lossy();
        o.insert(
            "default".into(),
            if integer {
                d.parse::<u64>().map_or(json!(d), |n| json!(n))
            } else {
                json!(d)
            },
        );
    }
    if ty == "choice" {
        let cs: Vec<Value> = choices
            .iter()
            .filter(|p| !p.is_hide_set())
            .map(|p| json!({ "value": p.get_name(), "label": p.get_name() }))
            .collect();
        o.insert("choices".into(), json!(cs));
    }
    if let Some(p) = ann.pattern {
        o.insert("pattern".into(), json!(p));
    }
    if let Some(n) = ann.min_length {
        o.insert("min_length".into(), json!(n));
    }
    if let Some(n) = ann.max_length {
        o.insert("max_length".into(), json!(n));
    }
    if integer {
        o.insert("min".into(), json!(0));
    }
    if let Some(n) = ann.max {
        o.insert("max".into(), json!(n));
    }
    if let Some(sep) = a.get_value_delimiter() {
        o.insert("separator".into(), json!(sep.to_string()));
    }
    // clap checks conflicts both ways; list them both ways.
    let mut conflicts: Vec<String> = c
        .get_arguments()
        .filter(|b| b.get_id() != a.get_id() && !b.is_hide_set())
        .filter(|b| {
            c.get_arg_conflicts_with(a)
                .iter()
                .any(|x| x.get_id() == b.get_id())
                || c.get_arg_conflicts_with(b)
                    .iter()
                    .any(|x| x.get_id() == a.get_id())
        })
        .map(|b| b.get_id().to_string())
        .collect();
    conflicts.sort();
    if !conflicts.is_empty() {
        o.insert("conflicts_with".into(), json!(conflicts));
    }
    if let Some(r) = ann.role {
        o.insert("role".into(), json!(r));
    }
    if let Some(s) = ann.stdin_value {
        o.insert("stdin_value".into(), json!(s));
    }
    Value::Object(o)
}

/// The manifest object (the value of the `manifest` line's `manifest` field).
pub fn build() -> Value {
    let cli = crate::Cli::command();
    let mut commands = Vec::new();
    for (words, c) in leaves(&cli) {
        let id = words.join(".");
        let Some(ann) = COMMANDS.iter().find(|a| a.id == id) else {
            // The drift test fails on this; in a release, leave it out.
            continue;
        };
        let mut positional = 0;
        let options: Vec<Value> = visible_args(c)
            .filter_map(|a| {
                let oa = ann.opts.iter().find(|o| o.id == a.get_id())?;
                let pos = a.is_positional().then(|| {
                    positional += 1;
                    positional - 1
                });
                Some(option_json(c, a, oa, pos))
            })
            .collect();
        let mut v = json!({
            "id": id,
            "words": words,
            "label": ann.label,
            "description": first_line(c.get_about()),
            "group": ann.group,
            "icon": ann.icon,
            "interactive": ann.interactive,
            "long_running": ann.long_running,
            "options": options,
        });
        let aliases: Vec<Vec<String>> = c
            .get_visible_aliases()
            .map(|al| {
                let mut w = words.clone();
                *w.last_mut().unwrap() = al.to_string();
                w
            })
            .collect();
        if !aliases.is_empty() {
            v["aliases"] = json!(aliases);
        }
        if ann.read_only {
            v["read_only"] = json!(true);
        }
        if ann.needs_terminal {
            v["needs_terminal"] = json!(true);
        }
        if ann.stop {
            v["stop"] = json!({ "type": "stop" });
        }
        if ann.json_input {
            v["json_input"] = json!({
                "option": "from",
                "schema": from_schema(),
                "dry_run": "dry_run",
                "picker": {
                    "command": "browse",
                    "options": { "depth": 3 },
                    "select_kinds": ["repo", "symlink"],
                    "into": "/repos",
                },
            });
        }
        commands.push(v);
    }
    let json_flag = cli
        .get_arguments()
        .find(|a| a.get_id() == "json")
        .map(|a| first_line(a.get_help()))
        .unwrap_or_default();
    json!({
        "manifest_version": MANIFEST_VERSION,
        "standard": STANDARD,
        "tool": {
            "name": "kex",
            "version": env!("CARGO_PKG_VERSION"),
            "label": "kex",
            "description": env!("CARGO_PKG_DESCRIPTION"),
            "homepage": env!("CARGO_PKG_REPOSITORY"),
        },
        "features": ui::FEATURES,
        "global_options": [{
            "id": "json", "flag": "--json", "type": "boolean", "label": "JSON output",
            "description": json_flag, "role": "json",
        }],
        "commands": commands,
        "exit_codes": EXIT_CODES.iter().map(|(c, t, title, d)| json!({
            "code": c, "tone": t, "title": title, "description": d,
        })).collect::<Vec<_>>(),
        "views": { "kinds": VIEW_KINDS, "value_types": VALUE_TYPES },
        "event_types": EVENT_TYPES.iter().map(|(t, k, d)| json!({
            "type": t, "view_kind": k, "description": d,
        })).collect::<Vec<_>>(),
        "messages": view::MESSAGES.iter().map(|(k, t)| (k.to_string(), json!(t))).collect::<Map<_, _>>(),
    })
}

/// RFC 8785 (JCS): keys sorted by UTF-16 code units, no whitespace. kex's manifest holds
/// only strings, integers, booleans, arrays and objects; integers print as JSON does.
pub fn canonical(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).unwrap_or_default());
                out.push(':');
                write_canonical(&o[k.as_str()], out);
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(x, out);
            }
            out.push(']');
        }
        // serde_json escapes strings as JCS does (\" \\ \b \f \n \r \t, other controls as
        // lowercase \u00xx, everything else as UTF-8).
        other => out.push_str(&other.to_string()),
    }
}

/// `hello.manifest_hash`: the first 16 hex characters of SHA-256 over the canonical
/// manifest. Computed once per process.
pub fn hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| hash_of(&build()))
}

pub fn hash_of(manifest: &Value) -> String {
    let digest = Sha256::digest(canonical(manifest).as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// `kex manifest`.
pub fn command() -> Result<u8> {
    let m = build();
    if ui::json() {
        ui::emit("manifest", json!({ "manifest": m }));
        return Ok(0);
    }
    println!(
        "kex {} · {STANDARD} · manifest {MANIFEST_VERSION} · {}",
        env!("CARGO_PKG_VERSION"),
        hash()
    );
    let commands = m["commands"].as_array().cloned().unwrap_or_default();
    let mut groups: Vec<&str> = Vec::new();
    for c in &commands {
        let g = c["group"].as_str().unwrap_or("");
        if !groups.contains(&g) {
            groups.push(g);
        }
    }
    for g in groups {
        println!("{g}");
        for c in commands.iter().filter(|c| c["group"] == g) {
            let mut words = c["words"]
                .as_array()
                .map(|w| {
                    w.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            if let Some(a) = c["aliases"].as_array() {
                let al: Vec<String> = a
                    .iter()
                    .filter_map(|w| w.as_array()?.last()?.as_str().map(String::from))
                    .collect();
                words = format!("{words} ({})", al.join(", "));
            }
            let mut notes = Vec::new();
            if c["interactive"] == true {
                notes.push("interactive".to_string());
            }
            if c["long_running"] == true {
                notes.push("long-running".to_string());
            }
            if c["needs_terminal"] == true {
                notes.push("terminal only".to_string());
            }
            if c.get("json_input").is_some() {
                notes.push("JSON input --from".to_string());
            }
            println!(
                "  {words:<12} {}{}",
                c["description"].as_str().unwrap_or(""),
                if notes.is_empty() {
                    String::new()
                } else {
                    format!("  [{}]", notes.join(" · "))
                }
            );
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every visible clap command and option has an annotation, and every annotation names
    /// a command or option clap has.
    #[test]
    fn annotations_match_the_cli() {
        let cli = crate::Cli::command();
        let leaves = leaves(&cli);
        for (words, c) in &leaves {
            let id = words.join(".");
            let ann = COMMANDS
                .iter()
                .find(|a| a.id == id)
                .unwrap_or_else(|| panic!("command {id} has no annotation in src/manifest.rs"));
            for a in visible_args(c) {
                assert!(
                    ann.opts.iter().any(|o| o.id == a.get_id()),
                    "option {id} {} has no annotation",
                    a.get_id()
                );
            }
            for o in ann.opts {
                assert!(
                    visible_args(c).any(|a| a.get_id() == o.id),
                    "annotation {id} {} names no option",
                    o.id
                );
            }
        }
        for a in COMMANDS {
            assert!(
                leaves.iter().any(|(w, _)| w.join(".") == a.id),
                "annotation {} names no command",
                a.id
            );
        }
    }

    #[test]
    fn the_manifest_validates_and_its_references_exist() {
        let m = build();
        let schema: Value =
            serde_json::from_str(include_str!("../spec/cli-events-1.schema.json")).unwrap();
        let v = jsonschema::validator_for(&schema).unwrap();
        let line = json!({ "type": "manifest", "manifest": m });
        let errors: Vec<String> = v.iter_errors(&line).map(|e| e.to_string()).collect();
        assert!(errors.is_empty(), "{errors:#?}");
        let mut ids = std::collections::HashSet::new();
        for c in m["commands"].as_array().unwrap() {
            assert!(ids.insert(c["id"].as_str().unwrap()), "command id twice");
            let opts = c["options"].as_array().unwrap();
            let names: Vec<&str> = opts.iter().map(|o| o["id"].as_str().unwrap()).collect();
            for o in opts {
                for r in o["conflicts_with"].as_array().into_iter().flatten() {
                    assert!(names.contains(&r.as_str().unwrap()), "{c} {o}");
                }
            }
        }
        assert!(canonical(&m).len() < 512 * 1024);
    }

    #[test]
    fn descriptions_end_at_a_clause_outside_parentheses() {
        assert_eq!(first_clause("Set up (x); --yes this"), "Set up (x)");
        assert_eq!(
            first_clause("How many levels (default 3; with --repos-only: no limit)"),
            "How many levels (default 3; with --repos-only: no limit)"
        );
    }

    #[test]
    fn canonical_json_sorts_keys_and_drops_whitespace() {
        let v = json!({ "b": [1, {"z": true, "a": null}], "a": "é\n\u{1}", "A": 0 });
        assert_eq!(
            canonical(&v),
            r#"{"A":0,"a":"é\n\u0001","b":[1,{"a":null,"z":true}]}"#
        );
        // Key order in the input doesn't change the hash.
        let w = json!({ "A": 0, "a": "é\n\u{1}", "b": [1, {"a": null, "z": true}] });
        assert_eq!(hash_of(&v), hash_of(&w));
        assert_eq!(hash_of(&v).len(), 16);
    }
}
