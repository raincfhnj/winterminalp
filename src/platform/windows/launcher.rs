use std::process::{Child, Command};

use super::error::{PlatformError, PlatformResult};

/// Launches Windows Terminal in a new window through its app execution alias.
///
/// This adapter intentionally exposes no generic `wt.exe` argument list and
/// never uses `wt.exe` to route pane actions. It passes a fixed `-w new`
/// selector directly to `CreateProcess` through `std::process::Command`.
pub fn launch_windows_terminal() -> PlatformResult<Child> {
    Command::new("wt.exe")
        .args(["-w", "new"])
        .spawn()
        .map_err(|source| PlatformError::TerminalLaunch { source })
}
