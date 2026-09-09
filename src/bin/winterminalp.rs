use std::process::ExitCode;

fn main() -> ExitCode {
    winterminalp::cli::run(env!("CARGO_BIN_NAME"))
}
