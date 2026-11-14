# kex JSON protocol (version 1)

This document is for programs that drive `kex`: gateways, agents, editors, web UIs. It
defines every message kex writes and reads in `--json` mode, so a client can show kex's
questions as its own UI (buttons, checklists, forms) and answer them.

- Transport: kex's **stdout** (events from kex) and **stdin** (answers from you).
- Framing: **NDJSON**. One JSON object per line, UTF-8, `\n`-terminated. In JSON mode kex
  writes nothing else to stdout. Tools kex starts that would print to the terminal (`git
  push`, install commands) write to **stderr** instead. Read stderr only for diagnostics.
- Text fields never contain ANSI color codes.
- Turn it on with `--json` on any command: `kex pull --json`, `kex run --json`, `kex --json`
  (the menu).
- The protocol version is the `protocol` field of the first event (`hello`). Additions
  within version 1 are backwards compatible: new event types, new optional fields. **Ignore
  unknown event types and fields.**

## Lifecycle

```
spawn  kex <command> --json            (stdin, stdout piped)
  ← hello                              always first
  ← project                            once the project is resolved (most commands)
  ← log / pull / status / prs / …      results while the command works
  ← prompt                             kex blocks until you answer
  → answer                             one line on stdin
  ← error {id}                         only if the answer was invalid; send another
  ← …
  ← exit {code}                        always last; then the process exits with that code
```

If kex fails before it can finish, it sends `error` (no `id`) and then `exit` with code 2.

Exit codes: `0` ok · `1` some repo failed (others may have worked) · `2` error (config,
usage, network) · `3` a required tool is missing (e.g. `gh`) · `130` cancelled.

## TypeScript types

