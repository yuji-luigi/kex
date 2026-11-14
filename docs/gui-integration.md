# Setting up kex workspaces from a GUI

This is for a GUI that sets up kex projects for its user with a file-manager-style screen,
like Sakabar's "New kex workspace" spell. It explains which kex calls to make, what comes
back, and how to show it. The exact message types are in
[json-protocol.md](json-protocol.md); this page says how to use them together.

## The calls

`kex browse <dir> --json` per folder, then one `kex init --from - --json` with the repos the
user picked. Every call is short, asks nothing and can be retried, so it survives a dropped
connection, and the client owns navigation and selection. (`kex init --json` without
`--from` asks nothing either: it's an `error` pointing to these two calls. `kex add` is
another name for `kex init`.)

## Recommended flow: "New workspace"

The repos are the party members; kex is told the party once, at the end.

### 1. Entry

**Detect support first**: run `kex --json browse --no-git --depth 1 <some folder>` (or any
`--json` command) and read `features` in the first event, `hello`. Offer "New workspace…"
when it contains `browse` and `from` (and use `dry_run`, `pin_ids`, `paging`,
`repos_only`, `no_git` when present). Features are only ever added; prefer them over
comparing `kex` versions. An older kex has no `features` (or fails on `browse`): don't
offer the flow.

Offer the flow from the menu, and whenever kex says a folder has no kex.json (for example
`kex status --json` fails with "not a kex repo": `error` and exit 2).

### 2. Suggestions first

List the folder the user is in, or the one around it, and look at the workspace files kex
found:

```
spawn  kex browse /Users/me/src/shop --depth 2 --json        (no stdin needed)
```

```
← {"type":"hello","kex":"0.2.0","protocol":1,"features":["browse","from","dry_run","workspaces","pin_ids","paging","repos_only","no_git"]}
← {"type":"tree","root":"/Users/me/src/shop","parent":"/Users/me/src","kind":"dir","readable":true,"error":null,"depth":2,"max_entries":500,"count":6,"truncated":false,"offset":0,"children_total":5,"has_more":false,"next_offset":null,"git":true,"git_partial":false,"flat":false,"tree":[
    {"name":"admin","path":"/Users/me/src/shop/admin","kind":"kex_repo","depth":1,"hidden":false,"readable":true,"branch":"main","dirty":0,"remote":"git@github.com:acme/admin.git","remote_display":"github.com/acme/admin","last_commit_at":"2026-11-12T18:20:11+01:00","stack":["node"],"project":"shop-7f3kq2","repo":"admin"},
    {"name":"api","path":"/Users/me/src/shop/api","kind":"repo","depth":1,"hidden":false,"readable":true,"branch":"dev","dirty":2,"remote":"git@github.com:acme/api.git","remote_display":"github.com/acme/api","last_commit_at":"2026-11-13T09:02:40+01:00","stack":["node"]},
    {"name":"clients","path":"/Users/me/src/shop/clients","kind":"dir","depth":1,"hidden":false,"readable":true,"repos_below":1,"repos_below_truncated":false,"truncated":false,"modified_at":"2026-11-01T10:00:00Z","children":[
      {"name":"acme","path":"/Users/me/src/shop/clients/acme","kind":"dir","depth":2,"hidden":false,"readable":true,"repos_below":1,"repos_below_truncated":false,"truncated":true,"modified_at":"2026-11-01T10:00:00Z","children":[]}]},
    {"name":"linked","path":"/Users/me/src/shop/linked","kind":"symlink","depth":1,"hidden":false,"readable":true,"target":"/Users/me/elsewhere/ext","target_kind":"repo"},
    {"name":"web","path":"/Users/me/src/shop/web","kind":"repo","depth":1,"hidden":false,"readable":true,"branch":"main","dirty":0,"remote":null,"remote_display":null,"last_commit_at":"2026-11-10T16:45:03+01:00","stack":["flutter"]}]}
← {"type":"workspaces","workspaces":[{"path":"/Users/me/src/shop/shop.code-workspace","name":"shop","format":"vscode","folders":[
    {"path":"api","resolved":"/Users/me/src/shop/api","name":"API Server","exists":true,"kind":"repo","project":null,"repo":null,"default_name":"api-server","duplicate_of":null},
    {"path":"web","resolved":"/Users/me/src/shop/web","name":null,"exists":true,"kind":"repo","project":null,"repo":null,"default_name":"web","duplicate_of":null},
    {"path":"admin","resolved":"/Users/me/src/shop/admin","name":null,"exists":true,"kind":"kex_repo","project":"shop-7f3kq2","repo":"admin","default_name":"admin","duplicate_of":null},
    {"path":"../old","resolved":"/Users/me/src/old","name":null,"exists":false,"kind":null,"project":null,"repo":null,"default_name":"old","duplicate_of":null}],
    "counts":{"folders":4,"repos":3,"addable":2,"already_kex":1,"worktrees":0,"missing":1,"duplicates":0}}]}
← {"type":"exit","code":0}
```

