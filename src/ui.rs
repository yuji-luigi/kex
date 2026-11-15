//! Terminal output and questions, or the same as a JSON stream for other programs
//! (`--json`, BLUEPRINT §5.9).
//!
//! In JSON mode stdout carries one JSON object per line and nothing else. Questions become
//! `prompt` events; the caller answers with one line on stdin.

use std::io::{BufRead, Write};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use anyhow::{Result, bail};
use serde_json::{Map, Value, json};

pub const PROTOCOL: u32 = 1;

/// What this kex can do, sent in `hello`. Stable strings, only ever added: a client checks
/// for the one it needs instead of comparing versions (docs/json-protocol.md).
pub const FEATURES: &[&str] = &[
    "browse",
    "from",
    "dry_run",
    "workspaces",
    "pin_ids",
    "paging",
    "repos_only",
    "no_git",
    "manifest",
    "views",
    "init_prompts",
    "cli_events",
];

static JSON: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

pub fn set_json() {
    JSON.store(true, Ordering::SeqCst);
}

pub fn json() -> bool {
    JSON.load(Ordering::SeqCst)
}

/// The user (or the program driving kex) answered "cancel".
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}
impl std::error::Error for Cancelled {}

/// Set once `exit` is written: nothing may follow it (cli-events/1 §3).
static CLOSED: AtomicBool = AtomicBool::new(false);

