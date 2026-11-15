//! SIGINT / SIGTERM (Ctrl-C / Ctrl-Break on Windows) in JSON mode, for every command
//! (cli-events/1 §3: the tool cleans up and ends with `exit` tone `cancelled`).
//!
//! One process-wide handler. Outside a [`Cooperative`] section it ends the run at once: the
//! `exit` line (130, cancelled), the child processes kex started (tracked by [`output`] /
//! [`status`] / [`track`]) are killed, and the process exits 130. Inside one (the setup
//! writes, `kex run`'s supervisor) it only sets [`cancelled`]: that code undoes or stops what
//! it was doing and returns 130 itself.

use std::collections::HashSet;
use std::io;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

static CANCELLED: AtomicBool = AtomicBool::new(false);
static COOPERATIVE: AtomicUsize = AtomicUsize::new(0);

fn children() -> &'static Mutex<HashSet<u32>> {
    static C: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
    C.get_or_init(Mutex::default)
}

/// Install the handler (JSON mode). In the terminal, Ctrl-C reaches kex and its children
/// together, as before.
pub fn install() {
    let _ = ctrlc::set_handler(|| {
        CANCELLED.store(true, Ordering::SeqCst);
        if COOPERATIVE.load(Ordering::SeqCst) == 0 {
            finish();
        }
    });
}

/// A signal came.
pub fn cancelled() -> bool {
    CANCELLED.load(Ordering::SeqCst)
}

/// While alive, a signal only sets [`cancelled`]; the code inside handles it.
pub struct Cooperative;

impl Cooperative {
    pub fn enter() -> Self {
        COOPERATIVE.fetch_add(1, Ordering::SeqCst);
        Cooperative
    }
}

impl Drop for Cooperative {
    fn drop(&mut self) {
        COOPERATIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

/// End the run now: `exit` 130 (later lines are dropped), kill the children, exit.
fn finish() {
    crate::ui::event(
        "exit",
        serde_json::json!({ "code": 130, "tone": crate::manifest::exit_tone(130) }),
    );
    let pids: Vec<u32> = children()
        .lock()
        .map(|c| c.iter().copied().collect())
        .unwrap_or_default();
    for pid in pids {
        kill(pid);
    }
    std::process::exit(130);
}

fn kill(pid: u32) {
    #[cfg(unix)]
    // SAFETY: kill(2) on a child's pid.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Remember a child kex started, so a cancel can stop it.
pub fn track(child: &Child) {
    if let Ok(mut c) = children().lock() {
        c.insert(child.id());
    }
}

pub fn untrack(child: &Child) {
    if let Ok(mut c) = children().lock() {
        c.remove(&child.id());
    }
}

/// `cmd.output()`, tracked.
pub fn output(cmd: &mut Command) -> io::Result<Output> {
    // As `Command::output`: no stdin.
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    track(&child);
    let id = child.id();
    let out = child.wait_with_output();
    if let Ok(mut c) = children().lock() {
        c.remove(&id);
    }
    out
}

/// `cmd.status()`, tracked.
pub fn status(cmd: &mut Command) -> io::Result<std::process::ExitStatus> {
    let mut child = cmd.spawn()?;
    track(&child);
    let st = child.wait();
    untrack(&child);
    st
}
