use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::TerminalChannel;

pub const INTEGRATION_SCHEMA_VERSION: u32 = 1;
pub const MINIMUM_FRAGMENT_VERSION: &str = "1.21";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationConfig {
    pub local_app_data: PathBuf,
    pub state_dir: PathBuf,
    pub target_mode: TargetMode,
}

impl IntegrationConfig {
    #[must_use]
    pub fn new(local_app_data: impl Into<PathBuf>, state_dir: impl Into<PathBuf>) -> Self {
        Self {
            local_app_data: local_app_data.into(),
            state_dir: state_dir.into(),
            target_mode: TargetMode::Auto,
        }
    }

    pub fn from_environment() -> crate::AppResult<Self> {
        let local_app_data = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
            crate::AppError::InvalidConfiguration(
                "LOCALAPPDATA is required for Windows Terminal integration".to_owned(),
            )
        })?;
        let local_app_data = PathBuf::from(local_app_data);
        Ok(Self::new(
            &local_app_data,
            local_app_data.join("WinTerminalPP").join("integration"),
        ))
    }

    #[must_use]
    pub fn with_target_mode(mut self, target_mode: TargetMode) -> Self {
        self.target_mode = target_mode;
        self
    }

    #[must_use]
    pub fn fragment_path(&self) -> PathBuf {
        self.local_app_data
            .join("Microsoft")
            .join("Windows Terminal")
            .join("Fragments")
            .join("WinTerminalPP")
            .join("actions.json")
    }

    #[must_use]
    pub fn manifest_path(&self) -> PathBuf {
        self.state_dir.join("manifest.json")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetMode {
    /// Discover only channels whose settings file has already been initialized.
    Auto,
    /// Operate only on the explicitly supplied settings files.
    Explicit(Vec<TerminalSettingsTarget>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSettingsTarget {
    pub channel: TerminalChannel,
    pub settings_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeStatus {
    Create,
    Update,
    Unchanged,
    Conflict,
    Removed,
    Preserved,
    Missing,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictKind {
    SameIdDifferentBinding,
    SameChordDifferentBinding,
    MalformedKeybinding,
    InvalidSettingsShape,
    UnmanagedFragment,
    ConcurrentModification,
    InvalidManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationConflict {
    pub kind: ConflictKind,
    pub action_id: Option<String>,
    pub keys: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FragmentReport {
    pub path: PathBuf,
    pub status: ChangeStatus,
    pub current_sha256: Option<String>,
    pub desired_sha256: String,
    pub minimum_terminal_version: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetPlan {
    pub channel: TerminalChannel,
    pub settings_path: PathBuf,
    pub status: ChangeStatus,
    pub existing_binding_count: usize,
    pub bindings_to_add: usize,
    pub managed_binding_count: usize,
    pub conflicts: Vec<IntegrationConflict>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanReport {
    pub schema_version: u32,
    pub can_install: bool,
    pub fragment: FragmentReport,
    pub targets: Vec<TargetPlan>,
    pub manifest_path: PathBuf,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetInstallReport {
    pub channel: TerminalChannel,
    pub settings_path: PathBuf,
    pub status: ChangeStatus,
    pub added_binding_count: usize,
    pub backup_path: Option<PathBuf>,
    pub before_sha256: String,
    pub after_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    pub schema_version: u32,
    pub fragment: FragmentReport,
    pub targets: Vec<TargetInstallReport>,
    pub manifest_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetUninstallReport {
    pub channel: TerminalChannel,
    pub settings_path: PathBuf,
    pub status: ChangeStatus,
    pub removed_binding_count: usize,
    pub preserved_binding_count: usize,
    pub backup_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallReport {
    pub schema_version: u32,
    pub fragment_status: ChangeStatus,
    pub targets: Vec<TargetUninstallReport>,
    pub manifest_path: PathBuf,
    pub manifest_retained: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorTargetReport {
    pub channel: TerminalChannel,
    pub settings_path: PathBuf,
    pub initialized: bool,
    pub readable: bool,
    pub valid_jsonc: bool,
    pub managed_binding_count: usize,
    pub conflicts: Vec<IntegrationConflict>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub schema_version: u32,
    pub healthy: bool,
    pub fragment: FragmentReport,
    pub targets: Vec<DoctorTargetReport>,
    pub manifest_path: PathBuf,
    pub manifest_valid: bool,
    pub issues: Vec<String>,
}