/// Write one event line. `kind` becomes the `type` field. After `exit`, nothing is written.
pub fn emit(kind: &str, fields: Value) {
    let mut v = json!({ "type": kind });
    if let (Some(o), Value::Object(f)) = (v.as_object_mut(), fields) {
        o.extend(f);
    }
    let mut out = std::io::stdout().lock();
    if CLOSED.load(Ordering::SeqCst) {
        return;
    }
    if kind == "exit" {
        CLOSED.store(true, Ordering::SeqCst);
    }
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

/// Structured event, only in JSON mode (the human output says the same in words).
pub fn event(kind: &str, fields: Value) {
    if json() {
        emit(kind, fields);
    }
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // CSI: parameters, then one final byte in @..~
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            } else {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn say(text: &str) {
    if json() {
        emit("log", json!({ "level": "info", "text": strip_ansi(text) }));
    } else {
        println!("{text}");
    }
}

/// A ✓ line: level `win` in JSON mode.
pub fn say_win(text: &str) {
    if json() {
        emit("log", json!({ "level": "win", "text": strip_ansi(text) }));
    } else {
        println!("{text}");
    }
}

/// A line that says in words what the view with `key` shows (`restates`). `level`: info,
/// warn, error or win. In the terminal it goes to stdout, like [`say`].
pub fn say_about(key: &str, level: &str, text: &str) {
    if json() {
        emit(
            "log",
            json!({ "level": level, "text": strip_ansi(text), "restates": key }),
        );
    } else {
        println!("{text}");
    }
}

/// [`say_about`] for a warning: in the terminal it goes to stderr, like [`warn`].
pub fn warn_about(key: &str, text: &str) {
    if json() {
        say_about(key, "warn", text);
    } else {
        eprintln!("{text}");
    }
}

pub fn warn(text: &str) {
    if json() {
        emit("log", json!({ "level": "warn", "text": strip_ansi(text) }));
    } else {
        eprintln!("{text}");
    }
}

macro_rules! say {
    ($($t:tt)*) => { $crate::ui::say(&format!($($t)*)) };
}

macro_rules! warn {
    ($($t:tt)*) => { $crate::ui::warn(&format!($($t)*)) };
}

/// Where a child that would print to our terminal should write: in JSON mode stdout is
/// reserved for events, so it goes to stderr.
pub fn child_stdout() -> Stdio {
    if json() {
        Stdio::from(std::io::stderr())
    } else {
        Stdio::inherit()
    }
}

/// A question: its message (`{"text", "msg", "params"}`, from [`crate::view::msg`]) and
/// what else the prompt carries for programs (`columns`, `expect`, `tone`, `pattern`, …).
pub struct Q {
    msg: Value,
    extra: Map<String, Value>,
}

impl Q {
    pub fn new(id: &str, params: Value) -> Self {
        Q {
            msg: crate::view::msg(id, params),
            extra: Map::new(),
        }
    }
    pub fn with(mut self, key: &str, v: Value) -> Self {
        self.extra.insert(key.into(), v);
        self
    }
    pub fn text(&self) -> String {
        self.msg["text"].as_str().unwrap_or_default().to_string()
    }
    /// The prompt's fields: `message`, `msg`, `params`, the extras.
    fn fields(&self) -> Value {
        let mut f = self.extra.clone();
        f.insert("message".into(), json!(self.text()));
        f.insert("msg".into(), self.msg["msg"].clone());
        if let Some(p) = self.msg.get("params") {
            f.insert("params".into(), p.clone());
        }
        Value::Object(f)
    }
}

/// One choice in a list. `value` is a stable id a program can answer with; `data` carries
/// the structured details behind the label, `row` the same as a row of the prompt's
/// `columns`, `msg` the id of a fixed label.
pub struct Opt {
    pub value: String,
    pub label: String,
    pub data: Option<Value>,
    pub row: Option<Value>,
    pub msg: Option<Value>,
}

impl Opt {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Opt {
            value: value.into(),
            label: label.into(),
            data: None,
            row: None,
            msg: None,
        }
    }
    /// An option with a fixed label: the message's text, with its id.
    pub fn fixed(value: impl Into<String>, id: &str, params: Value) -> Self {
        let m = crate::view::msg(id, params);
        let mut o = Opt::new(value, m["text"].as_str().unwrap_or_default());
        o.msg = Some(m);
        o
    }
    pub fn data(mut self, d: Value) -> Self {
        self.data = Some(d);
        self
    }
    /// `{"cells": …, "badges"?, "tone"?, "detail"?}`.
    pub fn row(mut self, r: Value) -> Self {
        self.row = Some(r);
        self
    }
}

fn options_json(opts: &[Opt]) -> Value {
    Value::Array(
        opts.iter()
            .enumerate()
            .map(|(i, o)| {
                let mut v = json!({ "index": i, "value": o.value, "label": strip_ansi(&o.label) });
                if let Some(m) = &o.msg {
                    v["msg"] = m["msg"].clone();
                    if let Some(p) = m.get("params") {
                        v["params"] = p.clone();
                    }
                }
                if let Some(r) = &o.row {
                    v["view"] = r.clone();
                }
                if let Some(d) = &o.data {
                    v["data"] = d.clone();
                }
                v
            })
            .collect(),
    )
}

/// Emit a prompt and read answers until one is valid. `None` = the answer was `null` (skip).
fn ask<T>(kind: &str, fields: Value, parse: impl Fn(&Value) -> Result<T>) -> Result<Option<T>> {
    ask_or(kind, fields, None, parse)
}

/// [`ask`], where a `null` answer means `null_is` (checked like any answer) when given.
fn ask_or<T>(
    kind: &str,
    mut fields: Value,
    null_is: Option<Value>,
    parse: impl Fn(&Value) -> Result<T>,
) -> Result<Option<T>> {
    let id = format!("p{}", NEXT_ID.fetch_add(1, Ordering::SeqCst));
    fields["id"] = json!(id);
    fields["kind"] = json!(kind);
    emit("prompt", fields);
    loop {
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line)? == 0 {
            bail!(Cancelled); // stdin closed
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // `{"id":"p1","answer":…}`, `{"answer":…}`, `{"cancel":true}`, a bare JSON value,
        // or plain text.
        let v: Value = serde_json::from_str(line).unwrap_or_else(|_| json!(line));
        let answer = match &v {
            Value::Object(o) if o.get("cancel") == Some(&json!(true)) => bail!(Cancelled),
            Value::Object(o) if o.contains_key("answer") => {
                if let Some(other) = o.get("id").and_then(Value::as_str)
                    && other != id
                {
                    emit(
                        "error",
                        json!({ "id": id, "message": format!("answer is for {other}, waiting for {id}") }),
                    );
                    continue;
                }
                o["answer"].clone()
            }
            _ => v,
        };
        let answer = match (&answer, &null_is) {
            (Value::Null, Some(d)) => d.clone(),
            (Value::Null, None) => return Ok(None),
            _ => answer,
        };
        match parse(&answer) {
            Ok(t) => return Ok(Some(t)),
            Err(e) => emit("error", json!({ "id": id, "message": e.to_string() })),
        }
    }
}

/// An option by index (number) or by `value` / label (string).
fn find(opts: &[Opt], a: &Value) -> Result<usize> {
    if let Some(i) = a.as_u64() {
        if (i as usize) < opts.len() {
            return Ok(i as usize);
        }
        bail!("index {i} out of range (0..{})", opts.len());
    }
    if let Some(s) = a.as_str() {
        let s = s.trim();
        if let Some(i) = opts
            .iter()
            .position(|o| o.value == s || strip_ansi(&o.label).trim() == s)
        {
            return Ok(i);
        }
        bail!("no option \"{s}\"");
    }
    bail!("expected an option index or value")
}

/// Pick one. `None` when skipped (Esc, or `null` in JSON mode).
pub fn select(q: Q, opts: &[Opt], page_size: Option<usize>) -> Result<Option<usize>> {
    if json() {
        let mut f = q.fields();
        f["options"] = options_json(opts);
        return ask("select", f, |a| find(opts, a));
    }
    let labels: Vec<String> = opts.iter().map(|o| o.label.clone()).collect();
    let msg = q.text();
    let mut s = inquire::Select::new(&msg, labels);
    if let Some(n) = page_size {
        s = s.with_page_size(n);
    }
    Ok(s.raw_prompt_skippable()?.map(|c| c.index))
}

/// Pick one; skipping counts as cancelling.
pub fn select_one(q: Q, opts: &[Opt]) -> Result<usize> {
    select(q, opts, None)?.ok_or_else(|| Cancelled.into())
}

pub fn multi_select(q: Q, opts: &[Opt], defaults: &[usize]) -> Result<Vec<usize>> {
    if json() {
        let mut f = q.fields();
        f["options"] = options_json(opts);
        f["default"] = json!(defaults);
        let picked = ask("multiselect", f, |a| match a {
            Value::Array(items) => items.iter().map(|x| find(opts, x)).collect(),
            Value::String(s) if s == "all" => Ok((0..opts.len()).collect()),
            Value::String(s) if s == "default" => Ok(defaults.to_vec()),
            Value::String(s) => s
                .split(',')
                .filter(|x| !x.trim().is_empty())
                .map(|x| find(opts, &json!(x)))
                .collect(),
            _ => bail!("expected an array of option indexes or values"),
        })?;
        return Ok(picked.unwrap_or_default());
    }
    let labels: Vec<String> = opts.iter().map(|o| o.label.clone()).collect();
    Ok(inquire::MultiSelect::new(&q.text(), labels)
        .with_default(defaults)
        .raw_prompt()?
        .into_iter()
        .map(|c| c.index)
        .collect())
}

pub fn confirm(q: Q, default: bool) -> Result<bool> {
    if json() {
        let mut f = q.fields();
        f["default"] = json!(default);
        let a = ask("confirm", f, |a| match a {
            Value::Bool(b) => Ok(*b),
            Value::String(s) => match s.to_ascii_lowercase().as_str() {
                "y" | "yes" | "true" => Ok(true),
                "n" | "no" | "false" => Ok(false),
                _ => bail!("expected true or false"),
            },
            _ => bail!("expected true or false"),
        })?;
        return Ok(a.unwrap_or(default));
    }
    Ok(inquire::Confirm::new(&q.text())
        .with_default(default)
        .prompt()?)
}

/// A text question with an answer check: `check` says what's wrong (or `None`). In the
/// terminal a bad answer is explained and asked again; in JSON mode it is an `error` with the
/// prompt's id, and the prompt stays open.
pub fn text_checked(q: Q, default: &str, check: impl Fn(&str) -> Option<String>) -> Result<String> {
    if json() {
        let mut f = q.fields();
        f["default"] = json!(default);
        let a = ask_or("text", f, Some(json!(default)), |a| {
            let s = match a {
                Value::String(s) => s.trim().to_string(),
                Value::Number(n) => n.to_string(),
                _ => bail!("expected a string"),
            };
            match check(&s) {
                None => Ok(s),
                Some(problem) => bail!("{problem}"),
            }
        })?;
        return Ok(a.unwrap_or_default());
    }
    let msg = q.text();
    loop {
        let s = inquire::Text::new(&msg)
            .with_default(default)
            .prompt()?
            .trim()
            .to_string();
        match check(&s) {
            None => return Ok(s),
            Some(problem) => warn(&problem),
        }
    }
}

pub fn text(q: Q, default: Option<&str>) -> Result<String> {
    if json() {
        let mut f = q.fields();
        f["default"] = json!(default);
        let a = ask("text", f, |a| match a {
            Value::String(s) => Ok(s.clone()),
            Value::Number(n) => Ok(n.to_string()),
            _ => bail!("expected a string"),
        })?;
        return Ok(a.or(default.map(String::from)).unwrap_or_default());
    }
    let msg = q.text();
    let mut t = inquire::Text::new(&msg);
    if let Some(d) = default {
        t = t.with_default(d);
    }
    Ok(t.prompt()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi() {
        assert_eq!(strip_ansi("\x1b[1m== api\x1b[0m ok"), "== api ok");
        assert_eq!(strip_ansi("\x1b[38;5;12mx\x1b[K"), "x");
    }

    #[test]
    fn finds_options() {
        let o = [Opt::new("api", "api   node"), Opt::new("web", "web   node")];
        assert_eq!(find(&o, &json!(1)).unwrap(), 1);
        assert_eq!(find(&o, &json!("web")).unwrap(), 1);
        assert_eq!(find(&o, &json!("api   node")).unwrap(), 0);
        assert!(find(&o, &json!(5)).is_err());
        assert!(find(&o, &json!("x")).is_err());
    }
}
