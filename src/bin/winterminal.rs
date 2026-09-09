use std::process::ExitCode;

fn main() -> ExitCode {
    winterminal::cli::run(env!("CARGO_BIN_NAME"))
}
