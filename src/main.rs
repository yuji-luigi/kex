#[macro_use]
mod ui;
mod browse;
mod cancel;
mod config;
mod doctor;
mod flutter;
mod gh;
mod git;
mod init;
mod manifest;
mod pr;
mod project;
mod pull;
mod push;
mod registry;
mod run;
mod setup;
mod tree;
mod update;
mod util;
mod view;
mod workspace;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};

/// Pull, review and run all the repos of a multi-repo project.
///
/// Exit codes: 0 ok, 1 some repo failed, 2 error (config, usage), 3 missing tool, 130 cancelled.
#[derive(Parser)]
#[command(name = "kex", version)]
struct Cli {
    /// Machine-readable output: one JSON event per line on stdout; questions become
    /// `prompt` events answered with one JSON line on stdin (see README)
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Set up repos for kex: pick them in a folder browser (this repo starts checked);
    /// --yes sets up only this repo
    #[command(visible_alias = "add")]
    Init {
        /// Folder to browse (default: the folder around this repo); with --yes, the repo to
        /// set up
        dir: Option<PathBuf>,
        /// Project id to join (default: this repo's project, or ask)
        #[arg(long)]
        project: Option<String>,
        /// Repo name, used in flags like --repos (sets up only this repo)
        #[arg(long, conflicts_with_all = ["repos", "workspace"])]
        repo: Option<String>,
        /// Don't ask; set up only this repo with detected defaults
        #[arg(long, short)]
        yes: bool,
        /// Give an existing (copied) config a new repo_id and name
        #[arg(long, conflicts_with_all = ["from", "workspace", "repos"])]
        reset: bool,
        /// Set up these folders (next to this repo, or in DIR), e.g. api,web; no browser
        #[arg(long, value_delimiter = ',', conflicts_with = "workspace")]
        repos: Option<Vec<String>>,
        /// Take the repos from this workspace file (*.code-workspace)
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// Set up the repos listed in this JSON file (`-` = stdin), asking nothing
        #[arg(long, value_name = "FILE|-", conflicts_with_all = ["dir", "repo", "repos", "workspace"])]
        from: Option<String>,
        /// Only print what would be written
        #[arg(long)]
        dry_run: bool,
        /// How many folder levels the browser shows
        #[arg(long, default_value_t = 3)]
        depth: u32,
    },
    /// List a folder as a tree with its git repos, and the workspace files around it
    /// (for programs: with --json it asks nothing)
    Browse {
        /// Folder to list (default: the current one)
        dir: Option<PathBuf>,
        /// How many levels below it to list (default 3; with --repos-only: no limit)
        #[arg(long)]
        depth: Option<u32>,
        /// Stop after this many entries (the rest is marked truncated)
        #[arg(long, default_value_t = 500)]
        max_entries: usize,
        /// Only the git repos below DIR, at any depth, as a flat list
        #[arg(long, visible_alias = "flat")]
        repos_only: bool,
        /// Don't read branch, changed files, remote and last commit (fast)
        #[arg(long)]
        no_git: bool,
        /// Stop reading git fields after this many milliseconds (0 = no limit)
        #[arg(long, default_value_t = 5000)]
        git_budget_ms: u64,
        /// Show dot-folders
        #[arg(long)]
        hidden: bool,
        /// Also list node_modules, target, build, dist, Pods and vendor (dot-folders like
        /// .git and .dart_tool also need --hidden)
        #[arg(long)]
        all: bool,
        /// Skip this many of DIR's own subfolders: the next page, from `next_offset`
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
    /// Update every repo and show unmerged work
    Pull {
        /// Also list unmerged branches older than 30 days
        #[arg(long)]
        all: bool,
        /// Switch clean repos to their default prime branch first
        #[arg(long)]
        prime: bool,
    },
    /// Branch, ahead/behind and changed files of every repo
    Status,
    /// Start every repo (runtime detected from its files)
    Run {
        /// Only these repos, e.g. api,web
        #[arg(long, value_delimiter = ',')]
        repos: Option<Vec<String>>,
        /// Skip pull + install
        #[arg(long)]
        no_update: bool,
        /// Install dependencies even if the lockfile didn't change
        #[arg(long)]
        install: bool,
        /// iOS simulator name, e.g. "iPhone 17 Pro"
        #[arg(long)]
        sim: Option<String>,
        /// Start services even if their port is already in use
        #[arg(long)]
        force: bool,
    },
    /// Push the current branch of selected repos
    Push {
        /// Only these repos, e.g. api,web
        #[arg(long, value_delimiter = ',')]
        repos: Option<Vec<String>>,
        /// Don't ask for confirmation (protected branches still need it)
        #[arg(long, short)]
        yes: bool,
    },
    /// Pull requests
    Pr {
        #[command(subcommand)]
        command: PrCmd,
    },
    /// Show what kex knows about this project
    Doctor,
    /// Find kex repos under a folder and remember it as a search root
    Scan { dir: Option<PathBuf> },
    /// Register a repo by hand
    Link { path: PathBuf },
    /// Print shell completions
    Completions { shell: clap_complete::Shell },
    /// Describe kex's commands, options and exit codes for programs (cli-events/1)
    Manifest,
    /// Install the latest kex release
    Update {
        /// Only report whether a new version exists
        #[arg(long)]
        check: bool,
        /// Don't ask before installing
        #[arg(long, short)]
        yes: bool,
        /// Refresh the version cache silently (used by the background check)
        #[arg(long, hide = true)]
        quiet: bool,
    },
}

#[derive(Subcommand)]
enum PrCmd {
    /// List open PRs across repos, review and merge one
    Check {
        /// Only print the list
        #[arg(long)]
        list: bool,
    },
}

fn hello() {
    ui::set_json();
    ui::emit(
        "hello",
        serde_json::json!({
            "standard": manifest::STANDARD,
            "tool": { "name": "kex", "version": env!("CARGO_PKG_VERSION") },
            "manifest_version": manifest::MANIFEST_VERSION,
            "manifest_hash": manifest::hash(),
            "kex": env!("CARGO_PKG_VERSION"),
            "protocol": ui::PROTOCOL,
            "features": ui::FEATURES,
        }),
    );
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        // A usage error with --json is still a JSON stream: hello, error, exit 2.
        Err(e)
            if std::env::args().any(|a| a == "--json")
                && !matches!(
                    e.kind(),
                    clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
                ) =>
        {
            hello();
            let message = e.to_string();
            let message = message
                .lines()
                .next()
                .unwrap_or("")
                .trim_start_matches("error: ");
            ui::emit("error", serde_json::json!({ "message": message }));
            ui::emit(
                "exit",
                serde_json::json!({ "code": 2, "tone": manifest::exit_tone(2) }),
            );
            return ExitCode::from(2);
        }
        Err(e) => e.exit(),
    };
    if cli.json {
        hello();
        cancel::install();
    }
    let notify = !matches!(
        cli.command,
        Some(Cmd::Update { .. } | Cmd::Completions { .. } | Cmd::Manifest)
    );
    let code = match dispatch(cli.command) {
        Ok(code) => code,
        Err(e) => {
            let cancelled = e.downcast_ref::<ui::Cancelled>().is_some()
                || matches!(
                    e.downcast_ref::<inquire::InquireError>(),
                    Some(
                        inquire::InquireError::OperationCanceled
                            | inquire::InquireError::OperationInterrupted
                    )
                );
            if cancelled {
                130
            } else {
                if ui::json() {
                    ui::emit("error", serde_json::json!({ "message": format!("{e:#}") }));
                } else {
                    eprintln!("kex: {e:#}");
                }
                2
            }
        }
    };
    if notify && code != 130 {
        update::notice();
    }
    ui::event(
        "exit",
        serde_json::json!({ "code": code, "tone": manifest::exit_tone(code) }),
    );
    ExitCode::from(code)
}

