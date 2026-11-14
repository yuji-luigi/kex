//! Small helpers shared by every command: terminal output, paths, time.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// True when kex may ask questions: both stdin and stdout are a terminal, or `--json` (the
/// questions then go out as `prompt` events).
pub fn interactive() -> bool {
    crate::ui::json() || (std::io::stdin().is_terminal() && std::io::stdout().is_terminal())
}

fn color() -> bool {
    static COLOR: OnceLock<bool> = OnceLock::new();
    *COLOR.get_or_init(|| {
        !crate::ui::json()
            && std::env::var_os("NO_COLOR").is_none()
            && std::io::stdout().is_terminal()
    })
}

pub fn paint(code: &str, s: impl AsRef<str>) -> String {
    if color() {
        format!("\x1b[{code}m{}\x1b[0m", s.as_ref())
    } else {
        s.as_ref().to_string()
    }
}

pub fn bold(s: impl AsRef<str>) -> String {
    paint("1", s)
}
pub fn dim(s: impl AsRef<str>) -> String {
    paint("2", s)
}
pub fn red(s: impl AsRef<str>) -> String {
    paint("31", s)
}
pub fn green(s: impl AsRef<str>) -> String {
    paint("32", s)
}
pub fn yellow(s: impl AsRef<str>) -> String {
    paint("33", s)
}

pub fn canon(p: &Path) -> PathBuf {
    // dunce avoids Windows' `\\?\C:\…` form, which git and people don't expect.
    dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// `path` with the home folder shown as `~`.
pub fn tilde(p: &Path) -> String {
    match home().and_then(|h| p.strip_prefix(h).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => p.display().to_string(),
    }
}

/// Find an executable on PATH.
pub fn which(name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).find_map(|dir| {
        exts.iter()
            .map(|e| dir.join(format!("{name}{e}")))
            .find(|p| p.is_file())
    })
}

/// Glob with `*` only, e.g. `release/*`.
pub fn glob_match(pat: &str, s: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == s;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    let Some(mut rest) = s.strip_prefix(first) else {
        return false;
    };
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

/// Run `f` on every item at once, one thread each, keeping the order.
pub fn parallel<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let f = &f;
    std::thread::scope(|s| {
        let handles: Vec<_> = items.iter().map(|i| s.spawn(move || f(i))).collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("worker thread panicked"))
            .collect()
    })
}

/// Stable content hash (FNV-1a), used to notice lockfile changes.
pub fn fnv1a(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Longest repo name, and longest project name (before kex adds `-xxxxxx`).
pub const NAME_MAX: usize = 40;
/// Longest `description`.
pub const DESCRIPTION_MAX: usize = 200;

/// `My Shop_API` → `my-shop-api`, at most [`NAME_MAX`] characters.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(NAME_MAX).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "repo".into() } else { out }
}

/// Seconds since the epoch for `2026-11-10T09:03:00Z` (UTC only, as `gh` prints it).
pub fn unix_from_rfc3339(s: &str) -> Option<i64> {
    let b = s.get(0..19)?;
    let num = |r: std::ops::Range<usize>| b.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    // Days from civil date (Howard Hinnant's algorithm).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3600 + mm * 60 + ss)
}

/// `1794441600` → `2026-11-12T00:00:00Z` (UTC).
pub fn rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let t = secs.rem_euclid(86_400);
    // Civil date from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        t / 3600,
        t / 60 % 60,
        t % 60
    )
}

