//! Managed PowerShell shell integration.
//!
//! Windows Terminal only inherits the working directory of a duplicated pane
//! when the shell reports it through the `OSC 9;9` sequence. WinTerminal++
//! therefore appends a small, reversible prompt wrapper to the PowerShell
//! profiles so that `splitPane` with `splitMode: duplicate` starts in the same
//! directory as the focused pane.

use std::path::{Path, PathBuf};

use crate::{AppError, AppResult};

use super::transaction::{atomic_replace, create_backup, read_optional_snapshot};
use super::types::{ChangeStatus, IntegrationConfig, ShellIntegrationReport, ShellKind};

const BEGIN_MARKER: &str = "# >>> WinTerminalPP shell integration >>>";
const END_MARKER: &str = "# <<< WinTerminalPP shell integration <<<";

const SNIPPET_BODY: &str = r#"# Managed by WinTerminal++. Run `wter uninstall` to remove this block.
if (-not $Global:__WinTerminalPPPromptWrapped) {
    $Global:__WinTerminalPPPromptWrapped = $true
    $Global:__WinTerminalPPOriginalPrompt = $function:prompt
    function global:prompt {
        $__wtppLocation = $ExecutionContext.SessionState.Path.CurrentLocation
        $__wtppOsc = "$([char]27)]9;9;`"$__wtppLocation`"$([char]7)"
        $__wtppBase = if ($Global:__WinTerminalPPOriginalPrompt) {
            & $Global:__WinTerminalPPOriginalPrompt
        }
        else {
            "PS $__wtppLocation> "
        }
        if ($__wtppBase -is [System.Array]) {
            return @($__wtppOsc) + @($__wtppBase)
        }
        return $__wtppOsc + [string]$__wtppBase
    }
}"#;

fn managed_block(newline: &str) -> String {
    let body = SNIPPET_BODY.replace('\n', newline);
    format!("{BEGIN_MARKER}{newline}{body}{newline}{END_MARKER}")
}

fn profile_targets(config: &IntegrationConfig) -> [(ShellKind, PathBuf); 2] {
    [
        (
            ShellKind::WindowsPowerShell,
            config
                .documents_dir
                .join("WindowsPowerShell")
                .join("Microsoft.PowerShell_profile.ps1"),
        ),
        (
            ShellKind::PowerShell,
            config
                .documents_dir
                .join("PowerShell")
                .join("Microsoft.PowerShell_profile.ps1"),
        ),
    ]
}

/// Plans both profiles. Per-profile failures are reported as conflicts instead
/// of aborting the whole command, so a broken profile never blocks the
/// Terminal action bridge.
pub(crate) fn plan(config: &IntegrationConfig) -> Vec<ShellIntegrationReport> {
    profile_targets(config)
        .into_iter()
        .map(|(shell, path)| plan_profile(shell, &path))
        .collect()
}

pub(crate) fn install(config: &IntegrationConfig) -> Vec<ShellIntegrationReport> {
    profile_targets(config)
        .into_iter()
        .map(|(shell, path)| install_profile(config, shell, &path))
        .collect()
}

pub(crate) fn uninstall(config: &IntegrationConfig) -> Vec<ShellIntegrationReport> {
    profile_targets(config)
        .into_iter()
        .map(|(shell, path)| uninstall_profile(config, shell, &path))
        .collect()
}

fn plan_profile(shell: ShellKind, path: &Path) -> ShellIntegrationReport {
    plan_profile_inner(shell, path).unwrap_or_else(|error| shell_error(shell, path, error))
}

fn plan_profile_inner(shell: ShellKind, path: &Path) -> AppResult<ShellIntegrationReport> {
    let Some(snapshot) = read_optional_snapshot(path)? else {
        return Ok(report(
            shell,
            path,
            skipped_or(ChangeStatus::Create, path),
            None,
            None,
        ));
    };
    let (_, text) = decode_profile(path, &snapshot.bytes)?;
    let desired = managed_block(detect_newline(&text));
    Ok(match block_state(&text) {
        BlockState::Absent => report(shell, path, ChangeStatus::Update, None, None),
        BlockState::Present { start, end } => {
            if normalized(&text[start..end]) == normalized(&desired) {
                report(shell, path, ChangeStatus::Unchanged, None, None)
            } else {
                report(shell, path, ChangeStatus::Update, None, None)
            }
        }
        BlockState::Malformed => report(
            shell,
            path,
            ChangeStatus::Conflict,
            None,
            Some("profile contains an incomplete WinTerminalPP marker block".to_owned()),
        ),
    })
}

fn install_profile(
    config: &IntegrationConfig,
    shell: ShellKind,
    path: &Path,
) -> ShellIntegrationReport {
    install_profile_inner(config, shell, path)
        .unwrap_or_else(|error| shell_error(shell, path, error))
}

fn install_profile_inner(
    config: &IntegrationConfig,
    shell: ShellKind,
    path: &Path,
) -> AppResult<ShellIntegrationReport> {
    let Some(snapshot) = read_optional_snapshot(path)? else {
        if !path.parent().is_some_and(Path::is_dir) {
            return Ok(report(
                shell,
                path,
                ChangeStatus::Skipped,
                None,
                Some("profile folder is not present".to_owned()),
            ));
        }
        let block = managed_block("\r\n");
        let mut bytes = block.into_bytes();
        bytes.push(b'\r');
        bytes.push(b'\n');
        atomic_replace(path, None, &bytes)?;
        return Ok(report(shell, path, ChangeStatus::Create, None, None));
    };

    let (encoding, text) = decode_profile(path, &snapshot.bytes)?;
    let newline = detect_newline(&text);
    let desired = managed_block(newline);
    match block_state(&text) {
        BlockState::Malformed => Ok(report(
            shell,
            path,
            ChangeStatus::Conflict,
            None,
            Some("profile contains an incomplete WinTerminalPP marker block".to_owned()),
        )),
        BlockState::Present { start, end } => {
            if normalized(&text[start..end]) == normalized(&desired) {
                return Ok(report(shell, path, ChangeStatus::Unchanged, None, None));
            }
            let backup = create_backup(&config.state_dir, "shell", path, &snapshot)?;
            let mut updated = String::with_capacity(text.len());
            updated.push_str(&text[..start]);
            updated.push_str(&desired);
            updated.push_str(&text[end..]);
            write_profile(path, encoding, &updated, Some(&snapshot.sha256))?;
            Ok(report(
                shell,
                path,
                ChangeStatus::Update,
                Some(backup.backup_path),
                None,
            ))
        }
        BlockState::Absent => {
            let backup = create_backup(&config.state_dir, "shell", path, &snapshot)?;
            let mut updated = text.clone();
            if !updated.is_empty() {
                if !updated.ends_with('\n') {
                    updated.push_str(newline);
                }
                updated.push_str(newline);
            }
            updated.push_str(&desired);
            updated.push_str(newline);
            write_profile(path, encoding, &updated, Some(&snapshot.sha256))?;
            Ok(report(
                shell,
                path,
                ChangeStatus::Update,
                Some(backup.backup_path),
                None,
            ))
        }
    }
}

fn uninstall_profile(
    config: &IntegrationConfig,
    shell: ShellKind,
    path: &Path,
) -> ShellIntegrationReport {
    uninstall_profile_inner(config, shell, path)
        .unwrap_or_else(|error| shell_error(shell, path, error))
}

fn uninstall_profile_inner(
    config: &IntegrationConfig,
    shell: ShellKind,
    path: &Path,
) -> AppResult<ShellIntegrationReport> {
    let Some(snapshot) = read_optional_snapshot(path)? else {
        return Ok(report(
            shell,
            path,
            ChangeStatus::Missing,
            None,
            Some("profile is not present".to_owned()),
        ));
    };
    let (encoding, text) = decode_profile(path, &snapshot.bytes)?;
    match block_state(&text) {
        BlockState::Absent => Ok(report(
            shell,
            path,
            ChangeStatus::Missing,
            None,
            Some("managed block is not present".to_owned()),
        )),
        BlockState::Malformed => Ok(report(
            shell,
            path,
            ChangeStatus::Conflict,
            None,
            Some("profile contains an incomplete WinTerminalPP marker block".to_owned()),
        )),
        BlockState::Present { start, end } => {
            let desired = managed_block(detect_newline(&text));
            if normalized(&text[start..end]) != normalized(&desired) {
                return Ok(report(
                    shell,
                    path,
                    ChangeStatus::Preserved,
                    None,
                    Some("managed block was edited and was left in place".to_owned()),
                ));
            }
            let backup = create_backup(&config.state_dir, "shell", path, &snapshot)?;
            let mut updated = String::with_capacity(text.len());
            updated.push_str(&text[..start]);
            updated.push_str(&text[end..]);
            write_profile(path, encoding, &updated, Some(&snapshot.sha256))?;
            Ok(report(
                shell,
                path,
                ChangeStatus::Removed,
                Some(backup.backup_path),
                None,
            ))
        }
    }
}

fn report(
    shell: ShellKind,
    path: &Path,
    status: ChangeStatus,
    backup_path: Option<PathBuf>,
    message: Option<String>,
) -> ShellIntegrationReport {
    ShellIntegrationReport {
        shell,
        path: path.to_path_buf(),
        status,
        backup_path,
        message,
    }
}

fn shell_error(shell: ShellKind, path: &Path, error: AppError) -> ShellIntegrationReport {
    report(
        shell,
        path,
        ChangeStatus::Conflict,
        None,
        Some(error.to_string()),
    )
}

fn skipped_or(status: ChangeStatus, path: &Path) -> ChangeStatus {
    if path.parent().is_some_and(Path::is_dir) {
        status
    } else {
        ChangeStatus::Skipped
    }
}

/// Encoding of an existing profile. The managed block is written back in the
/// same encoding so non-ASCII profile content is never corrupted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfileEncoding {
    Utf8 { bom: bool },
    Utf16Le,
}

fn decode_profile(path: &Path, bytes: &[u8]) -> AppResult<(ProfileEncoding, String)> {
    if let Some(content) = bytes.strip_prefix(b"\xef\xbb\xbf") {
        return Ok((
            ProfileEncoding::Utf8 { bom: true },
            decode_utf8(path, content)?,
        ));
    }
    if let Some(content) = bytes.strip_prefix(b"\xff\xfe") {
        if content.len() % 2 != 0 {
            return Err(AppError::Settings {
                path: path.to_path_buf(),
                message: "PowerShell profile has a truncated UTF-16LE byte sequence".to_owned(),
            });
        }
        let units = content
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        let text = String::from_utf16(&units).map_err(|_| AppError::Settings {
            path: path.to_path_buf(),
            message: "PowerShell profile is not valid UTF-16LE".to_owned(),
        })?;
        return Ok((ProfileEncoding::Utf16Le, text));
    }
    Ok((
        ProfileEncoding::Utf8 { bom: false },
        decode_utf8(path, bytes)?,
    ))
}

fn decode_utf8(path: &Path, content: &[u8]) -> AppResult<String> {
    std::str::from_utf8(content)
        .map(str::to_owned)
        .map_err(|_| AppError::Settings {
            path: path.to_path_buf(),
            message: "PowerShell profile is neither UTF-8 nor UTF-16LE".to_owned(),
        })
}

fn write_profile(
    path: &Path,
    encoding: ProfileEncoding,
    text: &str,
    expected_sha256: Option<&str>,
) -> AppResult<()> {
    let bytes = match encoding {
        ProfileEncoding::Utf8 { bom } => {
            let mut bytes = Vec::with_capacity(text.len() + usize::from(bom) * 3);
            if bom {
                bytes.extend_from_slice(b"\xef\xbb\xbf");
            }
            bytes.extend_from_slice(text.as_bytes());
            bytes
        }
        ProfileEncoding::Utf16Le => {
            let mut bytes = Vec::with_capacity(text.len() * 2 + 2);
            bytes.extend_from_slice(b"\xff\xfe");
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes
        }
    };
    atomic_replace(path, expected_sha256, &bytes)?;
    Ok(())
}

fn detect_newline(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

fn normalized(text: &str) -> String {
    text.replace("\r\n", "\n")
}

enum BlockState {
    Absent,
    Present { start: usize, end: usize },
    Malformed,
}

fn block_state(text: &str) -> BlockState {
    match (text.find(BEGIN_MARKER), text.find(END_MARKER)) {
        (None, None) => BlockState::Absent,
        (Some(begin), Some(end)) if end >= begin => BlockState::Present {
            start: begin,
            end: end + END_MARKER.len(),
        },
        _ => BlockState::Malformed,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn config_with(root: &Path) -> IntegrationConfig {
        IntegrationConfig::new(root, root.join("state"), root.join("Documents"))
    }

    #[test]
    fn install_appends_a_single_managed_block_and_is_idempotent() {
        let temp = tempfile::tempdir().expect("temporary directory should be created");
        let config = config_with(temp.path());
        let profile_dir = config.documents_dir.join("WindowsPowerShell");
        fs::create_dir_all(&profile_dir).expect("profile directory should be created");
        let profile = profile_dir.join("Microsoft.PowerShell_profile.ps1");
        fs::write(&profile, "Set-Alias ll Get-ChildItem\n")
            .expect("user profile should be written");

        let first = install(&config);
        let windows = first
            .iter()
            .find(|entry| entry.shell == ShellKind::WindowsPowerShell)
            .expect("windows powershell target");
        assert_eq!(windows.status, ChangeStatus::Update);

        let text = fs::read_to_string(&profile).expect("profile should be readable");
        assert_eq!(text.matches(BEGIN_MARKER).count(), 1);
        assert_eq!(text.matches(END_MARKER).count(), 1);
        assert!(text.contains("]9;9;"));
        assert!(text.contains("Set-Alias ll Get-ChildItem"));

        let second = install(&config);
        let windows = second
            .iter()
            .find(|entry| entry.shell == ShellKind::WindowsPowerShell)
            .expect("windows powershell target");
        assert_eq!(windows.status, ChangeStatus::Unchanged);
    }

    #[test]
    fn uninstall_removes_only_the_managed_block() {
        let temp = tempfile::tempdir().expect("temporary directory should be created");
        let config = config_with(temp.path());
        let profile_dir = config.documents_dir.join("WindowsPowerShell");
        fs::create_dir_all(&profile_dir).expect("profile directory should be created");
        let profile = profile_dir.join("Microsoft.PowerShell_profile.ps1");
        fs::write(&profile, "Set-Alias ll Get-ChildItem\n")
            .expect("user profile should be written");
        install(&config);

        let report = uninstall(&config);
        let windows = report
            .iter()
            .find(|entry| entry.shell == ShellKind::WindowsPowerShell)
            .expect("windows powershell target");
        assert_eq!(windows.status, ChangeStatus::Removed);

        let text = fs::read_to_string(&profile).expect("profile should be readable");
        assert!(!text.contains(BEGIN_MARKER));
        assert!(text.contains("Set-Alias ll Get-ChildItem"));
    }

    #[test]
    fn uninstall_preserves_a_user_edited_block() {
        let temp = tempfile::tempdir().expect("temporary directory should be created");
        let config = config_with(temp.path());
        let profile_dir = config.documents_dir.join("WindowsPowerShell");
        fs::create_dir_all(&profile_dir).expect("profile directory should be created");
        let profile = profile_dir.join("Microsoft.PowerShell_profile.ps1");
        fs::write(&profile, "").expect("empty profile should be written");
        install(&config);
        let edited = fs::read_to_string(&profile)
            .expect("profile should be readable")
            .replace("PS $__wtppLocation> ", "PS> ");
        fs::write(&profile, edited).expect("edited profile should be written");

        let report = uninstall(&config);
        let windows = report
            .iter()
            .find(|entry| entry.shell == ShellKind::WindowsPowerShell)
            .expect("windows powershell target");
        assert_eq!(windows.status, ChangeStatus::Preserved);
        assert!(
            fs::read_to_string(&profile)
                .expect("profile should remain readable")
                .contains(BEGIN_MARKER)
        );
    }

    #[test]
    fn install_skips_a_shell_without_a_profile_folder() {
        let temp = tempfile::tempdir().expect("temporary directory should be created");
        let config = config_with(temp.path());
        fs::create_dir_all(config.documents_dir.join("WindowsPowerShell"))
            .expect("profile directory should be created");

        let report = install(&config);
        let powershell = report
            .iter()
            .find(|entry| entry.shell == ShellKind::PowerShell)
            .expect("powershell target");
        assert_eq!(powershell.status, ChangeStatus::Skipped);
    }

    #[test]
    fn utf16le_profile_is_preserved_and_written_back_as_utf16le() {
        let temp = tempfile::tempdir().expect("temporary directory should be created");
        let config = config_with(temp.path());
        let profile_dir = config.documents_dir.join("WindowsPowerShell");
        fs::create_dir_all(&profile_dir).expect("profile directory should be created");
        let profile = profile_dir.join("Microsoft.PowerShell_profile.ps1");
        let original = "Set-Alias é Get-ChildItem\r\n";
        let mut bytes = vec![0xff, 0xfe];
        for unit in original.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        fs::write(&profile, &bytes).expect("utf16 profile should be written");

        let report = install(&config);
        let windows = report
            .iter()
            .find(|entry| entry.shell == ShellKind::WindowsPowerShell)
            .expect("windows powershell target");
        assert_eq!(windows.status, ChangeStatus::Update);

        let written = fs::read(&profile).expect("profile should be readable");
        assert_eq!(&written[..2], &[0xff, 0xfe]);
        let text = String::from_utf16(
            &written[2..]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
        )
        .expect("utf16 profile should decode");
        assert!(text.contains("Set-Alias é Get-ChildItem"));
        assert!(text.contains("]9;9;"));
    }

    #[test]
    fn unsupported_profile_encoding_is_reported_without_blocking() {
        let temp = tempfile::tempdir().expect("temporary directory should be created");
        let config = config_with(temp.path());
        let profile_dir = config.documents_dir.join("WindowsPowerShell");
        fs::create_dir_all(&profile_dir).expect("profile directory should be created");
        let profile = profile_dir.join("Microsoft.PowerShell_profile.ps1");
        fs::write(&profile, b"\x81\x8d invalid windows-1252").expect("profile should be written");

        let report = install(&config);
        let windows = report
            .iter()
            .find(|entry| entry.shell == ShellKind::WindowsPowerShell)
            .expect("windows powershell target");
        assert_eq!(windows.status, ChangeStatus::Conflict);
        assert_eq!(
            fs::read(&profile).expect("profile should remain readable"),
            b"\x81\x8d invalid windows-1252"
        );
    }
}