```ts
/** Every line kex writes to stdout. Switch on `type`. */
type KexEvent =
  | Hello | Log | ErrorEvent | Exit
  | ProjectEvent | Prompt
  | PullEvent | StatusEvent | PrsEvent | MergedEvent | PushedEvent | AddedEvent
  | TreeEvent | WorkspacesEvent | PlanEvent
  | StartedEvent | OutputEvent | ReadyEvent | ExitedEvent
  | VersionEvent | UpdateAvailableEvent | UpdatedEvent;

// ── Session ──────────────────────────────────────────────────────────────────

interface Hello {
  type: "hello";
  kex: string;          // kex version, e.g. "0.2.0"
  protocol: 1;          // this document
  /** What this kex can do. Stable strings, only ever added (never renamed or removed
   *  without a `protocol` bump). **Check for the feature you need instead of comparing
   *  versions.** Absent in kex releases before these existed: treat that as `[]`.
   *  "browse" (`kex browse`), "from" (`--from`), "dry_run", "workspaces" (workspace files),
   *  "pin_ids" (`repo_id` / `project.id` in `--from`), "paging" (`kex browse --offset`),
   *  "repos_only" (`kex browse --repos-only`), "no_git" (`--no-git`, `--git-budget-ms`). */
  features?: string[];
}

/** Everything a person would read in the terminal, line by line or as a block. */
interface Log {
  type: "log";
  level: "info" | "warn";
  text: string;         // may contain "\n" (e.g. `gh pr view` output)
}

interface ErrorEvent {
  type: "error";
  message: string;
  /** Set when this rejects your answer to that prompt; kex then waits for a new answer.
   *  Absent for a fatal error, which is followed by `exit`. */
  id?: string;
  /** `--from` input problems (see "Setting up repos from a list"): which repo
   *  (`repos[index]`) and which key. kex sends one `error` per problem, all of them, then
   *  `exit` 2. `null`/absent when the problem isn't about one repo or one key. */
  index?: number | null;
  field?: string | null;
}

interface Exit {
  type: "exit";
  code: 0 | 1 | 2 | 3 | 130;
}

// ── Project ──────────────────────────────────────────────────────────────────

interface ProjectEvent {
  type: "project";
  id: string;                                   // e.g. "acme-shop-7f3kq2"
  repos: { repo: string; path: string; worktree: boolean }[];
  /** Repos listed by the project that aren't on this machine. */
  missing: { repo: string; remote: string }[];
  warnings: string[];                           // e.g. a copied kex.json
}

// ── Prompts ──────────────────────────────────────────────────────────────────

type Prompt = SelectPrompt | MultiSelectPrompt | ConfirmPrompt | TextPrompt;

interface PromptBase {
  type: "prompt";
  id: string;           // "p1", "p2", … unique within this process
  message: string;      // the question, show it as the title
}

interface Option {
  index: number;        // position, 0-based
  value: string;        // stable id: answer with this (preferred) or with `index`
  label: string;        // human text as kex would show it (column-aligned for monospace)
  /** Structured details behind the label, for building your own UI.
   *  Shape depends on the prompt; see "Prompt catalogue". */
  data?: Record<string, unknown>;
}

interface SelectPrompt extends PromptBase {
  kind: "select";
  options: Option[];
}

interface MultiSelectPrompt extends PromptBase {
  kind: "multiselect";
  options: Option[];
  default: number[];    // indexes that start checked
}

interface ConfirmPrompt extends PromptBase {
  kind: "confirm";
  default: boolean;
}

interface TextPrompt extends PromptBase {
  kind: "text";
  default: string | null;
}

// ── Command results ──────────────────────────────────────────────────────────

/** `kex pull` (one per checkout) and the update step of `kex run`. */
interface PullEvent {
  type: "pull";
  repo: string;
  path: string;
  branch: string;       // "(detached)" when HEAD is detached
  ok: boolean;          // false = fetch/pull failed (exit code will be 1)
  status: string;       // e.g. "✓ up to date", "↓ 3 commits pulled", "⚠ uncommitted changes, pull skipped"
  /** Only from `kex pull`. Branches on the shared remote not merged into a prime branch.
   *  null when not reported: a second clone of the same repo, or there is a note instead. */
  unmerged?: UnmergedBranch[] | null;
  /** Why there is no list, e.g. "no remote", "no prime branch found on origin". */
  unmerged_note?: string | null;
}

interface UnmergedBranch {
  branch: string;
  prime: string;        // the prime branch it is closest to
  ahead: number;        // commits on the branch not in `prime`
  behind: number;       // commits in `prime` not in the branch
  author: string;       // last committer name
  date: string;         // "YYYY-MM-DD HH:MM", local time
}

/** `kex status`. Sent once; it replaces both the `project` event and the table. */
interface StatusEvent {
  type: "status";
  project: string;
  repos: {
    repo: string; path: string; branch: string;
    upstream: boolean;  // has an upstream branch
    ahead: number; behind: number;
    changed: number;    // files with changes (`git status` entries)
    worktree: boolean;
    description: string | null;  // kex.json `description`
  }[];
  missing: string[];    // repo names not on this machine
}

/** `kex pr check`: all open PRs across repos. Sent before the PR prompt, and again after
 *  returning from a PR (the list is refetched). */
interface PrsEvent {
  type: "prs";
  items: PullRequest[];
}

interface PullRequest {
  repo: string;
  number: number;
  title: string;
  head: string;         // source branch
  base: string;         // target branch
  author: string;       // GitHub login
  created_at: string;   // RFC 3339, UTC
  draft: boolean;
  production: boolean;  // base is protected / production: show a warning
}

interface MergedEvent {
  type: "merged";
  repo: string;
  number: number;
  state: string;        // "MERGED", or e.g. "OPEN" when a merge queue / checks are pending
}

interface PushedEvent {
  type: "pushed";
  repo: string;
  branch: string;
  ok: boolean;
}

/** `kex init` (alias `kex add`) in any mode (`--from`, `--yes`, `--repos`, `--workspace`, or
 *  the terminal browser): sent once at the end, when the request was valid. */
interface AddedEvent {
  type: "added";
  /** The project the repos joined. */
  project: string;
  /** One entry per repo that got a new kex.json. */
  repos: {
    repo: string;       // repo name
    repo_id: string;    // e.g. "r_9xk2mq"
    path: string;       // repo root, absolute
    config: string;     // the written kex.json (absolute; `.config/kex.json` when .config/ exists)
    prime_branches: { branch: string; stage: "dev" | "production"; description: string }[];
    peers: string[];    // repo names listed in its `peers` (only repos with a remote)
    description: string | null;   // from the request; null when it gave none
  }[];
  /** Repos of the request that are already in `project` (so the same request can be sent
   *  again). */
  skipped: {
    index: number;        // which entry of the request
    path: string;
    folder: string;
    reason: "already_kex";
    in_project: true;
    repo: string;         // its repo name
    project: string;      // its project id (= `project`)
    /** What the request asked for that differs from the existing kex.json, which kex
     *  doesn't change ("name", "description", "repo_id"), with a sentence in `note`. */
    differs?: string[];
    note?: string;
  }[];
  /** The repo whose write failed; exit code 1. Writing is all or nothing: `repos` is then
   *  `[]` and `rolled_back` says whether configs already written were removed again. */
  failed: { repo: string; path: string; message: string }[];
  rolled_back?: boolean;
}

// ── Folders, workspace files, plans (`kex browse`, `kex init`) ───────────────

/** One folder in a listing. Paths are absolute. Fields by kind; a client must treat an
 *  unknown `kind` as `"dir"` (kinds may be added, e.g. `submodule`). */
interface BrowseEntry {
  name: string;               // folder name as listed (a symlink's own name)
  path: string;
  kind: "dir" | "repo" | "kex_repo" | "worktree" | "symlink" | string;
  depth: number;              // 1 = directly in the listed folder
  hidden: boolean;            // name starts with "."
  readable: boolean;          // false: it can't be listed; see `error`
  error?: string;             // e.g. "permission denied"
  // dir
  repos_below?: number;       // git repos anywhere inside, any depth (the same skips as
                              // the listing; repos aren't looked into, links not followed).
                              // Below where the listing stopped, kex walks the rest (at most
                              // 5,000 folders per dir, 20,000 and 2 s per listing).
  repos_below_truncated?: boolean;  // a walk hit those limits: repos_below is a minimum
  truncated?: boolean;        // its subfolders weren't (all) listed: depth or entry cap.
                              // List it again (`kex browse <path>`) to load them.
  modified_at?: string | null;  // ISO 8601, UTC
  children?: BrowseEntry[];   // nested form only (the `tree` arrays)
  // repo, kex_repo, worktree (only these run git, so they cost more). The four git fields
  // (branch, dirty, remote/remote_display, last_commit_at) are absent with --no-git, or for
  // the repos the time budget didn't reach (tree.git_partial).
  branch?: string;            // "(detached)" when detached
  dirty?: number;             // changed + untracked files
  remote?: string | null;     // shared remote URL as configured (upstream, else origin, …)
  remote_display?: string | null;  // "github.com/acme/api"
  last_commit_at?: string | null;  // ISO 8601 with offset; null in an empty repo
  stack?: string[];           // from marker files: node deno flutter rust go python ruby
                              // php java gradle swift elixir (may grow)
  project?: string | null;    // kex_repo: its project id (null: config can't be read)
  repo?: string | null;       // kex_repo: its repo name
  config_error?: string;      // kex_repo with a broken config
  main_repo?: string | null;  // worktree: the main checkout
  // symlink (`kex browse` doesn't follow links)
  target?: string | null;     // resolved target
  target_kind?: "dir" | "repo" | "kex_repo" | "worktree" | "missing";
}

/** `kex browse`: the listing. The request is echoed so a late answer can't be shown for
 *  the wrong folder. */
interface TreeEvent {
  type: "tree";
  root: string;               // absolute, resolved (symlinks, `~`, `..`)
  parent: string | null;      // null at the filesystem root
  kind: string;               // what root itself is
  readable: boolean;
  error: string | null;
  depth: number | null;       // as requested (null: --repos-only without --depth)
  max_entries: number;        // as requested
  count: number;              // entries listed
  truncated: boolean;         // the entry cap stopped the listing somewhere
  /** The root's own subfolders. When the cap cut *them* (not just deeper levels),
   *  `has_more` is true and `next_offset` is where the next page starts: ask again with
   *  `--offset <next_offset>` (same folder, same options). Dirs inside cut by the cap
   *  carry `truncated: true` instead; list those folders on their own. */
  offset: number;             // as requested (--offset), default 0
  children_total: number;     // the root's subfolders after the skips (hidden, --all)
  has_more: boolean;
  next_offset: number | null;
  git: boolean;               // false with --no-git: no entry has git fields
  git_partial: boolean;       // the git budget (--git-budget-ms) ran out: some repos lack them
  flat: boolean;              // --repos-only: `tree` is a flat list of repos (no children),
                              // sorted by path, each with `depth` below root
  tree: BrowseEntry[];        // nested; children sorted by name (case-insensitive)
}

/** One workspace file and its folders. Supported: VS Code `*.code-workspace` (also
 *  Cursor, VSCodium, Windsurf): JSON with comments and trailing commas, `folders` with a
 *  `path` (relative to the file) or a `file://` `uri`. */
