//! Shared command-line implementation for `wter.exe` and `winterminal.exe`.

use std::env;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use serde::Serialize;

use crate::integration::{IntegrationConfig, doctor, install, plan, uninstall};
use crate::platform::windows::relaunch_current_process_elevated;
use crate::{
    AppError, AppResult, ControllerConfig, ControllerOptions, bridge_is_ready,
    config::default_config_path, run_controller,
};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "tmux-style keyboard control for the native Windows Terminal"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Show, locate, or edit the user shortcut configuration.
    Config {
        /// Print only the configuration file path.
        #[arg(long = "path", conflicts_with = "edit")]
        path_only: bool,
        /// Open the configuration file in Notepad.
        #[arg(long, conflicts_with = "path_only")]
        edit: bool,
    },
    /// Show the settings changes that install would make.
    Plan,
    /// Install the managed action fragment and hidden bridge keybindings.
    Install,
    /// Remove only integration entries still owned by WinTerminal++.
    Uninstall,
    /// Diagnose the current Windows Terminal integration.
    Doctor,
    /// Run the keyboard controller in this process.
    Run {
        /// Do not open a new native Windows Terminal window.
        #[arg(long)]
        no_launch: bool,
    },
    /// Start a background controller and open native Windows Terminal.
    Launch,
}

/// Parses `args` using `bin_name` for usage text and runs the selected command.
pub fn run(bin_name: &'static str) -> ExitCode {
    let matches = Cli::command().name(bin_name).get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };
    match execute(cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute(cli: Cli) -> AppResult<ExitCode> {
    let command = cli.command.unwrap_or(CliCommand::Launch);
    if let CliCommand::Config { path_only, edit } = command {
        return configure(path_only, edit);
    }

    let integration = IntegrationConfig::from_environment()?;
    match command {
        CliCommand::Plan => {
            let report = plan(&integration)?;
            print_json(&report)?;
            Ok(if report.can_install {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            })
        }
        CliCommand::Install => {
            let report = install(&integration)?;
            print_json(&report)?;
            Ok(ExitCode::SUCCESS)
        }
        CliCommand::Uninstall => {
            let report = uninstall(&integration)?;
            print_json(&report)?;
            Ok(ExitCode::SUCCESS)
        }
        CliCommand::Doctor => {
            let path = default_config_path()?;
            if path.exists() {
                let _config = ControllerConfig::load(&path)?;
            }
            let report = doctor(&integration)?;
            print_json(&report)?;
            Ok(if bridge_is_ready(&report) {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            })
        }
        CliCommand::Run { no_launch } => run_command(&integration, no_launch),
        CliCommand::Launch => launch(&integration),
        CliCommand::Config { .. } => unreachable!("config is handled before integration is loaded"),
    }
}

fn run_command(integration: &IntegrationConfig, no_launch: bool) -> AppResult<ExitCode> {
    let arguments = if no_launch {
        ["run", "--no-launch"].as_slice()
    } else {
        ["run"].as_slice()
    };
    if relaunch_controller_elevated(arguments)? {
        return Ok(ExitCode::SUCCESS);
    }
    let report = doctor(integration)?;
    let bridge_ready = bridge_is_ready(&report);
    if !bridge_ready {
        return Err(AppError::InvalidConfiguration(
            "Windows Terminal action bridge is not ready; run `wter doctor` and `wter install` first"
                .to_owned(),
        ));
    }
    let config = ControllerConfig::load_or_create(&default_config_path()?)?;
    let controller_report = run_controller(
        &config,
        ControllerOptions {
            launch_terminal: config.launch_terminal_on_start && !no_launch,
            bridge_ready,
            ..ControllerOptions::default()
        },
    )?;
    print_json(&controller_report)?;
    Ok(ExitCode::SUCCESS)
}

fn configure(path_only: bool, edit: bool) -> AppResult<ExitCode> {
    let path = default_config_path()?;
    if path_only {
        println!("{}", path.display());
    } else if edit {
        if !path.exists() {
            let _config = ControllerConfig::load_or_create(&path)?;
        }
        let _editor = Command::new("notepad.exe")
            .arg(&path)
            .spawn()
            .map_err(|error| AppError::io("open controller config editor", &path, error))?;
        println!(
            "Opened {}. Restart the controller after saving.",
            path.display()
        );
    } else {
        let config = ControllerConfig::load_or_create(&path)?;
        println!("# path = {}", path.display());
        print!("{}", config.to_pretty_toml()?);
    }
    Ok(ExitCode::SUCCESS)
}

fn launch(integration: &IntegrationConfig) -> AppResult<ExitCode> {
    if relaunch_controller_elevated(&["launch"])? {
        return Ok(ExitCode::SUCCESS);
    }
    let _config = ControllerConfig::load_or_create(&default_config_path()?)?;
    let report = doctor(integration)?;
    if !bridge_is_ready(&report) {
        return Err(AppError::InvalidConfiguration(
            "Windows Terminal integration is not ready; run `wter doctor` and `wter install` first"
                .to_owned(),
        ));
    }

    spawn_background_controller()?;
    println!("WinTerminal++ elevated controller is starting; Windows Terminal will open elevated.");
    Ok(ExitCode::SUCCESS)
}

fn spawn_background_controller() -> AppResult<()> {
    let current_exe = env::current_exe()
        .map_err(|error| AppError::io("resolve current executable", PathBuf::from("."), error))?;
    let sibling_daemon = current_exe.with_file_name("winterminald.exe");
    let (program, arguments) = if sibling_daemon.is_file() {
        (sibling_daemon, vec!["--launch"])
    } else {
        (current_exe, vec!["run", "--no-launch"])
    };

    let mut command = Command::new(&program);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let _controller = command
        .spawn()
        .map_err(|error| AppError::io("start background controller", program, error))?;
    Ok(())
}

fn relaunch_controller_elevated(arguments: &[&str]) -> AppResult<bool> {
    relaunch_current_process_elevated(arguments)
        .map_err(|error| AppError::Native(error.to_string()))
}

fn print_json(value: &impl Serialize) -> AppResult<()> {
    let json = serde_json::to_string_pretty(value).map_err(|error| {
        AppError::InvalidConfiguration(format!("serialize command report: {error}"))
    })?;
    println!("{json}");
    Ok(())
}