fn dispatch(cmd: Option<Cmd>) -> Result<u8> {
    match cmd {
        None => menu(),
        Some(Cmd::Init {
            dir,
            project,
            repo,
            yes,
            reset,
            repos,
            workspace,
            from,
            dry_run,
            depth,
        }) => init::run(init::Opts {
            dir,
            project,
            repo,
            yes,
            reset,
            repos,
            workspace,
            from,
            dry_run,
            depth,
        }),
        Some(Cmd::Browse {
            dir,
            depth,
            max_entries,
            hidden,
            all,
            offset,
            repos_only,
            no_git,
            git_budget_ms,
        }) => browse::command(browse::BrowseOpts {
            dir,
            depth,
            repos_only,
            git: !no_git,
            git_budget_ms,
            max_entries,
            hidden,
            all,
            offset,
        }),
        Some(Cmd::Pull { all, prime }) => pull::run(all, prime),
        Some(Cmd::Status) => pull::status(),
        Some(Cmd::Run {
            repos,
            no_update,
            install,
            sim,
            force,
        }) => run::run(run::Opts {
            repos,
            no_update,
            install,
            sim,
            force,
        }),
        Some(Cmd::Push { repos, yes }) => push::run(repos, yes),
        Some(Cmd::Pr {
            command: PrCmd::Check { list },
        }) => pr::run(list),
        Some(Cmd::Doctor) => doctor::doctor(),
        Some(Cmd::Scan { dir }) => doctor::scan(dir),
        Some(Cmd::Link { path }) => doctor::link(&path),
        Some(Cmd::Update { check, yes, quiet }) => update::run(check, quiet, yes),
        Some(Cmd::Manifest) => manifest::command(),
        Some(Cmd::Completions { shell }) => {
            if ui::json() {
                anyhow::bail!("completions write a shell script, not JSON: run it without --json");
            }
            clap_complete::generate(shell, &mut Cli::command(), "kex", &mut std::io::stdout());
            Ok(0)
        }
    }
}

