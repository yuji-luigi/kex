//! `kex push`: pick repos, push each one's current branch. No PR, no merge, no commit
//! (BLUEPRINT §5.5).

use std::process::Command;

use anyhow::{Result, bail};

use crate::project::{self, Repo};
use crate::{git, ui, util};

struct Item<'a> {
    repo: &'a Repo,
    branch: String,
    ahead: u32,
    upstream: bool,
    dirty: bool,
}

pub fn run(names: Option<Vec<String>>, yes: bool) -> Result<u8> {
    let p = project::resolve(&std::env::current_dir()?)?;
    p.print_header();
    let mut items = Vec::new();
    for name in p.names() {
        let r = p.pick(&name)?;
        let upstream = git::has_upstream(&r.path);
        let ahead = if upstream {
            git::count(&r.path, &["@{u}..HEAD"])
        } else {
            git::count(&r.path, &["HEAD", "--not", "--remotes"])
        };
        items.push(Item {
            repo: r,
            branch: git::current_branch(&r.path),
            ahead,
            upstream,
            dirty: git::is_dirty(&r.path),
        });
    }
    let describe = |it: &Item| {
        let state = match (it.upstream, it.ahead) {
            (_, 0) => util::dim("nothing to push"),
            (true, n) => format!("↑{n} to push"),
            (false, n) => format!("↑{n}, no upstream yet (will be set)"),
        };
        let dirty = if it.dirty {
            util::yellow("  ⚠ uncommitted changes are not pushed")
        } else {
            String::new()
        };
        format!("{:<8} {:<24} {state}{dirty}", it.repo.name, it.branch)
    };

    let chosen: Vec<&Item> = match &names {
        Some(names) => {
            let mut v = Vec::new();
            for n in names {
                match items.iter().find(|i| &i.repo.name == n) {
                    Some(i) => v.push(i),
                    None => bail!("no repo named \"{n}\" in this project"),
                }
            }
            v
        }
        None if util::interactive() => {
            let opts: Vec<ui::Opt> = items
                .iter()
                .map(|it| {
                    ui::Opt::new(&it.repo.name, describe(it)).data(serde_json::json!({
                        "branch": it.branch, "ahead": it.ahead,
                        "upstream": it.upstream, "dirty": it.dirty,
                    }))
                })
                .collect();
            let preselected: Vec<usize> =
                (0..items.len()).filter(|i| items[*i].ahead > 0).collect();
            ui::multi_select(
                "Push the current branch of which repos?",
                &opts,
                &preselected,
            )?
            .into_iter()
            .map(|i| &items[i])
            .collect()
        }
        None => bail!("no terminal to ask in: pass --repos a,b (and --yes)"),
    };
    if chosen.is_empty() {
        say!("nothing selected");
        return Ok(0);
    }
    if names.is_some() && !yes {
        if !util::interactive() {
            bail!("pass --yes to push without a terminal");
        }
        for it in &chosen {
            say!("  {}", describe(it));
        }
        if !ui::confirm("Push these?", true)? {
            return Ok(0);
        }
    }

    let mut code = 0;
    for it in chosen {
        let name = &it.repo.name;
        if it.branch == "(detached)" {
            say!("{} {name}: detached HEAD, skipped", util::yellow("⚠"));
            continue;
        }
        if it.ahead == 0 && it.upstream {
            say!("{} {name}: nothing to push", util::dim("-"));
            continue;
        }
        if it.repo.is_protected(&it.branch) {
            // Typed confirmation, never skipped by --yes (§7).
            if !util::interactive() {
                say!(
                    "{} {name}: {} is protected; run interactively to confirm",
                    util::red("✗"),
                    it.branch
                );
                code = 1;
                continue;
            }
            let typed = ui::text(
                &format!(
                    "{name}: {} is protected. Type `{}` to push:",
                    it.branch, it.branch
                ),
                None,
            )?;
            if typed.trim() != it.branch {
                say!("{name}: skipped");
                continue;
            }
        }
        say!(
            "{} {name}: git push -u origin {}",
            util::bold("==>"),
            it.branch
        );
        let ok = Command::new("git")
            .args(["push", "-u", "origin", &it.branch])
            .current_dir(&it.repo.path)
            .stdout(ui::child_stdout())
            .status()
            .is_ok_and(|s| s.success());
        ui::event(
            "pushed",
            serde_json::json!({ "repo": name, "branch": it.branch, "ok": ok }),
        );
        if !ok {
            say!("{} {name}: push failed", util::red("✗"));
            code = 1;
        }
    }
    Ok(code)
}
