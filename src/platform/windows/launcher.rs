use std::process::{Child, Command};

use super::error::{PlatformError, PlatformResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalLaunchTarget {
    NewWindow,
    NamedWindow(String),
}

/// Launches Windows Terminal through its app execution alias.
///
/// This adapter intentionally exposes no generic `wt.exe` argument list and
/// never uses `wt.exe` to route pane actions. It passes a fixed `-w` launch
/// selector directly to `CreateProcess` through `std::process::Command`.
pub fn launch_windows_terminal(target: &TerminalLaunchTarget) -> PlatformResult<Child> {
    let window_selector = match target {
        TerminalLaunchTarget::NewWindow => "new",
        TerminalLaunchTarget::NamedWindow(name) => {
            validate_window_name(name)?;
            name
        }
    };

    Command::new("wt.exe")
        .args(["-w", window_selector])
        .spawn()
        .map_err(|source| PlatformError::TerminalLaunch { source })
}

fn validate_window_name(name: &str) -> PlatformResult<()> {
    let is_valid = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        && !matches!(
            name.to_ascii_lowercase().as_str(),
            "new" | "last" | "0" | "-1" | "_quake"
        );

    if is_valid {
        Ok(())
    } else {
        Err(PlatformError::InvalidWindowName(name.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_namespaced_managed_window_name() {
        assert!(validate_window_name("winterminalpp.controller-1").is_ok());
    }

    #[test]
    fn rejects_reserved_or_parser_sensitive_window_names() {
        for name in ["", "new", "last", "0", "-1", "_quake", "bad;split-pane"] {
            assert!(
                matches!(
                    validate_window_name(name),
                    Err(PlatformError::InvalidWindowName(_))
                ),
                "{name:?} should be rejected"
            );
        }
    }
}