/// `kex` with no command (§5.1).
fn menu() -> Result<u8> {
    if !util::interactive() {
        Cli::command().print_help()?;
        return Ok(0);
    }
    let gh_ready = gh::state() == gh::State::Ready;
    let item = |value: &str, words: &str, params: serde_json::Value| {
        let d = view::msg(&format!("menu.{value}"), params);
        let needs_gh = value == "pr" && !gh_ready;
        let label = format!(
            "{words:<9} {}{}",
            d["text"].as_str().unwrap_or_default(),
            if needs_gh { "  [needs gh]" } else { "" }
        );
        let description = view::cell(d["text"].clone(), None, Some(d.clone()));
        let mut row =
            serde_json::json!({ "cells": { "command": words, "description": description } });
        if needs_gh {
            row["badges"] = serde_json::json!([view::badge("needs_gh", "warn")]);
        }
        ui::Opt::new(value, label).row(row)
    };
    let none = serde_json::Value::Null;
    let mut items = vec![
        item("pull", "pull", none.clone()),
        item("run", "run", none.clone()),
        item("pr", "pr check", none.clone()),
        item("status", "status", none.clone()),
        item("push", "push", none.clone()),
        item("doctor", "doctor", none.clone()),
    ];
    if let Some(v) = update::available() {
        items.push(item(
            "update",
            "update",
            serde_json::json!({ "version": v, "current": update::CURRENT }),
        ));
    }
    let q = ui::Q::new("menu.pick", none).with(
        "columns",
        serde_json::json!([
            view::col("command", "code"),
            view::col("description", "text")
        ]),
    );
    let choice = ui::select_one(q, &items)?;
    dispatch(Some(match items[choice].value.as_str() {
        "pull" => Cmd::Pull {
            all: false,
            prime: false,
        },
        "run" => Cmd::Run {
            repos: None,
            no_update: false,
            install: false,
            sim: None,
            force: false,
        },
        "pr" => Cmd::Pr {
            command: PrCmd::Check { list: false },
        },
        "status" => Cmd::Status,
        "push" => Cmd::Push {
            repos: None,
            yes: false,
        },
        "doctor" => Cmd::Doctor,
        _ => Cmd::Update {
            check: false,
            yes: true,
            quiet: false,
        },
    }))
}
