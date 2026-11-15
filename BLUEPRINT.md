# kex: blueprint (draft)

> **Status: draft / first guideline.** This document describes the direction of the
> project. Nothing here is final. Open points are collected in [Open questions](#10-open-questions).

`kex` (from "make ex") is an open-source CLI for projects split across several git repos
(for example an API, a web app and a mobile app). Instead of copying the same Makefile into
every project, each repo gets a small `kex.json`, and one tool does the rest.

**The core idea: pull all repos, run all repos, from any of them, on any folder layout.**

---

## 1. Why

Multi-repo projects usually grow a Makefile (or scripts) that pull every repo and start every
dev server. That file gets copied from project to project and drifts. It also hard-codes the
folder layout (`../web`), so a teammate with a different layout has to edit it or keep a
git-ignored path file in every project.

kex replaces that with:

- one binary, installed once per machine
- one committed config per repo that says *what* the repo is
- a machine-local registry that remembers *where* each repo is, filled automatically

## 2. Requirements

### 2.1 Core (v1)

| # | Requirement |
|---|---|
| C1 | **Pull all**: `kex pull` updates every repo of the project, from inside any of them. |
| C2 | **See unmerged work**: while pulling, list other branches not merged into a prime branch yet, with author and date. |
| C3 | **PR list**: `kex pr check`, one list of open PRs across repos → select → detail → confirm → type to accept. Uses `gh`; when it's missing, PR features are grayed out with install help. |
| C4 | **Run all**: `kex run` starts every repo (or a selection) with combined output and one Ctrl-C to stop everything. **How** to run each repo is detected from its files (§5.3.1). `run` in `kex.json` is only needed to override that. |
| C5 | **Flutter on iOS simulator** (macOS): a built-in runtime (pick a simulator, boot it, `flutter run`, hot reload on save). |

### 2.2 Needed to make the core work (v1)

| # | Requirement |
|---|---|
| F1 | One config per repo: `.config/kex.json` if `.config/` exists, else `kex.json`. |
| F2 | Find related repos on any folder layout, without hard-coded paths or per-project path files. |
| F3 | Each repo declares its prime branches, with what each one is for. |
| F4 | Git worktrees (e.g. created by agents) don't break or pollute anything. |
| F5 | Copied configs and duplicate clones are detected and can be fixed. |
| F6 | Every interactive prompt has a flag form, and kex never prompts when there's no terminal (scripts, CI, agents). |

### 2.3 Next (v1.x)

| # | Requirement |
|---|---|
| N1 | **Push selected repos**: `kex push` lets you pick repos and pushes the current branch of each. No automatic PRs. |
| N2 | More runtimes: Android emulator (works on Windows and Linux too), Docker Compose, Rust, Go, Python. |

### 2.4 Later (ideas, not committed)

- `kex clone`: clone missing repos of a project from one URL
- custom tasks per repo (`kex deploy-ios` → a script in the repo)
- syncing git-ignored `.env` files between machines
- one terminal pane per service (tmux, herdr…)

### 2.5 Dropped

- Trying unmerged branches locally (merging other devs' branches into a throwaway branch). Too risky for its value; C2 already shows what exists.
- Automatic PR creation on push.
- Merging the prime branch into your branch (or yours into it) and pushing.

### 2.6 Because it's public open source

| # | Requirement |
|---|---|
| O1 | **Nothing specific to one team**: no assumed org, folder layout, branch names, package manager or language. Presets are optional sugar on top of plain shell commands. |
| O2 | **Platforms**: macOS, Windows and Linux, all in v1 (§8.3). OS-specific features are grayed out with a reason on other systems, e.g. the iOS preset is macOS only. |
| O3 | **Hosting**: git features work with any remote. PR features use the host's CLI (`gh` for GitHub, §3.4), and are grayed out per repo when there's none. |
| O4 | **Install**: Homebrew tap (macOS/Linux), winget + Scoop (Windows), prebuilt release binaries, and a one-line install script. |
| O5 | **Stable config format**: `kex.json` has a `version` field and a published JSON Schema (`$schema`) so editors autocomplete and validate it. A breaking change bumps the version, and kex can migrate old files. |
| O6 | **Private by default**: no telemetry. The registry and all paths stay on the machine. |
| O7 | **License**: Apache-2.0. Plus a README, examples and a contributing guide. |

---

## 3. Config: `kex.json`

### 3.1 Location

Look in the repo root, stop at the first hit:

1. `.config/kex.json`, if a `.config/` folder exists
2. `kex.json`

Both existing at once is an error.

### 3.2 Shape (draft)

The file is **committed**, so it only contains things that are true for every user.
It is **plain JSON, no comments**: kex rewrites it itself (§4.4, §4.6), and comments would be
lost. The `//` comments below are only explanations for this document.

Personal values live outside the committed file:

- `kex.local.json` next to `kex.json` (git-ignored by `kex init`): per-repo overrides of `run` only, e.g. a different port or simulator.
- Global config `~/.config/kex/config.json` (`%APPDATA%\kex\config.json` on Windows): `roots`, default simulator, Windows shell. The registry is a separate file next to it, written only by kex.

```jsonc
// shop-api/kex.json
{
  "$schema": "https://raw.githubusercontent.com/yuji-luigi/kex/main/schema/kex.schema.json",
  "version": 1,

  "project": "acme-shop-7f3kq2",      // group id: name + random suffix, made by `kex init`
  "repo": "api",                      // short name inside the project, unique, used in flags (`--repos api,web`)
  "repo_id": "r_9xk2mq",              // unique id of THIS repo, made by `kex init`, never edited by hand
  "hosting": "github.com",            // optional, see §3.4

  "prime_branches": [
    { "branch": "dev",  "stage": "dev",        "description": "Integration branch, PRs go here" },
    { "branch": "main", "stage": "production", "description": "Live" }
  ],
  "protected": ["release/*"],         // extra protected branches; `stage: production` is protected automatically
  "ignore_branches": ["deploy_*"],    // long-lived branches to leave out of the unmerged report

  "peers": [                          // the other repos of the project (§4.6)
    { "repo": "web", "remote": "git@github.com:acme/shop-web.git" },
    { "repo": "app", "remote": "git@github.com:acme/shop-app.git" }
  ],

  "run": {                            // optional: detected when missing (§5.3.1)
    "install": "pnpm install",
    "dev": "pnpm run dev",            // or per OS: { "default": "pnpm run dev", "windows": "pnpm run dev:win" }
    "port": 3000,                     // optional: used to detect "already running" (§5.3)
    "ready": "http://localhost:3000/health"
  }
}
```

```jsonc
// shop-app/kex.json
{
  "version": 1,
  "project": "acme-shop-7f3kq2",
  "repo": "app",
  "repo_id": "r_c41pzt",
  "prime_branches": [{ "branch": "main", "stage": "dev", "description": "Integration branch" }]
}
```

No `run` here: kex finds `pubspec.yaml` with a Flutter dependency and uses the `flutter-ios` runtime on macOS.

### 3.3 `prime_branches`

| Field | Meaning |
|---|---|
| `branch` | git branch name in this repo |
| `stage` | `dev` · `staging` · `production`. `production` means protected. |
| `description` | free text, shown in menus |

A `client` field (one set of branches per client) was considered and left out until a command needs it.

The default prime branch, when a command needs one, is the first entry with `stage: dev`.

### 3.4 `hosting`: which CLI handles PRs

`hosting` names the domain where the repo lives. kex uses it to pick the CLI for PR features:

| `hosting` | CLI | Notes |
|---|---|---|
| `github.com` | `gh` | v1 |
| GitHub Enterprise domain, e.g. `github.acme.com` | `gh` with `GH_HOST` | needs `"provider": "github"`, since the domain alone doesn't say it's GitHub |
| `gitlab.com` (or self-hosted + `"provider": "gitlab"`) | `glab` | later |
| anything else | none | PR features grayed out for that repo |

```jsonc
"hosting": "github.com"                                          // short form
"hosting": { "domain": "git.acme.com", "provider": "gitlab" }    // self-hosted
```

**Resolution order:**

1. the `hosting` field
2. the domain of the shared remote (§5.2; `git@github.com:…` → `github.com`). An unknown domain → PR features off for that repo.
3. only if the repo has no remote at all: `github.com`

Step 2 means most repos never need the field. It only matters for self-hosted servers or
unusual remotes. When the field and the remote disagree, `kex doctor` warns.

---

## 4. Locating the related repos

### 4.1 Principle: "id in the repo, paths on the machine"

- **What a repo is** (ids, branches, run commands) → committed `kex.json`, the same for everyone.
- **Where it is** → the registry, local, never committed, written by kex:
  `~/.config/kex/registry.json` on macOS/Linux, `%APPDATA%\kex\registry.json` on Windows.

```jsonc
// ~/.config/kex/registry.json
{
  "roots": ["~/Development"],          // where to search when something is missing
  "projects": {
    "acme-shop-7f3kq2": {
      "r_9xk2mq": { "repo": "api", "root_commit": "a1b2c3…", "paths": ["~/Development/acme/shop-api"] },
      "r_4hd8wn": { "repo": "web", "root_commit": "d4e5f6…", "paths": ["~/Development/acme/shop-web"] },
      "r_c41pzt": { "repo": "app", "root_commit": "0718ab…", "paths": ["~/Development/acme/shop-app", "~/tmp/shop-app-2"],
                    "preferred": { "run": "~/Development/acme/shop-app" } }
    }
  },
  "not_found": { "acme-shop-7f3kq2/web": "2026-11-11T10:00:00Z" }   // negative cache, §4.3
}
```

The file is written atomically (temp file + rename) under a lock, because several kex
processes (e.g. parallel agents) can run at once.

### 4.2 Options considered

| Option | Verdict |
|---|---|
| Hard-coded relative paths (`../web`) | ❌ breaks on any other layout |
| Git-ignored path file per project | ❌ the pain kex exists to remove |
| Scan the disk on every command | ❌ slow on big home folders |
| **Committed ids + local registry filled by discovery** | ✅ |

### 4.3 Resolution (every command)

1. **Where am I?** Walk up from the current directory to the git root and read the config. Not found → "not a kex repo, run `kex init`".
2. **Register myself**, unless this checkout is a worktree (§4.5).
3. **Load the project from the registry.** Drop paths that no longer exist, or whose root commit changed.
   `kex.json` is committed, so it changes with the checked-out branch, and an old branch may not have it at all. In that case the repo **stays** in the project, and kex reads the config from the default prime branch (`git show <prime>:kex.json`). Only a config with *other* ids removes a path.
4. **Discover missing repos**, cheapest first, stop when complete. Inside a worktree, start from the main checkout's folder, not the worktree's.
   1. siblings `../*/`
   2. parent's siblings `../../*/`
   3. `roots`, max depth ~4, skipping `node_modules`, `.git`, `build`, `Pods`, `.dart_tool`, and worktree folders (`.worktrees`, `.claude/worktrees`…)
5. **Remember misses.** Siblings (steps 4.1–4.2) are always checked, because that's cheap and finds a fresh clone right away. Only the deep `roots` scan is skipped for a day after a miss (`not_found`), so a never-cloned repo doesn't trigger it on every command. `kex scan` and `kex doctor` always do a full search.
6. **Check for duplicates** (§4.4).
7. **Still missing?** Say so, and continue with the repos that were found.

### 4.4 Duplicates and copied configs

**Identity = first commit.** Every repo has a root commit (`git rev-list --max-parents=0 HEAD`).
Clones, forks and worktrees of the same repo share it. A config copied into an unrelated repo
almost never does. Remotes aren't used for identity, because forks, renamed repos and
not-yet-pushed repos would cause false alarms. They're only a hint.

When the root commit isn't reliable, kex falls back to `repo_id` + remote and says why in `kex doctor`. That happens in shallow clones (`--depth 1`, common in CI), where the real first commit isn't there, and after history rewrites (`git filter-repo`), which change it.

| Two checkouts with the same `repo_id` and… | It is | kex |
|---|---|---|
| one is a worktree | a worktree | ignores it (§4.5) |
| the same root commit | a second clone or a fork | keeps both (§4.5) |
| **different root commits** | **a copied config** | **asks you to fix it** |

The same check runs when two different `repo_id`s use the same `repo` name in one project.

```
$ kex
⚠ Copied kex config: repo_id r_9xk2mq ("api", project acme-shop-7f3kq2)
  is used by 2 unrelated repos:

  1) ~/Development/acme/shop-api        ← listed in the project's peers (likely the original)
  2) ~/Development/other/new-api

Which ones do NOT belong to this project? (space = select, enter = confirm)
  [ ] 1
  [x] 2

What should happen to ~/Development/other/new-api?
  › Join another project / create a new one (new ids, new repo name)
    Rename it and keep it in this project (new repo_id, new repo name)
    Remove its kex.json
    Ignore for now
```

- Every fix that keeps the repo in the project also asks for a **new `repo` name**. Otherwise the name still clashes.
- Fixing edits a committed file. kex shows the diff and never commits for you.
- With no terminal, kex prints the warning and continues. It never blocks.

### 4.5 Worktrees and second clones

**Worktrees are ignored.**

- Detection: in a worktree, `git rev-parse --git-dir` ≠ `git rev-parse --git-common-dir`.
- A worktree shares `.git` with its main checkout, so one fetch covers both.
- Never saved in the registry, never part of the duplicate check, skipped by discovery.
- **Exception:** when you run kex *inside* a worktree, that worktree is the current repo for that command. The other repos come from their main checkouts. This way an agent can `kex run` its own branch of `app` with the normal `api`.
- Git won't check out a branch that's already checked out in another worktree. When that happens, kex names the worktree that holds the branch instead of showing git's raw error.
- Only worktrees (bare clone + worktrees), no main checkout: kex asks once which one to use (`kex link`).

**Second clones** (same root commit, not worktrees):

| Command | Behaviour |
|---|---|
| `pull`, `status` | all clones (each has its own `.git`, so each is fetched). The unmerged report is shown once per repo, not once per clone. |
| `run`, `push` | ask which clone (or `--path`). "Remember" stores `preferred` in the registry. |
| `pr check` | one list per remote, no duplicates |

### 4.6 `peers`

Discovery only finds folders that exist. If someone cloned only `shop-api`, kex can't know
that `shop-web` and `shop-app` exist at all. `peers` lists the other repos by name + git URL,
so kex can say:

```
$ kex run
  ✓ api   ~/work/shop-api
  ✗ web   not on this machine   (git@github.com:acme/shop-web.git)
  ✗ app   not on this machine   (git@github.com:acme/shop-app.git)
```

In v1 this is only used for that message and for the "likely original" hint in §4.4.
`kex clone` (cloning them for you) comes later.

kex treats the **union** of `peers` across found repos as the project's list. `kex init` and
`kex doctor` offer to add newly seen repos, and `kex doctor --prune` removes repos that left
the project, since a union alone never forgets anything.

### 4.7 Helper commands

- `kex init [dir]` (visible alias `kex add`): the one setup command. Every mode builds the
  same request `--from` takes and hands it to `src/setup.rs`, which checks it (names, paths,
  project, ids), plans and writes it; nothing else writes a `kex.json`. Modes
  (`src/init.rs`): the terminal browser (default); `--yes` (this repo, or `DIR` when it is
  one; joins the only project found next to it, else a new one; also plain `kex init`
  without a terminal, as in 0.1); `--repos a,b` (folders next to this repo or in `DIR`);
  `--workspace <file>`; `--from <file|->`. `--project` (an unknown id makes a project with
  that id, except with `--from`), `--repo`, `--depth`, `--dry-run` (any mode), `--reset`
  (unchanged). With `--json` and none of those flags it sets up this repo and asks its
  project and name as prompts (as 0.1 did); several repos: `kex browse --json` + `--from -`.
  Terminal steps: (1) workspace file (`src/workspace.rs`, a `Format` trait; VS Code
  `*.code-workspace`, JSONC, paths relative to the file) found in the folder, the repo or its
  parent → "Use it?" → what it lists → "Init these N repos?" (no = the browser).
  (2) Browser (`src/browse.rs` on `src/tree.rs`, the same listing as `kex browse`): a tree
  `--depth` levels deep (default 3), breadth-first with a 200-entry cap (a folder with more
  subfolders gets `… N more` page rows), repos are leaves, symlinks followed by real path;
  Enter opens (new root) or checks, Space checks, `../`, Tab/`d` done, status line
  `N selected: a, b, +K more`. (3) Project (inside a kex repo: its own; else join one of the
  kex repos next to the picked ones, in the workspace file or seen while browsing, or a new
  one). (4) Names confirmed together, else asked one by one with the same check `setup.rs`
  applies. Every new config lists the existing members and the other new repos as `peers`.
  Existing configs are not changed.
- `--from` input: `[{path, name?, description?, repo_id?}]` or `{project?, repos}`; ids can
  be pinned (`repo_id`, `{new, id}`), so a dry run's `plan.request` writes exactly the plan.
  Everything is validated first (all problems reported as `error{index, field}`, exit 2),
  already-in-project repos are skipped, `--dry-run` → `plan` event.
- Writes (`setup.rs`): all or nothing. kex.json and the `.gitignore` line go to temp files
  next to them and are renamed into place; the first failure undoes the run's writes
  (exit 1, `added.rolled_back`); SIGINT/SIGTERM during the writes undo them too (exit 130);
  the registry is saved after.
- `kex browse [dir]`: the tree for programs (`tree` + `workspaces` events): symlinks not
  followed, `--max-entries` (with `has_more`/`next_offset` and `--offset` paging for the
  root's own list), `--hidden`, `--all`, `--repos-only`/`--flat` (every repo below, flat),
  `--no-git` and `--git-budget-ms` (git fields within a time budget, `git_partial`);
  `repos_below` counts the whole subtree within caps (`repos_below_truncated`); limits:
  repo name 1–40, description ≤ 200 one line, project name 1–40; `hello.features`, per-repo branch/dirty/remote/last commit/
  stack. Integration guide: docs/gui-integration.md.
- `kex link <path>`: register a repo by hand
- `kex scan [dir]`: index every kex repo under a folder
- `kex doctor`: project, paths, clones, ignored worktrees, duplicates, missing peers, prime branches that don't exist on the remote, `gh` state

---

## 5. Commands

### 5.1 `kex`: menu

```
$ kex
  pull      update all repos + show unmerged work
  run       start services
  status    git status of every repo
  push      push current branch of selected repos
  pr check  review and merge PRs                    [needs gh]
  add       set up more repos of this project
```

### 5.2 `kex pull` (C1, C2)

Per repo:

1. `git fetch --all --prune`
2. Fast-forward the current branch (`--ff-only`). A dirty tree or a diverged branch → skip with a warning, never auto-merge.
   `--prime` first switches each clean repo to its default prime branch.
3. Report unmerged branches: branches on the **shared remote** not merged into any prime branch.

The shared remote is where the team's branches live: the remote whose URL matches the
project's `peers`/`hosting`, usually `upstream` in a fork setup, otherwise `origin`. `kex push`
always pushes to `origin` (your own fork, if you have one).

```
== api (dev) ✓ up to date
   unmerged:
   feature/login       → dev   +4 / -0   Jane Doe   2026-10-24 18:12
   fix/push-notif      → dev   +1 / -12  John Roe   2026-10-20 09:03
== web (dev) ↓ 3 commits pulled
   unmerged:
   feature/calendar    → dev   +2 / -0   Jane Doe   2026-10-23 11:40
== app (main) ⚠ uncommitted changes, pull skipped
```

How the report avoids noise:

- **Squash and rebase merges:** GitHub's squash merge leaves the branch's commits out of the prime branch's history, so git alone thinks the branch is unmerged forever. With `gh` (already in v1 for this lookup), branches whose PR is merged are dropped. Without `gh`, `git cherry` only catches rebase merges and one-commit branches. Squash-merged branches with several commits still show up, marked `(may be squash-merged)` when their PR state can't be checked.
- Only branches active in the last 30 days (`--all` shows everything).
- Prime, protected and `ignore_branches` branches are left out.

Known limits: the date is the **commit** date, not the push date. The author is the author of
the last commit, not necessarily the branch owner.

### 5.3 `kex run` (C4, C5)

1. **Select** services. All are checked by default; enter = run all. `--repos api,web` skips the menu.
2. **Update first:** pull + `run.install` for the selected repos. A failed pull (e.g. offline) only warns. A failed install stops. `install` only runs when the lockfile changed since the last successful install (kex stores its hash in the registry), or with `--install`. `--no-update` skips this step.
3. **Clones:** a repo with several clones asks which one (§4.5).
4. **Ports:** if `run.port` is already in use, kex says so (and by which process when it can tell). This is common when an agent runs the same stack from a worktree. The choices are skip, stop, or run anyway.
5. **Output:** one terminal, colored prefix per service (`[api]`, `[web]`), Ctrl-C stops everything.
6. **Keyboard:** services don't read the terminal directly, so typed keys don't get split between processes. kex owns the keyboard:
   - `r` hot reload / `R` hot restart (sent to the flutter service as `SIGUSR1` / `SIGUSR2`)
   - `q` stops everything
   - `1`–`9` show only one service's output, `0` shows all again
7. **`ready`:** optional URL to wait for before printing "ready".

Preset `flutter-ios` (macOS only; on Windows and Linux it shows as `app (iOS simulator: macOS only)`, grayed out and not selectable): pick a simulator (`--sim "iPhone 17 Pro"` to skip), boot it, open
the Simulator app, `flutter run` (via `fvm` when present), hot reload on save: kex watches
`lib/` itself and sends `SIGUSR1` to `flutter run`. No `fswatch` needed.

#### 5.3.1 How kex decides how to run a repo

For each selected repo, the first match wins:

1. `run` in `kex.local.json` (personal override)
2. `run` in `kex.json` (project override)
3. **Detection** from files in the repo root:

| Found | Runtime | Install | Start |
|---|---|---|---|
| `pubspec.yaml` with a `flutter` dependency | `flutter-ios` (macOS only) | `flutter pub get` (`fvm flutter` if `.fvmrc` / `.fvm/` exists) | simulator flow (§5.3) |
| `package.json` | `node` | `<pm> install` | `<pm> run <script>` |

Node details:

- **Package manager:** the `packageManager` field in `package.json`, else the lockfile (`pnpm-lock.yaml` → pnpm, `yarn.lock` → yarn, `bun.lock`/`bun.lockb` → bun, `package-lock.json` → npm), else npm.
- **Script:** the first of `dev`, `start`, `serve` that exists in `scripts`.
- **Port:** `PORT=` from `.env` when present (used for the "already running" check).

When nothing matches (or the detected runtime isn't available on this OS), the repo is shown
grayed out with the reason, e.g. `app (Flutter: iOS runtime needs macOS)` or
`tools (don't know how to run it: add "run" to kex.json)`, and the others still start.

kex always shows what it detected before starting, so nothing is magic:

```
  [x] api   node · pnpm run dev       (pnpm-lock.yaml, script "dev")
  [x] web   node · npm run start      (package-lock.json, script "start")
  [x] app   flutter-ios               (pubspec.yaml)
```

`kex init` offers to write the detected `run` into `kex.json`. That's only needed when the
detection is wrong.

### 5.4 `kex status`

Branch, ahead/behind, and dirty files for every repo. Read-only.

### 5.5 `kex push` (N1, v1.x)

```
$ kex push
Push the current branch of which repos?   (space = select, enter = push)
  [x] api   feature/login   ↑2 to push
  [ ] web   dev             nothing to push
  [x] app   feature/maps    ↑1, no upstream yet (will be set)
        ⚠ uncommitted changes are not pushed
```

- Repos with commits to push are pre-selected.
- Pushes `git push -u origin <current branch>`. No PR, no merge, no commit.
- A protected or `stage: production` branch needs a typed confirmation.
- `--repos api,app --yes` for scripts.

### 5.6 `kex pr check` (C3)

```
$ kex pr check
Open PRs (acme-shop-7f3kq2)              ↑↓ move · enter open · / filter · q quit
› api  #142  Fix push notification on iOS   fix/push-notif → dev   John  2d  ✓ checks
  api  #139  Login screen                   feature/login → dev    Jane  5d  ✗ checks
  web  #88   Calendar view                  feature/calendar → dev Jane  1d  … pending
  app  #57   Release 2.3                    dev → main             Jane  3h  ✓ checks  PRODUCTION
```

1. **List**: open PRs of all repos in one list. PRs into protected branches are marked `PRODUCTION`.
2. **Select** with arrow keys.
3. **Detail**: description, files changed, checks, reviews, mergeable state. `d` opens the diff, `b` goes back.
4. **Confirm**: `Merge this PR? [y/N]`
5. **Type to accept**: `accept`, or the base branch name for a `PRODUCTION` PR.
6. **Merge** (`gh pr merge --merge`), then re-check: `✓ merged into dev` or `still OPEN (merge queue / pending checks)`.

---

## 6. GitHub CLI (`gh`) (C3)

kex never stores GitHub credentials. Anything that talks to GitHub goes through `gh`.

| Needs `gh` | Works without it |
|---|---|
| `pr check`, squash-merge filtering in the pull report (v1) | pull, run, status, push, init, doctor |

- **First run / install docs** say: install `gh` to enable PR features: `brew install gh` (macOS), `winget install --id GitHub.cli` (Windows), see https://github.com/cli/cli#installation for Linux, then `gh auth login`. Docs: https://cli.github.com/
- **Menu**: `gh` missing → GitHub items are grayed out but selectable. Selecting one shows the install command for the current OS + docs link. kex shows the command and never runs it.
- **Not logged in** is its own state: `Run: gh auth login`.
- **Wrong account** (logged in, but the account can't see the repo): on failure, show the active account and suggest `gh auth switch`.
- **Not on GitHub**: PR features are grayed out for that repo only.

## 7. Safety rules

1. Read-only by default. Anything that writes says so first.
2. Never auto-commit, never force-push, never rebase, never merge implicitly.
3. Protected branches always need a typed confirmation.
4. One repo failing doesn't stop the others. Summary at the end, non-zero exit if any repo failed.
5. No terminal → no prompts. Use flags, or get a clear error that names the flag.

## 8. Implementation sketch

- **Language: Rust.** One self-contained binary per OS. The user installs `kex` and runs it, with no Python, Node or script files next to it.
- **Shell out to `git` and `gh`** instead of using libraries, so the behaviour matches what users see by hand.
- **Modules:** `config` (find, parse, validate, migrate) · `registry` (local file, discovery, duplicates, worktrees, lock) · `git` · `hosting` (`gh` now, `glab` later) · `runner` (processes, prefixes, ports, presets) · `ui`.

### 8.1 Crates (draft)

| Need | Crate |
|---|---|
| Commands and flags (`kex run --repos api,web`) | `clap` |
| Read/write `kex.json` (plain JSON, keys kept in order) | `serde`, `serde_json` (`preserve_order` feature) |
| JSON Schema for editors | `schemars` (generates the schema from the Rust types, so the two can't drift) |
| Menus, multi-select, confirmations | `inquire` |
| `pr check` screen (custom keys `d`/`b`/`/`), `run` key handling | `ratatui` + `crossterm` |
| Colors | `owo-colors` or `anstream` |
| Config / registry location per OS | `directories` |
| Registry lock + atomic write | `fs4` (file lock), `tempfile` (write + rename) |
| Run services, stop them all on Ctrl-C | `std::process` + one thread per output stream (no `tokio`, to keep the binary small), `command-group` (process groups / Windows Job Objects), `ctrlc` |
| Hot reload on save | `notify`, which watches files on macOS, Windows and Linux. `fswatch` isn't needed anymore. |
| Find tools on PATH (`gh`, `fvm`, `flutter`) | `which` |
| Errors | `anyhow` (app), `thiserror` (modules) |
| Tests | `assert_cmd`, `tempfile` (real git repos in temp folders) |

### 8.2 Build and release

- `Cargo.toml` release profile: `lto = true`, `strip = true`, `codegen-units = 1`, `opt-level = "z"`. That should give a binary of a few MB.
- `cargo-dist` builds macOS (Intel + Apple Silicon), Windows and Linux binaries on each git tag. It also produces the Homebrew formula, a shell installer and a PowerShell installer. winget/Scoop manifests are added on top.
- Repo: `github.com/yuji-luigi/kex`. Homebrew tap: `github.com/yuji-luigi/homebrew-tap`, so users run `brew install yuji-luigi/tap/kex`.
- Not published on crates.io: the name `kex` is taken there by an unrelated crate, and we don't want a different name. Users install through Homebrew, winget/Scoop, the install scripts or release binaries, all named `kex`.

### 8.3 Windows from day one

Rust compiles for Windows from the same code. What has to be designed for:

| Area | macOS / Linux | Windows |
|---|---|---|
| Running `run.dev` | `sh -c` | **PowerShell** (the Windows default shell): `pwsh -NoProfile -Command` if PowerShell 7 is installed, else the built-in `powershell.exe`. Caveat: the built-in 5.1 doesn't support `&&`, so for chained commands use a per-OS override (§3.2) or a package script. Detected node commands (`pnpm run dev`) work in both. The shell can be changed in the global config. |
| Stopping all services on Ctrl-C | process group (`kill 0`) | Job Object, so child processes (node, flutter) die too |
| Registry path | `~/.config/kex/` | `%APPDATA%\kex\` |
| Paths in the registry | `/` | `\`, compared case-insensitively |
| Hot reload on save | `notify` crate | `notify` crate (same code; matters for a future `flutter-android` preset) |
| Colors / menus | any terminal | Windows Terminal and PowerShell are fine. Old `cmd.exe` falls back to plain text. |
| Install | Homebrew, script | winget, Scoop, zip from releases |

CI builds and tests on all three systems from the first commit, so Windows doesn't break
quietly.

Known name clash: Kali Linux on WSL ships its own `kex` command (Win-KeX). Only Kali WSL
users are affected. They can rename one of the two binaries or use an alias. See §11.

### 8.4 Recommended extras (small, worth doing in v1)

| Extra | Why |
|---|---|
| **Parallel fetch** in `pull`/`status` (one thread per repo) | 3–5 repos fetch in the time of one |
| **`--json` mode** on every command: NDJSON events on stdout, questions as `prompt` events answered on stdin (README "JSON mode"), conforming to cli-events/1 (`kex manifest`, a `view` on every result) | gateways, agents and scripts can drive kex without parsing terminal text, and generic clients without kex-specific code |
| **`--dry-run`** for `pull` and `push` | shows what would change, safe to try |
| **Shell completions**: `kex completions zsh\|bash\|fish\|powershell` (`clap_complete`) | tab-completes commands and repo names |
| **`NO_COLOR` / non-TTY plain output** | correct output in CI, pipes, old terminals |
| **Config errors with line and field** ("`kex.json:12: prime_branches[1].stage` must be dev, staging or production") | users fix their config without guessing |
| **Run logs**: each service's output is also saved to the kex state folder, `kex run --logs` reopens the last run | a crash that scrolled away can still be read |
| **Documented exit codes** (0 ok, 1 some repo failed, 2 config error, 3 missing tool) | scripts and agents can react properly |
| **Example projects in `examples/`** (node + flutter) used by the tests | contributors can try kex in a minute |

## 9. Milestones

1. `kex init`, config lookup + schema, registry, discovery, worktree detection, `kex doctor`
2. `kex pull` + `kex status`, with the unmerged report (including the `gh` lookup for merged PRs)
3. `kex run`, with runtime detection (node, `flutter-ios`) and `run` overrides
4. `kex pr check` + `gh` checks (missing / not logged in / wrong account)
5. Duplicate detection and fixing
6. **v1 release**: Homebrew tap, winget/Scoop, binaries for macOS/Windows/Linux, README, examples
7. `kex push`, more runtimes (v1.x)

Platform priority: features are built and tested on **macOS first**. Windows and Linux must build and pass CI, and get grayed-out or fallback behaviour where a feature is Mac-only.

## 10. Open questions

- None right now. New ones get added here as they come up.

## 11. Decided so far

- Core = pull all + PR list + run all with runtime detection (2026-11-12)
- v1 runtimes: `node` (npm/pnpm/yarn/bun) and `flutter-ios`. Others in v1.x (2026-11-12)
- Platform priority: macOS first; Windows/Linux build and degrade gracefully (2026-11-12)
- Windows shell: PowerShell (`pwsh` if installed, else `powershell.exe`) (2026-11-12)
- File name: `kex.json`, or `.config/kex.json` when `.config/` exists
- Each repo has a `repo_id`; repo identity = root commit; copied configs are fixed interactively
- `prime_branches` is a list (`branch`, `stage`, `description`). `client` dropped until a command needs it (2026-11-12)
- `kex.json` is plain JSON, no comments (2026-11-12)
- Worktrees ignored, except the one you run kex from
- Second clones: all for `pull`/`status`, pick one for `run`/`push`
- `kex push`: select repos, push the current branch, nothing else
- `kex pr check`: one list, detail view, two confirmations
- GitHub only through `gh`; without it, GitHub features are grayed out with install help
- Dropped: trying unmerged branches locally, automatic PRs, merge-and-push
- Public open source: generic, no telemetry, Apache-2.0
- Platforms: macOS, Windows and Linux in v1. OS-specific features (iOS) are grayed out elsewhere.
- `hosting` field picks the PR CLI. Order: field → `origin` domain → `github.com`.
- Name: `kex`. Free on Homebrew; `github.com/kex` is an inactive account, so the repo lives under the `yuji-luigi` user account; npm, crates.io and PyPI packages called `kex` exist; Kali Win-KeX uses the `kex` command on WSL (checked 2026-11-11)
- Language: Rust. One binary, no script files shipped (2026-11-11)
- Repo: `github.com/yuji-luigi/kex`, Homebrew tap `yuji-luigi/homebrew-tap` (2026-11-11)
- The name is `kex` everywhere. No crates.io package, because `kex` is taken there (2026-11-11)

## 12. Implementation status (v0.1, 2026-11-12)

Built and tested (unit tests + end-to-end tests on real git repos; compiles for macOS, Windows and Linux):

- `kex init` (join a project found next to the repo or create one, `--reset` for copied configs), `kex doctor`, `kex scan`, `kex link`
- Later: `kex init` sets up several repos (terminal tree browser, workspace files, `--repos`; alias `kex add`), `--from` with `--dry-run` (all or nothing, one writer for every mode), `kex browse`, optional `description` in kex.json
- Discovery: registry, siblings always, deep scan of `roots` at most once a day on a miss, worktrees ignored except the current one, config read from the prime branch when the checked-out branch has none
- `kex pull` (parallel fetch, fast-forward only, unmerged report from the shared remote, `gh` lookup of merged PRs), `kex status` (`--json`)
- `kex run`: detection (node via `packageManager`/lockfile, `flutter-ios`), `run` overrides, `kex.local.json`, install only when the lockfile changed, port check, prefixed output, Ctrl-C stops all, `ready` URL, PowerShell on Windows
- `flutter-ios`: simulator picker, boot, hot reload on save (`notify` + `SIGUSR1`)
- `kex pr check` (list → detail → confirm → type to accept), `kex push` (select repos, current branch only)
- `gh` states (missing / logged out / wrong account) with install help, shell completions, exit codes, `NO_COLOR`
- JSON Schema at `schema/kex.schema.json` (hand-written for now), CI on all three systems

Simplified on purpose (can be upgraded later):

- Copied configs: kex warns and suggests `kex init --reset` instead of the interactive multi-select fix (§4.4).
- Keyboard in `kex run`: the flutter service gets the keyboard directly, so `r` / `R` / `q` are flutter's own. The `1`–`9` output filter is not built, and there's no `ratatui`. `pr check` uses `inquire` lists (type to filter).
- Port check says the port is busy, but not which process holds it.
- Registry: atomic write (temp file + rename) without a file lock.

Added after v0.1:

- `--json` (global): one JSON event per line, every `inquire` question also available as a `prompt` event (`src/ui.rs`); `kex run` takes `input` / `stop` messages on stdin. Protocol version 1.
- Releases (§8.5).
- cli-events/1 (`spec/cli-events-1.md`, `docs/self-description.md`): `kex manifest` (generated
  from clap + `src/manifest.rs`'s annotation table, with a drift test), `manifest_hash` in
  `hello` (RFC 8785 + SHA-256), a generic `view` and `key` on every result event, message ids
  for UI text (`src/view.rs`), `restates` on log lines, prompt `columns` / option rows,
  actions with `effect` / `confirm`, `exit.tone`, feature `cli_events`. `kex init --json`
  asks for this repo's project and name (`init_prompts`). Limits: `browse --max-entries` ≤
  1,000, `output` lines cut at 16 KiB, `--from` ≤ 1 MiB. The CLI tests validate every `--json`
  line against the schema.

Not built yet: run logs, `kex doctor --prune`, `kex clone`, Homebrew tap, winget/Scoop.

### 8.5 Releases and self-update (built)

- release-please (`.github/workflows/release.yml`, `release-please-config.json`) keeps a release PR open from Conventional Commits; merging it creates tag `vX.Y.Z` and a GitHub release.
- The same workflow builds `kex-<target>.tar.gz` / `.zip` + `.sha256` for aarch64/x86_64 macOS, Linux (musl, static) and Windows, and uploads them to the release.
- `install.sh` / `install.ps1` install the latest release (checksum verified).
- `kex update` downloads the archive for this platform with `curl`, checks the sha256, unpacks with `tar` and replaces the running binary (on Windows the old exe is renamed away first). Installs from Homebrew/Scoop/winget are told to use their package manager.
- Version check: at most once a day, a detached `kex update --check --quiet` refreshes `update.json` in the kex folder; commands never wait on the network. When a newer version is known, kex prints a note after the command (stderr; an `update_available` event in JSON mode) and the menu shows an "update" entry. Off with `KEX_NO_UPDATE_CHECK=1` and in CI.
