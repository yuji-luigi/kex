# kex's profile of cli-events/1

kex implements [cli-events/1](../spec/cli-events-1.md), a small tool-neutral standard for CLIs
driven by programs: a **manifest** that describes kex's commands, options and exit codes, and
a generic **view** on every result. A client that speaks cli-events can offer every kex
command, build its form, run it, answer its prompts and draw its results with no code for
kex. This page says how kex does it: what it adds to its lines, how the manifest is built,
which view each kex event carries, and how repo setup works as a generic flow.

**Status: implemented.** A kex whose `hello.features` has **`"cli_events"`** sends every line
as described here: each line conforms to cli-events/1, and every result carries a `view`. A
client that finds `cli_events` can switch to generic rendering; without it (an older kex), it
falls back to the kex-specific events of [json-protocol.md](json-protocol.md). The example
lines below are checked by kex's tests (schema, message ids, actions); versions, paths and
ids in them are illustrative.

Everything is **additive** to the [kex JSON protocol](json-protocol.md) (`protocol: 1`):
existing types and fields stay exactly as they are, so current clients keep working. A
cli-events client reads only the standard fields; a kex-json client reads only the old
ones; both work on the same stream.

## Contents

1. [What kex adds to its lines](#1-what-kex-adds-to-its-lines)
2. [`kex manifest`](#2-kex-manifest)
3. [Commands and their annotations](#3-commands-and-their-annotations)
4. [Exit codes](#4-exit-codes)
5. [Events and their views](#5-events-and-their-views)
6. [Prompts as tables](#6-prompts-as-tables)
7. [Setting up repos with generic widgets](#7-setting-up-repos-with-generic-widgets)
8. [Actions, limits and stdin](#8-actions-limits-and-stdin)
9. [Messages](#9-messages)
10. [What today's Sakabar cards show, and where it is in a view](#10-what-todays-sakabar-cards-show-and-where-it-is-in-a-view)
11. [Implementation and conformance tests (phase 2)](#11-implementation-and-conformance-tests-phase-2)

## 1. What kex adds to its lines

| line | added |
|---|---|
| `hello` | `standard: "cli-events/1"`, `tool: {name: "kex", version}`, `manifest_version: 1`, `manifest_hash` (section 2.1). Keeps `kex`, `protocol`, `features`. New features: `"manifest"`, `"views"`, `"init_prompts"` (section 7.4), and `"cli_events"`: every line conforms to cli-events/1 with views (switch to generic rendering when present). |
| `exit` | `tone` (section 4), always. |
| every result event | `key` and `view` (section 5). |
| `log` | `msg` / `params` when the text is a fixed template; `level` can now also be `"win"` (a ✓ line) or `"error"` (a ✗ line), which old clients treat as `info`; `restates` when the line says what a view already shows. |
| `error` | `msg` / `params` for fixed messages. `index` / `field` as today. |
| `prompt` | `msg` / `params`; `columns`; per option `view` (cells, badges, tone), `msg` / `params`; `expect` and `tone: "danger"` on typed confirmations; `pattern` / `min_length` / `max_length` on name questions; `picks_from`. |
| stdin | kex also accepts `{"type":"answer","id":…,"answer":…}` and `{"type":"answer","id":…,"cancel":true}` (the old forms still work). |

```ndjson
{"type":"hello","standard":"cli-events/1","tool":{"name":"kex","version":"0.1.0"},"manifest_version":1,"manifest_hash":"05293a7ceda48c3e","kex":"0.1.0","protocol":1,"features":["browse","from","dry_run","workspaces","pin_ids","paging","repos_only","no_git","manifest","views","init_prompts","cli_events"]}
```

(The hash changes whenever the manifest does.) The envelope's reserved names (`type`, `key`, `view`, `ts`) are
free in every kex event today. `id` is the standard prompt id only on `prompt` / `answer` / `error`; kex's `project`
event keeps its own `id` (the project id), as the standard allows. kex sends no `ts`.

## 2. `kex manifest`

`kex manifest --json` sends `hello`, one `manifest` line, and `exit` 0. Without `--json` it
prints the same as text:

```
kex 0.2.0 · cli-events/1 · manifest 1 · 05293a7ceda48c3e
setup
  init (add)   Set up repos for kex: pick them in a folder browser (this repo starts checked)  [interactive · JSON input --from]
  browse       List a folder as a tree with its git repos, and the workspace files around it (for programs: with --json it asks nothing)
  scan         Find kex repos under a folder and remember it as a search root
  link         Register a repo by hand
sync
  pull         Update every repo and show unmerged work
  status       Branch, ahead/behind and changed files of every repo
  push         Push the current branch of selected repos  [interactive]
…
```

The manifest is **generated**, not written by hand:

- **From clap** (`Cli::command()`, `CommandFactory`): command words and aliases (visible ones),
  the one-line description (the first line of the doc comment), every option's id, `--flag`,
  `-short`, positional index, required, default, delimiter (`separator`), possible values
  (`choices`), `conflicts_with` and `requires`, and the type where clap knows it (`bool` flag →
  `boolean`, integer parsers → `integer`, `PathBuf` → `path`, a value enum → `choice`, a
  delimiter → `list`). Hidden commands and options (`update --quiet`) and clap's own
  `--help` / `--version` are left out.
- **From one small annotation table** in `src/manifest.rs`, one entry per command: `label`,
  `group`, `icon`, `interactive`, `long_running`, `read_only`, `stop`, `needs_terminal`,
  `json_input`, and per
  option the `label` and what clap can't know (`type` overrides like `json`, `placeholder`,
  `pattern`, lengths, `min`, `role`, `stdin_value`).
- **Exit codes, event types, view kinds, value types and messages** from constants next to
  the code that uses them.

**It can't drift.** A unit test walks `Cli::command()` and fails when a visible command or
option has no annotation, or an annotation names a command or option clap doesn't have. A
second test checks the manifest against `spec/cli-events-1.schema.json`.

### 2.1 `manifest_hash`

kex computes the hash **at run time**, from the same generator `kex manifest` uses, so the two
can't disagree: build the manifest value, serialize it canonically (RFC 8785: keys sorted,
no whitespace; kex sorts keys itself because its `serde_json` keeps insertion order), SHA-256
it (`sha2`, already a dependency), take the first 16 hex characters. It is computed once per
process, only in JSON mode, before `hello` (well under a millisecond). The manifest holds
nothing volatile: no paths, no times, no "update available" state; the `features` list is
fixed per build.

## 3. Commands and their annotations

The menu (`kex` with no command) isn't in `commands`: a client builds its own menu from them.

| id | words (aliases) | label | group | icon | interactive | long_running | notes |
|---|---|---|---|---|---|---|---|
| `init` | `init` (`add`) | Set up repos | setup | `init` | yes | no | `json_input` (section 7) |
| `browse` | `browse` | Browse folders | setup | `folder` | no | no | `read_only`; its `tree` view drives the picker |
| `scan` | `scan` | Scan for repos | setup | `scan` | no | no | |
| `link` | `link` | Link a repo | setup | `link` | no | no | |
| `pull` | `pull` | Pull | sync | `pull` | no | no | |
| `status` | `status` | Status | sync | `status` | no | no | |
| `push` | `push` | Push | sync | `push` | yes¹ | no | |
| `run` | `run` | Run | run | `run` | yes¹ | yes | `stop: {"type":"stop"}` |
| `pr.check` | `pr check` | Review PRs | review | `pr` | yes | no | |
| `doctor` | `doctor` | Doctor | info | `doctor` | no | no | |
| `update` | `update` | Update kex | meta | `update` | yes | no | |
| `manifest` | `manifest` | Manifest | meta | `manifest` | no | no | `read_only` |
| `completions` | `completions` | Shell completions | meta | `terminal` | no | no | `needs_terminal: true` (writes a script, not JSON) |

¹ Besides their own questions, they ask "`<repo>` has N clones. Which one?" when a repo is
checked out twice.

`read_only` is set only on `browse` and `manifest`: they ask nothing and write nothing.
`status` and `doctor` ask nothing either but may update kex's registry of where repos are (a
file), so they don't get it. The tree's navigation (`open`, `up`, `next`) runs `browse`, so a
client may follow it without a confirm.

Options (types from clap unless marked *annotation*):

| command | option id | flag / position | type | label | constraints and notes |
|---|---|---|---|---|---|
| global | `json` | `--json` | boolean | JSON output | `role: "json"` |
| init | `dir` | positional 0 | path | Folder | conflicts with `from` |
| init | `project` | `--project` | string | Project | *annotation*: `pattern ^[a-z0-9][a-z0-9-]*$`, max 47 |
| init | `repo` | `--repo` | string | Repo name | *annotation*: `pattern ^[a-z0-9][a-z0-9-]*$`, 1–40; conflicts with `repos`, `workspace` |
| init | `yes` | `--yes`, `-y` | boolean | Don't ask | |
| init | `reset` | `--reset` | boolean | New ids for a copied config | conflicts with `from`, `workspace`, `repos` |
| init | `repos` | `--repos` | list | Folders | `separator ","`, placeholder `api,web`; conflicts with `workspace` |
| init | `workspace` | `--workspace` | path | Workspace file | placeholder `shop.code-workspace` |
| init | `from` | `--from` | json *annotation* | Repos (JSON) | `stdin_value "-"`; conflicts with `dir`, `repo`, `repos`, `workspace` |
| init | `dry_run` | `--dry-run` | boolean | Only show what would be written | `role: "dry_run"` |
| init | `depth` | `--depth` | integer | Browser depth | default 3; `min 0` (unsigned, from clap) |
| browse | `dir` | positional 0 | path | Folder | default: kex's folder |
| browse | `depth` | `--depth` | integer | Depth | `min 0` (unsigned) |
| browse | `max_entries` | `--max-entries` | integer | At most | default 500, `min 0` |
| browse | `repos_only` | `--repos-only` (`--flat`) | boolean | Only repos, flat | |
| browse | `no_git` | `--no-git` | boolean | Skip git details (fast) | |
| browse | `git_budget_ms` | `--git-budget-ms` | integer | Git time budget (ms) | default 5000, `min 0` |
| browse | `hidden` | `--hidden` | boolean | Show dot-folders | |
| browse | `all` | `--all` | boolean | Also node_modules, target, … | |
| browse | `offset` | `--offset` | integer | Skip | default 0, `min 0` |
| pull | `all` | `--all` | boolean | Also old branches | |
| pull | `prime` | `--prime` | boolean | Switch to the prime branch first | |
| run | `repos` | `--repos` | list | Repos | `separator ","`, `pattern ^[a-z0-9][a-z0-9-]*$` |
| run | `no_update` | `--no-update` | boolean | Skip pull + install | |
| run | `install` | `--install` | boolean | Always install | |
| run | `sim` | `--sim` | string | iOS simulator | placeholder `iPhone 17 Pro` |
| run | `force` | `--force` | boolean | Start even if the port is taken | |
| push | `repos` | `--repos` | list | Repos | as run |
| push | `yes` | `--yes`, `-y` | boolean | Don't ask | protected branches still ask |
| pr.check | `list` | `--list` | boolean | Only list | |
| scan | `dir` | positional 0 | path | Folder | |
| link | `path` | positional 0 | path | Repo | required |
| completions | `shell` | positional 0 | choice | Shell | required; `bash` `elvish` `fish` `powershell` `zsh` |
| update | `check` | `--check` | boolean | Only check | |
| update | `yes` | `--yes`, `-y` | boolean | Don't ask | |

## 4. Exit codes

| code | tone | title | description |
|---|---|---|---|
| 0 | `ok` | Done | Everything worked. |
| 1 | `partial` | Some repos failed | The others worked: see their results. |
| 2 | `error` | Error | A config, usage or network problem: see the error. |
| 3 | `missing_tool` | A tool is missing | kex needs a tool that isn't installed (e.g. `gh`): see the error. |
| 130 | `cancelled` | Cancelled | Stopped before the end. |

```ndjson
{"type":"exit","code":1,"tone":"partial"}
```

## 5. Events and their views

Every kex event a client should show carries a `key` and a `view`. `hello`, `prompt` and `exit`
are standard lines and carry none; `log` and `error` are standard lines drawn as the log.

| event | key | view kind | what the view holds | log lines it restates |
|---|---|---|---|---|
| `project` | `project` | card | id (subtitle), repos, missing (warn), warnings (notes) | `project <id>`, `⚠ …` |
| `status` | `status` | table | repo, branch, ↑, ↓ (or "no upstream"), changed; badge worktree; description as row detail; missing as a note | — (JSON mode prints no table) |
| `pull` | `pull:<path>` | card | repo, branch, tone, status as summary (with its own msg), path; a table section of unmerged branches; `unmerged_note` as a note | `== repo (branch) status` and its unmerged lines |
| `prs` | `prs` | table | #number + title, repo, author, into (base), from (head), opened; badges production (danger), draft (muted); `empty` | `no open PRs` and the `--list` lines |
| `pr` *(new)* | `pr:<repo>#<n>` | card | title, repo, author, from → into, url, state, draft, production; sections: description, files (+/-), checks (state with tone) | the `gh pr view` / files / checks text |
| `merged` | `merged:<repo>#<n>` | log | win "#12 merged into dev", or warn "still OPEN" | `✓ #12 merged into dev` / `… still …` |
| `pushed` | `push:<repo>` | log | win "Pushed dev (api)" or error "Push of dev failed (api)" | `✗ api: push failed` |
| `started` | `run:<repo>` | service | name, state `starting` (or `running` when kex won't wait for a URL), command, pid, actions (Flutter: hot reload, hot restart, quit) | — |
| `output` | `run:<repo>` | service, append | one line, with its stream | — |
| `ready` | `run:<repo>` | service, append | state `ready`, url, tone ok | — |
| `stopping` | `run:<repo>` | service, append | state `stopping` (a `stop` line or a signal) | — |
| `exited` | `run:<repo>` | service, append | state `exited` (it ended by itself with success, or was stopped: tone muted) or `failed` (danger), exit code | — |
| `version` | `version` | card | "kex 0.3.0 is available" (info) or "… is the latest" (ok); current, latest; action Install | `✓ kex … is the latest`, `kex … is available`, the release URL |
| `update_available` | `update_available` | card | "kex 0.3.0 is available"; action Install | — |
| `updated` | `version` | card | "Updated to kex 0.3.0" (ok): replaces the version card | `✓ kex … installed to …` |
| `added` | `added` | table | project (subtitle), summary; rows written (repo, path, repo_id, prime branches), skipped (muted, badge "already in the project", `note` as detail), failed (danger, message as detail); notes: commit hint, or "nothing was written" | `project …`, `✓ …`, `· … already in …`, `✗ …`, the hint |
| `plan` | `plan` | table | project (subtitle: new or existing), rows (repo, path, repo_id, kex.json path, .gitignore line; the full config in `details`), skipped rows; errors as notes; action "Write N configs" when valid | — |
| `tree` | `tree` | tree | the listing as nodes (section 7.1); `open`, `up`, `next` | — |
| `workspaces` | `workspaces` | cards | one row per workspace file: name, path, repos, to set up, already kex, missing; action "Set up its N repos" (a dry run); unreadable files as danger rows | — |

Views of `run`'s update step are the same `pull` cards.

### 5.1 Literal lines

`project`:

```ndjson
{"type":"project","id":"acme-shop-7f3kq2","repos":[{"repo":"api","path":"/Users/me/src/shop/api","worktree":false},{"repo":"web","path":"/Users/me/src/shop/web","worktree":false}],"missing":[{"repo":"app","remote":"git@github.com:acme/app.git"}],"warnings":[],"key":"project","view":{"kind":"card","title":{"text":"Project","msg":"project.title"},"subtitle":"acme-shop-7f3kq2","fields":[{"key":"repos","label":{"text":"Repos","msg":"field.repos"},"value":["api","web"]},{"key":"missing","label":{"text":"Not on this computer","msg":"field.missing"},"value":["app"],"tone":"warn"}]}}
{"type":"log","level":"info","text":"project acme-shop-7f3kq2","restates":"project"}
```

`status` as a table:

```ndjson
{"type":"status","project":"acme-shop-7f3kq2","repos":[{"repo":"api","path":"/Users/me/src/shop/api","branch":"dev","upstream":true,"ahead":2,"behind":0,"changed":3,"worktree":false,"description":"Orders and payments API"},{"repo":"web","path":"/Users/me/src/shop/web","branch":"feature/cart","upstream":false,"ahead":0,"behind":0,"changed":0,"worktree":true,"description":null}],"missing":["app"],"key":"status","view":{"kind":"table","title":{"text":"Status","msg":"status.title"},"subtitle":"acme-shop-7f3kq2","columns":[{"key":"repo","label":{"text":"Repo","msg":"col.repo"}},{"key":"branch","label":{"text":"Branch","msg":"col.branch"},"type":"branch"},{"key":"ahead","label":{"text":"↑","msg":"col.ahead"},"type":"count"},{"key":"behind","label":{"text":"↓","msg":"col.behind"},"type":"count"},{"key":"changed","label":{"text":"Changed","msg":"col.changed"},"type":"count"}],"rows":[{"key":"/Users/me/src/shop/api","cells":{"repo":"api","branch":"dev","ahead":{"value":2,"tone":"info"},"behind":0,"changed":{"value":3,"tone":"warn"}},"detail":"Orders and payments API"},{"key":"/Users/me/src/shop/web","cells":{"repo":"web","branch":"feature/cart","ahead":{"value":null,"text":{"text":"no upstream","msg":"status.no_upstream"},"tone":"muted"},"behind":null,"changed":0},"badges":[{"text":{"text":"worktree","msg":"badge.worktree"},"tone":"muted"}]}],"notes":[{"level":"warn","text":{"text":"Not on this computer: app","msg":"status.missing","params":{"repos":"app"}}}]}}
```

A `pull` card with its unmerged branches:

```ndjson
{"type":"pull","repo":"api","path":"/Users/me/src/shop/api","branch":"dev","ok":true,"status":"↓ 3 commits pulled","unmerged":[{"branch":"feature/login","prime":"dev","ahead":4,"behind":0,"author":"Jane Doe","date":"2026-10-24 18:12"}],"unmerged_note":null,"key":"pull:/Users/me/src/shop/api","view":{"kind":"card","title":"api","subtitle":"dev","tone":"ok","summary":{"text":"↓ 3 commits pulled","msg":"pull.pulled","params":{"n":3}},"fields":[{"key":"path","label":{"text":"Path","msg":"field.path"},"value":"/Users/me/src/shop/api","type":"path"}],"sections":[{"kind":"table","title":{"text":"Not merged yet","msg":"pull.unmerged"},"columns":[{"key":"branch","label":{"text":"Branch","msg":"col.branch"},"type":"branch"},{"key":"ahead","label":{"text":"↑","msg":"col.ahead"},"type":"count"},{"key":"behind","label":{"text":"↓","msg":"col.behind"},"type":"count"},{"key":"prime","label":{"text":"Prime","msg":"col.prime"},"type":"branch"},{"key":"author","label":{"text":"Author","msg":"col.author"}},{"key":"last_commit","label":{"text":"Last commit","msg":"col.last_commit"},"type":"date"}],"rows":[{"key":"feature/login","cells":{"branch":"feature/login","ahead":4,"behind":0,"prime":"dev","author":"Jane Doe","last_commit":"2026-10-24T18:12:00+02:00"}}]}]}}
{"type":"log","level":"info","text":"== api (dev) ↓ 3 commits pulled","restates":"pull:/Users/me/src/shop/api"}
```

The tone and summary of a pull card by status:

| status | `msg` (params) | tone |
|---|---|---|
| ✓ up to date | `pull.up_to_date` | ok |
| ↓ N commits pulled | `pull.pulled` (`n`) | ok |
| ⚠ uncommitted changes, pull skipped | `pull.dirty` | warn |
| ⚠ diverged (A ahead, B behind), pull skipped | `pull.diverged` (`ahead`, `behind`) | warn |
| no upstream, pull skipped | `pull.no_upstream` | muted |
| ✗ fetch failed: … | `pull.fetch_failed` (`error`) | danger |
| ✗ could not switch to P: … | `pull.switch_failed` (`branch`, `error`) | danger |
| ✗ pull failed: … | `pull.failed` (`error`) | danger |

`unmerged_note` becomes a muted note: `pull.no_remote`, `pull.no_prime` (`remote`).

`prs`, and the PR prompt that picks from it:

```ndjson
{"type":"prs","items":[{"repo":"api","number":12,"title":"Login","head":"feature/login","base":"dev","author":"jane","created_at":"2026-11-10T09:03:00Z","draft":false,"production":false},{"repo":"web","number":7,"title":"Release","head":"dev","base":"main","author":"sam","created_at":"2026-11-12T16:20:00Z","draft":false,"production":true}],"key":"prs","view":{"kind":"table","title":{"text":"Open PRs","msg":"prs.title"},"columns":[{"key":"pr","label":{"text":"PR","msg":"col.pr"}},{"key":"repo","label":{"text":"Repo","msg":"col.repo"}},{"key":"author","label":{"text":"Author","msg":"col.author"}},{"key":"base","label":{"text":"Into","msg":"col.base"},"type":"branch"},{"key":"head","label":{"text":"From","msg":"col.head"},"type":"branch"},{"key":"opened","label":{"text":"Opened","msg":"col.opened"},"type":"date"}],"rows":[{"key":"api#12","cells":{"pr":{"value":12,"text":"#12 Login"},"repo":"api","author":"jane","base":"dev","head":"feature/login","opened":"2026-11-10T09:03:00Z"},"badges":[]},{"key":"web#7","cells":{"pr":{"value":7,"text":"#7 Release"},"repo":"web","author":"sam","base":"main","head":"dev","opened":"2026-11-12T16:20:00Z"},"badges":[{"text":{"text":"production","msg":"badge.production"},"tone":"danger"}]}],"empty":{"text":"No open PRs","msg":"prs.empty"}}}
{"type":"prompt","id":"p1","kind":"select","message":"Open PRs","msg":"pr.pick","picks_from":"prs","columns":[{"key":"pr","label":{"text":"PR","msg":"col.pr"}},{"key":"repo","label":{"text":"Repo","msg":"col.repo"}},{"key":"author","label":{"text":"Author","msg":"col.author"}},{"key":"base","label":{"text":"Into","msg":"col.base"},"type":"branch"}],"options":[{"index":0,"value":"api#12","label":"api      #12    Login","view":{"cells":{"pr":{"value":12,"text":"#12 Login"},"repo":"api","author":"jane","base":"dev"}},"data":{"repo":"api","number":12,"title":"Login","head":"feature/login","base":"dev","author":"jane","created_at":"2026-11-10T09:03:00Z","draft":false,"production":false}},{"index":1,"value":"web#7","label":"web      #7     Release","view":{"cells":{"pr":{"value":7,"text":"#7 Release"},"repo":"web","author":"sam","base":"main"},"badges":[{"text":{"text":"production","msg":"badge.production"},"tone":"danger"}]},"data":{"repo":"web","number":7,"title":"Release","head":"dev","base":"main","author":"sam","created_at":"2026-11-12T16:20:00Z","draft":false,"production":true}},{"index":2,"value":"quit","label":"Quit","msg":"pr.quit"}]}
```

A PR's detail (new `pr` event, sent before the merge / diff / back question):

```ndjson
{"type":"pr","repo":"api","number":12,"key":"pr:api#12","view":{"kind":"card","title":"#12 Login","subtitle":"api","fields":[{"key":"author","label":{"text":"Author","msg":"field.author"},"value":"jane"},{"key":"branches","label":{"text":"Branches","msg":"field.branches"},"value":"feature/login → dev","type":"code"},{"key":"url","label":{"text":"Link","msg":"field.url"},"value":"https://github.com/acme/api/pull/12","type":"url"}],"sections":[{"kind":"card","title":{"text":"Description","msg":"pr.body"},"fields":[{"key":"description","label":{"text":"Description","msg":"field.description"},"value":"Adds the login form.","type":"code"}]},{"kind":"table","title":{"text":"Files changed","msg":"pr.files"},"columns":[{"key":"file","label":{"text":"File","msg":"col.file"},"type":"path"},{"key":"additions","label":{"text":"+","msg":"col.additions"},"type":"count"},{"key":"deletions","label":{"text":"−","msg":"col.deletions"},"type":"count"}],"rows":[{"key":"src/login.ts","cells":{"file":"src/login.ts","additions":120,"deletions":4}}]},{"kind":"table","title":{"text":"Checks","msg":"pr.checks"},"columns":[{"key":"check","label":{"text":"Check","msg":"col.check"}},{"key":"state","label":{"text":"State","msg":"col.state"},"type":"badge"}],"rows":[{"key":"0:ci","cells":{"check":"ci","state":{"value":"success","tone":"ok"}}}]}]}}
```

`merged`, `pushed`:

```ndjson
{"type":"merged","repo":"api","number":12,"state":"MERGED","key":"merged:api#12","view":{"kind":"log","lines":[{"level":"win","text":{"text":"#12 merged into dev","msg":"pr.merged","params":{"number":12,"base":"dev"}}}]}}
{"type":"pushed","repo":"api","branch":"dev","ok":true,"key":"push:api","view":{"kind":"log","lines":[{"level":"win","text":{"text":"Pushed dev (api)","msg":"push.done","params":{"branch":"dev","repo":"api"}}}]}}
{"type":"pushed","repo":"web","branch":"main","ok":false,"key":"push:web","view":{"kind":"log","lines":[{"level":"error","text":{"text":"Push of main failed (web)","msg":"push.failed","params":{"branch":"main","repo":"web"}}}]}}
{"type":"log","level":"error","text":"✗ web: push failed","restates":"push:web"}
```

A service (`kex run`): started, a line, ready, exited:

```ndjson
{"type":"started","repo":"app","cmd":"flutter run -d 5C1F2A","pid":4242,"path":"/Users/me/src/shop/app","key":"run:app","view":{"kind":"service","name":"app","state":"running","command":"flutter run -d 5C1F2A","pid":4242,"actions":[{"label":{"text":"Hot reload","msg":"run.hot_reload"},"send":{"type":"input","repo":"app","text":"r"}},{"label":{"text":"Hot restart","msg":"run.hot_restart"},"send":{"type":"input","repo":"app","text":"R"}},{"label":{"text":"Quit","msg":"run.quit_app"},"send":{"type":"input","repo":"app","text":"q"}}]}}
{"type":"started","repo":"api","cmd":"pnpm run dev","pid":4250,"path":"/Users/me/src/shop/api","key":"run:api","view":{"kind":"service","name":"api","state":"starting","command":"pnpm run dev","pid":4250}}
{"type":"output","repo":"api","stream":"stdout","line":"listening on :3000","key":"run:api","view":{"kind":"service","mode":"append","lines":[{"text":"listening on :3000","stream":"stdout"}]}}
{"type":"ready","repo":"api","url":"http://localhost:3000","key":"run:api","view":{"kind":"service","mode":"append","state":"ready","url":"http://localhost:3000","tone":"ok"}}
{"type":"exited","repo":"api","code":0,"success":true,"key":"run:api","view":{"kind":"service","mode":"append","state":"exited","exit_code":0,"tone":"muted"}}
{"type":"exited","repo":"app","code":1,"success":false,"key":"run:app","view":{"kind":"service","mode":"append","state":"failed","exit_code":1,"tone":"danger"}}
```

The manifest's `run` command has `stop: {"type":"stop"}`: a client's Stop button sends it.

`version`, `update_available`, `updated`:

```ndjson
{"type":"version","current":"0.2.0","latest":"0.3.0","update":true,"key":"version","view":{"kind":"card","title":{"text":"kex 0.3.0 is available","msg":"update.available","params":{"latest":"0.3.0"}},"tone":"info","fields":[{"key":"installed","label":{"text":"Installed","msg":"field.installed"},"value":"0.2.0","type":"code"},{"key":"latest","label":{"text":"Latest","msg":"field.latest"},"value":"0.3.0","type":"code"}],"actions":[{"label":{"text":"Install 0.3.0","msg":"update.install","params":{"version":"0.3.0"}},"effect":"install","confirm":{"text":"Install kex 0.3.0 now?","msg":"update.install_confirm","params":{"version":"0.3.0"}},"run":{"command":"update","options":{"yes":true}}}]}}
{"type":"update_available","current":"0.2.0","latest":"0.3.0","key":"update_available","view":{"kind":"card","title":{"text":"kex 0.3.0 is available","msg":"update.available","params":{"latest":"0.3.0"}},"tone":"info","actions":[{"label":{"text":"Install 0.3.0","msg":"update.install","params":{"version":"0.3.0"}},"effect":"install","confirm":{"text":"Install kex 0.3.0 now?","msg":"update.install_confirm","params":{"version":"0.3.0"}},"run":{"command":"update","options":{"yes":true}}}]}}
{"type":"updated","version":"0.3.0","key":"version","view":{"kind":"card","title":{"text":"Updated to kex 0.3.0","msg":"update.done","params":{"version":"0.3.0"}},"tone":"ok"}}
```

`added` (one written, one already in the project):

```ndjson
{"type":"added","project":"shop-7f3kq2","repos":[{"repo":"web","repo_id":"r_k2m9qa","path":"/Users/me/src/shop/web","config":"/Users/me/src/shop/web/kex.json","prime_branches":[{"branch":"main","stage":"production","description":"Live"}],"peers":["admin"],"description":null}],"skipped":[{"index":1,"path":"/Users/me/src/shop/admin","folder":"admin","reason":"already_kex","in_project":true,"repo":"admin","project":"shop-7f3kq2"}],"failed":[],"key":"added","view":{"kind":"table","title":{"text":"Set up","msg":"added.title"},"subtitle":"shop-7f3kq2","tone":"ok","summary":{"text":"1 repo set up in shop-7f3kq2","msg":"added.summary_one","params":{"n":1,"project":"shop-7f3kq2"}},"columns":[{"key":"repo","label":{"text":"Repo","msg":"col.repo"}},{"key":"path","label":{"text":"Folder","msg":"col.folder"},"type":"path"},{"key":"repo_id","label":{"text":"Id","msg":"col.repo_id"},"type":"code"},{"key":"prime_branches","label":{"text":"Prime branches","msg":"col.prime_branches"},"type":"branch"}],"rows":[{"key":"/Users/me/src/shop/web","cells":{"repo":"web","path":"/Users/me/src/shop/web","repo_id":"r_k2m9qa","prime_branches":["main"]}},{"key":"/Users/me/src/shop/admin","cells":{"repo":"admin","path":"/Users/me/src/shop/admin","repo_id":null,"prime_branches":null},"tone":"muted","badges":[{"text":{"text":"already in the project","msg":"badge.already_kex"},"tone":"muted"}]}],"notes":[{"level":"info","text":{"text":"Check the prime branches, then commit kex.json in each repo.","msg":"added.commit_hint"}}]}}
```

When a write fails: `tone: "danger"`, summary `added.failed` ("Nothing was written (all or
nothing)"), the failed repo's row with `tone: "danger"` and its message as `detail`.

`error` and the exit after it:

```ndjson
{"type":"error","message":"/src/shop/docs isn't a git repo","msg":"from.not_git_repo","params":{"path":"/src/shop/docs"},"index":2,"field":"path"}
{"type":"exit","code":2,"tone":"error"}
```

`plan`, `tree` and `workspaces` are in section 7.

## 6. Prompts as tables

A select / multiselect that lists things (repos, PRs, simulators, clones, projects, the menu's
commands) gets `columns`, and each of those options a `view` row whose cells are under those
columns. Fixed choices (`pr.merge` / `pr.diff` / `pr.back`, `pr.quit`, `init.no_workspace`)
carry only a `msg` label and no row: a client draws them from `label`. Every question has a
`msg`.

| prompt (`msg`) | columns | badges / tone |
|---|---|---|
| menu `kex` (`menu.pick`) | command, description | `pr`: "needs gh" (warn) when gh isn't ready |
| Run which repos? (`run.pick`) | repo, runtime, port (number), why (reason) | — |
| iOS simulator (`run.simulator`) | name, runtime | booted (ok) |
| `<repo>` has N clones. Which one? (`clones.pick`, `repo`, `n`) | path | — |
| Remember this choice? (`clones.remember`) | confirm | — |
| Push the current branch of which repos? (`push.pick`) | repo, branch, ↑ ahead | changes (warn), new branch (muted) |
| Push these? (`push.confirm`) | confirm | — |
| Type `<branch>` to push (`push.typed`, `repo`, `branch`) | text, `expect: <branch>`, `tone: danger` | — |
| Open PRs (`pr.pick`) | PR, repo, author, into; `picks_from: "prs"` | production (danger), draft (muted) |
| `#12 Login` (`pr.action`) | none: merge, diff, back are fixed options with `pr.merge`, `pr.diff`, `pr.back` | — |
| Merge #12 …? (`pr.confirm_merge`) | confirm, `tone: danger` when production | — |
| Type `accept` to merge (`pr.typed`, `word`) | text, `expect: <word>`, `tone: danger` | — |
| Install kex V? (`update.confirm`, `version`) | confirm | — |
| Project (`init.project`) | project, repos | — |
| New project name: (`init.new_project_name`) | text, 1–40 | — |
| Repo name for `<folder>`: (`init.repo_name`, `folder`) | text, `pattern`, 1–40 | — |

A multiselect with columns:

```ndjson
{"type":"prompt","id":"p1","kind":"multiselect","message":"Run which repos?","msg":"run.pick","columns":[{"key":"repo","label":{"text":"Repo","msg":"col.repo"}},{"key":"runtime","label":{"text":"Runs with","msg":"col.runtime"},"type":"code"},{"key":"port","label":{"text":"Port","msg":"col.port"},"type":"number"},{"key":"reason","label":{"text":"Why","msg":"col.reason"}}],"options":[{"index":0,"value":"api","label":"api      node · pnpm run dev          (pnpm-lock.yaml, script \"dev\")","view":{"cells":{"repo":"api","runtime":"node · pnpm run dev","port":3000,"reason":"pnpm-lock.yaml, script \"dev\""}},"data":{"runtime":"node · pnpm run dev","reason":"pnpm-lock.yaml, script \"dev\"","port":3000,"path":"/Users/me/src/shop/api"}},{"index":1,"value":"app","label":"app      flutter-ios                  (pubspec.yaml)","view":{"cells":{"repo":"app","runtime":"flutter-ios","port":null,"reason":"pubspec.yaml"}},"data":{"runtime":"flutter-ios","reason":"pubspec.yaml","port":null,"path":"/Users/me/src/shop/app"}}],"default":[0,1]}
```

A typed confirmation:

```ndjson
{"type":"prompt","id":"p4","kind":"text","message":"Type `main` to merge:","msg":"pr.typed","params":{"word":"main"},"default":null,"expect":"main","tone":"danger"}
{"type":"answer","id":"p4","answer":"main"}
```

## 7. Setting up repos with generic widgets

`init` is a JSON-input command. Its manifest entry:

```ndjson
{"type":"manifest","manifest":{"manifest_version":1,"standard":"cli-events/1","tool":{"name":"kex","version":"0.1.0","label":"kex","description":"Pull, review and run all the repos of a multi-repo project","homepage":"https://github.com/yuji-luigi/kex"},"features":["browse","from","dry_run","workspaces","pin_ids","paging","repos_only","no_git","manifest","views","init_prompts","cli_events"],"global_options":[{"id":"json","flag":"--json","type":"boolean","label":"JSON output","description":"Machine-readable output: one JSON event per line on stdout","role":"json"}],"commands":[{"id":"init","words":["init"],"label":"Set up repos","description":"Set up repos for kex: pick them in a folder browser (this repo starts checked)","group":"setup","icon":"init","interactive":true,"long_running":false,"options":[{"id":"dir","positional":0,"type":"path","label":"Folder","description":"Folder to browse (default: the folder around this repo)","conflicts_with":["from"]},{"id":"project","flag":"--project","type":"string","label":"Project","description":"Project id to join (default: this repo's project, or ask)","pattern":"^[a-z0-9][a-z0-9-]*$","max_length":47},{"id":"repo","flag":"--repo","type":"string","label":"Repo name","description":"Repo name, used in flags like --repos (sets up only this repo)","pattern":"^[a-z0-9][a-z0-9-]*$","min_length":1,"max_length":40,"conflicts_with":["from","repos","workspace"]},{"id":"yes","flag":"--yes","short":"-y","type":"boolean","label":"Don't ask","description":"Don't ask"},{"id":"reset","flag":"--reset","type":"boolean","label":"New ids for a copied config","description":"Give an existing (copied) config a new repo_id and name","conflicts_with":["from","repos","workspace"]},{"id":"repos","flag":"--repos","type":"list","label":"Folders","description":"Set up these folders (next to this repo, or in DIR), e.g. api,web","placeholder":"api,web","separator":",","conflicts_with":["from","repo","reset","workspace"]},{"id":"workspace","flag":"--workspace","type":"path","label":"Workspace file","description":"Take the repos from this workspace file (*.code-workspace)","placeholder":"shop.code-workspace","conflicts_with":["from","repo","repos","reset"]},{"id":"from","flag":"--from","type":"json","label":"Repos (JSON)","description":"Set up the repos listed in this JSON file (`-` = stdin), asking nothing","conflicts_with":["dir","repo","repos","reset","workspace"],"stdin_value":"-"},{"id":"dry_run","flag":"--dry-run","type":"boolean","label":"Only show what would be written","description":"Only print what would be written","role":"dry_run"},{"id":"depth","flag":"--depth","type":"integer","label":"Browser depth","description":"How many folder levels the browser shows","default":3,"min":0}],"aliases":[["add"]],"json_input":{"option":"from","schema":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","required":["repos"],"additionalProperties":false,"properties":{"project":{"oneOf":[{"type":"string","title":"Join this project","pattern":"^[a-z0-9][a-z0-9-]*$","maxLength":47},{"type":"object","title":"A new project","required":["new"],"additionalProperties":false,"properties":{"new":{"type":"string","title":"Name","minLength":1,"maxLength":40},"id":{"type":"string","pattern":"^[a-z0-9][a-z0-9-]*$","maxLength":47}}}]},"repos":{"type":"array","minItems":1,"items":{"type":"object","required":["path"],"additionalProperties":false,"properties":{"path":{"type":"string","title":"Folder","minLength":1},"name":{"type":"string","title":"Repo name","pattern":"^[a-z0-9][a-z0-9-]*$","minLength":1,"maxLength":40},"description":{"type":"string","title":"Description","maxLength":200,"pattern":"^[^\\u0000-\\u001f\\u007f]*$"},"repo_id":{"type":"string","pattern":"^r_[a-z0-9]{1,32}$"}}}}}},"dry_run":"dry_run","picker":{"command":"browse","options":{"depth":3},"select_kinds":["repo","symlink"],"into":"/repos"}}}],"exit_codes":[{"code":0,"tone":"ok","title":"Done","description":"Everything worked."},{"code":1,"tone":"partial","title":"Some repos failed","description":"The others worked: see their results."},{"code":2,"tone":"error","title":"Error","description":"A config, usage or network problem: see the error."},{"code":3,"tone":"missing_tool","title":"A tool is missing","description":"kex needs a tool that isn't installed (e.g. gh): see the error."},{"code":130,"tone":"cancelled","title":"Cancelled","description":"Stopped before the end."}],"views":{"kinds":["card","table","cards","tree","service","log"],"value_types":["text","number","count","branch","path","url","date","badge","code"]},"event_types":[{"type":"added","view_kind":"table","description":"kex init: the repos set up."},{"type":"plan","view_kind":"table","description":"kex init --dry-run: what would be written."},{"type":"tree","view_kind":"tree","description":"kex browse: a folder as a tree."}],"messages":{"badge.kex_project":"kex: {project}","init.repo_name":"Repo name for {folder}:","added.title":"Set up","plan.title":"Will set up","plan.apply":"Write {n} configs","plan.apply_one":"Write {n} config"}}}
```

(An excerpt: the real manifest lists every command, event type and message.) The schema is
the `FromInput` of [json-protocol.md](json-protocol.md#setting-up-repos-from-a-list---from-and-kex-browse),
object form (kex also takes a bare array; a client builds the object).

### 7.1 Pick: `kex browse`'s tree view

Each `BrowseEntry` becomes a node:

| entry | node |
|---|---|
| any | `key` and `value` = path, `label` = name, `kind` = entry kind, `children` |
| `repo` | `selectable`, `item: {path, name}` (name = kex's default name), cells branch, changed (`dirty`), stack; `checked` when it's the repo kex runs in |
| `kex_repo` | not selectable; badge "kex: `<project>`" (muted); broken config: badge "broken kex.json" (danger) |
| `worktree` | not selectable; badge "worktree" (muted) |
| `symlink` to a repo | selectable, `item: {path: <link>, name}`; detail "→ target" |
| other `symlink` | not selectable, muted, detail "→ target" (or "missing") |
| `dir` | detail "N repos" (`repos_below`); `more: true` when `truncated` |
| unreadable | tone danger, detail = its error |
| `git_partial` | a note: "Some repos have no git details (time budget)" |

The view's `open` is `browse` with the same options and `dir` = the node's value; `up` is
`browse` on `parent`; `next` is `browse` with `offset: next_offset` when `has_more`.

```ndjson
{"type":"tree","root":"/Users/me/src/shop","parent":"/Users/me/src","kind":"dir","readable":true,"error":null,"depth":1,"max_entries":500,"count":3,"truncated":false,"offset":0,"children_total":3,"has_more":false,"next_offset":null,"git":true,"git_partial":false,"flat":false,"tree":[],"key":"tree","view":{"kind":"tree","root":{"label":"~/src/shop","value":"/Users/me/src/shop"},"columns":[{"key":"branch","label":{"text":"Branch","msg":"col.branch"},"type":"branch"},{"key":"changed","label":{"text":"Changed","msg":"col.changed"},"type":"count"},{"key":"stack","label":{"text":"Stack","msg":"col.stack"},"type":"badge"}],"nodes":[{"key":"/Users/me/src/shop/admin","label":"admin","value":"/Users/me/src/shop/admin","kind":"kex_repo","selectable":false,"badges":[{"text":{"text":"kex: shop-7f3kq2","msg":"badge.kex_project","params":{"project":"shop-7f3kq2"}},"tone":"muted"}]},{"key":"/Users/me/src/shop/api","label":"api","value":"/Users/me/src/shop/api","kind":"repo","selectable":true,"checked":true,"item":{"path":"/Users/me/src/shop/api","name":"api"},"cells":{"branch":"dev","changed":{"value":2,"tone":"warn"},"stack":["node"]}},{"key":"/Users/me/src/shop/clients","label":"clients","value":"/Users/me/src/shop/clients","kind":"dir","detail":{"text":"1 repo","msg":"tree.repos_below_one","params":{"n":1}},"more":true,"children":[]}],"open":{"run":{"command":"browse","options":{"depth":1}},"option":"dir"},"up":{"label":{"text":"../","msg":"tree.up"},"run":{"command":"browse","options":{"dir":"/Users/me/src","depth":1}}}}}
```

(The raw `tree` array is left empty here for length; kex sends it in full, as today.)

Workspace files found next to the folder come as `cards`, each with a dry-run action, so a
client can offer "use this workspace" with no extra code:

```ndjson
{"type":"workspaces","workspaces":[],"key":"workspaces","view":{"kind":"cards","title":{"text":"Workspace files","msg":"workspaces.title"},"columns":[{"key":"name","label":{"text":"Name","msg":"col.name"}},{"key":"file","label":{"text":"File","msg":"col.file"},"type":"path"},{"key":"addable","label":{"text":"To set up","msg":"col.addable"},"type":"count"},{"key":"already_kex","label":{"text":"Already kex","msg":"col.already_kex"},"type":"count"},{"key":"missing","label":{"text":"Missing","msg":"col.missing"},"type":"count"}],"rows":[{"key":"/Users/me/src/shop/shop.code-workspace","cells":{"name":"shop","file":"/Users/me/src/shop/shop.code-workspace","addable":2,"already_kex":1,"missing":1},"actions":[{"label":{"text":"Set up its 2 repos","msg":"workspaces.use","params":{"n":2}},"confirm":{"text":"Check the 2 repos of shop.code-workspace (nothing is written yet)?","msg":"workspaces.use_confirm","params":{"n":2,"file":"shop.code-workspace"}},"run":{"command":"init","options":{"workspace":"/Users/me/src/shop/shop.code-workspace","dry_run":true}}}]}],"empty":{"text":"No workspace files here","msg":"workspaces.empty"}}}
```

### 7.2 Edit

The client shows the picked items as a form from `json_input.schema`: each repo's folder
(read-only, it came from the tree), name (prefilled from `item.name`, `pattern`, 1–40),
description (≤ 200, one line), and the project: one of the ids in the tree's `kex: …`
badges, or a new name.

### 7.3 Review and run

The client runs `init` with `from: "-"`, `dry_run: true` and the document on stdin. Problems
come back as `error` lines with `index` / `field` (the client shows each at its item, back to
7.2). A valid request comes back as the `plan` table with the real run as its action, the
request pinned (project and repo ids), so what is written is exactly what was shown:

```ndjson
{"type":"plan","valid":true,"project":{"id":"shop-k2m9qa","new":true,"name":"Shop"},"repos":[],"skipped":[],"errors":[],"request":{"project":{"new":"Shop","id":"shop-k2m9qa"},"repos":[{"path":"/Users/me/src/shop/api","name":"api","repo_id":"r_9xk2mq"}]},"key":"plan","view":{"kind":"table","title":{"text":"Will set up","msg":"plan.title"},"subtitle":{"text":"New project shop-k2m9qa","msg":"plan.new_project","params":{"id":"shop-k2m9qa"}},"tone":"info","columns":[{"key":"repo","label":{"text":"Repo","msg":"col.repo"}},{"key":"path","label":{"text":"Folder","msg":"col.folder"},"type":"path"},{"key":"repo_id","label":{"text":"Id","msg":"col.repo_id"},"type":"code"},{"key":"gitignore","label":{"text":".gitignore","msg":"col.gitignore"},"type":"code"}],"rows":[{"key":"/Users/me/src/shop/api","cells":{"repo":"api","path":"/Users/me/src/shop/api","repo_id":"r_9xk2mq","gitignore":"kex.local.json"},"details":[{"key":"config_path","label":{"text":"Writes","msg":"field.writes"},"value":"/Users/me/src/shop/api/kex.json","type":"path"},{"key":"config","label":{"text":"kex.json","msg":"field.config"},"value":"{\n  \"version\": 1,\n  \"project\": \"shop-k2m9qa\",\n  \"repo\": \"api\",\n  \"repo_id\": \"r_9xk2mq\"\n}","type":"code"}]}],"actions":[{"label":{"text":"Write 1 config","msg":"plan.apply_one","params":{"n":1}},"tone":"ok","effect":"write","confirm":{"text":"Write 1 kex.json into shop-k2m9qa?","msg":"plan.apply_confirm","params":{"n":1,"project":"shop-k2m9qa"}},"run":{"command":"init","options":{"from":"-"},"stdin":{"project":{"new":"Shop","id":"shop-k2m9qa"},"repos":[{"path":"/Users/me/src/shop/api","name":"api","repo_id":"r_9xk2mq"}]}}}]}}
```

(`repos` of the raw event left empty here for length.) An invalid plan has `tone: "danger"`,
its errors as notes, and no action. Pressing the action runs it; the `added` table shows the
result.

### 7.4 `kex init --json` asks again (single repo)

`kex init --json` without `--from`, `--yes`, `--repos` or `--workspace` sets up **the repo kex
runs in** (or `DIR`) and asks the two questions of kex 0.1: the project (when a project is
found nearby; else it goes straight to a new one) and the repo name. See
[json-protocol.md](json-protocol.md#setting-up-repos-kex-init-alias-kex-add). Feature
`"init_prompts"` says a kex does this.

```ndjson
{"type":"prompt","id":"p1","kind":"select","message":"Project","msg":"init.project","columns":[{"key":"project","label":{"text":"Project","msg":"col.project"},"type":"code"},{"key":"repos","label":{"text":"Repos","msg":"col.repos"}}],"options":[{"index":0,"value":"shop-7f3kq2","label":"Join shop-7f3kq2 (admin, web)","msg":"init.join","params":{"project":"shop-7f3kq2","repos":"admin, web"},"view":{"cells":{"project":"shop-7f3kq2","repos":["admin","web"]}},"data":{"repos":["admin","web"]}},{"index":1,"value":"new","label":"Create a new project","msg":"init.new_project","view":{"cells":{"project":{"value":null,"text":{"text":"Create a new project","msg":"init.new_project"}},"repos":null}}}]}
{"type":"prompt","id":"p2","kind":"text","message":"Repo name for api:","msg":"init.repo_name","params":{"folder":"api"},"default":"api","pattern":"^[a-z0-9][a-z0-9-]*$","min_length":1,"max_length":40}
```

### 7.5 What a client still needs

No kex screen, but these generic widgets (cli-events section 10): a **tree picker** (checkboxes
on selectable nodes, open / up / next), a **form** from a JSON Schema (strings with pattern
and length, a choice between a string and an object for `project`), a **table** with
expandable rows (`details`) and row actions, **cards**, the **log**, buttons for
**actions**. kex can't make the choice of project a picker by itself: the client fills it
from the `kex: <project>` badges it has seen, or a text field.

## 8. Actions, limits and stdin

**The actions kex sends.** Every one passes cli-events' RunAction checks against kex's own
manifest (a phase 2 test runs them all), and kex never sets `cwd`: a new run starts where
the current one did.

| where | action | kind | `effect` / `tone` | confirm |
|---|---|---|---|---|
| `version`, `update_available` card | Install 0.3.0 → `update --yes` | run | `install` | "Install kex 0.3.0 now?" |
| `plan` table (valid) | Write N configs → `init --from -` with the pinned request on stdin | run | `write`, ok | "Write N kex.json into `<project>`?" |
| `workspaces` row | Set up its N repos → `init --workspace <file> --dry-run` | run | — | "Check the N repos of `<file>` (nothing is written yet)?" |
| `tree` `open` / `up` / `next` | `browse` another folder or page | navigation | — | none needed: `browse` is `read_only` |
| Flutter `service` | Hot reload / Hot restart / Quit → `{"type":"input","repo":…,"text":"r"\|"R"\|"q"}` | send | — | — |

Whether a client offers Install from a phone is its own policy (Sakabar: yes, after the
confirm, pressed by a person).

**stdin is untrusted.** kex already checks every answer against the open prompt (an
`error` with the prompt's `id` otherwise) and never runs stdin through a shell. During
`kex run` it accepts only `input` (to a service that is running; `text` is written to that
service's stdin as typed, the same as a person at the terminal) and `stop`; anything else
is a `log` with `level: "error"` and a `msg` (`run.unknown_message`, …): the run goes on, so
it isn't an `error` line, which would mean the run failed. `--from` input is checked in full
by kex whatever the client checked, and read only up to the limit.

**Signals.** In JSON mode one handler (`src/cancel.rs`) takes SIGINT / SIGTERM (Ctrl-C /
Ctrl-Break on Windows) for every command: kex sends `exit` 130 `cancelled`, stops the
programs it started (git, gh, installs) and exits, also while a prompt is open. Two places
clean up first: the setup writes (everything written is undone, then 130) and `kex run`
(each service gets `stopping`, then `exited` muted, then 130). A Stop line (`{"type":"stop"}`)
stops `kex run` the same way but ends with the run's own code (0 when all went well). Nothing
is sent after `exit`.

**Failing without `gh`.** `kex pr check` without a usable GitHub CLI sends a fatal `error`
(`msg` `gh.missing` or `gh.logged_out`, the install help in `detail`), then `exit` 3
(`missing_tool`).

**Discovery is quiet.** `kex manifest --json` sends exactly `hello`, `manifest`, `exit`: no
update notice and no update check.

**Browsing anywhere.** `browse` is read-only navigation, so a client may click its way to
`/`. A folder that doesn't answer within 1 s is shown unreadable (`timed out`); system
folders (`/dev`, `/proc`, `/sys`, `/System`, `/Volumes`, `/net`, `/Network`, `/cores`, macOS's
autofs `/home`) are listed but not looked into from their parent; the `repos_below` walks keep
to the listing's 2 s budget even when a folder blocks. `kex browse /` answers in about 3 s.

**Limits** (cli-events section 9):

- `kex browse --max-entries` is at most 1,000, the node limit of one `tree` line (more is an
  error, exit 2: page with `--offset`). A test lists 1,000 folders and checks that the line,
  with both the raw `tree` and the view, stays under 1 MiB.
- `output` lines longer than 16 KiB are cut, ending in `…` (both `line` and the view).
- The manifest line is far below 512 KiB; a test checks it.
- A `--from` document over 1 MiB is an `error` (`from.too_big`, exit 2).

## 9. Messages

Fixed strings get ids in a few namespaces; params are named in each English template, which
`manifest.messages` lists in full:

| prefix | for |
|---|---|
| `col.*` | column labels (`col.repo`, `col.branch`, `col.ahead`, …) |
| `field.*` | card field labels |
| `badge.*` | badges (`badge.production`, `badge.draft`, `badge.worktree`, `badge.already_kex`, `badge.changes`, `badge.new_branch`, `badge.booted`, `badge.needs_gh`, `badge.kex_project`) |
| `project.*`, `status.*`, `pull.*`, `prs.*`, `pr.*`, `push.*`, `run.*`, `clones.*`, `update.*`, `init.*`, `added.*`, `plan.*`, `tree.*`, `workspaces.*`, `from.*`, `menu.*` | the titles, statuses, questions, log lines and errors of that command |

Data is never a message: repo and branch names, paths, PR titles, authors, git and gh output,
service output, error text from git. A log line made only of data (gh's PR text) has no
`msg`. A log line whose template includes data passes the data as params
(`pull.pulled`, `{"n":3}`). Lines that only restate a view, like `✗ web: push failed`, carry
`restates` instead of a `msg`.

## 10. What today's Sakabar cards show, and where it is in a view

Checked against Sakabar's web client (`spellRuns.ts`, `BattleField.svelte`,
`BattleMenu.svelte`). Everything they show today comes in a view:

| Sakabar today | in the views |
|---|---|
| kex version (from `hello.kex`) | `hello.tool.version` |
| project id, its repos, "Not on this computer", warnings | `project` card: subtitle, `repos`, `missing` (warn), notes |
| status table: repo + worktree badge, branch, ↑ ↓ or "no upstream", changed or — | `status` table: same columns; `count` draws 0 as — |
| pull cards: repo, branch, green / red edge, status, unmerged branch / ↑ / ↓ prime / author / date, note | `pull` card: title, subtitle, tone (now also warn and muted), summary, section table, notes |
| open PRs: repo, #, title, author, production / draft badges; hidden while the prompt lists them | `prs` table; `picks_from: "prs"` on the prompt |
| services: repo, starting / ready + URL link / exited code (ok or danger), command, output with stderr marked, Flutter's three keys (guessed from the command), Stop all | `service` views: `state`, `url`, `exit_code`, `tone`, `command`, `lines[].stream`, `actions` (sent by kex for Flutter only), manifest `stop` |
| battle log: info / warn / error / win, victories for merged, pushed, updated, version | `log` lines with level; `merged` / `pushed` log views; `version` / `updated` cards |
| outcome: Victory / Some repos failed / Error / A tool is missing / Fled | Sakabar's own title chosen by `exit.tone` (always present), with kex's `exit_codes` title under it |
| prompt rows guessed from `data`: PR, repo to run, repo to push, simulator; else the label split into columns | `columns` + option `view` for every select / multiselect, incl. the menu |
| typed confirmation guessed from "Type \`x\`" | `expect` |

Gaps in the kex-json events that the views close:

1. **PR detail** was only text (`gh pr view`, files, checks as `log` lines): the `pr` event
   has it as a card (from `gh pr view --json`).
2. **No open PRs** sent only the log line `no open PRs`: kex now sends `prs` with `items: []`
   (and the table's `empty` text) first.
3. **A pushed repo** got no line on success: the `pushed` log view says "Pushed dev (api)".
4. **Services without a URL** stayed "starting" forever: they start as `running`.
5. **Dates**: `unmerged[].date` is local time without an offset; the view's cell is RFC 3339
   (`committerdate:iso-strict`).
6. **Duplicate lines**: kex's `log` lines repeat what the events say; `restates` lets a client
   drop them.

## 11. Implementation and conformance tests

Code:

- `src/manifest.rs`: the annotation table, the generator (clap + annotations + constants),
  the hash, the text summary, the `manifest` command.
- `src/view.rs`: the message catalogue (`MESSAGES`, which is the manifest's `messages`) and
  small builders (`msg`, `col`, `cell`, `badge`, `field`, `note`, `view`). UI text comes only
  from `msg`, which fills the template, so the English always matches its id.
- `ui.rs`: prompts take a `Q` (message id, params, `columns` / `expect` / `tone` / `pattern`
  …); options carry a `row`; `say_about` / `warn_about` send `restates`. `exit` gets `tone`,
  `hello` its standard fields.
- Each command builds its views next to the event it sends (`project.rs`, `pull.rs`, `pr.rs`,
  `push.rs`, `run.rs`, `update.rs`, `setup.rs`, `browse.rs`); `init.rs` asks the JSON
  single-repo questions (section 7.4).
- `kex completions --json` is an error (exit 2): it writes a shell script, not JSON.

Tests (`cargo test`):

1. **Every line, every test.** The CLI tests' helper validates every stdout line of every
   `--json` run against `spec/cli-events-1.schema.json` (the `jsonschema` crate) and fails on
   any invalid line. Old fields stay asserted as before.
2. **Stream rules**, in the same helper: `hello` first (with the manifest's hash) and `exit`
   last, once each; `exit.tone` is the manifest's tone for its code; every non-standard type
   is in `manifest.event_types`; every `msg` is in `manifest.messages`, its params are
   placeholders of the template and its English is the template filled in; no line over
   1 MiB.
3. **Actions**, in the same helper: every RunAction (and navigation) passes cli-events 5.4
   against the manifest: known command, not `needs_terminal`, options valid as a form, stdin
   only for the JSON input and valid against its schema, no `cwd`, a `confirm` unless it is
   read-only navigation.
4. **Manifest drift** (unit): every visible clap command and option has an annotation and
   every annotation names one; the manifest validates against the schema; its references
   exist; it is under 512 KiB.
5. **The docs are true.** Every ```` ```ndjson ```` line of `spec/cli-events-1.md` and this page
   validates against the schema; this page's lines also pass the message and action checks.
6. **Hash.** `hello.manifest_hash` equals the RFC 8785 + SHA-256 hash the test computes from
   `kex manifest --json`, is the same across runs and in every command's `hello`.
7. **Limits.** A 1,000-entry `browse` is one line under 1 MiB; `--max-entries 1001` and a
   `--from` request over 1 MiB are errors; long `output` lines are cut (unit).
8. **Cells.** Every cell of a table / cards row, tree node and prompt option is under a column
   (in every line the tests see, and in the docs' examples).
9. **Signals and stop** (unix: SIGINT to the kex child): with a prompt open → 130
   `cancelled`; during `run` → `stopping`, `exited` muted, 130; the Stop line → 0 `ok`, with a
   rejected line logged (`run.unknown_message`) and no `error` line. `pr check` without gh →
   `error` `gh.missing`, exit 3. `kex manifest --json` with an update known → exactly three
   lines. `init --reset --json` asks `init.reset_name` (a bad name re-asked). A tree of
   6,000 folders below one answers quickly with `repos_below_truncated`.
10. **Flows.** The generic setup flow with nothing kex-specific: the picker command, the
   `item`s of selectable nodes, a dry run, then the `plan` action run exactly as given (argv
   built from the manifest: `--flag=value`, `--` before positionals). `kex init --json`
   answering the project and name prompts, with a rejected name.

Not covered by tests (they need `gh` or a listening service): the `prs`, `pr` and `merged`
views and `ready`; their shapes are in the examples above, which the docs test checks.
