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
  | PullEvent | StatusEvent | PrsEvent | MergedEvent | PushedEvent
  | StartedEvent | OutputEvent | ReadyEvent | ExitedEvent
  | VersionEvent | UpdateAvailableEvent | UpdatedEvent;

// ── Session ──────────────────────────────────────────────────────────────────

interface Hello {
  type: "hello";
  kex: string;          // kex version, e.g. "0.2.0"
  protocol: 1;          // this document
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
| `init` | `Project` | select | project id, or `new` | `{ repos: string[] }` (none on `new`) |
| `init` | `New project name:` / `Repo name (…)` / `New repo name:` | text | | |
| `update` | `Install kex <version>?` | confirm | | |

The typed confirmations (`Type … to merge/push`) are safety checks for a person. A UI
should show them as a field the user fills in, not answer them automatically.

`kex run` does not ask "Run which repos?" when only one repo can run, or with `--repos`.
`push --repos a,b --yes` and `pr check --list` don't ask anything.

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

`kex pr check --json`:

```
← {"type":"hello","kex":"0.2.0","protocol":1}
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
