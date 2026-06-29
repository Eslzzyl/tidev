use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

/// Registry of active child process PIDs spawned by the bash tool.
/// Used during program exit to prevent orphaned processes.
static ACTIVE_CHILDREN: LazyLock<Mutex<HashSet<u32>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Register a child PID so it can be killed on program exit.
pub fn register_child(pid: u32) {
    ACTIVE_CHILDREN.lock().unwrap().insert(pid);
}

/// Unregister a child PID that has exited normally.
pub fn unregister_child(pid: u32) {
    ACTIVE_CHILDREN.lock().unwrap().remove(&pid);
}

/// Kill all tracked child processes. Two-phase: SIGTERM → brief wait → SIGKILL.
#[cfg(unix)]
pub fn kill_all_children() {
    let pids: Vec<u32> = ACTIVE_CHILDREN.lock().unwrap().iter().copied().collect();
    if pids.is_empty() {
        return;
    }

    // Phase 1: SIGTERM — graceful shutdown
    for &pid in &pids {
        let _ = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    }

    // Give them a moment to exit cleanly
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Phase 2: SIGKILL — force kill survivors
    for &pid in &pids {
        let _ = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    }
}

/// Kill all tracked child processes (no-op on non-Unix).
#[cfg(not(unix))]
pub fn kill_all_children() {
    // Windows support could be added later using TerminateProcess
}

/// Kill a process group by its leader PID.
///
/// Uses two-phase termination (SIGTERM → brief wait → SIGKILL) to give
/// the process and its descendants a chance to clean up (e.g. restore
/// terminal settings) before being forcefully killed.
///
/// After `setsid()` in pre_exec, the child's PID equals its PGID
/// (process group ID), so `kill(-pid, ...)` sends the signal to the
/// entire process group — including any grandchildren (git, editor, pager).
#[cfg(unix)]
pub fn kill_process_group(pid: u32) {
    unsafe {
        let _ = libc::kill(-(pid as i32), libc::SIGTERM);
    }
    // Give them a moment to exit cleanly (restore terminal, etc.)
    std::thread::sleep(std::time::Duration::from_millis(200));
    unsafe {
        let _ = libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

/// Kill a process group by its leader PID (no-op on non-Unix).
#[cfg(not(unix))]
pub fn kill_process_group(pid: u32) {
    // Fallback: just kill the individual process on non-Unix platforms.
    // This won't kill grandchildren, but it's the best we can do without
    // process group support.
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .output();
}

/// Truncate a string in place at a UTF-8 safe boundary, appending a
/// `[truncated]` marker if the string exceeds `max_bytes`.
pub fn truncate_in_place(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }

    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }

    value.truncate(end);
    value.push_str("\n[truncated]");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_in_place_short() {
        let mut s = String::from("hello");
        truncate_in_place(&mut s, 100);
        assert_eq!(s, "hello");
    }

    #[test]
    fn test_truncate_in_place_exact() {
        let mut s = String::from("hello");
        truncate_in_place(&mut s, 5);
        assert_eq!(s, "hello");
    }

    #[test]
    fn test_truncate_in_place_long() {
        let mut s = String::from("hello world");
        truncate_in_place(&mut s, 5);
        assert_eq!(s, "hello\n[truncated]");
    }

    #[test]
    fn test_truncate_in_place_utf8_boundary() {
        // "hello" is 5 bytes, "hello" + "中" is 8 bytes
        let mut s = String::from("hello中");
        truncate_in_place(&mut s, 6);
        // "hello" (5 bytes) + "中" starts at byte 5, byte 6 is mid-char
        // Should truncate to "hello" (5 bytes, which is a char boundary)
        assert_eq!(s, "hello\n[truncated]");
    }
}
