//! Respawn the editor binary from within a running instance.
//!
//! Used to close the "runtime-loaded game can't register systems"
//! gap: after a game is scaffolded + built + installed, we need the
//! user's next session to pick it up at startup via
//! `DylibLoaderPlugin::build`, which is the only place we have
//! `&mut App` to hand the game's plugin. A full process restart is
//! the simplest way to get there.
//!
//! The respawn inherits the full env (so `LD_LIBRARY_PATH` etc.
//! survive). The current process exits with code 0 immediately after
//! the child spawns — the user sees their window close and a fresh
//! one appear. Because jackdaw persists "recent projects" via
//! [`crate::project`], the new instance reopens the same project.
//!
//! Callers should flush any pending work (save-on-exit, close
//! pipelines, etc.) before invoking this — once the child is
//! spawned we don't come back.

use std::path::PathBuf;
use std::process::Command;

use bevy::log::{info, warn};

/// Env var the parent process sets before respawning, signalling
/// to the child "the game you're about to load was just rebuilt
/// and installed — skip the initial-build step in the launcher and
/// go straight to the editor." Prevents the scaffold → build →
/// restart → auto-open → build → restart infinite loop.
pub const ENV_SKIP_INITIAL_BUILD: &str = "JACKDAW_SKIP_INITIAL_BUILD";

/// Spawn a fresh copy of the running binary with the same command-
/// line arguments and environment, then exit the current process.
/// Never returns.
pub fn restart_jackdaw() -> ! {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            warn!("restart_jackdaw: current_exe failed ({e}); exiting without respawn");
            std::process::exit(1);
        }
    };
    let args: Vec<_> = std::env::args_os().skip(1).collect();

    info!(
        "Respawning jackdaw as {} with {} arg(s)",
        exe.display(),
        args.len()
    );

    let child = Command::new(&exe)
        .args(&args)
        .env(ENV_SKIP_INITIAL_BUILD, "1")
        .spawn();
    match child {
        Ok(_) => {
            // Best-effort flush. We can't hold on for graceful bevy
            // shutdown because the new process is already coming up
            // and will compete for the window/input device.
            std::process::exit(0);
        }
        Err(e) => {
            warn!(
                "restart_jackdaw: failed to spawn {} ({e}); staying in current process",
                exe.display()
            );
            // The caller typically can't recover from this, but we
            // leave the decision to them by returning from `exit` —
            // which this function never does, so log and exit(1).
            std::process::exit(1);
        }
    }
}

/// Attempt to verify we *can* restart (binary path is discoverable)
/// before committing to flushing state. Does not spawn anything.
pub fn can_restart() -> Option<PathBuf> {
    std::env::current_exe().ok()
}
