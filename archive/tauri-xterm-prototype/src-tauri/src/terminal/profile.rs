use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};

use portable_pty::CommandBuilder;

use super::{TerminalError, TerminalProfile};

const POWERSHELL_ID: &str = "powershell";
const CMD_ID: &str = "cmd";
const WSL_ID: &str = "wsl";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuiltinProfileId {
    PowerShell,
    Cmd,
    Wsl,
}

impl BuiltinProfileId {
    const ALL: [Self; 3] = [Self::PowerShell, Self::Cmd, Self::Wsl];

    pub(crate) fn parse(value: &str) -> Result<Self, TerminalError> {
        match value {
            POWERSHELL_ID => Ok(Self::PowerShell),
            CMD_ID => Ok(Self::Cmd),
            WSL_ID => Ok(Self::Wsl),
            _ => Err(TerminalError::unknown_profile(value)),
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::PowerShell => POWERSHELL_ID,
            Self::Cmd => CMD_ID,
            Self::Wsl => WSL_ID,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::PowerShell => "PowerShell",
            Self::Cmd => "Command Prompt",
            Self::Wsl => "WSL",
        }
    }

    fn arguments(self) -> &'static [&'static str] {
        match self {
            Self::PowerShell => &["-NoLogo"],
            Self::Cmd | Self::Wsl => &[],
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::PowerShell => "powershell",
            Self::Cmd => "terminal-cmd",
            Self::Wsl => "linux",
        }
    }

    fn program(self) -> PathBuf {
        let system_root = windows_system_root();
        match self {
            Self::PowerShell => system_root
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe"),
            Self::Cmd => system_root.join("System32").join("cmd.exe"),
            Self::Wsl => system_root.join("System32").join("wsl.exe"),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProfileSpec {
    pub(crate) id: BuiltinProfileId,
    pub(crate) program: PathBuf,
    pub(crate) arguments: Vec<String>,
    pub(crate) default_directory: Option<PathBuf>,
    pub(crate) environment: BTreeMap<String, String>,
}

impl ProfileSpec {
    pub(crate) fn from_id(id: BuiltinProfileId) -> Self {
        Self {
            id,
            program: id.program(),
            arguments: id
                .arguments()
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            default_directory: default_directory(),
            environment: BTreeMap::new(),
        }
    }

    pub(crate) fn ensure_available(&self) -> Result<(), TerminalError> {
        if self.program.is_file() {
            Ok(())
        } else {
            Err(TerminalError::profile_unavailable(self.id.as_str()))
        }
    }

    pub(crate) fn command(&self, cwd: Option<&Path>) -> CommandBuilder {
        let mut command = CommandBuilder::new(&self.program);
        command.args(&self.arguments);

        if let Some(cwd) = cwd {
            command.cwd(cwd);
        }

        for (key, value) in &self.environment {
            command.env(key, value);
        }

        command
    }

    fn public_profile(&self) -> TerminalProfile {
        TerminalProfile {
            id: self.id.as_str().to_owned(),
            name: self.id.name().to_owned(),
            program: self.program.to_string_lossy().into_owned(),
            arguments: self.arguments.clone(),
            default_directory: self
                .default_directory
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            environment: self.environment.clone(),
            icon: self.id.icon().to_owned(),
            theme: "system".to_owned(),
            cursor_style: "block".to_owned(),
            is_default: self.id == BuiltinProfileId::PowerShell,
            available: self.program.is_file(),
        }
    }
}

pub(crate) fn list_profiles() -> Vec<TerminalProfile> {
    BuiltinProfileId::ALL
        .into_iter()
        .map(ProfileSpec::from_id)
        .map(|profile| profile.public_profile())
        .collect()
}

pub(crate) fn resolve_working_directory(
    requested: Option<&Path>,
    profile: &ProfileSpec,
) -> Option<PathBuf> {
    requested
        .filter(|path| path.is_dir())
        .map(Path::to_path_buf)
        .or_else(|| {
            profile
                .default_directory
                .as_ref()
                .filter(|path| path.is_dir())
                .cloned()
        })
}

fn windows_system_root() -> PathBuf {
    env::var_os("SystemRoot")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
}

fn default_directory() -> Option<PathBuf> {
    env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .or_else(|| env::current_dir().ok().filter(|path| path.is_dir()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_ids_are_an_exact_allowlist() {
        assert_eq!(
            BuiltinProfileId::parse("powershell").expect("built-in profile"),
            BuiltinProfileId::PowerShell
        );
        assert_eq!(
            BuiltinProfileId::parse("cmd").expect("built-in profile"),
            BuiltinProfileId::Cmd
        );
        assert_eq!(
            BuiltinProfileId::parse("wsl").expect("built-in profile"),
            BuiltinProfileId::Wsl
        );

        for unsafe_id in [
            "PowerShell",
            " powershell",
            "powershell.exe",
            r"C:\Windows\System32\cmd.exe",
            "cmd /c whoami",
            "../cmd",
            "",
        ] {
            let error = BuiltinProfileId::parse(unsafe_id)
                .expect_err("non-canonical profile IDs must be rejected");
            assert_eq!(error.code, "INVALID_PROFILE");
        }
    }

    #[test]
    fn built_in_profiles_expose_stable_ids_without_accepting_program_input() {
        let profiles = list_profiles();
        let ids = profiles
            .iter()
            .map(|profile| profile.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["powershell", "cmd", "wsl"]);
        assert!(profiles[0].is_default);
        assert!(profiles.iter().all(|profile| !profile.program.is_empty()));
    }

    #[test]
    fn valid_requested_directory_wins_over_profile_default() {
        let profile = ProfileSpec::from_id(BuiltinProfileId::PowerShell);
        let current = env::current_dir().expect("the test process should have a cwd");

        assert_eq!(
            resolve_working_directory(Some(&current), &profile),
            Some(current)
        );
    }
}