/// `a/./b/../c` → `a/c`, without touching the filesystem (for paths that don't exist).
pub fn normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// A repo name kex accepts (the `repo` pattern in schema/kex.schema.json): lowercase
/// letters, digits and `-`, starting with a letter or digit.
pub fn valid_repo_name(n: &str) -> bool {
    let mut cs = n.chars();
    cs.next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && cs.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Why `n` isn't a repo name kex accepts, or `None`.
pub fn repo_name_problem(n: &str) -> Option<String> {
    if n.is_empty() {
        Some("the repo name can't be empty".into())
    } else if n.chars().count() > NAME_MAX {
        Some(format!(
            "the repo name can be at most {NAME_MAX} characters"
        ))
    } else if !valid_repo_name(n) {
        Some(format!(
            "\"{n}\" isn't a repo name: use lowercase letters, digits and -, starting with a letter or digit"
        ))
    } else {
        None
    }
}

/// Why `d` can't be a description: too long, or not one line of text.
pub fn description_problem(d: &str) -> Option<String> {
    if d.chars().count() > DESCRIPTION_MAX {
        Some(format!(
            "the description can be at most {DESCRIPTION_MAX} characters"
        ))
    } else if d.chars().any(char::is_control) {
        Some("the description must be one line, without control characters".into())
    } else {
        None
    }
}

/// Why `n` can't be a new project's name (kex makes the id from it: `<slug>-xxxxxx`).
pub fn project_name_problem(n: &str) -> Option<String> {
    let n = n.trim();
    if n.is_empty() {
        Some("the project name can't be empty".into())
    } else if n.chars().count() > NAME_MAX {
        Some(format!(
            "the project name can be at most {NAME_MAX} characters"
        ))
    } else if n.chars().any(char::is_control) {
        Some("the project name must be one line".into())
    } else {
        None
    }
}

/// `~/x` → `$HOME/x`; anything else unchanged.
pub fn expand_tilde(s: &str) -> PathBuf {
    match (
        s.strip_prefix("~/")
            .or(if s == "~" { Some("") } else { None }),
        home(),
    ) {
        (Some(rest), Some(h)) => h.join(rest),
        _ => PathBuf::from(s),
    }
}

/// `90` → `1m`, `7200` → `2h`, `172800` → `2d`.
pub fn ago(secs: i64) -> String {
    match secs.max(0) {
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match("release/*", "release/1.2"));
        assert!(glob_match("deploy_*", "deploy_prod"));
        assert!(glob_match("*prod*", "client-a_prod_x"));
        assert!(!glob_match("release/*", "main"));
        assert!(glob_match("main", "main"));
    }

    #[test]
    fn rfc3339() {
        assert_eq!(unix_from_rfc3339("1970-01-02T00:00:00Z"), Some(86_400));
        assert_eq!(
            unix_from_rfc3339("2026-11-12T00:00:00Z"),
            Some(1_794_441_600)
        );
    }

    #[test]
    fn rfc3339_round_trips() {
        for t in [0, 86_400, 1_794_441_600, 951_782_400 + 3_723] {
            assert_eq!(unix_from_rfc3339(&super::rfc3339(t)), Some(t), "{t}");
        }
        assert_eq!(super::rfc3339(1_794_441_600), "2026-11-12T00:00:00Z");
    }

    #[test]
    fn normalizes_paths() {
        assert_eq!(normalize(Path::new("/a/./b/../c")), PathBuf::from("/a/c"));
        assert_eq!(
            normalize(Path::new("/w/shop/../missing")),
            PathBuf::from("/w/missing")
        );
        assert!(valid_repo_name("a-1") && !valid_repo_name("Bad Name,x"));
    }

    #[test]
    fn limits() {
        let long = "a".repeat(41);
        assert_eq!(slug(&long).len(), 40);
        assert_eq!(slug(&format!("{}-b", "a".repeat(39))), "a".repeat(39));
        assert!(repo_name_problem(&long).unwrap().contains("40"));
        assert!(repo_name_problem(&"a".repeat(40)).is_none());
        assert!(description_problem(&"x".repeat(201)).is_some());
        assert!(description_problem("two\nlines").is_some());
        assert!(description_problem("Orders and payments API").is_none());
        assert!(project_name_problem(&long).is_some() && project_name_problem("My Shop").is_none());
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("My Shop_API"), "my-shop-api");
        assert_eq!(slug("--"), "repo");
    }
}
