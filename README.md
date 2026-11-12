# kex

Pull, review and run all the repos of a multi-repo project, from any of them, on any folder
layout. One binary instead of a Makefile copied into every project.

```
$ kex pull
project acme-shop-7f3kq2
== api (dev) ✓ up to date
   unmerged:
   feature/login              → dev      +4 / -0  Jane Doe  2026-10-24 18:12
== web (dev) ↓ 3 commits pulled
== app (main) ⚠ uncommitted changes, pull skipped

$ kex run
? Run which repos?
  [x] api      node · pnpm run dev          (pnpm-lock.yaml, script "dev")
  [x] web      node · npm run start         (package-lock.json, script "start")
  [x] app      flutter-ios                  (pubspec.yaml)
```

## Install

From source (needs [Rust](https://rustup.rs)):

```
cargo install --git https://github.com/yuji-luigi/kex
```

Homebrew, winget/Scoop and prebuilt binaries will follow with the first release.

For PR features, also install the [GitHub CLI](https://cli.github.com/) and run `gh auth login`.

## Set up a project

In each repo of the project:

```
kex init
```

The first repo creates the project. The others find it next to them and join it. Commit the
generated `kex.json`. Each teammate can keep their own folder layout: kex finds the repos by
the ids in `kex.json` and remembers where they are on each machine. That memory lives in
`~/.config/kex/` (`%APPDATA%\kex` on Windows) and is never committed.

If a repo lives far from the others, run `kex scan ~/Development` once, or `kex link <path>`.

## Commands

| Command | What it does |
|---|---|
| `kex` | menu |
| `kex pull` | fetch + fast-forward every repo, list branches not merged into a prime branch yet |
| `kex run` | detect how to run each repo, start them together, Ctrl-C stops all |
| `kex pr check` | open PRs of all repos in one list → detail → confirm → type to accept |
| `kex status` | branch, ahead/behind and changes of every repo (`--json`) |
| `kex push` | pick repos, push each one's current branch |
| `kex doctor` | what kex found: paths, ids, remotes, hosting, `gh` state |
| `kex init --reset` | new ids for a config copied from another repo |
| `kex completions zsh` | shell completions (bash, zsh, fish, powershell) |

kex never commits, force-pushes, rebases or merges on its own. Protected branches always
need a typed confirmation.

## `kex.json`

```json
{
  "$schema": "https://raw.githubusercontent.com/yuji-luigi/kex/main/schema/kex.schema.json",
  "version": 1,
  "project": "acme-shop-7f3kq2",
  "repo": "api",
  "repo_id": "r_9xk2mq",
  "prime_branches": [
    { "branch": "dev", "stage": "dev", "description": "Integration branch" },
    { "branch": "main", "stage": "production", "description": "Live" }
  ]
}
```

`run` is optional. kex detects:

- **node** from `package.json`. The package manager comes from `packageManager` or the
  lockfile, the script from `dev`, `start` or `serve`, and the port from `PORT=` in `.env`.
- **flutter-ios** from a Flutter `pubspec.yaml` (macOS). It picks a simulator, boots it and
  runs `flutter run` with hot reload on save. `r` / `R` / `q` work as usual.

To override detection:

```json
"run": { "install": "make deps", "dev": { "default": "make dev", "windows": "make.exe dev" }, "port": 8080 }
```

Personal overrides go in `kex.local.json` next to `kex.json`. It's git-ignored by `kex init`
and holds only `run`.

Exit codes: 0 ok, 1 some repo failed, 2 error, 3 missing tool, 130 cancelled.

## License

Apache-2.0
