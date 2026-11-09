//! Child processes: run-with-timeout for git/gh, and detached self-spawns for
//! background workers.

use std::{
    env,
    ffi::OsStr,
    fs,
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};

/// stdout of a finished, successful command, or None — git/gh may be absent,
/// unauthed, or hung (killed at the timeout).
pub fn output(cmd: &mut Command, timeout: Duration) -> Option<String> {
    let child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let pid = child.id().cast_signed();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || tx.send(child.wait_with_output()));
    match rx.recv_timeout(timeout) {
        Ok(Ok(out)) if out.status.success() => String::from_utf8(out.stdout).ok().map(|s| s.trim().to_owned()),
        Ok(_) => None,
        Err(_) => {
            // SAFETY: not yet reaped, so the pid is still our child's.
            unsafe { libc::kill(pid, libc::SIGKILL) };
            None
        }
    }
}

pub fn self_path() -> PathBuf {
    env::current_exe().and_then(fs::canonicalize).unwrap_or_else(|_| "statusline".into())
}

/// Run this binary again in its own session, fully detached from the render.
pub fn spawn_detached(args: impl IntoIterator<Item = impl AsRef<OsStr>>) {
    let mut cmd = Command::new(self_path());
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let _ = cmd.spawn();
}
