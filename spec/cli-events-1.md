# cli-events/1

**A small standard for command-line tools that talk to programs in NDJSON.**

- Identifier: `cli-events/1` · Status: draft · Schema: [`cli-events-1.schema.json`](cli-events-1.schema.json)
  (JSON Schema 2020-12)
- Licence: MIT (see [LICENSE](LICENSE)). Anyone may implement, copy, adapt or publish this
  specification and its schema.
- Language: English. Clients translate by id (see [Text and ids](#4-text-and-ids)).

A tool that speaks cli-events describes itself (its commands, their options, its exit codes)
in a **manifest**, and describes how to show every result in a generic **view**. A client
(a GUI, a phone app, an editor, a chat agent's host) can then offer the tool's commands,
build their forms, run them, answer their questions and draw their results **without code
for that tool**. A new tool release needs no new client release.

The examples use `acmectl`, a fictional deploy tool. Nothing here is specific to any
real tool.

The key words MUST, MUST NOT, REQUIRED, SHOULD, SHOULD NOT, RECOMMENDED and MAY are to be
interpreted as described in RFC 2119 when, and only when, they appear in capitals. Text
without them (descriptions, examples, the TypeScript comments that only say what a field
holds) is not normative.

**Tool** is the program that writes events. **Client** is the program that starts the tool
and reads them. **Person** is the human using the client. **Run** is one start of the tool,
from its process start to its exit.

## Contents

1. [Transport](#1-transport)
2. [The envelope](#2-the-envelope)
3. [Standard types](#3-standard-types)
4. [Text and ids](#4-text-and-ids)
5. [Views](#5-views)
6. [Prompts and answers](#6-prompts-and-answers)
7. [The manifest](#7-the-manifest)
8. [JSON input commands: the generic flow](#8-json-input-commands-the-generic-flow)
9. [Limits](#9-limits)
10. [What a client needs](#10-what-a-client-needs)
11. [Versioning and compatibility](#11-versioning-and-compatibility)
12. [Security considerations](#12-security-considerations)
13. [Conformance](#13-conformance)
14. [Governance](#14-governance)
15. [Related work](#15-related-work)
16. [Examples](#16-examples)

## 1. Transport

- The client starts the tool as a child process, with the tool's JSON flag (the manifest's
  `global_options` entry with `role: "json"`, usually `--json`), stdin and stdout piped.
- **stdout** carries lines from the tool and **stdin** lines from the client. **stderr** is
  free text for diagnostics: a client MAY show it, and MUST NOT parse it.
- Framing is NDJSON: one JSON object per line, UTF-8, ended by `\n`. In JSON mode a tool MUST
  NOT write anything else to stdout; output of programs it starts goes to stderr.
- Strings MUST NOT contain ANSI escape codes.
- Key order inside an object is not significant; a client MUST NOT depend on it.
- A client MUST build argv as a list and MUST NOT pass it through a shell. It MUST pass an
  option with a value as one element `--flag=value` (so a value starting with `-` can't be
  taken for a flag) and MUST put positional values after a `--` element. A tool MUST accept
  both forms.

### Discovery and caching

A client finds a tool's manifest by running its manifest command, by convention
`<tool> manifest --json`. That argv (and how to install the tool) is the only thing a client
needs to know about a tool in advance.

Every `hello` carries `manifest_hash`, a short hash of the tool's manifest (section 7.1). A
client SHOULD cache the manifest per tool executable and `manifest_hash`, and SHOULD read the
manifest again when a run's `hello` has a hash it doesn't hold. A client MUST use, for a
run, the manifest whose hash that run's `hello` announced (section 12).

## 2. The envelope

Every line is a JSON object with a string `type`. These top-level names are **reserved** on
every line; a tool MUST NOT use them with another meaning:

| field | meaning |
|---|---|
| `type` | REQUIRED. A standard type (section 3) or a tool-specific one. |
| `key` | identity of the line's `view`: a later line with the same `key` replaces it, or adds to it (see [Keys and modes](#keys-and-modes)). |
| `view` | how to show this line (section 5). |
| `ts` | time the line was written, RFC 3339 with offset. Optional; a client MUST NOT require it. |

`id` has its standard meaning (a prompt's id) only on `prompt`, `answer` and `error` lines.
On a tool-specific type it is the tool's own field.

**Tool-specific types are allowed** (`deploy`, `build_step`, …) and MAY carry any fields, but
each one MUST carry a `view`. A client that knows nothing about a tool draws a whole run from
the standard types and the views alone, and ignores the other fields.

## 3. Standard types

Lines from the tool: `hello`, `manifest`, `prompt`, `log`, `error`, `result`, `exit`.
Lines from the client: `answer`, and the literal `send` object of a view action (5.4). A
client MUST NOT send other lines.

```ts
type Line = Hello | Manifest | Prompt | Log | ErrorLine | Result | Exit | Answer | ToolLine;

/** Always the first line, exactly once. */
interface Hello {
  type: "hello";
  standard: "cli-events/1";
  tool: { name: string; version: string };   // name: the executable's name
  manifest_version: 1;
  manifest_hash: string;     // 16 lowercase hex characters (section 7.1)
}

/** Only from the manifest command; see section 7. */
interface ManifestLine { type: "manifest"; manifest: Manifest }

/** Section 6. */
type Prompt = SelectPrompt | MultiSelectPrompt | ConfirmPrompt | TextPrompt;

/** Everything a person would read in the terminal. Shown in a transcript. */
interface Log extends Translatable {
  type: "log";
  text: string;
  level: "info" | "warn" | "error" | "win";   // win: something succeeded (✓)
  /** The key of a view this line repeats in words (see below). */
  restates?: string;
}

interface ErrorLine extends Translatable {
  type: "error";
  message: string;
  /** Present: this rejects the answer to prompt `id`; the prompt stays open.
   *  Absent: a fatal error; `exit` follows. */
  id?: string;
  /** For JSON-input commands (section 8): which item of the input list (`index`) and which
   *  key (`field`) the problem is about. `null`/absent: not about one item / one key. */
  index?: number | null;
  field?: string | null;
}

/** A result with nothing tool-specific to say: just a view. */
interface Result { type: "result"; key?: string; view: View }

/** Always the last line, exactly once; the process then exits with `code`. */
interface Exit {
  type: "exit";
  code: number;
  tone: ExitTone;
}
type ExitTone = "ok" | "partial" | "error" | "missing_tool" | "cancelled";

/** Any other type, from the tool. */
interface ToolLine { type: string; key?: string; view: View; [field: string]: unknown }
```

Rules:

- `hello` MUST be the first line and `exit` the last, each exactly once.
- A tool that fails MUST send a fatal `error` (one without `id`), then `exit`. When several
  input problems are found (section 8), the tool SHOULD report all of them, one `error` each.
- `exit.tone` is REQUIRED. When the manifest lists `exit.code`, the tone MUST be the
  manifest's tone for it; for a code the manifest doesn't list, the tone still says how the
  run ended, so a client always has one.
- `restates`: a client that shows the view with that key MAY hide the log line; a client
  that doesn't show that view MUST show the line.
- Cancelling: closing stdin while a prompt is open cancels the run. To cancel while no
  prompt is open, the client sends the tool SIGINT (on Windows, CTRL_BREAK). Either way the
  tool SHOULD clean up and MUST then send `exit` with tone `cancelled`. A long-running
  command MAY also declare a softer `stop` line (section 7).

`Translatable` (section 4) puts `msg` and `params` next to a line's main string: `text` of
a `log`, `message` of an `error` or a `prompt`, `label` of a prompt option.

## 4. Text and ids

Every string a person reads is English and is either **data** (a branch name, a path, a
commit title, someone's output: never translated) or **UI text** (a label, a status, a
fixed sentence: translatable). UI text has a stable id next to it, so a client can look up
its own translation, and keeps its English text, so a client without translations still
works.

```ts
/** A field holding human text in a view or a prompt. A plain string is data. */
type Text = string | Msg;

/** UI text. */
interface Msg extends Translatable {
  text: string;                        // English, as the tool would print it
}

/** The id of a string and the values in it. On a standard line these sit next to the
 *  line's main string (`log.text`, `prompt.message`, …). */
interface Translatable {
  msg?: string;
  params?: Record<string, string | number | boolean | null>;
}
```

Rules:

- **Text is always there.** A `Msg` MUST have `text`. A client that has no translation for
  an id MUST show `text`.
- **UI text has an id.** A tool MUST send UI text in views and prompts as a `Msg` with a
  `msg`, and SHOULD give fixed log lines and errors a `msg`. Data MUST NOT be sent as a `Msg`
  (it would be translated).
- **Syntax**: lowercase, dot-separated segments of `[a-z0-9_]`, starting with a letter:
  `^[a-z][a-z0-9_]*(\.[a-z0-9_]+)*$`, e.g. `pull.pulled`, `col.branch`, `badge.production`.
  Ids are per tool (a client keeps one catalogue per tool name).
- **Stable**: an id MUST keep its meaning and its params across releases. Rewording the
  English keeps the id; a different meaning gets a new id. A tool MAY stop using an id.
- **Params**: the values that vary, by name. The English template is in the manifest's
  `messages` (`"pull.pulled": "{n} commits pulled"`); placeholders are `{name}`, and every
  param a line sends MUST be a placeholder of its template. A `{…}` that names no param of
  the line is literal text (`"expected an object {path, name}"`). Plurals and word order are the
  translator's job: params give the numbers. String params are data.
- **Structural ids**: the manifest's command, option and exit-code strings have no `msg`;
  they are translated by position, with the keys `cmd.<command id>.label`,
  `cmd.<command id>.description`, `cmd.<command id>.opt.<option id>.label` (`.description`,
  `.placeholder`), `cmd.<command id>.opt.<option id>.choice.<value>`,
  `global.<option id>.label`, `exit.<code>.title` (`.description`).
- **Conventions**: a column's label id is `col.<column key>`, a badge's `badge.<name>`.

```json
{"text": "3 commits pulled", "msg": "pull.pulled", "params": {"n": 3}}
```

## 5. Views

```ts
type View = LogView | CardView | TableView | CardsView | TreeView | ServiceView
          | ProgressView | NoneView;

/** Shared by every kind. */
interface ViewBase {
  kind: string;
  title?: Text;
  subtitle?: Text;          // one line under the title (often data: a branch, a path)
  summary?: Text;           // one sentence: what happened
  tone?: Tone;              // colour of the whole view (its edge, its icon)
  badges?: Badge[];
  notes?: LogLine[];        // short lines under the view (a warning, why a list is empty)
  actions?: Action[];       // buttons (5.4)
  mode?: "replace" | "append";   // with the line's `key`; default "replace"
}

type Tone = "ok" | "warn" | "danger" | "muted" | "info";
interface Badge { text: Text; tone?: Tone }   // a chip; `text.msg` is its id
interface LogLine { level: "info" | "warn" | "error" | "win"; text: Text }
```

### 5.1 Kinds

```ts
/** Lines of text. */
interface LogView extends ViewBase { kind: "log"; lines: LogLine[] }

/** One thing, as label: value pairs. `sections` are views drawn inside it (a table of
 *  details, more cards); clients may draw them collapsed. */
interface CardView extends ViewBase {
  kind: "card";
  fields?: Field[];
  sections?: View[];
}
interface Field { key: string; label: Text; value: Value; type?: ValueType; tone?: Tone }

/** Rows under named columns. */
interface TableView extends ViewBase {
  kind: "table";
  columns: Column[];
  rows: Row[];
  empty?: Text;             // shown instead of an empty table ("No open PRs")
  next?: RunAction;         // the next page of rows, when cut (section 9). Navigation.
}

/** The same rows, drawn as one card each (on a phone, say). The first column (or
 *  `title_column`) is each card's title; the other cells its fields. */
interface CardsView extends Omit<TableView, "kind"> {
  kind: "cards";
  title_column?: string;
}

interface Column {
  key: string;              // cells are looked up by this key
  label: Text;
  type?: ValueType;         // default "text"
  hidden?: boolean;         // carried in the rows (for details, for sorting), not shown
}

interface Row {
  key: string;              // unique within the view (append mode matches rows by key)
  cells: Record<string, Value | Cell>;  // by column key; missing = empty
  badges?: Badge[];
  tone?: Tone;
  detail?: Text;            // a second line under the row
  details?: Field[];        // more, shown when the row is opened
  actions?: Action[];       // buttons on this row
}
/** A cell that needs more than its value. */
interface Cell { value: Value; text?: Text; tone?: Tone }   // text: show this instead

type Value = string | number | boolean | null | string[];

/** Folders, groups, anything nested. */
interface TreeView extends ViewBase {
  kind: "tree";
  root?: { label: Text; value?: string };
  columns?: Column[];       // for the nodes' cells
  nodes: TreeNode[];
  /** How to look inside a node that has `more: true`: run `open.run` with option
   *  `open.option` set to the node's `value`. Navigation (see 5.4). */
  open?: { run: RunAction["run"]; option: string };
  up?: RunAction;           // the parent ("../"). Navigation.
  next?: RunAction;         // the next page of `nodes`, when cut. Navigation.
}
interface TreeNode {
  key: string;
  label: Text;
  value?: string;           // what `open` and pickers pass back (a path, an id)
  kind?: string;            // the tool's own kind ("dir", "repo"); pickers filter on it
  cells?: Record<string, Value | Cell>;
  badges?: Badge[];
  tone?: Tone;
  detail?: Text;
  selectable?: boolean;     // can be picked in a picker (section 8)
  checked?: boolean;        // picked at first
  /** Picked: the object this node adds to a JSON input's list (section 8). */
  item?: Record<string, unknown>;
  children?: TreeNode[];
  more?: boolean;           // has children not sent here: `open` loads them
}

/** A long-running process: state, output lines, buttons. */
interface ServiceView extends ViewBase {
  kind: "service";
  name: Text;
  state: "starting" | "running" | "ready" | "stopping" | "exited" | "failed";
  command?: string;         // shown as code
  url?: string | null;      // where it answers, once ready
  pid?: number | null;
  exit_code?: number | null;
  lines?: { text: string; stream?: "stdout" | "stderr" }[];
  /** Buttons that only make sense while it runs (hot reload, …). */
  actions?: Action[];
}

/** Something counting up. `total: null` = unknown (a spinner with a count). */
interface ProgressView extends ViewBase {
  kind: "progress";
  current: number;
  total: number | null;
  label?: Text;
}

/** Nothing to draw: the line is data for clients that know its type. */
interface NoneView { kind: "none" }
```

A client MUST offer a service view's `actions` only while its `state` is `starting`,
`running` or `ready`.

### 5.2 Value types

| `type` | value | draw it as |
|---|---|---|
| `text` | string | plain text (the default) |
| `number` | number | a number, right-aligned in tables |
| `count` | integer | a number; `0` muted or `—` |
| `bool` | boolean | ✓ / — |
| `branch` | string | a branch name (monospace, maybe an icon) |
| `path` | string | a file path; a client may shorten the home folder to `~` |
| `url` | string | a link |
| `date` | string | RFC 3339 with offset; the client formats it (relative, local) |
| `badge` | string | a chip with that text (the cell's `tone`) |
| `code` | string | monospace, kept as is (may have several lines) |

A `string[]` value is a list: draw its items joined with `, ` (as chips for `badge`).
A `Cell` with `text` is drawn as that text (`{"value": null, "text": {"text": "no upstream",
"msg": "status.no_upstream"}, "tone": "muted"}`). A `url` value is data from the tool: a
client MUST open it only when the person follows it, and SHOULD show only `http`/`https`
links as links.

### Keys and modes

A client keeps the views it has drawn **by `key`** (the line's envelope `key`):

- A line with a `key` and `mode: "replace"` (the default) replaces the view with that key,
  in place; with a new key it adds a view at the end.
- `mode: "append"` adds to the view with that key (creating it if there is none): `lines`
  (log, service) and `rows` (table, cards) are appended; a row whose `key` is already there
  replaces that row; every other field the line has (`state`, `url`, `tone`, `title`, …)
  overwrites the old one.
- A line without a `key` is a new view each time.

In append mode a view needs only its `kind` (and what it adds): the fields a kind requires
(a table's `columns`, a service's `name` and `state`) came with the first line.

### 5.3 Tone

`ok` green, `warn` amber, `danger` red, `muted` grey, `info` neutral/blue. `Log.level`
maps to tone as `win → ok`, `warn → warn`, `error → danger`, `info → info`.
`Exit.tone` maps as `ok → ok`, `partial → warn`, `error → danger`, `missing_tool → warn`,
`cancelled → muted`.

### 5.4 Actions

A button on a view. An action is a **request from the tool**, not a command: the client
checks it, asks the person, and only then acts.

```ts
type Action = SendAction | RunAction;
interface ActionBase {
  label: Text;
  tone?: Tone;
  /** What it does, beyond showing things: "write" (changes files or state), "install"
   *  (installs or updates software), "delete" (removes something). Absent: changes nothing
   *  the person would need to confirm, beyond what `tone` says. */
  effect?: "write" | "install" | "delete";
  confirm?: Text;           // the question to ask before acting
}

/** Write this line to the running tool's stdin, as is. */
interface SendAction extends ActionBase { send: Record<string, unknown> }

/** Ask to start the tool again: command `command` (an id from the manifest) with these
 *  option values (by option id). `cwd`: a folder hint (rules below). `stdin`: a JSON value
 *  to write as one line to the new process's stdin, which is then closed. */
interface RunAction extends ActionBase {
  run: { command: string; options?: Record<string, unknown>; stdin?: unknown; cwd?: string };
}
```

**Who presses.** Only a person may trigger an action, by pressing it. A client MUST NOT
trigger actions on its own, and MUST NOT let an automated agent (a program, a model, a
script acting through the client) trigger them.

**Confirm.** Before acting, a client MUST ask the person, and act only on a yes, for:

- every `RunAction` (except navigation, below);
- every action with `tone: "danger"`;
- every action with `effect` `"install"` or `"delete"`, or an `effect` it doesn't know.

It asks with `confirm` when the action has one, else with a default question ("Run
`<label>`?"). A tool SHOULD set `confirm` on all of these. A client SHOULD show what will
run (the command's label and the option values) with the question.

**SendAction.** A client MUST send `send` verbatim, MUST send it only to the run whose line
carried the action (never an action it built or edited itself, never one from another run),
and only while that run is alive (before its `exit`).

**RunAction.** A client MUST check each of these, and MUST refuse the action (and say why)
when one fails:

1. `run.command` is a command id in the manifest the client holds for the current run's
   `manifest_hash`, and that command doesn't have `needs_terminal: true`.
2. `run.options` validates against that command's options exactly as a form would: only
   known option ids, each value of the option's type, `choices`, `pattern`, lengths, `min` /
   `max`, `required`, `conflicts_with`, `requires`.
3. The client builds argv itself from the manifest (section 7): the command's `words`, then
   the options. It MUST NOT take argv words from the action or anywhere else in the tool's
   output.
4. `run.stdin`: when present, the command has `json_input`, `run.options` sets its option
   to the option's `stdin_value`, and `run.stdin` validates against `json_input.schema` and
   fits the size limit (section 9). Otherwise the action carries no `stdin`.
5. `run.cwd` is a hint. The client MUST check it against the folders it already allows for
   this person (for example the folder of the current session, or the root a picker was
   opened on) and refuse any other folder. A tool SHOULD omit `cwd` (the new run starts
   where the current one did) unless the folder came from a `tree` node the person opened.

**Navigation.** A tree's `open`, `up` and `next`, and a table's `next`, are navigation. A
client MUST check them as RunActions (rules 1–5) but MAY run them without a confirm when
their command has `read_only: true` in the manifest; when it doesn't, they are ordinary
RunActions.

**Policy.** Which actions a client offers at all (for example whether it allows
`install` actions from a phone) is the client's policy. A client MAY hide any action.

## 6. Prompts and answers

A prompt stops the tool until the client answers it. Kinds and answers are fixed:

```ts
interface PromptBase extends Translatable {
  type: "prompt";
  id: string;                  // unique within the run
  message: string;             // the question
  tone?: Tone;                 // danger: the answer does something hard to undo
}

interface PromptOption extends Translatable {
  index: number;               // 0-based
  value: string;               // stable: answer with this
  label: string;               // as the terminal shows it (may be column-aligned)
  /** The option as a row of `columns` (a Row without `key`: its key is `value`). */
  view?: { cells: Record<string, Value | Cell>; badges?: Badge[]; tone?: Tone; detail?: Text };
  data?: Record<string, unknown>;   // the tool's own details; ignore if unknown
}

interface SelectPrompt extends PromptBase {
  kind: "select";
  options: PromptOption[];
  columns?: Column[];          // draw the options as a table with these columns
  /** These options are the rows of the view with this key (drawn already): a client may
   *  show the choice there instead of a second list. */
  picks_from?: string;
}
interface MultiSelectPrompt extends Omit<SelectPrompt, "kind"> {
  kind: "multiselect";
  default: number[];           // indexes ticked at first
}
interface ConfirmPrompt extends PromptBase { kind: "confirm"; default: boolean }
interface TextPrompt extends PromptBase {
  kind: "text";
  default: string | null;
  pattern?: string;            // ECMAScript regex the whole answer must match
  min_length?: number;
  max_length?: number;
  /** A typed confirmation: the person must type exactly this (rules below). */
  expect?: string;
}
```

Options without `view` (and prompts without `columns`) are drawn from `label`.

The answer, one line on stdin:

```ts
type Answer =
  | { type: "answer"; id: string; answer: AnswerValue }
  | { type: "answer"; id: string; cancel: true };          // stop the run: exit tone "cancelled"
type AnswerValue =
  | string            // select: an option's value · text: the text
  | number            // select: an option's index
  | (string | number)[]   // multiselect
  | boolean           // confirm
  | null;             // select: skip / back · multiselect: none · confirm, text: the default
```

Rules:

- A rejected answer gets an `error` with the prompt's `id`; the prompt stays open, and the
  client MAY answer again.
- **Typed confirmations** (`expect`): a client MUST NOT prefill the field, and MUST NOT answer
  the prompt on the person's behalf: not automatically, not from saved or remembered answers,
  not through an automated agent. It SHOULD keep its submit button off until the text
  matches `expect`.
- **Unknown kinds**: a client MUST NOT guess an answer to a prompt kind it doesn't know. It
  MUST cancel the run and say why.
- Whether an automated agent may answer other prompts is the client's policy.

## 7. The manifest

`<tool> manifest --json` sends `hello`, one `manifest` line, `exit`.

```ts
interface Manifest {
  manifest_version: 1;
  standard: "cli-events/1";
  tool: { name: string; version: string; label?: string; description?: string; homepage?: string };
  features: string[];                 // what this build can do; stable strings, only added
  global_options: Option[];           // e.g. the JSON flag: { id: "json", flag: "--json", role: "json" }
  commands: Command[];
  exit_codes: ExitCode[];
  views: { kinds: string[]; value_types: string[] };   // what the tool sends
  event_types: { type: string; view_kind: string; description: string }[];
  messages: Record<string, string>;   // msg id → English template with {params}
}

interface Command {
  id: string;                  // dot case: "deploy", "env.list"
  words: string[];             // argv before the options: ["env", "list"]
  aliases?: string[][];        // other words that run it
  label: string;               // "Deploy"
  description: string;         // one line
  group?: string;              // id, for grouping: "deploy", "info", "setup"
  icon?: string;               // a hint ("deploy", "list"); clients map it to their own icons
  interactive: boolean;        // may send prompts (some options turn them off)
  long_running: boolean;       // keeps running until stopped
  /** Asks nothing and changes nothing, with any options: listing, showing, checking. */
  read_only?: boolean;
  /** long_running: the line that stops it (Stop button); cancel stops it too. */
  stop?: Record<string, unknown>;
  /** Doesn't speak JSON (it writes raw output). */
  needs_terminal?: boolean;
  options: Option[];
  /** Takes a JSON document as input (section 8). */
  json_input?: JsonInput;
  examples?: { label: Text; options: Record<string, unknown> }[];
}

interface Option {
  id: string;                  // snake case, stable: the key in RunAction.options
  flag?: string;               // "--dry-run"; absent for positionals
  short?: string;              // "-n"
  positional?: number;         // 0-based position (after `--`, section 1)
  type: "boolean" | "string" | "integer" | "choice" | "list" | "path" | "paths" | "json";
  label: string;
  description?: string;
  placeholder?: string;
  required?: boolean;
  default?: unknown;
  choices?: { value: string; label: string }[];   // type choice
  pattern?: string;            // each value (each item of a list)
  min_length?: number; max_length?: number;
  min?: number; max?: number;  // integer
  separator?: string;          // list: join items with this ("a,b"); absent: repeat the flag
  repeatable?: boolean;        // list / paths: the flag may be given several times
  conflicts_with?: string[];   // option ids
  requires?: string[];
  role?: "json" | "dry_run";   // the JSON flag; the "preview only" switch
  /** type json: the argument value that means "read stdin" ("-"). */
  stdin_value?: string;
}

interface ExitCode {
  code: number;
  tone: ExitTone;
  title?: string;              // "Some deploys failed"
  description?: string;
}
```

Rules:

- The manifest MUST list every command and option the tool accepts, except hidden ones,
  which it MAY leave out; every tool-specific line type it sends (`event_types`); and every
  `msg` id it sends (`messages`).
- A manifest's constraints (`pattern`, lengths, `min` / `max`, `choices`) MUST NOT be
  stricter than what the tool itself accepts, and SHOULD be as strict.
- A client MUST NOT offer a `needs_terminal` command.
- `read_only: true` MUST only be set on a command that, with any option values, sends no
  prompt and changes nothing outside the client's view (no files, no remote state, no
  installs).
- Argv from a form: `words`, then each option with a value as `--flag=value` (`boolean`: just
  `--flag` when true; `list`: items joined with `separator`, or the flag once per item;
  `paths`: the flag once per path), then the JSON flag, then `--`, then positionals in order.

**End of a run.** The outcome is `exit.tone`. A client MAY show its own title chosen by the
tone (its own wording or skin), and SHOULD then show the tool's `title` for the code, when
the manifest has one, under it.

### 7.1 `manifest_hash`

1. Take the value of the manifest line's `manifest` field.
2. Serialize it with the JSON Canonicalization Scheme (RFC 8785): object keys sorted, no
   whitespace, numbers and strings in their canonical form.
3. Hash the UTF-8 bytes with SHA-256; the first 16 lowercase hex characters are
   `manifest_hash`.

A manifest MUST NOT contain volatile values (the time, the machine's paths, random
values, the current folder): it depends only on the tool's build, so the hash changes
exactly when the manifest does. A client MAY compute the hash of a manifest it received and
compare it with `hello.manifest_hash`; on a mismatch it SHOULD read the manifest again and,
if it still doesn't match, MUST NOT use it for that run.

## 8. JSON input commands: the generic flow

Some commands take a document instead of flags: "set up these 20 things". The manifest says
how to build that document with generic widgets, so a client needs no screen for it:

```ts
interface JsonInput {
  option: string;              // the option (type json) that takes it, e.g. "from"
  schema: object;              // JSON Schema 2020-12 of the document
  /** How to fill the document's main list from a tree: */
  picker?: {
    command: string;           // a read_only command whose result is a `tree` view
    options?: Record<string, unknown>;
    select_kinds?: string[];   // only nodes of these `kind`s (and `selectable: true`)
    into: string;              // JSON Pointer of the list in the document, e.g. "/items"
  };
  /** The option (role dry_run) that only checks and previews. */
  dry_run?: string;
}
```

The flow, for any tool:

1. **Pick.** Run `picker.command` (with `picker.options`). Draw its `tree` view as a picker:
   nodes with `selectable: true` and a `kind` in `select_kinds` get a checkbox; `more`
   nodes open with `tree.open`; `up` and `next` move around. Each picked node's `item` is
   appended to the list at `picker.into`.
2. **Edit.** Show the list as a form built from `schema` (the items' properties: their types,
   patterns, lengths, descriptions). Other top-level properties too.
3. **Review.** Run the command with `option = stdin_value`, `dry_run = true` and the document
   on stdin. Problems come as `error` lines with `index` / `field` (show each at its item and
   field, go back to 2); otherwise the result is a view (a table of what will happen) whose
   `actions` include the real run.
4. **Run.** The person presses that action (a `RunAction` with the final document as
   `stdin`, often with ids pinned by the dry run), confirms, and its result views show what
   was done.

A client MUST validate the document against `schema` before steps 3 and 4 (the tool checks
it again). A tool MAY also put `RunAction`s that prefill the flow on other views (a found
group of things: "Use these 3").

## 9. Limits

So that both sides can cap without guessing. A tool SHOULD stay within these; a client
SHOULD accept at least these:

| what | limit |
|---|---|
| one line, either direction | 1 MiB (1,048,576 bytes, without the `\n`) |
| the manifest line | 512 KiB |
| rows of one `table` / `cards` line | 1,000 (more: page with `next`, or send them with `mode: "append"`) |
| nodes of one `tree` line, all levels | 1,000 (more: `next`, or `more: true` on the nodes) |
| options of one prompt | 1,000 |
| one service output line (`lines[].text`) | 16 KiB; a tool SHOULD cut a longer line and end it with `…` |
| service lines a client keeps per service | at least the last 500; it MAY drop older ones |
| `RunAction.stdin`, serialized | 1 MiB |

A client that gets a line over its limit, or one that isn't valid JSON, SHOULD drop it and
show a note ("A line from the tool was too long and was skipped"); the run goes on. Two
exceptions: a `prompt` it dropped can't be answered, so the client MUST cancel the run and
say why; a `manifest` over the limit is unusable, so the client MUST NOT offer the tool's
commands from it.

## 10. What a client needs

To run any cli-events tool, a client needs these generic widgets, and no others:

| widget | used for |
|---|---|
| form | a command's options (section 7), a JSON input's items (8.2), text prompts |
| list / table | `table`, `cards`, select and multiselect prompts with `columns` |
| card | `card` (with `sections`: nested views) |
| tree picker | `tree`, with checkboxes for the JSON input flow |
| log / transcript | `log` lines, `log` views, `notes` |
| service pane | `service`: state chip, output lines (auto-scroll), action buttons |
| progress | `progress` |
| buttons and confirms | `actions`, confirm prompts, a Stop button for long-running commands |
| outcome | the `exit` tone, with the manifest's title for the code |

Branding (icons per `icon` hint, skins per view kind or per exit tone) is the client's own.

## 11. Versioning and compatibility

- Within `cli-events/1` and `manifest_version: 1` every change is **additive**: new optional
  fields, new tool-specific types, new view kinds, value types, tones, effects, features,
  messages.
- A client MUST ignore fields and line types it doesn't know.
- An **unknown view kind**: a client MUST NOT drop the line silently. It SHOULD draw `title`,
  `summary` and `notes` if present, else a card of the view's other scalar fields.
- An **unknown value type** is drawn as `text`, an unknown tone as none, an unknown
  `Log.level` as `info`. An unknown `effect` is treated as needing a confirm (5.4).
- An **unknown prompt kind**: the run is cancelled (section 6).
- A breaking change is `cli-events/2`, with a new `manifest_version`. A tool MAY offer both,
  chosen by a flag; the manifest says which.

## 12. Security considerations

A client often relays a person's presses to a process on another machine, with that
person's rights. The tool's output is input to the client, and the client's lines are input
to the tool. Neither side trusts the other's lines. Collected from the sections above:

**Clients**

1. MUST NOT run the tool through a shell; MUST build argv itself, from the manifest only,
   with `--flag=value` and `--` before positionals (sections 1, 5.4, 7).
2. MUST let only a person trigger actions, never an automated agent, and only by pressing
   them (5.4).
3. MUST ask the person before every RunAction (except read-only navigation), every `danger`
   action and every `install` / `delete` / unknown-effect action (5.4).
4. MUST check every RunAction against the manifest of the current run (command id, not
   `needs_terminal`, options as a form, stdin against the input schema and the size limit,
   `cwd` against the folders it allows) and refuse it otherwise (5.4).
5. MUST send a SendAction's `send` verbatim, only to the run that carried it, only while that
   run is alive (5.4).
6. MUST NOT prefill or answer a typed confirmation (`expect`) on the person's behalf,
   including from saved answers or through an agent (section 6).
7. MUST NOT guess answers to unknown prompt kinds; MUST cancel the run (section 6).
8. MUST use the manifest matching the run's `manifest_hash` (sections 1, 7.1).
9. MUST NOT parse stderr; SHOULD apply the limits of section 9; MUST open `url` values only
   when the person follows them (sections 1, 5.2, 9).
10. MUST show strings as text, never as markup or code to run (a tool's strings are data even
    when they look like HTML or Markdown).

**Tools**

1. MUST treat every stdin line as untrusted input: validate answers against the open prompt,
   refuse lines they don't expect, never run their content through a shell.
2. MUST validate a JSON input document themselves, whatever the client checked.
3. SHOULD set `confirm` and the right `effect` and `tone` on actions that change things, and
   SHOULD omit `RunAction.cwd` unless the folder came from a node the person opened.
4. MUST NOT put secrets (tokens, passwords) in lines: clients may store and relay them.

## 13. Conformance

A tool conforms when:

1. every stdout line in JSON mode validates against
   [`cli-events-1.schema.json`](cli-events-1.schema.json) (`#/$defs/Line`), and it accepts
   the `answer` lines of section 6 on stdin;
2. `hello` is first and `exit` last, once each; `hello.manifest_hash` is the hash of its
   manifest (7.1); `exit.tone` is the manifest's tone for `exit.code` when the manifest lists
   the code;
3. every tool-specific line has a `view`; every `msg` it sends is in `messages`, with params
   that are placeholders of its template;
4. its manifest follows the rules of section 7, and every action it sends passes the checks
   of 5.4 against that manifest.

A client conforms when it follows the client rules of section 12 and the MUST rules of
sections 3–11.

The schema allows unknown fields everywhere (section 11), so it checks the shape of what is
there, not the absence of extras.

## 14. Governance

cli-events is maintained together by the maintainers of kex (its first tool) and of Sakabar
(its first client). A change to this specification or its schema needs the agreement of both;
when they disagree, the owner of the kex project decides. Proposals and discussion happen in
the kex repository.

## 15. Related work

- **OpenCLI Specification** describes a command-line tool's commands, arguments and options
  in a document, for documentation, completion and code generation. It overlaps with the
  cli-events manifest's `commands` and `options`. cli-events also covers what happens while
  a tool runs: streamed events, views, prompts and answers. An export from a cli-events
  manifest to an OpenCLI document may be added later.
- **NDJSON stream modes of other tools.** Many CLIs have a machine-readable output mode with
  one JSON object per line (for example build tools' JSON message formats, test runners'
  JSON event streams, infrastructure tools' JSON UI). Each defines its own events for its own
  domain. cli-events doesn't replace them: it is a common layer (an envelope, generic views,
  prompts, a manifest) that such a tool can add so one generic client can drive it.

## 16. Examples

`acmectl manifest --json` (shortened):

```ndjson
{"type":"hello","standard":"cli-events/1","tool":{"name":"acmectl","version":"2.4.0"},"manifest_version":1,"manifest_hash":"3f9a1c07d2e84b56"}
{"type":"manifest","manifest":{"manifest_version":1,"standard":"cli-events/1","tool":{"name":"acmectl","version":"2.4.0","label":"Acme"},"features":["deploy","envs"],"global_options":[{"id":"json","flag":"--json","type":"boolean","label":"JSON output","role":"json"}],"commands":[{"id":"deploy","words":["deploy"],"label":"Deploy","description":"Deploy services to an environment","group":"deploy","icon":"deploy","interactive":true,"long_running":false,"options":[{"id":"env","flag":"--env","type":"choice","label":"Environment","required":true,"choices":[{"value":"staging","label":"Staging"},{"value":"prod","label":"Production"}]},{"id":"services","flag":"--services","type":"list","separator":",","label":"Services","pattern":"^[a-z][a-z0-9-]*$","placeholder":"api,web"},{"id":"dry_run","flag":"--dry-run","type":"boolean","label":"Only show what would happen","role":"dry_run"}]},{"id":"env.list","words":["env","list"],"label":"Environments","description":"List the environments","group":"info","icon":"list","interactive":false,"long_running":false,"read_only":true,"options":[]}],"exit_codes":[{"code":0,"tone":"ok","title":"Deployed","description":"Everything worked."},{"code":1,"tone":"partial","title":"Some deploys failed","description":"The others worked."},{"code":2,"tone":"error","title":"Error"},{"code":130,"tone":"cancelled"}],"views":{"kinds":["card","table","log"],"value_types":["text","count","date"]},"event_types":[{"type":"deployed","view_kind":"card","description":"One service deployed."}],"messages":{"deploy.done":"Deployed in {seconds} s","deploy.done_line":"✓ {service} deployed to {env} in {seconds} s","deploy.again":"Deploy {service} again","deploy.again_confirm":"Deploy {service} to {env} again?","field.version":"Version","col.release":"Release","col.date":"Deployed","rollback.which":"Roll back which release?","badge.rollback":"rolled back","svc.reload":"Reload"}}}
{"type":"exit","code":0,"tone":"ok"}
```

(The hash is illustrative.) A tool-specific line with a card view and a confirmed
RunAction, and the log line that restates it:

```ndjson
{"type":"deployed","service":"api","env":"staging","seconds":42,"key":"deploy:api","view":{"kind":"card","title":"api","subtitle":"staging","tone":"ok","summary":{"text":"Deployed in 42 s","msg":"deploy.done","params":{"seconds":42}},"fields":[{"key":"version","label":{"text":"Version","msg":"field.version"},"value":"v812","type":"code"}],"actions":[{"label":{"text":"Deploy api again","msg":"deploy.again","params":{"service":"api"}},"effect":"write","confirm":{"text":"Deploy api to staging again?","msg":"deploy.again_confirm","params":{"service":"api","env":"staging"}},"run":{"command":"deploy","options":{"env":"staging","services":["api"]}}}]}}
{"type":"log","level":"win","text":"✓ api deployed to staging in 42 s","msg":"deploy.done_line","params":{"service":"api","env":"staging","seconds":42},"restates":"deploy:api"}
```

A select prompt drawn as a table:

```ndjson
{"type":"prompt","id":"p1","kind":"select","message":"Roll back which release?","msg":"rollback.which","columns":[{"key":"release","label":{"text":"Release","msg":"col.release"},"type":"code"},{"key":"date","label":{"text":"Deployed","msg":"col.date"},"type":"date"}],"options":[{"index":0,"value":"v811","label":"v811  2026-11-14 10:02","view":{"cells":{"release":"v811","date":"2026-11-14T10:02:00Z"}}},{"index":1,"value":"v810","label":"v810  2026-11-12 16:40","view":{"cells":{"release":"v810","date":"2026-11-12T16:40:00Z"},"badges":[{"text":{"text":"rolled back","msg":"badge.rollback"},"tone":"muted"}]}}]}
{"type":"answer","id":"p1","answer":"v811"}
```

A service, then an appended line, then its end:

```ndjson
{"type":"result","key":"svc:api","view":{"kind":"service","name":"api","state":"starting","command":"acmectl serve api","actions":[{"label":{"text":"Reload","msg":"svc.reload"},"send":{"type":"input","service":"api","text":"r"}}]}}
{"type":"result","key":"svc:api","view":{"kind":"service","mode":"append","lines":[{"text":"listening on :8080","stream":"stdout"}]}}
{"type":"result","key":"svc:api","view":{"kind":"service","mode":"append","state":"ready","url":"http://localhost:8080"}}
{"type":"result","key":"svc:api","view":{"kind":"service","mode":"append","state":"exited","exit_code":0,"tone":"muted"}}
{"type":"exit","code":0,"tone":"ok"}
```
