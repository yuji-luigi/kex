//! `kex pr check`: one list of open PRs across repos → detail → confirm → type to accept
//! (BLUEPRINT §5.6).

use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::project::{self, Repo};
use crate::{gh, ui, util, view};

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

fn row_key(row: &Row) -> String {
    format!("{}#{}", row.repo.name, row.pr.number)
}

/// The PR as cells of the `prs` table and the PR prompt.
fn cells(row: &Row) -> Value {
    let pr = format!("#{} {}", row.pr.number, row.pr.title);
    json!({
        "pr": view::cell(json!(row.pr.number), None, Some(json!(pr))),
        "repo": row.repo.name, "author": row.pr.author.login,
        "base": row.pr.base_ref_name, "head": row.pr.head_ref_name,
        "created_at": row.pr.created_at,
    })
}

fn badges(row: &Row) -> Vec<Value> {
    let mut b = Vec::new();
    if row.production {
        b.push(view::badge("production", "danger"));
    }
    if row.pr.is_draft {
        b.push(view::badge("draft", "muted"));
    }
    b
}

/// The `prs` event: the old `items`, and the table.
fn prs_event(rows: &[Row]) -> Value {
    let table_rows: Vec<Value> = rows
        .iter()
        .map(|r| json!({ "key": row_key(r), "cells": cells(r), "badges": badges(r) }))
        .collect();
    json!({
        "items": rows.iter().map(data).collect::<Vec<_>>(),
        "key": "prs",
        "view": view::view("table", json!({
            "title": view::m("prs.title"),
            "columns": [
                view::col("pr", "text"), view::col("repo", "text"), view::col("author", "text"),
                view::col("base", "branch"), view::col("head", "branch"),
                view::col("opened", "date"),
            ],
            "rows": table_rows,
            "empty": view::m("prs.empty"),
        })),
    })
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
        if ui::json() {
            // A fatal error, then exit 3 (cli-events/1 §3).
            let id = if st == gh::State::Missing {
                "gh.missing"
            } else {
                "gh.logged_out"
            };
            let mut e = view::m(id);
            e["message"] = e["text"].clone();
            e["text"] = Value::Null;
            if let Some(o) = e.as_object_mut() {
                o.remove("text");
                o.insert("detail".into(), json!(gh::help(st)));
            }
            ui::emit("error", e);
        } else {
            warn!("kex: pr check {}", gh::help(st));
        }
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
        let mut ev = prs_event(&rows);
        // The table's `opened` column is `created_at`.
        for r in ev["view"]["rows"].as_array_mut().into_iter().flatten() {
            let c = r["cells"].as_object_mut().unwrap();
            if let Some(v) = c.remove("created_at") {
                c.insert("opened".into(), v);
            }
        }
        ui::event("prs", ev);
        if rows.is_empty() {
            ui::say_about("prs", "info", "no open PRs");
            return Ok(0);
        }
        if list_only || !util::interactive() {
            for r in &rows {
                ui::say_about("prs", "info", &label(r));
            }
            return Ok(0);
        }
        let mut opts: Vec<ui::Opt> = rows
            .iter()
            .map(|r| {
                let mut cells = cells(r);
                let c = cells.as_object_mut().unwrap();
                c.remove("head");
                c.remove("created_at");
                ui::Opt::new(row_key(r), label(r))
                    .data(data(r))
                    .row(json!({ "cells": cells, "badges": badges(r) }))
            })
            .collect();
        opts.push(ui::Opt::fixed("quit", "pr.quit", Value::Null));
        let q = ui::Q::new("pr.pick", Value::Null)
            .with("picks_from", json!("prs"))
            .with(
                "columns",
                json!([
                    view::col("pr", "text"),
                    view::col("repo", "text"),
                    view::col("author", "text"),
                    view::col("base", "branch"),
                ]),
            );
        let Some(choice) = ui::select(q, &opts, Some(15))? else {
            return Ok(0);
        };
        if choice == rows.len() {
            return Ok(0);
        }
        review(&rows[choice])?;
    }
}