interface WorkspaceFile {
  path: string;               // the file
  name: string;               // file name without extension: "shop" for shop.code-workspace
  format: "vscode";
  folders: {
    path: string;             // as written in the file
    resolved: string | null;  // absolute; symlinks resolved when it exists, `.`/`..` removed
                              // when it doesn't (null: not a local folder, e.g. a remote URI)
    name: string | null;      // the name the file gives it
    exists: boolean;
    kind: string | null;      // as BrowseEntry.kind; null when missing
    project: string | null;   // kex_repo
    repo: string | null;      // kex_repo
    default_name: string;     // repo name kex would use: slug of `name`, else of the folder
    /** The file lists the same folder (same `resolved`, e.g. `web` and `./web`, or a link
     *  to it) earlier, at this index. Shown, but counted and set up once. */
    duplicate_of: number | null;
  }[];
  /** Each folder counted once (duplicates left out). `repos` = repos to set up + kex repos
   *  (worktrees are counted on their own: they can't be set up). */
  counts: {
    folders: number; repos: number; addable: number; already_kex: number;
    worktrees: number; missing: number; duplicates: number;
  };
}

/** `kex browse`: the workspace files in the folder (and, inside a git repo, in its top
 *  folder and the folder around it). A file that can't be read is `{path, error}`. */
