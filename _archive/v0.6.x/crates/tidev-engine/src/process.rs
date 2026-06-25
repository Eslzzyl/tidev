//! Process lifecycle utilities, including self-restart support.
//!
//! The primary function [`restart_self`] replaces the current process with
//! a fresh instance of the same binary using the original CLI arguments.
//! On Unix this is a true exec(3)-style replacement (PID stays the same);
//! on Windows the old process exits after spawning the new one.

use log;

/// Replace the current process with a fresh instance of the same binary.
///
/// All CLI arguments passed to the original invocation are preserved so the
/// new process starts in exactly the same mode (e.g. `tidev web --port 8080`).
///
/// ## Unix
/// Uses `execvp` via [`std::os::unix::process::CommandExt::exec`].
/// The process image is replaced immediately — this function never returns.
///
/// ## Windows
/// Spawns a new process via `std::process::Command` and then calls
/// `std::process::exit(0)`. The new process will have a different PID.
///
/// ## Panics
/// Panics if the current executable path cannot be determined or if the
/// exec/spawn fails.
pub fn restart_self() -> ! {
    let exe = std::env::current_exe().expect("cannot determine current executable path");
    let args: Vec<String> = std::env::args().collect();

    log::info!("Restarting: {} {}", exe.display(), args[1..].join(" "));

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(&exe).args(&args[1..]).exec();
        panic!("restart_self: exec failed: {err}");
    }

    #[cfg(not(unix))]
    {
        match std::process::Command::new(&exe).args(&args[1..]).spawn() {
            Ok(_) => std::process::exit(0),
            Err(e) => panic!("restart_self: spawn failed: {e}"),
        }
    }
}