/// The `pr` event: one PR as a card (`gh pr view --json`), sent before its question.
fn detail_event(row: &Row) -> Value {
    let n = row.pr.number.to_string();
    let fields =
        "title,body,author,baseRefName,headRefName,url,state,isDraft,files,statusCheckRollup";
    let d: Value = gh::gh(&row.repo.path, &["pr", "view", &n, "--json", fields])
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let s = |k: &str| d[k].as_str().unwrap_or_default().to_string();
    let mut fields = vec![
        view::field("author", json!(row.pr.author.login), "text"),
        view::field(
            "branches",
            json!(format!(
                "{} → {}",
                row.pr.head_ref_name, row.pr.base_ref_name
            )),
            "code",
        ),
    ];
    if !s("url").is_empty() {
        fields.push(view::field("url", json!(s("url")), "url"));
    }
    if !s("state").is_empty() {
        fields.push(view::field("state", json!(s("state")), "badge"));
    }
    let mut sections = Vec::new();
    if !s("body").trim().is_empty() {
        sections.push(view::view(
            "card",
            json!({
                "title": view::m("pr.body"),
                "fields": [view::field("description", json!(s("body")), "code")],
            }),
        ));
    }
    let files: Vec<Value> = d["files"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| {
            let path = f["path"].as_str().unwrap_or_default();
            json!({ "key": path, "cells": {
                "file": path, "additions": f["additions"], "deletions": f["deletions"],
            }})
        })
        .collect();
    if !files.is_empty() {
        sections.push(view::view(
            "table",
            json!({
                "title": view::m("pr.files"),
                "columns": [
                    view::col("file", "path"), view::col("additions", "count"),
                    view::col("deletions", "count"),
                ],
                "rows": files,
            }),
        ));
    }
    let checks: Vec<Value> = d["statusCheckRollup"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, c)| {
            let name = c["name"]
                .as_str()
                .or(c["context"].as_str())
                .unwrap_or("check");
            let state = c["conclusion"]
                .as_str()
                .filter(|x| !x.is_empty())
                .or(c["state"].as_str())
                .or(c["status"].as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let tone = match state.as_str() {
                "success" => "ok",
                "failure" | "error" | "cancelled" | "timed_out" | "action_required" => "danger",
                "neutral" | "skipped" | "stale" => "muted",
                _ => "warn",
            };
            json!({ "key": format!("{i}:{name}"), "cells": {
                "check": name, "state": view::cell(json!(state), Some(tone), None),
            }})
        })
        .collect();
    if !checks.is_empty() {
        sections.push(view::view(
            "table",
            json!({
                "title": view::m("pr.checks"),
                "columns": [view::col("check", "text"), view::col("state", "badge")],
                "rows": checks,
            }),
        ));
    }
    json!({
        "repo": row.repo.name, "number": row.pr.number,
        "key": format!("pr:{}", row_key(row)),
        "view": view::view("card", json!({
            "title": format!("#{} {}", row.pr.number, row.pr.title),
            "subtitle": row.repo.name,
            "badges": badges(row),
            "fields": fields,
            "sections": sections,
        })),
    })
}

fn review(row: &Row) -> Result<()> {
    let dir = &row.repo.path;
    let n = row.pr.number.to_string();
    let rule = |c: char| c.to_string().repeat(64);
    let key = format!("pr:{}", row_key(row));
    let about = |text: &str| ui::say_about(&key, "info", text);
    loop {
        if ui::json() {
            ui::event("pr", detail_event(row));
        }
        about(&rule('='));
        if ui::json() {
            if let Ok(t) = gh::gh(dir, &["pr", "view", &n]) {
                about(&t);
            }
        } else {
            gh::gh_inherit(dir, &["pr", "view", &n], true)?;
        }
        about(&format!("{}\nFiles changed:", rule('-')));
        let files = r#".files[] | "  +\(.additions) -\(.deletions)  \(.path)""#;
        if let Ok(f) = gh::gh(dir, &["pr", "view", &n, "--json", "files", "--jq", files]) {
            about(&f);
        }
        about("Checks:");
        match gh::gh(dir, &["pr", "checks", &n]) {
            Ok(c) if !c.is_empty() => c.lines().for_each(|l| about(&format!("  {l}"))),
            _ => about("  (none)"),
        }
        about(&rule('='));
        if row.production {
            say!(
                "{}",
                util::red(format!(
                    "!! This PR merges into {} (PRODUCTION)",
                    row.pr.base_ref_name
                ))
            );
        }
        let actions = [
            ui::Opt::fixed("merge", "pr.merge", Value::Null),
            ui::Opt::fixed("diff", "pr.diff", Value::Null),
            ui::Opt::fixed("back", "pr.back", Value::Null),
        ];
        let q = ui::Q::new(
            "pr.action",
            json!({ "number": row.pr.number, "title": row.pr.title }),
        );
        match ui::select(q, &actions, None)? {
            Some(1) => {
                gh::gh_inherit(dir, &["pr", "diff", &n], false)?;
                continue;
            }
            Some(0) => {}
            _ => return Ok(()),
        }
        let mut question = ui::Q::new(
            "pr.confirm_merge",
            json!({
                "number": row.pr.number, "title": row.pr.title, "repo": row.repo.name,
                "base": row.pr.base_ref_name,
            }),
        );
        if row.production {
            question = question.with("tone", json!("danger"));
        }
        if !ui::confirm(question, false)? {
            return Ok(());
        }
        let word = if row.production {
            row.pr.base_ref_name.clone()
        } else {
            "accept".into()
        };
        let q = ui::Q::new("pr.typed", json!({ "word": word }))
            .with("expect", json!(word))
            .with("tone", json!("danger"));
        let typed = ui::text(q, None)?;
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
            let key = format!("merged:{}", row_key(row));
            let line = if state == "MERGED" {
                let p = json!({ "number": row.pr.number, "base": row.pr.base_ref_name });
                view::note("win", view::msg("pr.merged", p))
            } else {
                let p = json!({ "number": row.pr.number, "state": state });
                view::note("warn", view::msg("pr.merge_pending", p))
            };
            ui::event(
                "merged",
                json!({
                    "repo": row.repo.name, "number": row.pr.number, "state": state,
                    "key": key, "view": view::view("log", json!({ "lines": [line] })),
                }),
            );
            if state == "MERGED" {
                let text = util::green(format!("✓ #{n} merged into {}", row.pr.base_ref_name));
                ui::say_about(&key, "win", &text);
            } else {
                let text = format!(
                    "#{n} merge requested, but it is still {state} (merge queue / pending checks?)"
                );
                ui::say_about(&key, "warn", &text);
            }
        } else {
            say!("{}", util::red(format!("✗ merge of #{n} failed")));
        }
        return Ok(());
    }
}