interface WorkspacesEvent {
  type: "workspaces";
  workspaces: (WorkspaceFile | { path: string; error: string })[];
}

/** `--from … --dry-run`: what would be written. Nothing is written; exit 0 if valid, else 2
 *  (the problems also come as `error` events before it). */
interface PlanEvent {
  type: "plan";
  valid: boolean;
  project: { id: string; new: boolean; name: string | null } | null;
  repos: {
    index: number;            // input entry
    path: string;             // repo root (resolved)
    repo: string;
    repo_id: string;
    config_path: string;      // the kex.json to write
    config: object;           // its full content
    gitignore: { path: string; add: string | null };  // line added to .gitignore, if any
  }[];
  skipped: AddedEvent["skipped"];
  errors: { message: string; index: number | null; field: string | null }[];
  /** Valid plans only: the input again with every id pinned (the new project's `id`, each
   *  repo's `repo_id`, names and resolved paths). Send it back as the `--from` input and
   *  kex writes exactly `repos[].config`. Without pinning, a real run makes new ids. */
  request?: FromInput;
}

// ── kex run: services ────────────────────────────────────────────────────────

interface StartedEvent {
  type: "started";
  repo: string;
  cmd: string;          // the shell command
  pid: number;
  path: string;
}

interface OutputEvent {
  type: "output";
  repo: string;
  stream: "stdout" | "stderr";
  line: string;         // one line, without the newline
}

/** The service's `ready` URL accepts connections. */
interface ReadyEvent {
  type: "ready";
  repo: string;
  url: string;
}

interface ExitedEvent {
  type: "exited";
  repo: string;
  code: number | null;  // null = killed by a signal
  success: boolean;
}

// ── Versions ─────────────────────────────────────────────────────────────────

/** `kex update` / `kex update --check`. */
interface VersionEvent {
  type: "version";
  current: string;
  latest: string;
  update: boolean;      // latest is newer than current
}

/** Sent near the end of any command when a newer release is known (checked daily). */
interface UpdateAvailableEvent {
  type: "update_available";
  current: string;
  latest: string;
}

interface UpdatedEvent {
  type: "updated";
  version: string;
}
```

## Answering prompts

Write one line to stdin per prompt:

```ts
/** What you write to stdin while a prompt is open. */
type Answer =
  | { id?: string; answer: AnswerValue }   // preferred; with `id`, kex checks it matches
  | { cancel: true };                      // abort the command → exit 130