(Each event is one line; wrapped here for reading.)

From `workspaces`, show one card per file: "Found `shop.code-workspace` · 3 repos · 1
already in kex" (`counts.repos` = repos to set up + kex repos; worktrees, missing folders
and duplicate entries are counted separately in `worktrees`, `missing`, `duplicates`). Tapping "Use these" ticks the folders with `kind: "repo"` (use `resolved`
as the path and `default_name` as the suggested name) and jumps to step 4. Folders with
`kind: "kex_repo"` tell you which project to suggest joining (`project`). Missing ones
(`exists: false`) can be listed greyed out.

kex looks for workspace files in the listed folder and, when it is inside a git repo, in
that repo's top folder and the folder around it. Supported: VS Code `*.code-workspace` (also
written by Cursor, VSCodium and Windsurf).

### 3. Browser

Show `tree` as a file manager:

- **Breadcrumbs** from `root` (always absolute and resolved: `~`, `..` and symlinks are
  gone). `parent` is the folder above (`null` at `/`). Drop a reply whose `root` isn't the
  folder you asked for (the user moved on).
- **Rows**: `children` are sorted by name (case-insensitive); sort them yourself if you
  want another order (`last_commit_at` for "recent first").
- **Opening a folder** = another `kex browse <path> --depth 1 --json`. Do this for every
  dir with `truncated: true` (its subfolders weren't listed, because of the depth or the
  cap) when the user opens it. A dir with `truncated: false` and no `children` is empty.
- **A cut root**: when the listed folder itself has more subfolders than the cap,
  `tree.has_more` is true and `next_offset` says where to go on. Show a "Load more" row at
  the end and fetch `kex browse <same root> --offset <next_offset> --json` (same `--depth`,
  `--max-entries`, `--hidden`, `--all`), then append its `tree`. `children_total` is the
  full count, for "200 of 640". Dirs with `truncated: true` are a different case: open
  them (above).
- **Hidden folders toggle** = `--hidden`. `--all` lists `node_modules`, `target`,
  `build`, `dist`, `Pods` and `vendor` (skipped by default, not walked). Dot-folders such
  as `.dart_tool` need `--hidden` (and `.git` both). Repos are leaves, so what's inside a
  repo is never listed.
- **Checkbox** only on `kind: "repo"`. Keep the selection on your side, keyed by `path`.
- **"Select all repos below"** on a dir with `repos_below > 0`:
  `kex browse <path> --repos-only --json` lists every repo below it at any depth, flat
  (`tree.flat: true`), and tick its `kind: "repo"` entries. If `truncated` is true the list
  is incomplete (more than `--max-entries` repos, or the walk stopped); say so.
  `repos_below` counts the whole subtree too (`repos_below_truncated: true` = at least
  that many), so "5 repos inside" and "select all" agree.
- **Bottom bar**: "N repos selected · Next".

Entry kinds (treat any **unknown kind as `dir`**; new ones like `submodule` may come):

| `kind` | icon / badge | selectable | open | notes |
|---|---|---|---|---|
| `dir` | folder; "N repos inside" from `repos_below` | no | yes (if `readable`) | `readable: false` + `error` ("permission denied"): show it, disabled |
| `repo` | repo icon from `stack` (`node`, `flutter`, `rust`, `go`, `python`, …); branch, `dirty` count, `remote_display` | **yes** | no | the only kind kex can set up |
| `kex_repo` | badge "in `<project>`" (`project`; null = broken config, `config_error`) | no | no | offer its `project` in step 4 |
| `worktree` | dimmed, "worktree of `<main_repo>`" | no | no | shares its main checkout's kex.json |
| `symlink` | link icon, `target`, `target_kind` (`missing` = dangling) | if `target_kind` is `repo` | browse `target` | `kex browse` doesn't follow links; `--from` accepts the link path and resolves it |

