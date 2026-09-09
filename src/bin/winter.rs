//! Short command-name entry point for WinTerminalP.
//!
//! Shares the exact CLI implementation with `winterminalp.exe`; only the
//! displayed binary name differs.

use std::process::ExitCode;

fn main() -> ExitCode {
    winterminalp::cli::run(env!("CARGO_BIN_NAME"))
}