type AnswerValue =
  | number            // select: option index
  | string            // select: option value or full label · text: the text · confirm: "yes"/"no"
  | (number | string)[]   // multiselect: indexes and/or values
  | "all" | "default"     // multiselect shortcuts
  | boolean           // confirm
  | null;             // select: skip/back · multiselect: none · confirm/text: the default
```

| kind | valid `answer` | `null` means |
|---|---|---|
| `select` | `index` or `value` | skip (usually "back"/"quit"; on required questions it cancels → 130) |
| `multiselect` | array of indexes/values, `"all"`, `"default"`, or `"api,web"` | nothing selected |
| `confirm` | `true` / `false` (`"y"`, `"yes"`, `"n"`, `"no"` also work) | use `default` |
| `text` | string | use `default` (empty string if none) |

Plain lines also work for quick scripts: a bare JSON value (`2`, `["api"]`, `true`) or plain
text (`web`) is taken as the answer. Closing stdin while a prompt is open cancels (130).

If the answer is invalid, kex sends `{"type":"error","id":"p3","message":"no option \"x\""}` and
waits for another line for the same prompt.

## Messages during `kex run`

After the services start, kex keeps reading stdin:

```ts
type RunCommand =
  /** Type into a service's stdin. `repo` defaults to the Flutter service.
   *  Flutter keys: "r" hot reload, "R" hot restart, "q" quit. */
  | { type: "input"; repo?: string; text: string }
  /** Stop all services (SIGINT, then kill after 5 s), then `exited` events and `exit`. */
  | { type: "stop" };
```

Unknown or invalid messages get an `error` event (no `id`). Closing stdin does **not** stop
the services. Send `stop`, or SIGINT / SIGTERM the kex process.

## Prompt catalogue

The prompts kex can ask, with their option `value`s and `data`. Match on `kind` + options,
not on `message` text, which may change.

| Command | message | kind | option `value` | option `data` |
|---|---|---|---|---|
| `kex` (menu) | `kex` | select | `pull` `run` `pr` `status` `push` `doctor` (+ `update` when a release is available) | — |
| `run` | `Run which repos?` | multiselect | repo name | `{ runtime: string, reason: string, port: number \| null, path: string }` |
| `run` | `iOS simulator` | select | simulator UDID | `{ name: string, runtime: string, booted: boolean }` |
| `run`, `push` | `<repo> has N clones. Which one?` | select | checkout path | — |
| any | `Remember this choice?` | confirm | | |
| `push` | `Push the current branch of which repos?` | multiselect | repo name | `{ branch: string, ahead: number, upstream: boolean, dirty: boolean }` |
| `push --repos` | `Push these?` | confirm | | |
| `push` | `<repo>: <branch> is protected. Type \`<branch>\` to push:` | text | answer with the branch name exactly | |
| `pr check` | `Open PRs` | select | `"<repo>#<number>"`, last option `quit` | `PullRequest` (same as in `prs`); none on `quit` |
| `pr check` | `#<n> <title>` | select | `merge` `diff` `back` | — |
| `pr check` | `Merge #<n> "<title>" (<repo>) into <base>?` | confirm | | |
| `pr check` | ``Type `<word>` to merge:`` | text | `<word>` is `accept`, or the base branch name for production PRs | |
| `update` | `Install kex <version>?` | confirm | | |

The typed confirmations (`Type … to merge/push`) are safety checks for a person. A UI
should show them as a field the user fills in, not answer them automatically.

`kex run` does not ask "Run which repos?" when only one repo can run, or with `--repos`.
`push --repos a,b --yes` and `pr check --list` don't ask anything.

### Setting up repos (`kex init`, alias `kex add`)

With `--json`, `kex init` asks nothing. It sets up the repos named by one of:

| flag | repos |
|---|---|
| `--from <file\|->` | the JSON request below (the way for programs) |
| `--yes` | the repo kex runs in (or `DIR` when it is one), with detected defaults |
| `--repos a,b` | these folders, next to the repo kex runs in (or in `DIR`), or absolute |
| `--workspace <file>` | the git repos without a kex config that a workspace file lists |