**Fast first paint, then details**: call `kex browse <dir> --no-git --json` first (no git
processes: a few ms to ~2 s at worst) and draw the rows; then the same call without
`--no-git` and fill in branch, dirty, remote and last commit by `path`. If the second call
says `git_partial: true`, some repos weren't reached within `--git-budget-ms` (default 5 s);
show them without details, or list their folder again.

**Timeout**: give `kex browse` **10 s**. With the defaults a call is bounded by about 2 s of
folder walking (`repos_below`, `--repos-only`) plus 5 s of git (a slower git command is
stopped at the budget) plus output. Measured on an 8-core Mac with an SSD: 200 repos at depth
≤ 2 in 1.9 s (`--no-git` 0.02 s); a real `~/Development` (19 repos) in 0.3 s. The cost is
about 9 ms per repo (3 git processes, 8 at a time), more for huge or very dirty repos, and
for network drives.

Costly fields (`branch`, `dirty`, `remote`, `last_commit_at`) are only read for git repos
in the listing, 8 at a time. A listing is capped at `--max-entries` (default 500, breadth
first): `truncated: true` at the top level means some folder was cut; those dirs carry
`truncated: true` too.

### 4. Review

One screen: the project and one row per repo.

- **Project**: "New project" with a name field, or "Join `<id>`" for each `project` seen on
  `kex_repo` entries or workspace folders. (kex also knows projects from its registry; a
  wrong id is reported in step 5's dry run.)
- **Repo rows**: an editable name (default: the workspace `default_name`, else the folder
  name lowercased with `-`: what kex would pick) and an optional description.
- Run a **dry run** and show kex's errors under their rows (`index` = the row, `field` =
  the input):

```
spawn  kex init --from - --dry-run --json       (cwd: any folder; stdin below, then close it)
→ {"project":{"new":"Shop"},"repos":[
     {"path":"/Users/me/src/shop/api","name":"api-server","description":"Orders and payments API"},
     {"path":"/Users/me/src/shop/web","name":"Web"},
     {"path":"/Users/me/src/shop/admin"}]}
← {"type":"hello","kex":"0.2.0","protocol":1,"features":["browse","from","dry_run","workspaces","pin_ids","paging","repos_only","no_git"]}
← {"type":"error","message":"already a kex repo in project shop-7f3kq2 (as \"admin\"); kex doesn't move repos between projects","index":2,"field":"path"}
← {"type":"error","message":"\"Web\" isn't a repo name: use lowercase letters, digits and -, starting with a letter or digit","index":1,"field":"name"}
← {"type":"plan","valid":false,"project":{"id":"shop-k2m9qa","new":true,"name":"Shop"},"repos":[],"skipped":[],"errors":[{"message":"already a kex repo in project shop-7f3kq2 …","index":2,"field":"path"},{"message":"\"Web\" isn't a repo name: …","index":1,"field":"name"}]}
← {"type":"exit","code":2}
```

Every problem comes at once (one `error` per problem, and the same list in `plan.errors`).
Fixed (join `shop-7f3kq2`, name `web`):

```
→ {"project":"shop-7f3kq2","repos":[
     {"path":"/Users/me/src/shop/api","name":"api-server","description":"Orders and payments API"},
     {"path":"/Users/me/src/shop/web","name":"web"},
     {"path":"/Users/me/src/shop/admin"}]}
← {"type":"hello","kex":"0.2.0","protocol":1,"features":["browse","from","dry_run","workspaces","pin_ids","paging","repos_only","no_git"]}
← {"type":"log","level":"info","text":"project shop-7f3kq2"}
← {"type":"log","level":"info","text":"would write /Users/me/src/shop/api/kex.json:"}
← … (one log per file, for a terminal reader)
← {"type":"plan","valid":true,"project":{"id":"shop-7f3kq2","new":false,"name":null},"repos":[
    {"index":0,"path":"/Users/me/src/shop/api","repo":"api-server","config_path":"/Users/me/src/shop/api/kex.json","config":{"$schema":"https://raw.githubusercontent.com/yuji-luigi/kex/main/schema/kex.schema.json","version":1,"project":"shop-7f3kq2","repo":"api-server","repo_id":"r_v6m5lx","description":"Orders and payments API","prime_branches":[{"branch":"dev","stage":"dev","description":"Integration branch"},{"branch":"main","stage":"production","description":"Live"}],"peers":[{"repo":"admin","remote":"git@github.com:acme/admin.git"}]},"gitignore":{"path":"/Users/me/src/shop/api/.gitignore","add":"kex.local.json"}},
    {"index":1,"path":"/Users/me/src/shop/web","repo":"web","config_path":"/Users/me/src/shop/web/kex.json","config":{…},"gitignore":{"path":"/Users/me/src/shop/web/.gitignore","add":"kex.local.json"}}],
  "skipped":[{"index":2,"path":"/Users/me/src/shop/admin","folder":"admin","reason":"already_kex","in_project":true,"repo":"admin","project":"shop-7f3kq2"}],"errors":[]}
← {"type":"exit","code":0}
```

