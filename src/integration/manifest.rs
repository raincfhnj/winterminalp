use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{AppError, AppResult, TerminalChannel};

use super::INTEGRATION_SCHEMA_VERSION;
use super::transaction::{atomic_replace, read_optional_snapshot};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct IntegrationManifest {
    pub schema_version: u32,
    pub fragment: Option<FragmentManifest>,
    pub targets: Vec<TargetManifest>,
}

impl Default for IntegrationManifest {
    fn default() -> Self {
        Self {
            schema_version: INTEGRATION_SCHEMA_VERSION,
            fragment: None,
            targets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FragmentManifest {
    pub path: PathBuf,
    pub installed_sha256: String,
    pub backup: Option<BackupManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TargetManifest {
    pub channel: TerminalChannel,
    pub settings_path: PathBuf,
    pub installed_sha256: String,
    pub backup: BackupManifest,
    pub managed_keybindings: Vec<ManagedKeybindingManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct BackupManifest {
    pub source_path: PathBuf,
    pub backup_path: PathBuf,
    pub sha256: String,
    pub byte_len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ManagedKeybindingManifest {
    pub canonical_id: String,
    pub canonical_chord: String,
    pub definition: Value,
}

pub(crate) struct LoadedManifest {
    pub manifest: IntegrationManifest,
    pub sha256: Option<String>,
}

pub(crate) fn load_manifest(path: &Path) -> AppResult<LoadedManifest> {
    let Some(snapshot) = read_optional_snapshot(path)? else {
        return Ok(LoadedManifest {
            manifest: IntegrationManifest::default(),
            sha256: None,
        });
    };
    let manifest: IntegrationManifest =
        serde_json::from_slice(&snapshot.bytes).map_err(|error| AppError::Settings {
            path: path.to_path_buf(),
            message: format!("integration manifest is invalid: {error}"),
        })?;
    if manifest.schema_version != INTEGRATION_SCHEMA_VERSION {
        return Err(AppError::Settings {
            path: path.to_path_buf(),
            message: format!(
                "unsupported manifest schema {}; expected {}",
                manifest.schema_version, INTEGRATION_SCHEMA_VERSION
            ),
        });
    }
    Ok(LoadedManifest {
        manifest,
        sha256: Some(snapshot.sha256),
    })
}

pub(crate) fn save_manifest(
    path: &Path,
    expected_sha256: Option<&str>,
    manifest: &IntegrationManifest,
) -> AppResult<String> {
    let mut bytes = serde_json::to_vec_pretty(manifest).map_err(|error| {
        AppError::InvalidConfiguration(format!("serialize integration manifest: {error}"))
    })?;
    bytes.push(b'\n');
    atomic_replace(path, expected_sha256, &bytes)
}

pub(crate) fn remove_manifest_if_unchanged(
    path: &Path,
    expected_sha256: Option<&str>,
) -> AppResult<bool> {
    let Some(expected_sha256) = expected_sha256 else {
        return Ok(false);
    };
    let Some(snapshot) = read_optional_snapshot(path)? else {
        return Ok(false);
    };
    if snapshot.sha256 != expected_sha256 {
        return Err(AppError::SettingsConflict(format!(
            "{} changed during uninstall; it was retained",
            path.display()
        )));
    }
    fs::remove_file(path)
        .map_err(|error| AppError::io("remove integration manifest", path, error))?;
    Ok(true)
}
