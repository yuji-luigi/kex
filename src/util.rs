//! Small helpers shared by every command: terminal output, paths, time.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// True when kex may ask questions (both stdin and stdout are a terminal).
pub fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn color() -> bool {
    static COLOR: OnceLock<bool> = OnceLock::new();
    *COLOR.get_or_init(|| std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal())
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

/// `My Shop_API` → `my-shop-api`.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
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
    fn slugs() {
        assert_eq!(slug("My Shop_API"), "my-shop-api");
        assert_eq!(slug("--"), "repo");
    }
}
