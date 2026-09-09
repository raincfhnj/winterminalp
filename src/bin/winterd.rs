#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::process::ExitCode;

use winterminalp::integration::{IntegrationConfig, doctor};
use winterminalp::platform::windows::{launch_windows_terminal, relaunch_current_process_elevated};
use winterminalp::{
    AppError, AppResult, ControllerConfig, ControllerOptions, bridge_is_ready,
    config::{default_app_data_dir, default_config_path},
    run_controller,
};

fn main() -> ExitCode {
    match execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            write_last_error(&error);
            ExitCode::FAILURE
        }
    }
}

fn execute() -> AppResult<()> {
    let launch_mode = parse_launch_mode()?;
    let elevation_arguments = launch_mode.elevation_arguments();
    if relaunch_current_process_elevated(elevation_arguments)
        .map_err(|error| AppError::Native(error.to_string()))?
    {
        return Ok(());
    }
    let integration = IntegrationConfig::from_environment()?;
    let doctor_report = doctor(&integration)?;
    if !bridge_is_ready(&doctor_report) {
        return Err(AppError::InvalidConfiguration(
            "Windows Terminal integration is not ready; run `winter doctor` and `winter install` first"
                .to_owned(),
        ));
    }

    let config = ControllerConfig::load_or_create(&default_config_path()?)?;
    let should_launch = launch_mode.should_launch(config.launch_terminal_on_start);
    let result = run_controller(
        &config,
        ControllerOptions {
            launch_terminal: should_launch,
            bridge_ready: true,
            ..ControllerOptions::default()
        },
    );

    match result {
        Err(AppError::ControllerAlreadyRunning) if should_launch => {
            let _child =
                launch_windows_terminal().map_err(|error| AppError::Native(error.to_string()))?;
            Ok(())
        }
        Err(AppError::ControllerAlreadyRunning) => Ok(()),
        result => result.map(|_| ()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DaemonLaunchMode {
    Config,
    Force,
    Suppress,
}

impl DaemonLaunchMode {
    const fn should_launch(self, configured: bool) -> bool {
        match self {
            Self::Config => configured,
            Self::Force => true,
            Self::Suppress => false,
        }
    }

    const fn elevation_arguments(self) -> &'static [&'static str] {
        match self {
            Self::Config => &[],
            Self::Force => &["--launch"],
            Self::Suppress => &["--no-launch"],
        }
    }
}

fn parse_launch_mode() -> AppResult<DaemonLaunchMode> {
    let mut launch_mode = DaemonLaunchMode::Config;
    for argument in env::args_os().skip(1) {
        match argument.to_string_lossy().as_ref() {
            "--launch" if launch_mode == DaemonLaunchMode::Config => {
                launch_mode = DaemonLaunchMode::Force;
            }
            "--no-launch" if launch_mode == DaemonLaunchMode::Config => {
                launch_mode = DaemonLaunchMode::Suppress;
            }
            _ => {
                return Err(AppError::InvalidConfiguration(format!(
                    "unknown or conflicting daemon argument: {}",
                    argument.to_string_lossy()
                )));
            }
        }
    }
    Ok(launch_mode)
}

fn write_last_error(error: &AppError) {
    let Ok(directory) = default_app_data_dir() else {
        return;
    };
    if fs::create_dir_all(&directory).is_err() {
        return;
    }
    let path = directory.join("last-error.log");
    let Ok(mut file) = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
    else {
        return;
    };
    let _ = writeln!(file, "{error}");
    let _ = file.sync_all();
}

#[cfg(test)]
mod tests {
    use super::DaemonLaunchMode;

    #[test]
    fn explicit_daemon_launch_mode_overrides_config_only_when_requested() {
        assert!(DaemonLaunchMode::Config.should_launch(true));
        assert!(!DaemonLaunchMode::Config.should_launch(false));
        assert!(DaemonLaunchMode::Force.should_launch(false));
        assert!(!DaemonLaunchMode::Suppress.should_launch(true));
    }
}
