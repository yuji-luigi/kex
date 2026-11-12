//! `kex pr check`: one list of open PRs across repos → detail → confirm → type to accept
//! (BLUEPRINT §5.6).

use anyhow::Result;
use serde::Deserialize;

use crate::project::{self, Repo};
use crate::{gh, ui, util};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pr {
    number: u64,
    title: String,
    head_ref_name: String,
    base_ref_name: String,
    author: Author,
    created_at: String,
    #[serde(default)]
    is_draft: bool,
}

#[derive(Deserialize)]
struct Author {
    login: String,
}

struct Row<'a> {
    repo: &'a Repo,
    pr: Pr,
    production: bool,
}

fn fetch<'a>(repos: &[&'a Repo]) -> Vec<Row<'a>> {
    let fields = "number,title,headRefName,baseRefName,author,createdAt,isDraft";
    let lists = util::parallel(repos, |r| {
        gh::gh(
            &r.path,
            &[
                "pr", "list", "--state", "open", "--limit", "100", "--json", fields,
            ],
        )
        .and_then(|s| Ok(serde_json::from_str::<Vec<Pr>>(&s)?))
    });
    let mut rows = Vec::new();
    for (r, list) in repos.iter().zip(lists) {
        match list {
            Ok(prs) => rows.extend(prs.into_iter().map(|pr| Row {
                production: r.is_protected(&pr.base_ref_name),
                repo: r,
                pr,
            })),
            Err(e) => warn!(
                "{} {}: {}",
                util::yellow("⚠"),
                r.name,
                e.to_string().lines().next().unwrap_or("")
            ),
        }
    }
    rows
}

fn label(row: &Row) -> String {
    let age = util::unix_from_rfc3339(&row.pr.created_at)
        .map(|t| util::ago(util::now() as i64 - t))
        .unwrap_or_default();
    let mut title: String = row.pr.title.chars().take(40).collect();
    if row.pr.title.chars().count() > 40 {
        title.push('…');
    }
    format!(
        "{:<8} #{:<5} {:<41} {} → {}  {}  {}{}{}",
        row.repo.name,
        row.pr.number,
        title,
        row.pr.head_ref_name,
        row.pr.base_ref_name,
        row.pr.author.login,
        age,
        if row.pr.is_draft { "  draft" } else { "" },
        if row.production { "  PRODUCTION" } else { "" },
    )
}

fn data(row: &Row) -> serde_json::Value {
    serde_json::json!({
        "repo": row.repo.name, "number": row.pr.number, "title": row.pr.title,
        "head": row.pr.head_ref_name, "base": row.pr.base_ref_name,
        "author": row.pr.author.login, "created_at": row.pr.created_at,
        "draft": row.pr.is_draft, "production": row.production,
    })
}

pub fn run(list_only: bool) -> Result<u8> {
    let st = gh::state();
    if st != gh::State::Ready {
        warn!("kex: pr check {}", gh::help(st));
        return Ok(3);
    }
    let p = project::resolve(&std::env::current_dir()?)?;
    p.print_header();
    // Clones share a remote, so one checkout per repo is enough.
    let mut repos: Vec<&Repo> = Vec::new();
    for name in p.names() {
        let r = p.checkouts(&name)[0];
        if gh::provider(r) == gh::Provider::GitHub {
            repos.push(r);
        } else {
            say!(
                "  {} {:<8} {}",
                util::dim("-"),
                r.name,
                util::dim("not on GitHub, PR features off")
            );
        }
    }
    loop {
        let rows = fetch(&repos);
        if rows.is_empty() {
            say!("no open PRs");
            return Ok(0);
        }
        ui::event(
            "prs",
            serde_json::json!({ "items": rows.iter().map(data).collect::<Vec<_>>() }),
        );
        if list_only || !util::interactive() {
            for r in &rows {
                say!("{}", label(r));
            }
            return Ok(0);
        }
        let mut opts: Vec<ui::Opt> = rows
            .iter()
            .map(|r| {
                ui::Opt::new(format!("{}#{}", r.repo.name, r.pr.number), label(r)).data(data(r))
            })
            .collect();
        opts.push(ui::Opt::new("quit", "Quit"));
        let Some(choice) = ui::select("Open PRs", &opts, Some(15))? else {
            return Ok(0);
        };
        if choice == rows.len() {
            return Ok(0);
        }
        review(&rows[choice])?;
    }
}

fn review(row: &Row) -> Result<()> {
    let dir = &row.repo.path;
    let n = row.pr.number.to_string();
    let rule = |c: char| c.to_string().repeat(64);
    loop {
        say!("{}", rule('='));
        gh::gh_inherit(dir, &["pr", "view", &n], true)?;
        say!("{}\nFiles changed:", rule('-'));
        let files = r#".files[] | "  +\(.additions) -\(.deletions)  \(.path)""#;
        if let Ok(f) = gh::gh(dir, &["pr", "view", &n, "--json", "files", "--jq", files]) {
            say!("{f}");
        }
        say!("Checks:");
        match gh::gh(dir, &["pr", "checks", &n]) {
            Ok(c) if !c.is_empty() => c.lines().for_each(|l| say!("  {l}")),
            _ => say!("  (none)"),
        }
        say!("{}", rule('='));
        if row.production {
            say!(
                "{}",
                util::red(format!(
                    "!! This PR merges into {} (PRODUCTION)",
                    row.pr.base_ref_name
                ))
            );
        }
        let title = format!("#{} {}", row.pr.number, row.pr.title);
        let actions = [
            ui::Opt::new("merge", "Merge"),
            ui::Opt::new("diff", "Show diff"),
            ui::Opt::new("back", "Back"),
        ];
        match ui::select(&title, &actions, None)? {
            Some(1) => {
                gh::gh_inherit(dir, &["pr", "diff", &n], false)?;
                continue;
            }
            Some(0) => {}
            _ => return Ok(()),
        }
        let question = format!(
            "Merge #{} \"{}\" ({}) into {}?",
            row.pr.number, row.pr.title, row.repo.name, row.pr.base_ref_name
        );
        if !ui::confirm(&question, false)? {
            return Ok(());
        }
        let word = if row.production {
            row.pr.base_ref_name.clone()
        } else {
            "accept".into()
        };
        let typed = ui::text(&format!("Type `{word}` to merge:"), None)?;
        if typed.trim() != word {
            say!("cancelled");
            return Ok(());
        }
        if gh::gh_inherit(dir, &["pr", "merge", &n, "--merge"], true)? {
            let state = gh::gh(
                dir,
                &["pr", "view", &n, "--json", "state", "--jq", ".state"],
            )
            .unwrap_or_default();
            ui::event(
                "merged",
                serde_json::json!({ "repo": row.repo.name, "number": row.pr.number, "state": state }),
            );
            if state == "MERGED" {
                say!(
                    "{}",
                    util::green(format!("✓ #{n} merged into {}", row.pr.base_ref_name))
                );
            } else {
                say!(
                    "#{n} merge requested, but it is still {state} (merge queue / pending checks?)"
                );
            }
        } else {
            say!("{}", util::red(format!("✗ merge of #{n} failed")));
        }
        return Ok(());
    }
}