Without one of them it sends an `error` ("list folders with `kex browse --json`, then send
the repos with `kex init --json --from -`") and exits 2. Every mode goes through the same
checks and the same all-or-nothing writer as `--from`, reports problems the same way
(`error` events with `index` / `field`, exit 2), takes `--dry-run`, and ends with `added`.
The project, unless given: the one of the kex repo kex runs in; with `--yes` / `--repos` /
`--workspace`, else the only project found next to the repos; else a new one. `--project
<id>` with an id kex doesn't know makes a project with that id (not with `--from`, where
`{"new": …, "id": …}` does that). The folder browser, the workspace-file question, the
project choice and the name questions are terminal-only.

### Setting up repos from a list (`--from`) and `kex browse`

For a GUI that browses on its own (a file manager), there are two calls that ask nothing:

- `kex browse [DIR] --json [--depth N] [--max-entries N] [--offset N] [--hidden] [--all]
  [--repos-only|--flat] [--no-git] [--git-budget-ms N]`:
  sends one `tree` event and one `workspaces` event, then `exit` 0. Default depth 3, cap
  500 entries (breadth first, so the shallow levels fill first). When the cap cuts DIR's
  own list, `has_more` / `next_offset` say so; `--offset <next_offset>` lists the next page.
  Skipped unless `--all`: `node_modules`, `target`, `build`, `dist`, `Pods`, `vendor`.
  Dot-folders (`.git`, `.dart_tool`, …) only with `--hidden` (both flags for `.git`);
  inside repos nothing is listed anyway. **Symlinks are not followed**: a link to a folder (or a dangling
  link) is a `symlink` entry with `target` and `target_kind`; links to files are left out.
  An unreadable folder stays in the list with `readable: false` and `error`. Git fields are
  read only for the repos in the listing (3 git commands each, 8 at a time, with
  `GIT_OPTIONAL_LOCKS=0`, so kex never takes a repo's index lock), within a time budget
  (`--git-budget-ms`, default 5000, 0 = none): a git command still running then is stopped,
  the remaining repos go without git fields, and `git_partial` is true. `--no-git` skips them
  altogether (kinds, `stack`, `project`/`repo` are still there). `--repos-only` (alias
  `--flat`): every git repo below DIR at any depth (or `--depth` levels), as a flat list,
  at most `--max-entries` (`truncated` when cut, or when the walk stopped at 20,000 folders
  or 2 s). `DIR` may start with `~`. A missing `DIR` is an `error`, exit 2.

  **Time.** Measured on a MacBook (8 cores, SSD), release and debug builds alike (git is the
  cost): a folder with 200 repos at depth ≤ 2 (20 of them with 300 untracked files) took
  1.9 s; `--no-git` 0.02 s. A real `~/Development` (91 entries, 19 repos) took 0.3 s
  (0.06 s with `--no-git`). The cost is about 9 ms per repo (3 git processes, 8 in
  parallel), more for big or very dirty repos (`git status`), plus the folder walks for
  `repos_below` and `--repos-only` (bounded: 2 s). With the defaults a call is bounded by
  about 2 s of walking + 5 s of git + output: give it a **10 s timeout**. For a phone,
  `--no-git` first (fast), then the same call without it to fill in the details.
- `kex init --from <file|-> --json`: sets up the repos the input
  lists, with no prompts. Input:

  ```ts
  type FromInput =
    | FromRepo[]
    | { project?: string | { new: string; id?: string }; repos: FromRepo[] };
  interface FromRepo {
    path: string;
    name?: string;          // 1–40 characters, ^[a-z0-9][a-z0-9-]*$
    description?: string;   // at most 200 characters, one line (no control characters)
    repo_id?: string;
  }
  ```

  `path`: absolute, `~/…`, or relative to kex's working folder; a symlink is resolved (the
  default name is the link's). `name`: 1–40 lowercase letters, digits and `-` (default: slug of
  the folder, cut to 40). `description`: at most 200 characters on one line, written to
  kex.json. A new project's name: 1–40 characters (its id is the slug + `-xxxxxx`). `project`: an id kex knows (registry,
  kex repos next to the given ones, or the repo kex runs in), or `{"new": "Shop"}` for a new
  one (`shop-xxxxxx`); `{"new": "Shop", "id": "shop-k2m9qa"}` uses that id (from a dry
  run's `plan.request`; if a project with that id exists by then, e.g. the request was sent
  before, the repos join it). `repo_id`: use this id (`r_` + 1–32 lowercase letters or
  digits) instead of a new one; it must not be used by a repo kex knows or twice in the
  input. Without `project`: `--project`, else the project of the kex repo kex runs
  in, else a new project named after the folder around the first repo.

  Everything is checked **before anything is written**, and every problem is reported at
  once: one `error` event per problem with `index` and `field`, then `exit` 2. Checked:
  valid JSON and shape, unknown keys (`field` = the key), `path` exists, is a folder, is the
  top folder of a git repo, isn't a linked worktree, isn't listed twice, isn't a kex repo of
  **another** project, has a readable `.gitignore`; `name` is valid, not taken in the project
  and not used twice; `repo_id` is valid and unused; `project` exists. A repo already in
  **this** project is not an error: it is in `added.skipped` (`reason: "already_kex"`,
  `in_project: true`, `index`), so the same request can be sent again. kex never edits an
  existing kex.json: when the input's `name`, `description` or `repo_id` differ from it,
  the skipped entry says so in `differs` and `note`. Moving a repo to another project isn't
  supported (a decision, not a gap): such a repo is an error with `field: "path"`.

  `--dry-run` sends a `plan` event (the project, each kex.json path with its full content,
  the `.gitignore` line, skipped, errors, and `request`: the input with all ids pinned) and
  writes nothing; exit 0 if valid, else 2. Send `plan.request` back without `--dry-run` and
  the files written are exactly the plan's `config`s.

  Writing is **all or nothing**: each file goes to a temporary file next to it and is
  renamed into place (never half-written), and if one repo fails, the ones already written
  are removed again (`added` with `repos: []`, `failed`, `rolled_back`; exit 1). SIGINT or
  SIGTERM during the writes undoes everything written so far and exits 130 (after a `log`
  "interrupted: nothing was written"). The registry is only updated after every file is in
  place. Success: `added` lists exactly the repos written.

