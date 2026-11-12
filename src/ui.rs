//! Terminal output and questions, or the same as a JSON stream for other programs
//! (`--json`, BLUEPRINT §5.9).
//!
//! In JSON mode stdout carries one JSON object per line and nothing else. Questions become
//! `prompt` events; the caller answers with one line on stdin.

use std::io::{BufRead, Write};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use anyhow::{Result, bail};
use serde_json::{Value, json};

pub const PROTOCOL: u32 = 1;

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

/// Write one event line. `kind` becomes the `type` field.
pub fn emit(kind: &str, fields: Value) {
    let mut v = json!({ "type": kind });
    if let (Some(o), Value::Object(f)) = (v.as_object_mut(), fields) {
        o.extend(f);
    }
    let mut out = std::io::stdout().lock();
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

/// One choice in a list. `value` is a stable id a program can answer with; `data` carries
/// the structured details behind the label.
pub struct Opt {
    pub value: String,
    pub label: String,
    pub data: Option<Value>,
}

impl Opt {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Opt {
            value: value.into(),
            label: label.into(),
            data: None,
        }
    }
    pub fn data(mut self, d: Value) -> Self {
        self.data = Some(d);
        self
    }
}

fn options_json(opts: &[Opt]) -> Value {
    Value::Array(
        opts.iter()
            .enumerate()
            .map(|(i, o)| {
                let mut v = json!({ "index": i, "value": o.value, "label": strip_ansi(&o.label) });
                if let Some(d) = &o.data {
                    v["data"] = d.clone();
                }
                v
            })
            .collect(),
    )
}

/// Emit a prompt and read answers until one is valid. `None` = the answer was `null` (skip).
fn ask<T>(kind: &str, mut fields: Value, parse: impl Fn(&Value) -> Result<T>) -> Result<Option<T>> {
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
        if answer.is_null() {
            return Ok(None);
        }
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
pub fn select(msg: &str, opts: &[Opt], page_size: Option<usize>) -> Result<Option<usize>> {
    if json() {
        let f = json!({ "message": msg, "options": options_json(opts) });
        return ask("select", f, |a| find(opts, a));
    }
    let labels: Vec<String> = opts.iter().map(|o| o.label.clone()).collect();
    let mut s = inquire::Select::new(msg, labels);
    if let Some(n) = page_size {
        s = s.with_page_size(n);
    }
    Ok(s.raw_prompt_skippable()?.map(|c| c.index))
}

/// Pick one; skipping counts as cancelling.
pub fn select_one(msg: &str, opts: &[Opt]) -> Result<usize> {
    select(msg, opts, None)?.ok_or_else(|| Cancelled.into())
}

pub fn multi_select(msg: &str, opts: &[Opt], defaults: &[usize]) -> Result<Vec<usize>> {
    if json() {
        let f = json!({ "message": msg, "options": options_json(opts), "default": defaults });
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
    Ok(inquire::MultiSelect::new(msg, labels)
        .with_default(defaults)
        .raw_prompt()?
        .into_iter()
        .map(|c| c.index)
        .collect())
}

pub fn confirm(msg: &str, default: bool) -> Result<bool> {
    if json() {
        let f = json!({ "message": msg, "default": default });
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
    Ok(inquire::Confirm::new(msg).with_default(default).prompt()?)
}

pub fn text(msg: &str, default: Option<&str>) -> Result<String> {
    if json() {
        let f = json!({ "message": msg, "default": default });
        let a = ask("text", f, |a| match a {
            Value::String(s) => Ok(s.clone()),
            Value::Number(n) => Ok(n.to_string()),
            _ => bail!("expected a string"),
        })?;
        return Ok(a.or(default.map(String::from)).unwrap_or_default());
    }
    let mut t = inquire::Text::new(msg);
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
