mod config;
mod doctor;
mod flutter;
mod gh;
mod git;
mod init;
mod pr;
mod project;
mod pull;
mod push;
mod registry;
mod run;
mod util;

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
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create kex.json in this repo (join a project next to it, or create one)
    Init {
        /// Project id to join
        #[arg(long)]
        project: Option<String>,
        /// Short repo name, used in flags like --repos
        #[arg(long)]
        repo: Option<String>,
        /// Don't ask; use detected defaults
        #[arg(long, short)]
        yes: bool,
        /// Give an existing (copied) config a new repo_id and name
        #[arg(long)]
        reset: bool,
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
    Status {
        #[arg(long)]
        json: bool,
    },
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

fn main() -> ExitCode {
    match dispatch(Cli::parse().command) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            if let Some(
                inquire::InquireError::OperationCanceled
                | inquire::InquireError::OperationInterrupted,
            ) = e.downcast_ref::<inquire::InquireError>()
            {
                return ExitCode::from(130);
            }
            eprintln!("kex: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn dispatch(cmd: Option<Cmd>) -> Result<u8> {
    match cmd {
        None => menu(),
        Some(Cmd::Init {
            project,
            repo,
            yes,
            reset,
        }) => init::run(init::Opts {
            project,
            repo,
            yes,
            reset,
        }),
        Some(Cmd::Pull { all, prime }) => pull::run(all, prime),
        Some(Cmd::Status { json }) => pull::status(json),
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
        Some(Cmd::Completions { shell }) => {
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
    let gh_note = if gh::state() == gh::State::Ready {
        ""
    } else {
        "  [needs gh]"
    };
    let items = vec![
        "pull      update all repos + show unmerged work".to_string(),
        "run       start services".to_string(),
        format!("pr check  review and merge PRs{gh_note}"),
        "status    git status of every repo".to_string(),
        "push      push current branch of selected repos".to_string(),
        "doctor    what kex knows about this project".to_string(),
    ];
    let choice = inquire::Select::new("kex", items).raw_prompt()?;
    dispatch(Some(match choice.index {
        0 => Cmd::Pull {
            all: false,
            prime: false,
        },
        1 => Cmd::Run {
            repos: None,
            no_update: false,
            install: false,
            sim: None,
            force: false,
        },
        2 => Cmd::Pr {
            command: PrCmd::Check { list: false },
        },
        3 => Cmd::Status { json: false },
        4 => Cmd::Push {
            repos: None,
            yes: false,
        },
        _ => Cmd::Doctor,
    }))
}