No mode sets up the same repo twice: a workspace file listing a folder twice (or under a
link) counts it once, and a request naming one repo twice is an error ("the same repo as
repos[i]").

### Limits

The same everywhere (terminal questions, `--from`, `--repos`, `--repo`, and
schema/kex.schema.json):

| value | rule |
|---|---|
| repo name | 1–40 characters, `^[a-z0-9][a-z0-9-]*$` |
| description | at most 200 characters, one line, no control characters |
| new project name | 1–40 characters, one line (the id is its slug + `-xxxxxx`, at most 47) |
| `repo_id` (pinned) | `r_` + 1–32 lowercase letters or digits |

Names kex makes from folder names are cut to 40. In a terminal a name breaking these is
asked again; otherwise it is an `error` with `index`/`field`, exit 2.

## Rendering guide

- `select` → buttons or a single-choice list. Show `label` (monospace keeps its columns) or
  build your own row from `data`. Send `value`.
- `multiselect` → checkboxes, pre-checked from `default`. Send the checked `value`s.
- `confirm` → yes/no buttons, `default` focused.
- `text` → input field prefilled with `default`.
- `log` → a console/transcript pane. `warn` in a warning color.
- `pull` → one card per repo. Use `ok` for the color, `status` as text, and a table of
  `unmerged`.
- `status` → a table. `prs` → a list or table, with a `production` badge.
- `output` → one terminal pane per `repo` (or tabs), with the `started` / `ready` /
  `exited` events as the service state (starting → ready → exited).
- Always handle `error` (with `id`: show it next to the open prompt) and `exit`.

## Example sessions

The lines below are what kex sends, but **key order is not significant**: kex may write an
object's keys in a different order than shown (prompts, for example, come out as `type`,
`message`, `options`, `data`, `id`, `kind`). Parse each line as JSON; never match on text.

`kex pr check --json`:

```
← {"type":"hello","kex":"0.2.0","protocol":1,"features":[…]}
← {"type":"project","id":"acme-shop-7f3kq2","repos":[{"repo":"api","path":"/src/api","worktree":false}],"missing":[],"warnings":[]}
← {"type":"prs","items":[{"repo":"api","number":12,"title":"Login","head":"feature/login","base":"dev","author":"jane","created_at":"2026-11-10T09:03:00Z","draft":false,"production":false}]}
← {"type":"prompt","id":"p1","kind":"select","message":"Open PRs","options":[{"index":0,"value":"api#12","label":"api      #12    Login …","data":{"repo":"api","number":12,…}},{"index":1,"value":"quit","label":"Quit"}]}
→ {"id":"p1","answer":"api#12"}
← {"type":"log","level":"info","text":"================…"}
← {"type":"log","level":"info","text":"title:\tLogin\n…"}
← {"type":"prompt","id":"p2","kind":"select","message":"#12 Login","options":[{"index":0,"value":"merge","label":"Merge"},{"index":1,"value":"diff","label":"Show diff"},{"index":2,"value":"back","label":"Back"}]}
→ {"id":"p2","answer":"merge"}
← {"type":"prompt","id":"p3","kind":"confirm","message":"Merge #12 \"Login\" (api) into dev?","default":false}
→ {"id":"p3","answer":true}
← {"type":"prompt","id":"p4","kind":"text","message":"Type `accept` to merge:","default":null}
→ {"id":"p4","answer":"accept"}
← {"type":"merged","repo":"api","number":12,"state":"MERGED"}
← {"type":"log","level":"info","text":"✓ #12 merged into dev"}
← {"type":"prs","items":[]}
← {"type":"log","level":"info","text":"no open PRs"}
← {"type":"exit","code":0}
```

`kex browse ~/src/shop --depth 1 --json` (one line per event, shortened):

```
← {"type":"hello","kex":"0.2.0","protocol":1,"features":[…]}
← {"type":"tree","root":"/Users/me/src/shop","parent":"/Users/me/src","kind":"dir","readable":true,"error":null,"depth":1,"max_entries":500,"count":4,"truncated":false,"offset":0,"children_total":4,"has_more":false,"next_offset":null,"git":true,"git_partial":false,"flat":false,"tree":[{"name":"admin","kind":"kex_repo","project":"shop-7f3kq2","repo":"admin",…},{"name":"api","kind":"repo","branch":"dev","dirty":2,"remote_display":"github.com/acme/api","stack":["node"],…},{"name":"clients","kind":"dir","repos_below":1,"truncated":true,"children":[],…},{"name":"linked","path":"/Users/me/src/shop/linked","kind":"symlink","target":"/Users/me/elsewhere/ext","target_kind":"repo",…}]}
← {"type":"workspaces","workspaces":[{"path":"/Users/me/src/shop/shop.code-workspace","name":"shop","format":"vscode","folders":[…],"counts":{"folders":4,"repos":3,"addable":2,"already_kex":1,"worktrees":0,"missing":1,"duplicates":0}}]}
← {"type":"exit","code":0}
```

`kex init --from - --json` with a problem in two entries (stdin: the JSON, then EOF):

```
→ {"project":{"new":"Shop"},"repos":[{"path":"/src/shop/api","name":"API"},{"path":"/src/shop/web"},{"path":"/src/shop/docs"}]}
← {"type":"hello","kex":"0.2.0","protocol":1,"features":[…]}
← {"type":"error","message":"/src/shop/docs isn't a git repo","index":2,"field":"path"}
← {"type":"error","message":"\"API\" isn't a repo name: use lowercase letters, digits and -, starting with a letter or digit","index":0,"field":"name"}
← {"type":"exit","code":2}
```

Minimal Node.js client:

```js
import { spawn } from "node:child_process";
import readline from "node:readline";

const kex = spawn("kex", ["run", "--json"], { cwd: repoDir, stdio: ["pipe", "pipe", "inherit"] });
const send = (msg) => kex.stdin.write(JSON.stringify(msg) + "\n");

readline.createInterface({ input: kex.stdout }).on("line", async (line) => {
  const ev = JSON.parse(line);
  switch (ev.type) {
    case "prompt": {
      const answer = await showPromptInMyUi(ev);   // returns value(s) / boolean / string / null
      send(answer === undefined ? { cancel: true } : { id: ev.id, answer });
      break;
    }
    case "output": appendToPane(ev.repo, ev.line); break;
    case "error":  showError(ev); break;
    case "exit":   done(ev.code); break;
    default:       handleOther(ev);               // ignore unknown types
  }
});
```
