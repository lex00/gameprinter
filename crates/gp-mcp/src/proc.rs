//! Processes the same way on macOS, Linux and Windows: whether one is
//! running, starting one in a group of its own, and stopping that group
//! with what it started. std has no killpg and these crates no libc, so on
//! unix it goes through kill(1); on Windows through tasklist and taskkill,
//! which are part of Windows.

use std::process::{Command, Stdio};
use std::time::Duration;

/// Whether process `pid` is running.
pub fn alive(pid: u64) -> bool {
    if pid == u64::from(std::process::id()) {
        return true;
    }
    #[cfg(unix)]
    {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(windows)]
    {
        // One CSV line per match, "gp.exe","1234",...; none prints "INFO: ...".
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .stderr(Stdio::null())
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")))
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

/// Starts `cmd` in a process group of its own, so [`stop_group`] can stop
/// it and every process it starts. On Windows a new process group keeps
/// the console's Ctrl+C from reaching it, as on unix.
pub fn own_group(cmd: &mut Command) -> &mut Command {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(cmd, 0);
    #[cfg(windows)]
    {
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        std::os::windows::process::CommandExt::creation_flags(cmd, CREATE_NEW_PROCESS_GROUP);
    }
    cmd
}

/// Asks the group `pid` leads (started with [`own_group`]) to stop, then
/// after `grace` stops what is left of it. On Windows the whole tree under
/// `pid` is ended at once (taskkill /T /F): Windows has no polite signal a
/// console program without a window reliably takes.
pub fn stop_group(pid: u32, grace: Duration) {
    #[cfg(unix)]
    for (i, signal) in ["-TERM", "-KILL"].iter().enumerate() {
        if i == 1 {
            std::thread::sleep(grace);
        }
        let _ = Command::new("kill")
            .args([signal, "--"])
            .arg(format!("-{pid}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = grace;
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(any(unix, windows)))]
    let _ = (pid, grace);
}

/// Whether `path` is a file this system would run: an execute bit on unix,
/// on Windows a file at all (Windows runs by extension, not by mode).
pub fn is_executable(path: &std::path::Path) -> bool {
    let Ok(m) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        m.is_file() && m.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        m.is_file()
    }
}

/// Marks `path` executable where that is a file's mode (unix, 0755); a
/// no-op on Windows.
pub fn make_executable(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_alive_and_a_finished_one_is_not() {
        assert!(alive(u64::from(std::process::id())));
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!alive(u64::from(pid)));
    }

    #[cfg(unix)]
    #[test]
    fn stop_group_ends_the_group_and_its_children() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30 & sleep 30"]);
        let mut child = own_group(&mut cmd).spawn().unwrap();
        stop_group(child.id(), Duration::from_millis(100));
        let st = child.wait().unwrap();
        assert!(!st.success());
    }

    #[test]
    fn executables() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("tool");
        std::fs::write(&f, "#!/bin/sh\n").unwrap();
        assert_eq!(is_executable(&f), cfg!(not(unix)));
        make_executable(&f).unwrap();
        assert!(is_executable(&f));
        assert!(!is_executable(&dir.path().join("none")));
    }
}
