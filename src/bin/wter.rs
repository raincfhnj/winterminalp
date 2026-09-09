//! Short command-name entry point for WinTerminal++.
//!
//! Shares the exact CLI implementation with `winterminal.exe`; only the
//! displayed binary name differs.

use std::process::ExitCode;

fn main() -> ExitCode {
    winterminal::cli::run(env!("CARGO_BIN_NAME"))
}