That's the "These files will be written" preview: `config_path` and `config` per repo, and
the `.gitignore` line. A repo already in the chosen project is `skipped`, not an error, so
the same request can be sent twice. If its kex.json has another name or description than
the row, the skipped entry has `differs: ["name"]` and a `note`; show it ("already set up as
`web`; not renamed"), since kex never edits an existing kex.json.

A valid plan also has `request`: your input with every id pinned (the new project's `id`,
each repo's `repo_id`, names and resolved paths):

```
← {"type":"plan","valid":true,…,"request":{"project":"shop-7f3kq2","repos":[
    {"path":"/Users/me/src/shop/api","name":"api-server","repo_id":"r_v6m5lx","description":"Orders and payments API"},
    {"path":"/Users/me/src/shop/web","name":"web","repo_id":"r_q81ozd"},
    {"path":"/Users/me/src/shop/admin"}]}}
```

For a new project it is `"project":{"new":"Shop","id":"shop-k2m9qa"}`. **Send
`plan.request` as the input of step 5** and kex writes exactly the files the user reviewed,
ids included. (Sending your original input instead works too, but then the project id and
the repo ids are made anew.) If the user edits anything after the preview, run the dry run
again.

### 5. Create

`plan.request` from step 4, without `--dry-run`, as a normal run:

```
spawn  kex init --from - --json            (stdin: plan.request, then close it)
← {"type":"hello","kex":"0.2.0","protocol":1,"features":["browse","from","dry_run","workspaces","pin_ids","paging","repos_only","no_git"]}
← {"type":"log","level":"info","text":"project shop-7f3kq2"}
← {"type":"log","level":"info","text":"✓ api-server   /Users/me/src/shop/api/kex.json (r_v6m5lx, prime dev, main)"}
← {"type":"log","level":"info","text":"✓ web          /Users/me/src/shop/web/kex.json (r_q81ozd, prime main)"}
← {"type":"log","level":"info","text":"  · admin        already in shop-7f3kq2"}
← {"type":"added","project":"shop-7f3kq2","repos":[
    {"repo":"api-server","repo_id":"r_v6m5lx","path":"/Users/me/src/shop/api","config":"/Users/me/src/shop/api/kex.json","prime_branches":[…],"peers":["admin"],"description":"Orders and payments API"},
    {"repo":"web","repo_id":"r_q81ozd","path":"/Users/me/src/shop/web","config":"/Users/me/src/shop/web/kex.json","prime_branches":[…],"peers":["admin","api-server"],"description":null}],
  "skipped":[{"index":2,"path":"/Users/me/src/shop/admin","folder":"admin","reason":"already_kex","in_project":true,"repo":"admin","project":"shop-7f3kq2"}],"failed":[]}
← {"type":"log","level":"info","text":"Check the prime branches, then commit kex.json in each repo."}
← {"type":"exit","code":0}
```

Show `added` as the result ("Party formed: 2 repos", with `skipped` and `failed`), then
offer the next steps: `kex status --json`, `kex pull --json` (run them in one of the new
repos). The user still has to commit each `kex.json`; say so.

**Which project when `project` is left out?** In this order: `--project <id>`, else the
project of the kex repo kex's cwd is in, else a **new project named after the folder around
the first repo** in `repos` (wherever the others are). The plan's `request` always names the
project, so after a dry run nothing depends on cwd; for "new project", send
`{"new": "<name>"}` rather than leaving it out.

Run from inside an existing kex repo, a missing `project` means "this repo's project".

## Input reference (`--from`)

```ts
type FromInput =
  | Repo[]                                              // project: see below
  | { project?: string | { new: string; id?: string }; repos: Repo[] };
interface Repo {
  path: string;          // absolute (recommended), "~/…", or relative to kex's cwd; a symlink is resolved
  name?: string;         // 1–40 chars, ^[a-z0-9][a-z0-9-]*$; default: the folder name lowercased with "-", cut to 40
  description?: string;  // ≤ 200 chars, one line; written to kex.json; shown by kex doctor and status --json
  repo_id?: string;      // pin the id (from plan.request); default: a new one
}
```

`project`: an existing id (kex must know it: its registry, a kex repo next to one of the
given repos, or the repo kex runs in), or `{"new": "<name>"}` → a new id `<slug>-xxxxxx`,
or `{"new": "<name>", "id": "<id>"}` to use that id (from `plan.request`; if it exists by
then, the repos join it, so a retry is harmless).
Left out: `--project <id>` if given, else the project of the kex repo kex runs in, else a new
project named after the folder around the first repo. Unknown keys are errors (`field` =
the key), so a typo doesn't go unnoticed.

Checked before anything is written, all at once: shape, `path` exists / is a folder / is the
top folder of a git repo (the message names the top folder when it's inside one) / isn't a
linked worktree / isn't listed twice / isn't a kex repo of another project / its
`.gitignore` can be read; `name` is valid, not taken in the project, not used twice;
`repo_id` has the form `r_` + 1–32 lowercase letters/digits, isn't used by a repo kex
knows and isn't given twice; `project` exists.

**Limits for the review form** (check them before the dry run): repo name 1–40 characters of
`a-z`, `0-9`, `-`, not starting with `-`; description at most 200 characters on one line; new
project name 1–40 characters. kex enforces the same everywhere.

## Writing, interruption and retries

- Every file is written to a temporary file next to it, then renamed into place, so a file
  is never half-written.
- All or nothing: if one repo can't be written, the ones already written are removed again
  and their `.gitignore` restored. `added` then has `repos: []`, `failed` (what went wrong)
  and `rolled_back: true` (exit 1). Nothing was changed; fix the cause and send the same
  request again.
- Kill it during the writes: SIGINT or SIGTERM (what a gateway sends) undoes everything
  written so far and exits 130; a `log` says "interrupted: nothing was written". After
  SIGKILL there are no half files either, but files already renamed stay: run the same
  request again (written repos are then `skipped`), or `--dry-run` to see the state.
- The registry (`~/.config/kex/`) is only updated once every file is in place.

## Errors and exit codes

| exit | meaning | what you get |
|---|---|---|
| 0 | done (also: nothing to do) | `tree`+`workspaces`, `plan`, or `added` |
| 1 | a write failed, rolled back | `added` with `failed`, `rolled_back` |
| 2 | bad input or usage | one `error` per problem (`index`, `field` for `--from`), `plan` with `valid: false` on `--dry-run` |
| 130 | interrupted / cancelled | maybe a `log`; nothing written |

Every run starts with `hello` and normally ends with `exit`. A run killed **outside** the
write phase (SIGINT/SIGTERM while listing or validating, or SIGKILL any time) ends without
`exit`: treat end of stdout (EOF) as the end of the run, and use the process exit status
then. Nothing is written before the write phase, so such a run changed nothing. Ignore event types and fields you don't
know (new ones are added without a protocol bump; removing or changing one would bump
`protocol`). `kex browse` on a missing folder: `error` + exit 2.

## What to disable in the UI

- Checkboxes on anything but `kind: "repo"` (and `symlink` whose `target_kind` is `repo`).
- Opening dirs with `readable: false`.
- "Next" with nothing selected (`--from` with no repos is an error).
- Moving a repo between projects. **Decision: not supported.** A repo that is already in
  another project is an error with `field: "path"` ("already a kex repo in project X (as
  "web"); kex doesn't move repos between projects"). Show that error under the row; don't
  offer a "move" action. (To move one by hand: edit its kex.json and commit it.)
- Don't prefill or auto-answer kex's typed confirmations in other flows (`pr check`,
  `push`); `kex init` has none.
