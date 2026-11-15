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

macOS / Linux:

```
curl -fsSL https://raw.githubusercontent.com/yuji-luigi/kex/main/install.sh | sh
```

Windows (PowerShell):

```
irm https://raw.githubusercontent.com/yuji-luigi/kex/main/install.ps1 | iex
```

From source (needs [Rust](https://rustup.rs)): `cargo install --git https://github.com/yuji-luigi/kex`

Update with `kex update`. kex checks for a new release once a day in the background, prints
a note after a command when one is out, and the `kex` menu offers an "update" entry.
Set `KEX_NO_UPDATE_CHECK=1` to turn the check off.

For PR features, also install the [GitHub CLI](https://cli.github.com/) and run `gh auth login`.

## Set up a project

Run `kex init` in any repo of the project, then pick the folders:

```
kex init
```

kex shows the folders around the repo as a tree, 3 levels deep, with the repo you're in
already checked. ↑/↓ move, Space checks a repo, Enter opens a folder (it becomes the top of
the tree) or checks a repo, ←/Backspace goes to `../`, Tab (or `d`) is done, Esc cancels.
Repos show their branch; kex repos show their project and can't be checked. kex then asks
for the project (join one found nearby, or a new one) and the repo names, and writes a
`kex.json` in each checked repo, all or nothing, each listing the others as `peers`.

```
? Init which repos? ~/work/shop
  ✓ Done (2 selected)
  ../
      ├─ admin           kex: shop-7f3kq2
  [x] ├─ api             dev
      ├─ clients/        3 repos
      │  └─ acme/        1 repo
  [ ] │     └─ portal    main
  [x] └─ web             main
2 selected: api, web
↑↓ move · enter open/check · space check · ← ../ · tab done · esc cancel
```

When a VS Code workspace file (`*.code-workspace`, also Cursor's) is in the folder, in the
repo or next to it, kex offers its repos first (no = the browser). Without the browser:
`--yes` sets up just this repo · `--repos api,web` these folders next to it · `--workspace
<file>` a workspace file's repos · `--from <file|->` a JSON list (for programs). `kex add`
is the same command.

Commit the generated `kex.json`. Each teammate can keep their own folder layout: kex finds the repos by
the ids in `kex.json` and remembers where they are on each machine. That memory lives in
`~/.config/kex/` (`%APPDATA%\kex` on Windows) and is never committed.

Unusual layouts: if a repo lives far from the others, run `kex scan ~/Development` once,
or `kex link <path>`. Programs (GUIs) list folders with `kex browse --json` and set repos up
with `kex init --from -`; see [docs/gui-integration.md](docs/gui-integration.md).

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
| `kex init [dir]` (alias `add`) | set up repos: pick them in a tree; `--yes`, `--repos`, `--workspace`, `--from` skip it; `--dry-run` previews |
| `kex browse [dir]` | folders as a tree with their repos, and workspace files (`--json` for programs; `--repos-only`, `--no-git`) |
| `kex init --reset` | new ids for a config copied from another repo |
| `kex update` | install the latest release (`--check` only reports) |
| `kex manifest` | every command, option and exit code, for programs (`--json`: a [cli-events/1](spec/cli-events-1.md) manifest) |
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
  "description": "Orders and payments API",
  "prime_branches": [
    { "branch": "dev", "stage": "dev", "description": "Integration branch" },
    { "branch": "main", "stage": "production", "description": "Live" }
  ]
}
```

`description` (optional) is shown by `kex doctor` and `kex status --json`. `run` is
optional. kex detects:

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

## JSON mode (for other programs)

Full reference with TypeScript types for every message: [docs/json-protocol.md](docs/json-protocol.md).

kex speaks [cli-events/1](spec/cli-events-1.md), a small tool-neutral standard (MIT): `kex
manifest --json` describes every command and option, and every result carries a generic
`view` (card, table, tree, service, log), so a client can drive kex with no kex-specific
code. `hello.features` has `"cli_events"` when it does; details in
[docs/self-description.md](docs/self-description.md).

Add `--json` to any command and stdout becomes a stream of JSON objects, one per line,
with nothing else mixed in (output of tools kex starts, like `git push`, goes to stderr).
Every question turns into a `prompt` event; answer it with one line on stdin.

```
$ kex run --json
{"type":"hello","standard":"cli-events/1",…,"kex":"0.2.0","protocol":1,"features":["browse","from",…,"cli_events"]}
{"type":"project","id":"acme-shop-7f3kq2","repos":[…],"missing":[],"warnings":[]}
{"type":"prompt","id":"p1","kind":"multiselect","message":"Run which repos?","default":[0,1],
 "options":[{"index":0,"value":"api","label":"api …","data":{"runtime":"node · pnpm run dev","port":3000,…}},
            {"index":1,"value":"web","label":"web …","data":{…}}]}
← {"id":"p1","answer":["api"]}
{"type":"started","repo":"api","cmd":"pnpm run dev","pid":4242,"path":"…"}
{"type":"output","repo":"api","stream":"stdout","line":"listening on :3000"}
{"type":"ready","repo":"api","url":"http://localhost:3000"}
← {"type":"stop"}
{"type":"exited","repo":"api","code":0,"success":true}
{"type":"exit","code":0,"tone":"ok"}
```

(`key` and `view` on the results are left out above.)

Answers:

| prompt `kind` | `answer` |
|---|---|
| `select` | option `index` or `value` (`null` = skip / back) |
| `multiselect` | array of indexes or values, `"all"`, `"default"` |
| `confirm` | `true` / `false` |
| `text` | a string (`null` = the `default`) |

`{"answer": …}` may leave out `id`; a bare JSON value or plain text line works too.
`{"cancel":true}` (or closing stdin) cancels, exit code 130. An invalid answer gets an
`error` event with the prompt's `id`, and kex waits for another line.

While `kex run` is running, stdin takes `{"type":"input","repo":"app","text":"r"}` (typed
into that service; `repo` defaults to the Flutter one, so `r` / `R` / `q` work) and
`{"type":"stop"}`.

`hello` carries `features` (e.g. `"browse"`, `"from"`): check those to know what this kex
can do, rather than its version. Events: `hello`, `project`, `log` (`level`, `text`: everything a person would see),
`prompt`, `error`, `pull` (per repo, with `unmerged` branches), `status`, `prs`, `merged`,
`pushed`, `added`, `tree`, `workspaces`, `plan`, `pr`, `manifest`, `started`, `output`, `ready`, `exited`, `version`, `update_available`, `updated`,
and always `exit` (`code`, `tone`) last.

## Releases

Commits follow [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`,
`fix:`, …). [release-please](https://github.com/googleapis/release-please) keeps a release PR
open with the changelog and version bump; merging it tags the release and CI attaches
binaries for macOS, Linux and Windows (x86_64 and arm64), which `kex update` installs.

## License

Apache-2.0
